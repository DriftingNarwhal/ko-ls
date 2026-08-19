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
use intranet_governance::{GovernanceLog, GovernanceState, LogEntry, wire};
use intranet_identity::{MasterSeed, NetworkId, PerNetworkIdentity};
use intranet_storage::Dek;
use kols_core::ChannelId;
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

    /// The records this member has written in one channel, as canonical bytes.
    ///
    /// Kept so the author log can be rebuilt on the next invocation: an author
    /// log is single-writer and append-only, so replaying our own records in
    /// order reproduces the same segment, the same chunks and the same CIDs —
    /// chunk encryption being deterministic per (chunk, DEK).
    pub fn own_records(&self, channel: &ChannelId) -> Result<Vec<Vec<u8>>, StoreError> {
        let path = self.channel_dir(channel);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let mut files: Vec<_> = fs::read_dir(&path)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect();
        files.sort();
        files.into_iter().map(|p| Ok(fs::read(p)?)).collect()
    }

    /// Records one of this member's own records in a channel.
    pub fn push_own_record(&self, channel: &ChannelId, bytes: &[u8]) -> Result<(), StoreError> {
        let dir = self.channel_dir(channel);
        fs::create_dir_all(&dir)?;
        let next = fs::read_dir(&dir)?.count();
        fs::write(dir.join(format!("{next:08}")), bytes)?;
        Ok(())
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

    /// The data-encryption key for one channel's segments.
    ///
    /// **This is a stand-in, and the shortcut is worth stating plainly.** Storage
    /// §5 gives every object a per-object DEK wrapped under the network epoch key,
    /// re-wrappable by any current member on rotation — which is what makes a
    /// removed member lose access to content published afterwards. None of that
    /// wrapping is wired up yet (it is E7/P2 work), and the existing two-node test
    /// papers over it with a hardcoded key on both sides.
    ///
    /// So this derives a key every member can recompute from the network id. The
    /// honest consequence: **anyone who learns the network id can decrypt any
    /// segment they can obtain.** What actually keeps a non-member out is that
    /// honest nodes refuse to serve them at all (Storage §5.4) — a serving policy
    /// rather than cryptography, which is weaker than the design promises. It is
    /// enough to carry a conversation between two members of one network and is
    /// not enough for anything else.
    pub fn channel_dek(&self, channel: &ChannelId) -> Dek {
        let mut enc = intranet_crypto::Enc::domain("kols.cli.placeholder-channel-dek.v1");
        enc.fixed(self.network.as_bytes());
        enc.fixed(channel.as_bytes());
        Dek::from_bytes(*intranet_crypto::hash_bytes(&enc.finish()).as_bytes())
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
