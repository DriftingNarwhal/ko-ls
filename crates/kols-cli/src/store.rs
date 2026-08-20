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
    pub fn default_root() -> PathBuf {
        if let Ok(explicit) = std::env::var("KOLS_HOME") {
            return PathBuf::from(explicit);
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_owned());
        Path::new(&home).join(".kols")
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
        write_private(&root.join("seed"), &entropy)?;
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
            let held_since = fs::read_to_string(&beat)
                .ok()
                .and_then(|text| text.trim().parse::<i64>().ok());
            let fresh = held_since
                .is_some_and(|when| now_millis().saturating_sub(when) < NODE_CLAIM_STALE);

            if !fresh {
                fs::create_dir_all(&path)?;
                let claim = NodeClaim { path };
                claim.beat();
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
        fs::write(dir.join(format!("{next:08}")), wire::encode_entry(entry))?;
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
        fs::write(path, record.canonical_bytes())?;
        Ok(true)
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
        write_private(&self.root.join("group"), &self.at_rest_key().seal_chunk(state))
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
        fs::write(self.root.join("label"), name)?;
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
        fs::write(self.root.join("addresses"), addresses.join("\n"))?;
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
        fs::write(self.root.join("peers"), addresses.join("\n"))?;
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
        fs::write(self.root.join("relays"), relays.join("\n"))?;
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
        fs::write(self.root.join("waiting"), identities.join("\n"))?;
        Ok(())
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
            write_private(&dir.join(to_hex(rotation.as_bytes())), &sealed)?;
        }
        fs::write(self.root.join("rotation"), current.as_bytes())?;
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
                write_private(&path, &refreshed)?;
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
        write_private(&self.dek_path(pointer), &epoch.wrap(pointer, dek))
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
        write_private(&self.dek_path(pointer), &epoch.wrap(pointer, &dek))?;
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
        fs::write(self.segment_path(cid, kind), raw)?;
        Ok(())
    }

    fn dek_path(&self, pointer: &PointerId) -> PathBuf {
        self.root.join("deks").join(to_hex(pointer.as_bytes()))
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn fixed<const N: usize>(bytes: &[u8], what: &str) -> Result<[u8; N], StoreError> {
    bytes
        .try_into()
        .map_err(|_| StoreError::Corrupt(format!("{what} is {} bytes, expected {N}", bytes.len())))
}

/// Held while a process is appending to the governance log.
///
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
        let _ = fs::write(self.path.join("heartbeat"), now_millis().to_string());
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
