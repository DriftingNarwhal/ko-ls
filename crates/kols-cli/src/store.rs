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
use intranet_storage::{Dek, EpochKey};
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

    /// The data-encryption key for one author log, wrapped under the epoch key.
    ///
    /// Storage §5's actual shape rather than a stand-in for it: the DEK is
    /// random per object, and what persists is the DEK **wrapped under the
    /// network's epoch key** — so recovering it requires key material only
    /// members hold, and a non-member who obtains the ciphertext has nothing to
    /// open it with.
    ///
    /// # What is still missing, stated rather than implied
    ///
    /// **Rotation.** Core §3.3 advances the epoch on every membership change,
    /// which is what stops a removed member reading anything published
    /// afterwards. Advancing it needs live MLS group state, and `GroupSession`
    /// holds an in-memory openmls provider with no persistence — so a process
    /// that exits cannot rotate, and this build does not. A removed member keeps
    /// the key, which is the naive scheme Core §3.2 rejects for exactly that
    /// reason. It is held deliberately and only until one of two things exists:
    /// an `intranet-epoch` that can persist a group, or a long-running node here
    /// that keeps one in memory.
    pub fn channel_dek(&self, pointer: &PointerId) -> Result<Dek, StoreError> {
        let epoch = self.epoch_key()?;
        let path = self.root.join("deks").join(to_hex(pointer.as_bytes()));
        if let Ok(wrapped) = fs::read(&path) {
            // Tried against **every** key this node holds, not just the current
            // one. A wrapping names the rotation it was made under, and the
            // epoch advances on every membership change — so a node that only
            // tried its newest key could not open its own content the moment
            // anybody joined or left. That is not hypothetical: it is what
            // happened the first time a revocation actually rotated anything.
            let keys = self.epoch_keys()?;
            let dek = keys
                .iter()
                .find_map(|(_, key)| key.unwrap_dek(pointer, &wrapped).ok())
                .ok_or_else(|| {
                    StoreError::Corrupt(format!(
                        "a stored DEK will not unwrap under any of this node's {} epoch key(s)",
                        keys.len()
                    ))
                })?;

            // Re-wrapped under the current epoch, which is exactly what Storage
            // §5.3 means by any current member re-wrapping on rotation: it keeps
            // the wrapping openable as superseded keys are eventually dropped,
            // and it is deterministic, so doing it repeatedly changes nothing.
            let refreshed = epoch.wrap(pointer, &dek);
            if refreshed != wrapped {
                write_private(&path, &refreshed)?;
            }
            return Ok(dek);
        }

        let mut raw = [0u8; 32];
        intranet_crypto::random_bytes(&mut raw)
            .map_err(|err| StoreError::Corrupt(format!("no entropy: {err}")))?;
        let dek = Dek::from_bytes(raw);
        fs::create_dir_all(self.root.join("deks"))?;
        // The wrapping is what persists, never the DEK. Wrapping is
        // deterministic per (pointer, epoch key) by requirement (§5.3), so a
        // re-wrap by another member produces identical bytes and creates no
        // conflict to resolve.
        write_private(&path, &epoch.wrap(pointer, &dek))?;
        Ok(dek)
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
