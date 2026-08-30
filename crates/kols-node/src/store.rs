//! What survives between invocations.
//!
//! # Why a directory of files rather than a database
//!
//! `design/05` §5 puts a SQLite projection in the desktop client and is explicit
//! that it is a projection, never the source of truth — it can be deleted and
//! rebuilt from the network. This CLI holds the *sources*: the master seed,
//! which exists nowhere else and cannot be rebuilt from anything, and the
//! governance log, which can. Those are two files and a directory of entries, so
//! a database would be machinery around three `read`s.
//!
//! # The seed is the one irreplaceable thing here
//!
//! Losing it loses every identity in every network, with no recovery service to
//! fall back on (`design/02` §6.3). It is written `0600` and never printed.
//! Key types in `intranet-*` deliberately implement no serialization, which is
//! why what is stored is the 32 bytes of entropy the seed is *derived from*
//! rather than the seed itself — the derivation is reproducible and the key
//! material never crosses a `Debug` or a serializer.

use intranet_crypto::{Hash, to_hex};
use intranet_governance::{GovernanceLog, GovernanceState, LogEntry, PointerId, wire};
use intranet_identity::{MasterSeed, NetworkId, PerNetworkIdentity, PerNetworkIdentityId};
use intranet_storage::{Cid, Dek, EpochKey};
use crate::secret;
use kols_core::{ChannelId, Record};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Everything one network's membership needs on disk.
pub struct Store {
    root: PathBuf,
    entropy: [u8; 32],
    network: NetworkId,
}

/// What can go wrong reading or writing the store.
#[derive(Debug)]
pub enum StoreError {
    /// The filesystem refused.
    Io(io::Error),
    /// No network has been created or joined here yet.
    NotInitialised(PathBuf),
    /// A network already exists here, and this would have overwritten it.
    AlreadyInitialised(PathBuf),
    /// A stored file was not the shape this build expects.
    Corrupt(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::NotInitialised(path) => write!(
                f,
                "no network at {}. Run `kols init <name>` to create one, or `kols join <invite>`",
                path.display()
            ),
            Self::AlreadyInitialised(path) => write!(
                f,
                "a network already exists at {}. Refusing to overwrite it — the seed there \
                 cannot be recovered if it is lost",
                path.display()
            ),
            Self::Corrupt(what) => write!(f, "stored state is unreadable: {what}"),
        }
    }
}

impl From<io::Error> for StoreError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl Store {
    /// Where state lives: `$KOLS_HOME`, else `~/.kols`.
    ///
    /// **Home is not one variable.** `HOME` is the Unix answer and is normally
    /// unset on Windows, where the profile is `USERPROFILE` — so reading only
    /// `HOME` there does not fail, it silently succeeds with the wrong answer:
    /// the fallback puts `.kols` in the *current directory*, and a client whose
    /// store follows you around is one that appears to lose a network whenever
    /// you run it from somewhere else. Found the first time `kols.exe` was run.
    pub fn default_root() -> PathBuf {
        if let Ok(explicit) = std::env::var("KOLS_HOME") {
            return PathBuf::from(explicit);
        }
        let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".kols")
    }

    /// Creates a store, generating a seed if there is not one already.
    ///
    /// Refuses to overwrite an existing network. The seed is the one thing here
    /// that no amount of syncing can rebuild, so clobbering it silently would be
    /// the single most destructive thing this program could do.
    pub fn create(root: PathBuf, network: NetworkId, entropy: [u8; 32]) -> Result<Self, StoreError> {
        if root.join("network").exists() {
            return Err(StoreError::AlreadyInitialised(root));
        }
        fs::create_dir_all(root.join("entries"))?;
        secret::write_private(&root.join("seed"), &entropy)?;
        fs::write(root.join("network"), network.as_bytes())?;
        Ok(Self {
            root,
            entropy,
            network,
        })
    }

    /// Opens an existing store.
    pub fn open(root: PathBuf) -> Result<Self, StoreError> {
        if !root.join("network").exists() {
            return Err(StoreError::NotInitialised(root));
        }
        let entropy = fixed(&fs::read(root.join("seed"))?, "seed")?;
        let network = NetworkId::from_bytes(fixed(&fs::read(root.join("network"))?, "network id")?);
        Ok(Self {
            root,
            entropy,
            network,
        })
    }

    /// The network this store belongs to.
    pub const fn network(&self) -> &NetworkId {
        &self.network
    }

    /// Where this store lives.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// This member's identity in this network.
    ///
    /// Derived per network by construction (Core §1.2), so the same seed in two
    /// networks yields two identities nobody can correlate.
    pub fn identity(&self) -> Result<PerNetworkIdentity, StoreError> {
        MasterSeed::from_entropy(self.entropy)
            .identity_for(&self.network)
            .map_err(|err| StoreError::Corrupt(format!("identity does not derive: {err}")))
    }

    /// Takes the store's append lock, waiting briefly for it.
    ///
    /// # Why this exists
    ///
    /// The store has two writers: one-shot commands and the daemon. Both append
    /// governance entries, and each parents its entry on the head *it* last saw.
    /// Without serialisation they append siblings — a fork, which the protocol
    /// handles correctly and which is nonetheless a disaster here, because
    /// fork-choice then voids one side. It cost a channel: `channel create` and
    /// the daemon's admission rotation landed on the same parent, the rotation
    /// branch won, and the channel simply stopped existing.
    ///
    /// So an append is: take this lock, re-read the head, write, release. The
    /// daemon additionally adopts whatever the store gained before appending, so
    /// its parent is the real head rather than the one it held a tick ago.
    ///
    /// `create_dir` is the primitive because it is atomic on every filesystem
    /// this runs on, and a lock that is only *usually* exclusive is worse than
    /// none — it would fail rarely enough to look like something else.
    pub fn lock(&self) -> Result<AppendLock, StoreError> {
        let path = self.root.join("lock");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match fs::create_dir(&path) {
                Ok(()) => return Ok(AppendLock { path }),
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                    if std::time::Instant::now() > deadline {
                        return Err(StoreError::Corrupt(format!(
                            "another kols process has held {} for ten seconds. If none is \
                             running, remove it",
                            path.display()
                        )));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(err) => return Err(StoreError::Io(err)),
            }
        }
    }

    /// Whether a node is currently running for this store.
    ///
    /// The same freshness rule [`Store::hold_node`] applies, asked without
    /// taking the claim — for a caller that needs to know rather than to hold,
    /// which is anything about to do something a running node would not survive.
    pub fn is_being_served(&self) -> bool {
        claim_is_fresh(&self.root.join("serving").join("heartbeat"))
    }

    /// Claims the right to run a node for this network.
    ///
    /// # Why this is separate from the append lock
    ///
    /// The append lock is held for a moment, around a read-head-then-write. This
    /// is held for as long as a node runs, and it guards something that lock
    /// cannot: **the MLS group is live state only one process can hold**. Two
    /// nodes on one store both restore the group, both advance it, and each
    /// saves a version the other has not seen — after which whichever wrote last
    /// decides what the network's key is, and anybody keyed in by the other is
    /// keyed into an epoch nobody agrees on.
    ///
    /// That has no symptom at the moment it happens, which is why this refuses
    /// up front rather than letting both run.
    ///
    /// # A claim has to expire, because processes do not always get to clean up
    ///
    /// A desktop window is usually closed by the window manager, which is not an
    /// exit that runs destructors — so a claim released only on `Drop` would
    /// leak on the *normal* way this application ends, and the next run would
    /// refuse to start a node for a network nothing is serving.
    ///
    /// So the holder writes a heartbeat and a claim older than
    /// [`NODE_CLAIM_STALE`] is taken over. Not a pid check: liveness is a
    /// different answer on every platform, and pids are reused, so a pid that
    /// looks alive may be somebody else's. A timestamp the holder must keep
    /// refreshing is the same question asked in a way that cannot be wrong for
    /// long.
    ///
    /// The cost is stated rather than hidden: after a crash, the next node waits
    /// out the window before it can start.
    pub fn hold_node(&self) -> Result<NodeClaim, StoreError> {
        let path = self.root.join("serving");
        let beat = path.join("heartbeat");

        // Waits a stale claim out rather than refusing on sight. Restarting a
        // node is ordinary — a window closed and reopened, a daemon stopped and
        // started — and the previous holder rarely got to clean up, so refusing
        // instantly would make the common case look like the failure case.
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(NODE_CLAIM_STALE as u64 + 2_000);
        loop {
            if !claim_is_fresh(&beat) {
                fs::create_dir_all(&path)?;
                let claim = NodeClaim { path };
                claim.beat();
                // Holding the claim is the one moment this process knows no
                // other node is writing to this store, which makes it the only
                // safe place to sweep what an interrupted write left behind.
                sweep_scratch(&self.root);
                return Ok(claim);
            }
            if std::time::Instant::now() > deadline {
                return Err(StoreError::Corrupt(format!(
                    "another kols process is already running a node for this network. \
                     Only one can: the network's key group is live state, and two would each \
                     advance it without seeing the other. If nothing is running, remove {}",
                    path.display()
                )));
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }

    /// Appends an entry to the stored governance log.
    ///
    /// Entries are files named by their position, so replay reads them back in
    /// the order they were accepted — which is what `GovernanceLog::insert`
    /// requires, since it refuses an entry whose parent it has not seen.
    pub fn append_entry(&self, entry: &LogEntry) -> Result<(), StoreError> {
        let dir = self.root.join("entries");
        let next = fs::read_dir(&dir)?.count();
        write_atomically(
            &self.root,
            dir.join(format!("{next:08}")),
            &wire::encode_entry(entry),
        )?;
        Ok(())
    }

    /// Reads the governance log back, ancestors first.
    pub fn log(&self) -> Result<GovernanceLog, StoreError> {
        let dir = self.root.join("entries");
        let mut files: Vec<_> = fs::read_dir(&dir)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect();
        files.sort();

        let mut log = GovernanceLog::new();
        for path in files {
            let bytes = fs::read(&path)?;
            let entry = wire::decode_entry(&bytes)
                .map_err(|err| StoreError::Corrupt(format!("{}: {err}", path.display())))?;
            log.insert(entry)
                .map_err(|err| StoreError::Corrupt(format!("{}: {err}", path.display())))?;
        }
        Ok(log)
    }

    /// Replays the stored log into current state.
    pub fn state(&self) -> Result<GovernanceState, StoreError> {
        let log = self.log()?;
        let chain: Vec<_> = log
            .canonical_chain()
            .iter()
            .filter_map(|hash| log.get(hash))
            .collect();
        GovernanceState::replay(chain)
            .map_err(|err| StoreError::Corrupt(format!("replay refused the stored log: {err}")))
    }

    /// The head of the canonical chain, which a new entry parents onto.
    pub fn head(&self) -> Result<Option<Hash>, StoreError> {
        Ok(self.log()?.canonical_chain().last().copied())
    }

    /// Every record this node holds for a channel, in merge order.
    ///
    /// Keyed by record id, so the same record learned twice — once live, once
    /// out of a fetched segment — is stored once. That is not a deduplication
    /// convenience: `design/01` §7 requires duplicate delivery to be idempotent,
    /// and content-addressing the file name is the cheapest way to mean it.
    ///
    /// Sorted by HLC and then by id, which is the merge order every node
    /// computes (`design/01` §4). Insertion order is deliberately not preserved,
    /// because it differs per node and ordering must not.
    pub fn records(&self, channel: &ChannelId) -> Result<Vec<Record>, StoreError> {
        let dir = self.channel_dir(channel).join("records");
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut records = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            records.push(
                Record::decode(&bytes)
                    .map_err(|err| StoreError::Corrupt(format!("{}: {err}", path.display())))?,
            );
        }
        records.sort_by(|a, b| {
            a.hlc
                .cmp(&b.hlc)
                .then_with(|| a.id().as_bytes().cmp(b.id().as_bytes()))
        });
        Ok(records)
    }

    /// This member's own records in a channel, in the order their log holds them.
    pub fn own_records(
        &self,
        channel: &ChannelId,
        author: &PerNetworkIdentityId,
    ) -> Result<Vec<Record>, StoreError> {
        Ok(self
            .records(channel)?
            .into_iter()
            .filter(|record| &record.author == author)
            .collect())
    }

    /// Stores a record, whoever wrote it.
    ///
    /// Returns whether it was new, so a caller can report what a sync actually
    /// brought in rather than how many records it looked at.
    pub fn put_record(&self, channel: &ChannelId, record: &Record) -> Result<bool, StoreError> {
        let dir = self.channel_dir(channel).join("records");
        fs::create_dir_all(&dir)?;
        let path = dir.join(to_hex(record.id().as_bytes()));
        if path.exists() {
            return Ok(false);
        }
        write_atomically(&self.root, path, &record.canonical_bytes())?;
        Ok(true)
    }

    /// Keeps a chunk this node fetched, so it survives being closed.
    ///
    /// # Why a node has to write these down
    ///
    /// Storage §4.2 makes holding the bytes the whole of swarm membership: a
    /// node is a place a chunk can be got from precisely for as long as it has
    /// the chunk. The transport's [`ChunkStore`](intranet_storage::ChunkStore)
    /// is a `BTreeMap` in memory, which made that membership last exactly as
    /// long as the process — so every close and reopen silently retired a
    /// node's entire contribution.
    ///
    /// Its own content came back anyway, because `publish_own_logs` re-derives
    /// this node's segments from these records at startup and re-announces
    /// them. Nothing did that for anybody else's, and nobody else *can*: a
    /// segment is encrypted under its author's per-segment key and named by the
    /// CID of that ciphertext, so only the author can produce those bytes
    /// again. A member who read a message, closed the app and reopened it could
    /// still see the message and could no longer pass it on, and the network's
    /// durability quietly collapsed to its authors' uptime.
    ///
    /// Named by content, so writing the same chunk twice is a no-op and two
    /// nodes never disagree about what a name holds.
    pub fn put_chunk(&self, cid: &Cid, bytes: &[u8]) -> Result<bool, StoreError> {
        let dir = self.root.join("chunks");
        fs::create_dir_all(&dir)?;
        let path = dir.join(to_hex(cid.hash().as_bytes()));
        if path.exists() {
            return Ok(false);
        }
        write_atomically(&self.root, path, bytes)?;
        Ok(true)
    }

    /// Every chunk this node kept, to be put back and re-announced at startup.
    ///
    /// Addressed by content, so the file name is a hint and the bytes are the
    /// truth — `ChunkStore::insert` re-derives the CID and refuses a mismatch,
    /// which is what makes a corrupted or tampered file a discarded chunk
    /// rather than one this node goes on to serve.
    pub fn chunks(&self) -> Result<Vec<Vec<u8>>, StoreError> {
        let dir = self.root.join("chunks");
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut chunks = Vec::new();
        for entry in fs::read_dir(&dir)? {
            chunks.push(fs::read(entry?.path())?);
        }
        Ok(chunks)
    }

    /// Keeps another member's pointer and the key wrappings that open it.
    ///
    /// Pointers live in memory beside the chunks and are lost the same way, and
    /// losing them is worse: without the pointer a node cannot say *which*
    /// segment is an author's head, so the chunks it kept name nothing it can
    /// find. Both halves have to come back or neither is worth keeping.
    ///
    /// Stored as an encoded [`PointerResponse::Records`] holding this one
    /// record. That type is the wire's, and it is used here because it already
    /// carries exactly a pointer with its wrappings and already verifies every
    /// signature on the way back in — a private on-disk format would be a
    /// second encoding of the same thing, checked less.
    ///
    /// One file per pointer, rewritten as it advances. Not one file for all of
    /// them: `MAX_POINTERS_PER_RESPONSE` caps a response at 256, and a node
    /// holding more than that would write a file it could never read back.
    pub fn put_pointer(
        &self,
        pointer: &intranet_storage::MutablePointer,
        wrappings: Vec<intranet_storage::DekWrapping>,
    ) -> Result<(), StoreError> {
        let dir = self.root.join("pointers");
        fs::create_dir_all(&dir)?;
        let encoded = intranet_storage::PointerResponse::Records {
            records: vec![intranet_storage::PointerRecord {
                pointer: pointer.clone(),
                wrappings,
            }],
            truncated: false,
        }
        .encode();
        write_atomically(
            &self.root,
            dir.join(to_hex(pointer.pointer_id.as_bytes())),
            &encoded,
        )?;
        Ok(())
    }

    /// Every pointer this node kept, with its wrappings.
    ///
    /// A file that will not decode is skipped rather than fatal. Decoding
    /// verifies signatures, so a refusal here means a pointer this node must
    /// not act on — and refusing to start would turn one bad file into a node
    /// that cannot open at all, when the honest consequence is one author's
    /// content being unreachable until it is learned again.
    pub fn pointers(
        &self,
    ) -> Result<Vec<intranet_storage::PointerRecord>, StoreError> {
        let dir = self.root.join("pointers");
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut records = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let Ok(bytes) = fs::read(entry?.path()) else {
                continue;
            };
            if let Ok(intranet_storage::PointerResponse::Records { records: held, .. }) =
                intranet_storage::PointerResponse::decode(&bytes)
            {
                records.extend(held);
            }
        }
        Ok(records)
    }

    /// The channels this node holds any records for.
    pub fn channels_with_records(&self) -> Result<Vec<ChannelId>, StoreError> {
        let dir = self.root.join("channels");
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let name = entry?.file_name();
            let Some(hex) = name.to_str() else { continue };
            let Some(bytes) = intranet_crypto::from_hex(hex) else {
                continue;
            };
            if let Ok(bytes) = <[u8; 32]>::try_from(bytes.as_slice()) {
                out.push(ChannelId::from_bytes(bytes));
            }
        }
        Ok(out)
    }

    /// Saves this node's MLS group state, sealed at rest.
    ///
    /// Core §3.3.1 requires this to survive a restart, and says plainly that the
    /// bytes are secret: they hold the group's secret tree and this member's
    /// signature private key, which together are enough to impersonate them and
    /// read the network. Sealed under the same seed-derived key as the epoch
    /// keys, and written `0600`.
    pub fn set_group_state(&self, state: &[u8]) -> Result<(), StoreError> {
        Ok(secret::write_private(
            &self.root.join("group"),
            &self.at_rest_key().seal_chunk(state),
        )?)
    }

    /// This node's saved MLS group state, if it has one.
    pub fn group_state(&self) -> Result<Option<Vec<u8>>, StoreError> {
        let Ok(sealed) = fs::read(self.root.join("group")) else {
            return Ok(None);
        };
        self.at_rest_key()
            .open_chunk(&sealed)
            .map(Some)
            .map_err(|err| StoreError::Corrupt(format!("group state will not open: {err}")))
    }

    /// A local label for this network.
    ///
    /// Local because spec 07 defines no policy key for a network name, and
    /// inventing vocabulary the normative document does not have is how two
    /// clients end up disagreeing about what a network is called.
    pub fn set_label(&self, name: &str) -> Result<(), StoreError> {
        write_atomically(&self.root, self.root.join("label"), name.as_bytes())?;
        Ok(())
    }

    /// This network's local label, if one was set.
    pub fn label(&self) -> Option<String> {
        fs::read_to_string(self.root.join("label")).ok()
    }

    /// Records the addresses this node is reachable on.
    ///
    /// Written by the daemon because only a running node knows them, and read by
    /// one-shot commands because only they need to hand them out. An invite that
    /// carries no bootstrap address cannot establish a connection, which is the
    /// one job it exists to do — so `kols invite` has to get them from
    /// somewhere, and a node that never wrote them down is that somewhere not
    /// existing.
    ///
    /// Last writer wins, which is right: these change when the daemon restarts
    /// on a new port, and the newest run is the one somebody can actually dial.
    pub fn set_addresses(&self, addresses: &[String]) -> Result<(), StoreError> {
        write_atomically(
            &self.root,
            self.root.join("addresses"),
            addresses.join("\n").as_bytes(),
        )?;
        Ok(())
    }

    /// The addresses the daemon last reported being reachable on.
    ///
    /// Stale by construction — the daemon may not be running, or may be running
    /// somewhere else. A caller handing these to somebody should say when they
    /// were last written rather than implying they are live.
    pub fn addresses(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("addresses"))
            .map(|text| {
                text.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Remembers addresses worth dialling — the ones an invite carried.
    ///
    /// Written by `kols join`, read by `kols serve`. Without this a joiner is
    /// handed everything they need to connect and must then be told an address
    /// by hand anyway, which is the friction the invite exists to remove.
    pub fn set_peers(&self, addresses: &[String]) -> Result<(), StoreError> {
        write_atomically(
            &self.root,
            self.root.join("peers"),
            addresses.join("\n").as_bytes(),
        )?;
        Ok(())
    }

    /// Addresses this node should dial on startup.
    pub fn peers(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("peers"))
            .map(|text| {
                text.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Caches the relays replay last named — Core §5.5.
    ///
    /// The carrier that exists because the others cannot reach a node before it
    /// connects: reading `NetworkPolicy.bootstrap_relays` needs a synced log,
    /// syncing needs a connection, and connecting is what a relay is for. A node
    /// that consulted only replayed state could never use it after a restart.
    pub fn set_relays(&self, relays: &[String]) -> Result<(), StoreError> {
        write_atomically(
            &self.root,
            self.root.join("relays"),
            relays.join("\n").as_bytes(),
        )?;
        Ok(())
    }

    /// The relays this node last knew the network to designate.
    pub fn relays(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("relays"))
            .map(|text| {
                text.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Records who is in the waiting room, as the daemon last saw it.
    ///
    /// The waiting room is live state in the running node (Core §2.4), so a
    /// one-shot command cannot ask it anything. This is the daemon writing down
    /// what it knows so `kols waiting` can read it — stale by construction, and
    /// worth saying so where it is displayed rather than pretending otherwise.
    pub fn set_waiting(&self, identities: &[String]) -> Result<(), StoreError> {
        write_atomically(
            &self.root,
            self.root.join("waiting"),
            identities.join("\n").as_bytes(),
        )?;
        Ok(())
    }

    /// Who this node is connected to, as the daemon last saw it.
    ///
    /// **"Connected to this node", never "online".** There is no routing here:
    /// a member is reachable directly or by hole punch and otherwise not at all
    /// (Core §5.2), and this client dials the peers it has addresses for rather
    /// than every member. So a member missing from this list may be offline,
    /// may be unreachable from here, or may simply be somebody this node has
    /// never had reason to dial — and nothing on this machine can tell those
    /// apart. Anywhere it is displayed has to say which question it answers.
    pub fn set_connected(&self, identities: &[String]) -> Result<(), StoreError> {
        write_atomically(
            &self.root,
            self.root.join("connected"),
            identities.join("\n").as_bytes(),
        )?;
        Ok(())
    }

    /// Who the daemon last saw this node connected to.
    pub fn connected(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("connected"))
            .map(|text| {
                text.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Who the daemon last saw waiting to be admitted.
    pub fn waiting(&self) -> Vec<String> {
        fs::read_to_string(self.root.join("waiting"))
            .map(|text| {
                text.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn channel_dir(&self, channel: &ChannelId) -> PathBuf {
        self.root.join("channels").join(to_hex(channel.as_bytes()))
    }

    /// A key derived from the seed, for sealing this store's own secrets at rest.
    ///
    /// Not a protocol key and never leaves this machine. It exists so nothing
    /// confidential sits on disk in the clear next to the seed that protects it.
    fn at_rest_key(&self) -> Dek {
        Dek::from_bytes(*intranet_crypto::keyed_hash(&self.entropy, b"kols.cli.at-rest.v1").as_bytes())
    }

    /// Records epoch keys this node holds.
    ///
    /// A **set**, not one key, and that is the whole point. Every membership
    /// change rotates the epoch (Core §3.3), so a network accumulates a chain of
    /// keys and a `DekWrapping` names which one it is under (Storage §5.3). A
    /// node that kept only the newest could not open anything wrapped before the
    /// last person joined — which is exactly the bug this replaced, and it
    /// presented as "the fetch works and the content will not decrypt".
    ///
    /// # Storing epoch keys at all is a decision
    ///
    /// `EpochKey::expose_for_delivery` says the only correct use is sealing to
    /// an identity already entitled to the key, and that storing it unsealed
    /// defeats the guarantee. So each is sealed under a key derived from the
    /// master seed — the same thing already protecting the identity they belong
    /// to — and written `0600`. The protocol's own answer for recovering keys is
    /// re-delivery from a peer (Core §3.5); sealing to ourselves is that
    /// operation aimed at the only member certain to be present.
    pub fn set_epoch_keys(
        &self,
        keys: &[(Hash, EpochKey)],
        current: Hash,
    ) -> Result<(), StoreError> {
        let dir = self.root.join("epochs");
        fs::create_dir_all(&dir)?;
        for (rotation, key) in keys {
            let sealed = self.at_rest_key().seal_chunk(key.expose_for_delivery());
            secret::write_private(&dir.join(to_hex(rotation.as_bytes())), &sealed)?;
        }
        write_atomically(&self.root, self.root.join("rotation"), current.as_bytes())?;
        Ok(())
    }

    /// Every epoch key this node holds, by the rotation it belongs to.
    pub fn epoch_keys(&self) -> Result<Vec<(Hash, EpochKey)>, StoreError> {
        let dir = self.root.join("epochs");
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            let Some(rotation) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(intranet_crypto::from_hex)
                .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            else {
                continue;
            };
            let sealed = fs::read(&path)?;
            let bytes = self
                .at_rest_key()
                .open_chunk(&sealed)
                .map_err(|err| StoreError::Corrupt(format!("an epoch key will not open: {err}")))?;
            out.push((
                Hash::from_bytes(rotation),
                EpochKey::from_bytes(fixed(&bytes, "epoch key")?),
            ));
        }

        // Current first, because every caller scans this list until something
        // opens and the current epoch is what a refreshed wrapping is under.
        // Filesystem order made the common case cost a scan of everything a
        // long-lived network had ever rotated through — measured at 0.72ms per
        // thousand keys, paid on every unwrap.
        if let Ok(current) = self.rotation_ref()
            && let Some(at) = out.iter().position(|(rotation, _)| rotation == &current)
        {
            out.swap(0, at);
        }
        Ok(out)
    }

    /// The epoch key new content should be wrapped under.
    pub fn epoch_key(&self) -> Result<EpochKey, StoreError> {
        let current = self.rotation_ref()?;
        self.epoch_keys()?
            .into_iter()
            .find(|(rotation, _)| rotation == &current)
            .map(|(_, key)| key)
            .ok_or_else(|| StoreError::Corrupt("no epoch key stored for this network".to_owned()))
    }

    /// The rotation this store's current epoch key belongs to.
    pub fn rotation_ref(&self) -> Result<Hash, StoreError> {
        let bytes = fs::read(self.root.join("rotation"))
            .map_err(|_| StoreError::Corrupt("no epoch key stored for this network".to_owned()))?;
        Ok(Hash::from_bytes(fixed(&bytes, "rotation reference")?))
    }

    /// The DEK this node already holds for an object, if any.
    ///
    /// Tries the current epoch key first and falls back through the rest, then
    /// **re-wraps under the current epoch** whenever it opened under an older
    /// one. That refresh is what keeps the scan short: a wrapping is upgraded
    /// once per rotation rather than re-scanned on every read, which is what
    /// Storage §5.3 means by any current member re-wrapping — and it is
    /// deterministic, so several members doing it produce identical bytes and
    /// create nothing to reconcile.
    ///
    /// `commitment`, when given, is the pointer's own commitment to its DEK. A
    /// cached key that no longer matches it is stale — the author sealed that
    /// object and started another — so it is discarded rather than returned to
    /// fail more confusingly at decryption.
    pub fn known_dek(
        &self,
        pointer: &PointerId,
        commitment: Option<&Hash>,
    ) -> Result<Option<Dek>, StoreError> {
        let path = self.dek_path(pointer);
        let Ok(wrapped) = fs::read(&path) else {
            return Ok(None);
        };
        let keys = self.epoch_keys()?;
        let Some(dek) = keys
            .iter()
            .find_map(|(_, key)| key.unwrap_dek(pointer, &wrapped).ok())
        else {
            return Ok(None);
        };
        if let Some(commitment) = commitment
            && &dek.commitment() != commitment
        {
            return Ok(None);
        }

        if let Ok(epoch) = self.epoch_key() {
            let refreshed = epoch.wrap(pointer, &dek);
            if refreshed != wrapped {
                secret::write_private(&path, &refreshed)?;
            }
        }
        Ok(Some(dek))
    }

    /// Records a DEK learned from somebody else's wrapping.
    ///
    /// Stored wrapped under the current epoch, which makes it both a cache and
    /// the re-wrap Storage §5.3 asks of a current member: the next read opens it
    /// in one attempt instead of scanning every key this node holds, and it
    /// stays openable as superseded keys are eventually retired.
    pub fn remember_dek(&self, pointer: &PointerId, dek: &Dek) -> Result<(), StoreError> {
        let epoch = self.epoch_key()?;
        fs::create_dir_all(self.root.join("deks"))?;
        Ok(secret::write_private(&self.dek_path(pointer), &epoch.wrap(pointer, dek))?)
    }

    /// The data-encryption key for one author log **this node owns**.
    ///
    /// Minting one when there is none is correct only here. Another member's DEK
    /// can only come from their wrapping, and minting one there would produce a
    /// key that opens nothing — foreign objects go through
    /// [`known_dek`](Self::known_dek) and [`remember_dek`](Self::remember_dek).
    ///
    /// # What is still missing, stated rather than implied
    ///
    /// **Nothing is ever retired.** Superseded epoch keys accumulate, and while
    /// a refreshed wrapping means they are rarely *scanned*, they are still held
    /// and this node can still read anything wrapped under them. Retiring them is
    /// `design/01` §8's retention question — content that stops being re-wrapped
    /// goes dark — and is a deliberate policy choice rather than a cleanup to do
    /// quietly, because dropping a key makes anything still wrapped under it
    /// unreadable forever.
    pub fn channel_dek(&self, pointer: &PointerId) -> Result<Dek, StoreError> {
        if let Some(dek) = self.known_dek(pointer, None)? {
            return Ok(dek);
        }
        let epoch = self.epoch_key()?;
        let mut raw = [0u8; 32];
        intranet_crypto::random_bytes(&mut raw)
            .map_err(|err| StoreError::Corrupt(format!("no entropy: {err}")))?;
        let dek = Dek::from_bytes(raw);
        fs::create_dir_all(self.root.join("deks"))?;
        secret::write_private(&self.dek_path(pointer), &epoch.wrap(pointer, &dek))?;
        Ok(dek)
    }

    /// Whether the chain *behind* the segment named by `cid` is entirely held.
    ///
    /// Backfill walks a `previous` chain backwards, and the walk has to be able
    /// to stop. Stopping on "this segment taught us nothing new" would be wrong:
    /// a walk interrupted midway leaves older segments unread, and a later tick
    /// that halts at the first already-known segment would never reach them
    /// again. This mark means the stronger thing — everything behind here is in
    /// — so a walk that reaches it can stop knowing nothing is missed.
    pub fn chain_whole(&self, cid: &Cid) -> bool {
        self.segment_path(cid, "whole").exists()
    }

    /// Records that everything behind `cid` is held.
    pub fn mark_chain_whole(&self, cid: &Cid) -> Result<(), StoreError> {
        self.write_segment_mark(cid, "whole", &[])
    }

    /// Where a held segment sits in its chain: its sequence and its predecessor.
    ///
    /// Present exactly when the segment's records are stored, which makes it
    /// two things at once — the link a walk needs to take its next hop, and the
    /// answer to "have I already read this one?".
    ///
    /// Keeping it is what makes a re-walk cheap. Without it, a walk that ends at
    /// a segment it cannot open — the ordinary steady state once retention is
    /// active, since a retired segment never becomes readable — would re-fetch,
    /// re-decrypt and re-verify every signature in the whole held chain on every
    /// tick, forever, to rediscover links it already knew.
    pub fn segment_link(&self, cid: &Cid) -> Option<(u64, Option<Cid>)> {
        let raw = fs::read(self.segment_path(cid, "link")).ok()?;
        let (sequence, previous) = raw.split_at_checked(8)?;
        let sequence = u64::from_be_bytes(sequence.try_into().ok()?);
        let previous = match previous.len() {
            0 => None,
            _ => Some(Cid::from_hash(Hash::from_bytes(previous.try_into().ok()?))),
        };
        Some((sequence, previous))
    }

    /// Records where a segment sits in its chain, once its records are stored.
    pub fn mark_segment_link(
        &self,
        cid: &Cid,
        sequence: u64,
        previous: Option<Cid>,
    ) -> Result<(), StoreError> {
        let mut raw = sequence.to_be_bytes().to_vec();
        if let Some(previous) = previous {
            raw.extend_from_slice(previous.hash().as_bytes());
        }
        self.write_segment_mark(cid, "link", &raw)
    }

    fn segment_path(&self, cid: &Cid, kind: &str) -> PathBuf {
        self.root
            .join("segments")
            .join(format!("{}.{kind}", to_hex(cid.hash().as_bytes())))
    }

    fn write_segment_mark(&self, cid: &Cid, kind: &str, raw: &[u8]) -> Result<(), StoreError> {
        fs::create_dir_all(self.root.join("segments"))?;
        write_atomically(&self.root, self.segment_path(cid, kind), raw)?;
        Ok(())
    }

    fn dek_path(&self, pointer: &PointerId) -> PathBuf {
        self.root.join("deks").join(to_hex(pointer.as_bytes()))
    }
}

/// This user's home directory, by whichever name the platform gives it.
///
/// `USERPROFILE` first on Windows and `HOME` first elsewhere, each falling back
/// to the other rather than to nothing: a Unix shell that exports `USERPROFILE`
/// is odd but not wrong, and Git Bash on Windows sets `HOME` and is common.
fn home_dir() -> Option<PathBuf> {
    let profile = std::env::var_os("USERPROFILE");
    let home = std::env::var_os("HOME");
    if cfg!(windows) {
        first_set(profile, home)
    } else {
        first_set(home, profile)
    }
}

/// The first of two candidates that is set and not empty.
///
/// Not `or_else` followed by a check: a variable set to the empty string would
/// win that race and then be discarded, throwing away a perfectly good second
/// answer. Empty means unset here.
fn first_set(
    first: Option<std::ffi::OsString>,
    second: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    first
        .into_iter()
        .chain(second)
        .find(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn fixed<const N: usize>(bytes: &[u8], what: &str) -> Result<[u8; N], StoreError> {
    bytes
        .try_into()
        .map_err(|_| StoreError::Corrupt(format!("{what} is {} bytes, expected {N}", bytes.len())))
}

/// Held while a process is appending to the governance log.
///
/// Writes a file so that nothing ever reads half of one.
///
/// # Why every durable write in this store goes through it
///
/// `fs::write` truncates the destination and then fills it. A process that ends
/// between those two steps leaves a file that is neither the old contents nor
/// the new — and "a process that ends" is not an exotic case here, it is the
/// user closing the window. There is no shutdown protocol in front of it: the
/// task is dropped and the process exits.
///
/// For most of what this store keeps, half a file is an empty list and the next
/// tick rewrites it. For `entries/` it is a governance log that no longer
/// decodes, and [`Store::log`] refuses the whole network rather than one file —
/// correctly, because a governance log with a hole in it is not a smaller
/// governance log. The window is small and the cost of landing in it is the
/// network, which is the wrong side of that trade to leave to chance.
///
/// # Two details that are load-bearing
///
/// The temporary lives in the store's own `tmp/`, and **not beside the file it
/// is about to become**. `Store::log` reads every file in `entries/` and decodes
/// it, `chunks` reads every file in `chunks/`, and `append_entry` numbers the
/// next entry by counting the directory — so a leaked temporary in one of those
/// would be a corrupt entry, a corrupt chunk, or a reused index. It stays on the
/// same filesystem, which is what keeps the rename atomic rather than a copy.
///
/// `std::fs::rename` replaces an existing destination on both platforms this
/// ships to — on Windows through `MoveFileEx` with `MOVEFILE_REPLACE_EXISTING`.
///
/// # What this is not
///
/// **Atomicity, not durability.** The bytes may still be in the page cache when
/// the process ends. They survive the process dying, which is what this is for,
/// and they would not survive the machine losing power. Guarding that means an
/// `fsync` per record, which is a real cost and a decision to make deliberately
/// — and losing the last message to a power cut is a different order of problem
/// from losing the network to a window closing.
fn write_atomically(root: &Path, path: impl AsRef<Path>, bytes: &[u8]) -> Result<(), StoreError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let path = path.as_ref();
    let scratch = root.join("tmp");
    fs::create_dir_all(&scratch)?;
    let temp = scratch.join(format!(
        "{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));

    fs::write(&temp, bytes)?;
    if let Err(err) = fs::rename(&temp, path) {
        // A temporary left behind is a file in a directory nothing scans, but it
        // is still litter and the failing path is exactly where it accumulates.
        let _ = fs::remove_file(&temp);
        return Err(err.into());
    }
    Ok(())
}

/// Removes temporaries an interrupted [`write_atomically`] left behind.
///
/// They sit in a directory nothing else scans, so they are litter rather than a
/// hazard — but the path that leaves them is a process dying, which is also the
/// path that happens over and over on a machine with a problem.
fn sweep_scratch(root: &Path) {
    let Ok(entries) = fs::read_dir(root.join("tmp")) else {
        return;
    };
    for entry in entries.flatten() {
        let _ = fs::remove_file(entry.path());
    }
}

/// Whether a claim's heartbeat is recent enough to mean somebody holds it.
///
/// One implementation, because [`Store::hold_node`] and
/// [`Store::is_being_served`] ask the same question and two copies of a
/// staleness rule drift in exactly the way that makes one of them wrong about a
/// running node.
fn claim_is_fresh(beat: &std::path::Path) -> bool {
    fs::read_to_string(beat)
        .ok()
        .and_then(|text| text.trim().parse::<i64>().ok())
        .is_some_and(|when| now_millis().saturating_sub(when) < NODE_CLAIM_STALE)
}

/// How long a node claim survives without a heartbeat.
///
/// Long enough that a slow tick does not hand the network's key group to a
/// second process, and short enough that reopening a window after a crash is a
/// pause rather than a support question. The node beats every tick, so this is
/// several missed beats rather than one.
///
/// **The case this does not cover, stated because it is real:** a holder
/// suspended for longer than this — a laptop asleep — can have its claim taken
/// over while it still believes it holds one, and on waking both would run. What
/// keeps that rare rather than impossible is that taking over requires somebody
/// to actually start a second node in that window. Making it impossible needs
/// the holder to re-check ownership as it beats, which is worth doing when
/// anything depends on it.
pub const NODE_CLAIM_STALE: i64 = 6_000;

/// The right to run a node for one network.
///
/// Released on drop, and expiring on its own if the holder never gets to drop
/// it — see [`Store::hold_node`] for why both are needed.
pub struct NodeClaim {
    path: PathBuf,
}

impl NodeClaim {
    /// Says the holder is still running.
    ///
    /// Called from the node's own loop, so a claim outlives the process holding
    /// it by at most [`NODE_CLAIM_STALE`].
    pub fn beat(&self) {
        // Atomic like every other durable write, and for a sharper reason than
        // most: a half-written heartbeat does not parse, an unparseable one
        // reads as *stale*, and a stale claim is one another process may take
        // over while this one is still running. The window is one tick wide and
        // self-healing, and it is the one direction of failure this file must
        // not have.
        if let Some(root) = self.path.parent() {
            let _ = write_atomically(root, self.path.join("heartbeat"), now_millis().to_string().as_bytes());
        }
    }
}

impl Drop for NodeClaim {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.path.join("heartbeat"));
        let _ = fs::remove_dir(&self.path);
    }
}

/// Wall-clock now, in milliseconds.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Released on drop, including on a panic — a lock that survived a crash would
/// need a human to clear it, and the failure it guards against is rarer than
/// the crashes it would cause.
pub struct AppendLock {
    path: PathBuf,
}

impl Drop for AppendLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::first_set;
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn os(value: &str) -> Option<OsString> {
        Some(OsString::from(value))
    }

    #[test]
    fn the_first_candidate_wins_when_it_is_set() {
        assert_eq!(
            first_set(os("/first"), os("/second")),
            Some(PathBuf::from("/first"))
        );
    }

    #[test]
    fn an_absent_first_candidate_falls_through() {
        // Which is the whole point on Windows, where HOME is normally unset and
        // USERPROFILE is the answer — and on Git Bash, where it is the reverse.
        assert_eq!(first_set(None, os("/second")), Some(PathBuf::from("/second")));
    }

    #[test]
    fn an_empty_first_candidate_falls_through_rather_than_winning() {
        // The case `or_else` gets wrong: set-but-empty is not an answer, and
        // treating it as one throws away a good second candidate to return a
        // path that is silently the current directory.
        assert_eq!(first_set(os(""), os("/second")), Some(PathBuf::from("/second")));
    }

    #[test]
    fn nothing_set_is_nothing() {
        assert_eq!(first_set(None, None), None);
        assert_eq!(first_set(os(""), os("")), None);
    }
}

#[cfg(test)]
mod contribution_tests {
    use super::Store;
    use intranet_identity::NetworkId;
    use intranet_storage::Cid;

    fn store(name: &str) -> Store {
        let root = std::env::temp_dir().join(format!("kols-contrib-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Store::create(root, NetworkId::from_bytes([3u8; 32]), [4u8; 32]).expect("a store")
    }

    #[test]
    fn a_chunk_written_once_comes_back_and_is_not_written_twice() {
        let store = store("chunks");
        let bytes = b"a segment's worth of ciphertext".to_vec();
        let cid = Cid::of(&bytes);

        assert!(store.put_chunk(&cid, &bytes).expect("writes"), "first write");
        assert!(
            !store.put_chunk(&cid, &bytes).expect("writes"),
            "the same chunk again is not new — it is addressed by content, so \
             the name cannot disagree with what is under it"
        );
        assert_eq!(store.chunks().expect("reads"), vec![bytes]);

        let _ = std::fs::remove_dir_all(store.root());
    }

    // The pointer round-trip is covered end to end by `three_nodes.rs` rather
    // than here. Minting one needs `MutablePointer::publish`, which takes a
    // `GovernanceState` and applies both §2.2 publish gates against it — so a
    // unit test would have to build a governance state to check a file write,
    // and would be testing a hand-made state as much as anything else.

    #[test]
    fn a_pointer_file_that_will_not_decode_is_skipped_rather_than_fatal() {
        // Decoding verifies signatures, so a refusal means a pointer this node
        // must not act on. Refusing to start would turn one bad file into a
        // node that cannot open at all.
        let store = store("bad-pointer");
        let dir = store.root().join("pointers");
        std::fs::create_dir_all(&dir).expect("a directory");
        std::fs::write(dir.join("deadbeef"), b"not a pointer response").expect("writes");

        assert!(store.pointers().expect("reads").is_empty());

        let _ = std::fs::remove_dir_all(store.root());
    }
}
