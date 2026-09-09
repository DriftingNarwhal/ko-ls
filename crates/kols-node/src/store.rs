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
use crate::readings::OwnReadings;
use crate::secret;
use kols_core::{ChannelId, Cursor, Hlc, Record, Window};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// What this machine contributes to one network — Core §4.3.
///
/// All four are contributions to *other members*. None of them is what this
/// member needs to use the application: a node contributing nothing still
/// fetches, reads, posts and keeps its own history, which is why zero
/// everywhere is an ordinary configuration rather than a broken one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Contribution {
    /// Bytes of other members' content this network may store here.
    pub storage_offered: u64,
    /// Bytes per second this node will upload for others.
    pub upload_offered: u64,
    /// Bytes per second this node will accept downstream.
    pub download_offered: u64,
    /// Whether this node volunteers as a bootstrap relay.
    pub relay_willing: bool,
}

/// Everything one network's membership needs on disk.
/// What a read of the store actually cost, in work that grows with history.
///
/// # Why this is counted rather than timed
///
/// `design/09` §4.4 requires the window's two-second tick to be bounded, and a
/// timing measurement cannot express that. Forty milliseconds at eight thousand
/// records reads as "flat enough" and is still linear; the curve only shows up
/// at a scale nobody tests at, which is to say in somebody's client after two
/// years. A count of the work done is the guarantee: if it is the same number at
/// two hundred records and at five thousand, it is the same number at a hundred
/// thousand, and no measurement is being trusted to notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Work {
    /// Directory entries enumerated.
    pub listed: u64,
    /// Files opened and read whole.
    pub read: u64,
    /// Records decoded and signature-checked.
    pub decoded: u64,
}

/// The counters behind [`Work`].
#[derive(Debug, Default)]
struct Counters {
    listed: std::sync::atomic::AtomicU64,
    read: std::sync::atomic::AtomicU64,
    decoded: std::sync::atomic::AtomicU64,
}

/// Everything one network's node keeps on this disk.
pub struct Store {
    root: PathBuf,
    /// What the reads through this handle have cost so far.
    ///
    /// Instrumentation, not state: nothing reads it to decide anything, and a
    /// wrong count changes no answer. It exists so a test can assert the shape
    /// of the cost rather than its magnitude.
    work: Counters,
    entropy: [u8; 32],
    network: NetworkId,
    /// The governance log and the state it replays to, held across calls.
    ///
    /// # Why this exists
    ///
    /// Reading the log means reading every entry file, decoding it and
    /// verifying its signature on insert — and one command did that **three
    /// times**: once for replayed state, once to read channels, once to read
    /// names. Measured at 152 ms for a single ordinary command against a network
    /// with three hundred channels, growing with the log, which only ever grows
    /// (`design/05` §5).
    ///
    /// # Why it is safe to hold
    ///
    /// The log is append-only on disk and one process holds the node claim, so
    /// "has it changed" is answered by counting entry files — no read, no
    /// decode, no verification. When the count is unchanged the cached answer is
    /// the same answer replay would produce, because replay is a pure function
    /// of the entries and the entries are the same. When it has changed the
    /// whole thing is rebuilt, which is what happened on every call before.
    ///
    /// It is a cache and the files stay the source of truth: dropping it costs
    /// one rebuild and changes no answer.
    log_cache: std::sync::Mutex<Option<CachedLog>>,
    /// Channel and category state, as of a known log generation.
    ///
    /// The other half of what a command was paying for. Caching the log stopped
    /// it being re-read and re-verified; this stops it being *walked* — the
    /// channel map is folded out of the canonical chain, and `submit` built it
    /// again for every command.
    ///
    /// A separate lock from the log's, deliberately: these are computed by
    /// `network`, which reads the log through this same store, and one lock
    /// covering both would deadlock the moment the derived answer had to be
    /// rebuilt.
    derived: std::sync::Mutex<Derived>,
    /// The read-side index, opened on first use.
    ///
    /// Lazily, because most of what a store does never touches it — a daemon
    /// syncing and publishing has no page to render — and opening a database to
    /// find that out would be a cost paid by everything to serve one path.
    projection: std::sync::OnceLock<Option<kols_store::Projection>>,
    /// This member's own identity id, derived once.
    ///
    /// Memoised because [`put_record`](Store::put_record) compares every stored
    /// record against it, and deriving a keypair per record on a sync burst
    /// would be paying for the comparison many times over.
    own: std::sync::OnceLock<Option<PerNetworkIdentityId>>,
    /// The last answer about held segments, and the tally it was true of.
    ///
    /// `history_incomplete` reads a `.link` mark per held segment, so asking it
    /// on every tick grows with history. Its answer changes only when a link is
    /// written — shedding deliberately leaves links alone, so that a member's
    /// history does not appear to shrink — which makes one tally enough to know
    /// the cached answer still stands.
    segments_seen: std::sync::Mutex<Option<(u64, std::collections::BTreeMap<ChannelId, bool>)>>,
}

/// What has been folded out of the log, and which log it was folded from.
#[derive(Default)]
struct Derived {
    generation: usize,
    channels: Option<std::sync::Arc<(crate::network::ChannelMap, Vec<String>)>>,
    categories: Option<std::sync::Arc<(crate::network::CategoryMap, Vec<String>)>>,
    names: Option<std::sync::Arc<kols_core::Names>>,
}

/// The log and its replayed state, as of a known number of entry files.
struct CachedLog {
    /// The tally this was built at.
    ///
    /// **The whole invalidation rule, and it costs one `stat`.** The tally is
    /// appended to before an entry is written, by whichever handle writes it, so
    /// a tally that has not moved means a log that has not moved. It is compared
    /// for equality and never used as a count — nothing derives a filename from
    /// it, so two handles appending at once cost a rebuild rather than an entry.
    tally: u64,
    /// How many entry files were folded in.
    ///
    /// Where a refresh starts reading from. Entries are only ever appended and
    /// are numbered by the directory's own size, so the files past this are
    /// exactly what is new.
    count: usize,
    log: std::sync::Arc<GovernanceLog>,
    /// The canonical chain as of that count.
    ///
    /// Kept so a refresh can tell an ordinary extension from a reorg. Fork
    /// choice may make a different branch canonical when an entry arrives
    /// (Core §2.7.1), and a state advanced along a chain that is no longer the
    /// one would be wrong in the quietest way available — so the prefix is
    /// compared rather than assumed.
    chain: Vec<Hash>,
    /// The state it replays to, when it replays at all.
    ///
    /// `None` for a log that cannot be replayed, which is an ordinary state
    /// rather than a fault: a node that has attached and not yet synced holds
    /// entries without a genesis in front of them. Reading the *log* must keep
    /// working there — the sync path is what fills the gap — so the two are
    /// cached apart rather than as one answer.
    state: Option<std::sync::Arc<GovernanceState>>,
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

/// The most one request from an interface may reach beyond what is loaded.
///
/// **Applied where untrusted input arrives and nowhere else.** It lived here
/// briefly and was wrong: the store silently truncated `Window::opening(MAX)` to
/// this, so `kols read` would have shown the last five hundred messages of a
/// channel and said nothing about the rest. A ceiling that quietly rewrites what
/// a caller asked for is the same class of failure as a page boundary that drops
/// a message — the answer is wrong and nothing reveals it.
///
/// So the store does what it is told, and the boundary clamps: `kols-app`'s
/// `WindowArg` applies this to what the window sends, and callers inside the
/// process ask for what they mean.
pub const MAX_REACH: usize = 500;

/// A channel's records as far as a reader has loaded them.
#[derive(Debug)]
pub struct Loaded {
    /// The range's records and everything acting on them, in merge order.
    pub records: Vec<Record>,
    /// Which of them the rate pass refused, and why.
    pub refused: std::collections::BTreeMap<kols_core::MessageId, kols_core::Rejection>,
    /// Where the range now starts.
    pub oldest: Option<Cursor>,
    /// Where it now ends, or absent when it runs to the tail.
    pub newest: Option<Cursor>,
    /// Whether this channel holds records before the range.
    pub older: bool,
    /// Whether it holds records after it.
    pub newer: bool,
    /// How many authors have written in the channel, whole-channel.
    pub authors: usize,
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
            work: Counters::default(),
            entropy,
            network,
            log_cache: std::sync::Mutex::new(None),
            derived: std::sync::Mutex::new(Derived::default()),
            projection: std::sync::OnceLock::new(),
            own: std::sync::OnceLock::new(),
            segments_seen: std::sync::Mutex::new(None),
        })
    }

    /// What reads through this handle have cost since [`Self::reset_work`].
    pub fn work(&self) -> Work {
        use std::sync::atomic::Ordering::Relaxed;
        Work {
            listed: self.work.listed.load(Relaxed),
            read: self.work.read.load(Relaxed),
            decoded: self.work.decoded.load(Relaxed),
        }
    }

    /// Starts the count again.
    pub fn reset_work(&self) {
        use std::sync::atomic::Ordering::Relaxed;
        self.work.listed.store(0, Relaxed);
        self.work.read.store(0, Relaxed);
        self.work.decoded.store(0, Relaxed);
    }

    /// Records work done.
    fn did(&self, listed: u64, read: u64, decoded: u64) {
        use std::sync::atomic::Ordering::Relaxed;
        self.work.listed.fetch_add(listed, Relaxed);
        self.work.read.fetch_add(read, Relaxed);
        self.work.decoded.fetch_add(decoded, Relaxed);
    }

    /// Opens an existing store.
    pub fn open(root: PathBuf) -> Result<Self, StoreError> {
        if !root.join("network").exists() {
            return Err(StoreError::NotInitialised(root));
        }
        let entropy = fixed(&fs::read(root.join("seed"))?, "seed")?;
        let network = NetworkId::from_bytes(fixed(&fs::read(root.join("network"))?, "network id")?);
        let store = Self {
            root,
            work: Counters::default(),
            entropy,
            network,
            log_cache: std::sync::Mutex::new(None),
            derived: std::sync::Mutex::new(Derived::default()),
            projection: std::sync::OnceLock::new(),
            own: std::sync::OnceLock::new(),
            segments_seen: std::sync::Mutex::new(None),
        };
        Ok(store)
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
                // The token is written *before* the first heartbeat, so a
                // concurrent taker sees a claim that is either wholly the old
                // holder's or wholly this one's. Written the other way round,
                // there would be an instant where a fresh heartbeat sat beside
                // somebody else's token and both processes read themselves as
                // the owner.
                let token = claim_token();
                let claim = NodeClaim { path, token };
                claim.write_owner()?;
                // Ours by construction — the owner was just written — so there
                // is nothing for the ownership check to tell us here.
                let _ = claim.beat();
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
        // **The tally is a change signal and never a count**, which is what
        // keeps two handles appending at once from colliding: nothing derives a
        // filename from it, so a lost or doubled mark costs a rebuild rather
        // than an entry.
        //
        // Marked before the entry is written, and the order is the safe one. A
        // tally ahead of the directory makes a reader rebuild for nothing; a
        // tally behind it makes a reader believe a governance act never
        // happened, which is a permission change that silently does not apply.
        //
        // Numbering still costs a listing, and that is the right place for it to
        // cost one: this runs when somebody governs, and the read path it feeds
        // runs every two seconds.
        append_to(self.entry_tally_path(), &[0])?;
        let next = fs::read_dir(&dir)?.inspect(|_| self.did(1, 0, 0)).count();
        write_atomically(
            &self.root,
            dir.join(format!("{next:08}")),
            &wire::encode_entry(entry),
        )?;
        Ok(())
    }

    /// Where the governance log's tally lives.
    fn entry_tally_path(&self) -> PathBuf {
        self.root.join("entries.tally")
    }

    /// How many marks the governance log has taken, in one `stat`.
    ///
    /// Compared for *equality* against what a cache was built at, never used as
    /// a count. A store written before this existed answers zero, which differs
    /// from nothing and therefore costs one rebuild rather than an error.
    fn entry_tally(&self) -> u64 {
        length_of(self.entry_tally_path())
    }

    /// Reads the governance log back, ancestors first.
    pub fn log(&self) -> Result<std::sync::Arc<GovernanceLog>, StoreError> {
        Ok(self.replayed()?.0)
    }

    /// Replays the stored log into current state.
    pub fn state(&self) -> Result<std::sync::Arc<GovernanceState>, StoreError> {
        let (log, state) = self.replayed()?;
        match state {
            Some(state) => Ok(state),
            // Recomputed only to produce the refusal, which is the one thing the
            // cache cannot hold: a `StoreError` is not clonable, and a log that
            // does not replay is a handful of entries rather than a history.
            None => {
                let chain: Vec<_> = log
                    .canonical_chain()
                    .iter()
                    .filter_map(|hash| log.get(hash))
                    .collect();
                GovernanceState::replay(chain).map(std::sync::Arc::new).map_err(|err| {
                    StoreError::Corrupt(format!("replay refused the stored log: {err}"))
                })
            }
        }
    }

    /// The log and the state together, from the cache or rebuilt.
    fn replayed(
        &self,
    ) -> Result<
        (
            std::sync::Arc<GovernanceLog>,
            Option<std::sync::Arc<GovernanceState>>,
        ),
        StoreError,
    > {
        let dir = self.root.join("entries");
        // **One `stat`, and in the settled case that is the whole of it.**
        //
        // This used to list the entries directory and sort it, which is a scan
        // of everything the network has ever done, run on the read path — the
        // saving it was written for (not *reading* every entry) left the
        // *listing* behind, and a listing still grows. The tally is appended to
        // by whichever handle wrote the entry, so its length is how many entries
        // exist, answered without opening a directory.
        let tally = self.entry_tally();

        let mut held = self
            .log_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(cached) = held.as_ref()
            && cached.tally == tally
        {
            return Ok((
                std::sync::Arc::clone(&cached.log),
                cached.state.clone(),
            ));
        }

        // Something moved, so the directory is read — off the settled path, and
        // exactly as often as the log actually changes.
        let mut files: Vec<_> = match fs::read_dir(&dir) {
            Ok(entries) => entries.filter_map(Result::ok).map(|e| e.path()).collect(),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(err) => return Err(err.into()),
        };
        files.sort();
        self.did(files.len() as u64, 0, 0);

        // **Only the entries that arrived.** Names are zero-padded indices and
        // the log is append-only, so the files past the cached count are exactly
        // what is new — and re-reading, re-decoding and re-verifying the rest to
        // learn about one new entry is the cost this whole cache exists to stop
        // paying. A cache built from a shorter log is extended; anything else
        // (a first read, or a directory that somehow shrank) is built whole.
        let (mut log, from) = match held.as_ref() {
            Some(cached) if cached.count < files.len() => {
                ((*cached.log).clone(), cached.count)
            }
            _ => (GovernanceLog::new(), 0),
        };
        for path in &files[from..] {
            let bytes = fs::read(path)?;
            let entry = wire::decode_entry(&bytes)
                .map_err(|err| StoreError::Corrupt(format!("{}: {err}", path.display())))?;
            log.insert(entry)
                .map_err(|err| StoreError::Corrupt(format!("{}: {err}", path.display())))?;
        }

        let chain = log.canonical_chain();

        // Advanced along the chain rather than replayed from genesis, when the
        // chain this state was built on is still a prefix of the one there is
        // now. Where it is not, fork choice has moved a branch out from under it
        // and the only honest answer is to replay: `apply` walks forward and has
        // no way to unapply an entry that is no longer canonical.
        let extended = held.as_ref().and_then(|cached| {
            let state = cached.state.as_ref()?;
            (chain.len() >= cached.chain.len() && chain.starts_with(&cached.chain))
                .then(|| (std::sync::Arc::clone(state), cached.chain.len()))
        });
        let state = match extended {
            Some((state, done)) => chain[done..]
                .iter()
                .filter_map(|hash| log.get(hash))
                .try_fold((*state).clone(), |state, entry| state.apply(entry))
                .ok()
                .map(std::sync::Arc::new),
            None => GovernanceState::replay(
                chain.iter().filter_map(|hash| log.get(hash)).collect::<Vec<_>>(),
            )
            .ok()
            .map(std::sync::Arc::new),
        };

        let log = std::sync::Arc::new(log);
        *held = Some(CachedLog {
            tally,
            count: files.len(),
            log: std::sync::Arc::clone(&log),
            chain,
            state: state.clone(),
        });
        Ok((log, state))
    }

    /// Which log the derived answers below belong to.
    ///
    /// The count of entry files as of the last refresh, which is what makes a
    /// cached fold safe to reuse: entries are only appended, so an unchanged
    /// count is an unchanged log.
    pub fn generation(&self) -> usize {
        self.log_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map_or(0, |cached| cached.count)
    }

    /// The channel fold for `generation`, if it has been computed.
    pub fn cached_channels(
        &self,
        generation: usize,
    ) -> Option<std::sync::Arc<(crate::network::ChannelMap, Vec<String>)>> {
        let held = self
            .derived
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (held.generation == generation).then(|| held.channels.clone()).flatten()
    }

    /// Keeps the channel fold, discarding anything from an older log.
    pub fn keep_channels(
        &self,
        generation: usize,
        folded: std::sync::Arc<(crate::network::ChannelMap, Vec<String>)>,
    ) {
        let mut held = self
            .derived
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if held.generation != generation {
            *held = Derived {
                generation,
                ..Derived::default()
            };
        }
        held.channels = Some(folded);
    }

    /// The category fold for `generation`, if it has been computed.
    pub fn cached_categories(
        &self,
        generation: usize,
    ) -> Option<std::sync::Arc<(crate::network::CategoryMap, Vec<String>)>> {
        let held = self
            .derived
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (held.generation == generation).then(|| held.categories.clone()).flatten()
    }

    /// Keeps the category fold, discarding anything from an older log.
    pub fn keep_categories(
        &self,
        generation: usize,
        folded: std::sync::Arc<(crate::network::CategoryMap, Vec<String>)>,
    ) {
        let mut held = self
            .derived
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if held.generation != generation {
            *held = Derived {
                generation,
                ..Derived::default()
            };
        }
        held.categories = Some(folded);
    }

    /// The display-name fold for `generation`, if it has been computed.
    pub fn cached_names(&self, generation: usize) -> Option<std::sync::Arc<kols_core::Names>> {
        let held = self
            .derived
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (held.generation == generation).then(|| held.names.clone()).flatten()
    }

    /// Keeps the display-name fold, discarding anything from an older log.
    pub fn keep_names(&self, generation: usize, folded: std::sync::Arc<kols_core::Names>) {
        let mut held = self
            .derived
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if held.generation != generation {
            *held = Derived {
                generation,
                ..Derived::default()
            };
        }
        held.names = Some(folded);
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
            self.did(1, 1, 1);
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
        // **The arrival log, appended before the record is written.**
        //
        // This is what lets a reader learn that something is new for the cost of
        // one `stat` — see [`append_to`]. The order matters and is the safe one:
        // an id here whose file never appeared is a write that did not complete,
        // and the fold skips it. The other order would let a record exist that
        // nothing names, which is a message no reader would ever see again.
        //
        // The duplicate check above runs first, so a record delivered twice is
        // named once.
        append_to(self.arrivals_path(channel), record.id().as_bytes())?;
        write_atomically(&self.root, path, &record.canonical_bytes())?;

        // **Here rather than at the one caller that writes this member's own
        // records**, because it is not the only one that can. A record this
        // member wrote also arrives from the network — refetched out of a
        // segment this node published and later lost, or, once `05` §6 lands,
        // written by another of their devices. An index maintained only where
        // the executor writes would be silently behind in exactly those cases,
        // and being behind makes `next_hlc` hand back a reading that is not
        // greater than one already published, which readers refuse.
        if self.is_own(&record.author) {
            self.note_own_reading(channel, record)?;
        }
        Ok(true)
    }

    /// Where a channel's arrival log lives.
    fn arrivals_path(&self, channel: &ChannelId) -> PathBuf {
        self.channel_dir(channel).join("arrivals")
    }

    /// How many records have ever been announced for a channel.
    ///
    /// One `stat`, whatever the channel holds. This is the number the fold
    /// compares against to decide whether there is anything to do at all, and it
    /// is why a settled read does not grow.
    fn arrivals(&self, channel: &ChannelId) -> u64 {
        length_of(self.arrivals_path(channel)) / 32
    }

    /// The ids announced from `from` onwards.
    ///
    /// Reads the tail of the arrival log and nothing else, so the cost is the
    /// number of records that are actually new — one, in the live case.
    fn arrived_since(
        &self,
        channel: &ChannelId,
        from: u64,
    ) -> Result<Vec<kols_core::MessageId>, StoreError> {
        use std::io::{Read, Seek, SeekFrom};
        let path = self.arrivals_path(channel);
        let Ok(mut file) = fs::File::open(&path) else {
            return Ok(Vec::new());
        };
        file.seek(SeekFrom::Start(from * 32))?;
        let mut raw = Vec::new();
        file.read_to_end(&mut raw)?;
        self.did(0, 1, 0);
        Ok(raw
            .chunks_exact(32)
            .map(|chunk| {
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(chunk);
                kols_core::MessageId::from_bytes(bytes)
            })
            .collect())
    }

    /// Rebuilds the arrival log from the record files.
    ///
    /// **The reconciliation, and it runs off the tick.** A store written before
    /// the log existed has none, and a crash between the append and the write
    /// can leave one naming a record that is not there. Both are answered by
    /// listing the directory once — which is what every read used to do every
    /// time — and neither is reachable from the path that runs every two
    /// seconds.
    fn rebuild_arrivals(&self, channel: &ChannelId) -> Result<(), StoreError> {
        let mut ids = self.record_ids(channel)?;
        ids.sort_by_key(|id| *id.as_bytes());
        let mut raw = Vec::with_capacity(ids.len() * 32);
        for id in &ids {
            raw.extend_from_slice(id.as_bytes());
        }
        write_atomically(&self.root, self.arrivals_path(channel), &raw)
    }

    /// Which records a channel holds, from the names of their files.
    ///
    /// A record's file is named by its id, so this is the same set
    /// [`Self::records`] would produce without reading or decoding a byte of
    /// content. What that buys is the ordinary case: a channel whose records the
    /// index has already folded is recognised as unchanged for the cost of one
    /// directory listing.
    fn record_ids(
        &self,
        channel: &ChannelId,
    ) -> Result<Vec<kols_core::MessageId>, StoreError> {
        let dir = self.channel_dir(channel).join("records");
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        let mut listed = 0u64;
        for entry in fs::read_dir(&dir)? {
            listed += 1;
            let name = entry?.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            // Anything that is not a record id is not a record. A stray file
            // here is skipped rather than failing the read: the records are the
            // source of truth and one unreadable name must not make a channel
            // unopenable.
            if let Some(raw) = intranet_crypto::from_hex(name)
                && let Ok(bytes) = <[u8; 32]>::try_from(raw)
            {
                ids.push(kols_core::MessageId::from_bytes(bytes));
            }
        }
        self.did(listed, 0, 0);
        Ok(ids)
    }

    /// One stored record, by id.
    fn record(&self, channel: &ChannelId, id: &kols_core::MessageId) -> Option<Record> {
        let path = self
            .channel_dir(channel)
            .join("records")
            .join(to_hex(id.as_bytes()));
        self.did(0, 1, 1);
        Record::decode(&fs::read(path).ok()?).ok()
    }

    /// The read-side index, opened on first use — `design/05` §5.
    ///
    /// `None` where it could not be opened at all, which is survivable rather
    /// than fatal: the records are the source of truth and the slow path over
    /// them still works, so a projection that will not open costs speed and
    /// never an answer.
    fn projection(&self) -> Option<&kols_store::Projection> {
        self.projection
            .get_or_init(|| {
                kols_store::Projection::open(&self.root.join("projection.sqlite"))
                    .ok()
                    .map(|(projection, _)| projection)
            })
            .as_ref()
    }

    /// A channel's loaded range, with what the rate pass refused.
    ///
    /// # What this replaces
    ///
    /// Reading every record in the channel, decoding each one and folding the
    /// reader-side limits over the whole set — for a screenful. This asks the
    /// index which records the range holds, reads those files and the ones
    /// acting on them, and takes the refusals from rows already decided.
    ///
    /// # The range grows and never slides
    ///
    /// `design/09` §4.4: what the interface holds is the range it has drawn,
    /// extended by `back` before its older end and `forward` after its newer
    /// one. A range whose newer end is absent is *live* — it runs to the tail,
    /// which is what makes arrivals appear without anything asking, and what
    /// makes backfill landing inside it appear at all.
    ///
    /// Returns `None` when there is no index to ask, so a caller falls back to
    /// the slow path rather than showing an empty channel.
    pub fn load(
        &self,
        channel: &ChannelId,
        limits: &kols_core::ReaderLimits,
        window: &Window,
    ) -> Option<Loaded> {
        let projection = self.projection()?;
        self.fold_into(projection, channel, limits).ok()?;

        let (back, forward) = (window.back, window.forward);

        // Two cases, and a jump is not a third: opening *at* a message is a
        // range whose ends are both that message, reached from both directions.
        let mut ids = match window.oldest {
            None => projection.before(channel, None, back.max(1)).ok()?,
            Some(oldest) => {
                let mut earlier = projection.before(channel, Some(oldest), back).ok()?;
                earlier.extend(projection.between(channel, oldest, window.newest, usize::MAX).ok()?);
                if window.newest.is_some() {
                    earlier.extend(projection.after(channel, window.newest, forward).ok()?);
                }
                earlier
            }
        };
        ids.dedup();

        let oldest = self.cursor_of(channel, ids.first())?;
        let newest = self.cursor_of(channel, ids.last())?;

        // **Whether anything is left in each direction**, asked of the index
        // rather than guessed from whether a page came back full. A page can be
        // exactly the size of what remains, and "full" would then claim there is
        // more forever.
        let older = oldest
            .map(|c| projection.before(channel, Some(c), 1).map(|r| !r.is_empty()))
            .transpose()
            .ok()?
            .unwrap_or(false);
        let newer = match (window.newest, newest) {
            // Already live: it runs to the tail by construction.
            (None, _) => false,
            (Some(_), Some(end)) => !projection.after(channel, Some(end), 1).ok()?.is_empty(),
            (Some(_), None) => false,
        };

        let mut wanted: std::collections::BTreeSet<_> = ids.iter().copied().collect();
        wanted.extend(projection.acting_on(channel, &ids).ok()?);

        let mut records = Vec::with_capacity(wanted.len());
        let mut refused = std::collections::BTreeMap::new();
        for id in &wanted {
            let Some(record) = self.record(channel, id) else {
                // A row naming a file that is gone. The index is derived, so the
                // honest answer is to rebuild rather than to render a hole.
                let _ = projection.forget(channel);
                return None;
            };
            if let Some(why) = projection.verdict(id).ok().flatten().and_then(kols_store::Verdict::rejection)
            {
                refused.insert(*id, why);
            }
            records.push(record);
        }
        records.sort_by_key(kols_core::Record::cursor);

        Some(Loaded {
            records,
            refused,
            oldest,
            // **Reattaching is what running out of newer records means.** A
            // reader who scrolled down to the tail is looking at the present,
            // and holding the range detached would then freeze it there — the
            // same failure as reattaching them on a timer, from the other side.
            newest: newer.then_some(newest).flatten(),
            older,
            newer,
            // From the fold's own row rather than a `COUNT(DISTINCT author)`
            // over the channel, which was the last whole-channel scan on this
            // path (`design/09` §4.4).
            authors: projection
                .folded_state(channel)
                .ok()
                .flatten()
                .map(|state| state.authors as usize)
                .unwrap_or(0),
        })
    }

    /// The position of a record named by the index.
    ///
    /// `Some(None)` for "there was no record", which is an empty channel and not
    /// a failure; `None` only when the file behind a row could not be read.
    fn cursor_of(
        &self,
        channel: &ChannelId,
        id: Option<&kols_core::MessageId>,
    ) -> Option<Option<kols_core::Cursor>> {
        match id {
            None => Some(None),
            Some(id) => self.record(channel, id).map(|r| Some(r.cursor())),
        }
    }

    /// Brings a channel's rows up to date with its record files.
    ///
    /// Three cases, and the middle one is why this is not simply a rebuild.
    /// Nothing new: no work. Records that all sort **after** everything folded:
    /// they change no verdict already decided, so they are folded on the end.
    /// Anything reaching back into the fold — which is what backfill does — and
    /// every verdict after it may move, so the channel is folded again.
    ///
    /// A change to the limits themselves also re-folds, because a verdict is
    /// only true of the rules that produced it and a stale refusal is a message
    /// left hidden after the rule that hid it was relaxed.
    fn fold_into(
        &self,
        projection: &kols_store::Projection,
        channel: &ChannelId,
        limits: &kols_core::ReaderLimits,
    ) -> Result<(), StoreError> {
        let under = kols_store::FoldedUnder {
            message_rate: limits.message_rate_per_minute,
            reaction_rate: limits.reaction_rate_per_minute,
            slowmode: limits.slowmode_seconds,
        };
        let mut stale = projection
            .folded_under(channel)
            .ok()
            .flatten()
            .is_some_and(|held| held != under);

        // **One `stat` and one row, and in the settled case that is the whole
        // of it.**
        //
        // The arrival log grows by 32 bytes a record and is appended to by
        // whichever handle stored it, so its length is how many records this
        // channel has ever held — answered without opening it. `consumed` is how
        // many of them this fold has taken in. Equal means there is nothing to
        // do, and nothing here has looked at the channel.
        //
        // Every earlier version of this check was a scan wearing a different
        // hat: reading and decoding every record file, then listing the
        // directory, then a `COUNT(*)`. All three answer "has anything changed"
        // by examining everything, which is the shape `design/09` §4.4 says the
        // two-second tick may not have.
        let state = projection.folded_state(channel).ok().flatten().unwrap_or_default();
        let announced = self.arrivals(channel);
        if !stale && announced == state.consumed {
            return Ok(());
        }

        // Something is new, so the tail of the log names exactly what — no
        // directory listing, and no diff against the ids already held.
        let mut missing: Vec<Record> = self
            .arrived_since(channel, state.consumed)?
            .into_iter()
            .filter(|id| !projection.holds(id).unwrap_or(false))
            .filter_map(|id| self.record(channel, &id))
            .collect();
        missing.sort_by_key(kols_core::Record::cursor);

        let newest = projection.newest(channel).ok().flatten();
        // Compared on the whole position rather than the reading, for the same
        // reason a page boundary is: two records can share a reading, and a
        // comparison that could not separate them would call an arrival
        // *forward* when it lands beside the newest row rather than after it.
        if let (Some(newest), Some(first)) = (newest, missing.first())
            && first.cursor() <= newest
        {
            stale = true;
        }

        // A re-fold is the only path that still reads the whole channel, which is
        // what it is: every verdict after the arrival may move, so every record
        // has to go through the pass again. Rare by construction — it takes
        // backfill reaching into the fold, or a governance act changing the
        // limits. **Off the tick**, which is the property that matters.
        let fold: Vec<Record> = if stale {
            let _ = projection.forget(channel);
            // The log is what the fold is measured against, so a rebuild rebuilds
            // it too — that is the one path allowed to list the directory, and
            // it is also where a log left short by a crash is repaired.
            self.rebuild_arrivals(channel)?;
            self.records(channel)?
        } else {
            missing
        };
        // **Whether an arriving author is new is asked before the insert**, on
        // the author key the index already carries. That is a seek rather than a
        // scan, and it is bounded by the roster rather than by history — which
        // is what lets the whole-channel author count be *maintained* instead of
        // recomputed as a `COUNT(DISTINCT)` on every read.
        let mut added = 0u64;
        let mut folded_in = 0u64;
        for record in &fold {
            if projection.holds(&record.id()).unwrap_or(false) {
                continue;
            }
            let first_from_them = !projection
                .has_author(channel, &record.author)
                .unwrap_or(true);
            let verdict = kols_store::decide(projection, record, limits)
                .map_err(|err| StoreError::Corrupt(err.to_string()))?;
            projection
                .insert(record, verdict)
                .map_err(|err| StoreError::Corrupt(err.to_string()))?;
            folded_in += 1;
            if first_from_them {
                added += 1;
            }
        }

        // A re-fold forgot everything first, so its counts start from nothing;
        // an ordinary arrival adds to what was there.
        let base = if stale {
            kols_store::Folded::default()
        } else {
            state
        };
        let folded = kols_store::Folded {
            // Taken from the log again after a rebuild, since rebuilding it may
            // have changed its length.
            consumed: if stale { self.arrivals(channel) } else { announced },
            records: base.records + folded_in,
            authors: base.authors + added,
        };
        let _ = projection.folded(channel, under, folded);
        Ok(())
    }

    /// Whether an identity is the one this store holds the seed for.
    ///
    /// A seed that does not derive answers `false` rather than failing: this
    /// decides whether to update a cache, and a store whose identity is gone has
    /// a larger problem that every other path reports properly. Refusing to keep
    /// somebody else's record because our own seed is broken would be the wrong
    /// place to notice.
    fn is_own(&self, author: &PerNetworkIdentityId) -> bool {
        self.own
            .get_or_init(|| self.identity().map(|id| id.id()).ok())
            .as_ref()
            .is_some_and(|own| own == author)
    }

    /// What this member has written in a channel, as far as a writer needs.
    ///
    /// The three questions a send asks — the newest reading, the newest
    /// `Message` reading, and how many of a class fall in the trailing minute —
    /// answered without replaying anything. `readings::OwnReadings` carries why
    /// that matters; in short, each of them used to be a full scan of the
    /// channel's record directory, so a send was linear in everything this
    /// member had ever written and three times over.
    ///
    /// **A cache, with the records still the source of truth.** Missing or
    /// unreadable, it is rebuilt from them and written back — which is also what
    /// happens once for every channel a store already holds, since nothing wrote
    /// this file before it existed.
    pub fn own_readings(&self, channel: &ChannelId) -> Result<OwnReadings, StoreError> {
        let path = self.channel_dir(channel).join("own-readings");
        if let Ok(text) = fs::read_to_string(&path)
            && let Some(readings) = OwnReadings::decode(&text)
        {
            return Ok(readings);
        }

        let author = self.identity()?.id();
        let rebuilt = OwnReadings::of(&self.own_records(channel, &author)?);
        self.write_readings(channel, &rebuilt)?;
        Ok(rebuilt)
    }

    /// Folds one of this member's records into the index.
    fn note_own_reading(&self, channel: &ChannelId, record: &Record) -> Result<(), StoreError> {
        let mut readings = self.own_readings(channel)?;
        readings.note(record.hlc, record.body.class());
        self.write_readings(channel, &readings)
    }

    /// The newest reading this node has published a log through, per channel.
    ///
    /// What lets a tick tell "nothing has changed here" from "I have not looked",
    /// which is the difference between skipping a channel and skipping a publish
    /// that was owed. Absent means the second.
    pub fn published_through(&self, channel: &ChannelId) -> Option<Hlc> {
        let text = fs::read_to_string(self.channel_dir(channel).join("published-through")).ok()?;
        let mut parts = text.split_whitespace();
        let wall = parts.next()?.parse().ok()?;
        let counter = parts.next()?.parse().ok()?;
        Some(Hlc::new(wall, counter))
    }

    /// Records that this channel's log is published through `reading`.
    pub fn set_published_through(
        &self,
        channel: &ChannelId,
        reading: Hlc,
    ) -> Result<(), StoreError> {
        let dir = self.channel_dir(channel);
        fs::create_dir_all(&dir)?;
        write_atomically(
            &self.root,
            dir.join("published-through"),
            format!("{} {}", reading.wall_millis, reading.counter).as_bytes(),
        )
    }

    /// Forgets when logs were last announced, so the next pass announces them.
    ///
    /// **Called once when a node starts, because an announcement is live state
    /// and the record of it is not.** Chunks survive a restart — they are on this
    /// disk — but the provider records naming this node as a holder live in the
    /// DHT and in a swarm that has just been replaced. A node that trusted a
    /// persisted "announced an hour ago" would come back holding content nobody
    /// could find, and would look entirely healthy doing it.
    pub fn forget_announcements(&self) -> Result<(), StoreError> {
        let channels = self.root.join("channels");
        if !channels.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(&channels)? {
            self.did(1, 0, 0);
            let path = entry?.path().join("announced");
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    /// When this node last announced a channel's log, in wall milliseconds.
    pub fn last_announced(&self, channel: &ChannelId) -> Option<i64> {
        fs::read_to_string(self.channel_dir(channel).join("announced"))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// Records that a channel's log was announced at `now`.
    pub fn set_last_announced(&self, channel: &ChannelId, now: i64) -> Result<(), StoreError> {
        let dir = self.channel_dir(channel);
        fs::create_dir_all(&dir)?;
        write_atomically(&self.root, dir.join("announced"), now.to_string().as_bytes())
    }

    fn write_readings(&self, channel: &ChannelId, readings: &OwnReadings) -> Result<(), StoreError> {
        let dir = self.channel_dir(channel);
        fs::create_dir_all(&dir)?;
        write_atomically(&self.root, dir.join("own-readings"), readings.encode().as_bytes())
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
        fs::create_dir_all(self.root.join("chunks"))?;
        let path = self.chunk_path(cid);
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
            self.did(1, 0, 0);
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
            self.did(1, 0, 0);
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
            self.did(1, 0, 0);
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

    /// Records what this machine contributes to this network — Core §4.3.
    ///
    /// One file rather than four, written whole. The four values are set
    /// together by one command, so splitting them would create three ways for a
    /// process that died mid-write to leave a contribution nobody chose.
    pub fn set_contribution(&self, offer: &Contribution) -> Result<(), StoreError> {
        let text = format!(
            "{}\n{}\n{}\n{}\n",
            offer.storage_offered,
            offer.upload_offered,
            offer.download_offered,
            u8::from(offer.relay_willing),
        );
        write_atomically(
            &self.root,
            self.root.join("contribution"),
            text.as_bytes(),
        )
    }

    /// What this machine contributes here, or `None` if nobody has said.
    ///
    /// **`None` and a zeroed offer are different answers and the caller must
    /// keep them apart.** Nothing set means this node has never been asked and
    /// takes whatever defaults the daemon ships; zeros are a member having said
    /// *contribute nothing*, which is a decision rather than an absence. A
    /// reader that collapsed them would quietly restore a default over somebody
    /// opting out — the same mistake spec 07 §2.8's sentinel rule exists to
    /// prevent for retention.
    ///
    /// A file that does not parse whole reads as unset rather than as partly
    /// set, for the same reason: a corrupt value must not be read as the
    /// strongest opinion a member could have expressed.
    pub fn contribution(&self) -> Option<Contribution> {
        let text = fs::read_to_string(self.root.join("contribution")).ok()?;
        let mut lines = text.lines();
        let mut next = || lines.next()?.trim().parse::<u64>().ok();
        let storage_offered = next()?;
        let upload_offered = next()?;
        let download_offered = next()?;
        let relay_willing = next()? != 0;
        Some(Contribution {
            storage_offered,
            upload_offered,
            download_offered,
            relay_willing,
        })
    }

    /// Records that this node has been confirmed reachable from outside.
    ///
    /// Written by the daemon, which is the only thing that can know it, and read
    /// by the interface to decide whether volunteering as a bootstrap relay is
    /// worth *offering*. A relay's job is being dialable by two peers who cannot
    /// dial each other (Core §5.5), so this is the difference between a real
    /// option and one that advertises something the node cannot do.
    pub fn set_reachable(&self, address: &str) -> Result<(), StoreError> {
        write_atomically(&self.root, self.root.join("reachable"), address.as_bytes())
    }

    /// The external address this node was last confirmed reachable on.
    ///
    /// **`None` means *not confirmed*, which is weaker than *not reachable*.** A
    /// node that has just started, or has met nobody who could tell it, has no
    /// confirmation yet and may be perfectly reachable. An interface must say
    /// the weaker thing — the same distinction `design/09` §4.1 draws between
    /// having heard from a member and their being offline.
    pub fn reachable(&self) -> Option<String> {
        fs::read_to_string(self.root.join("reachable"))
            .ok()
            .filter(|address| !address.trim().is_empty())
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

    /// What this member has chosen to tell the network about themselves.
    ///
    /// **Persisted, unlike the beats themselves**, and the asymmetry is the
    /// point. `design/01` §9 makes presence ephemeral and dropped on restart —
    /// that is about what has been *heard*, which is an observation and goes
    /// stale. A member's own choice is a setting, and one of its values is
    /// invisible: a choice that did not survive a restart would put somebody
    /// back on the network's roster the next time they opened the application,
    /// which is the one failure this setting exists to prevent.
    ///
    /// `None` means never chosen, which the caller reads as the default rather
    /// than as invisible — the safe direction here is the ordinary one, since a
    /// member who has not chosen is not asking to hide.
    pub fn set_presence(&self, choice: kols_core::Presence) -> Result<(), StoreError> {
        write_atomically(
            &self.root,
            self.root.join("presence"),
            choice.name().as_bytes(),
        )?;
        Ok(())
    }

    /// What this member last chose, or `None` if they never have.
    ///
    /// An unreadable or unrecognised value reads as `None` rather than as
    /// anything in particular. The alternative is guessing, and every guess here
    /// is either publishing somebody who asked to hide or hiding somebody who
    /// did not ask to.
    pub fn presence(&self) -> Option<kols_core::Presence> {
        let raw = fs::read_to_string(self.root.join("presence")).ok()?;
        kols_core::Presence::from_name(raw.trim())
    }

    /// Records a beat this node heard, replacing any earlier one from that member.
    ///
    /// `heard_at` is **this node's** clock rather than the sender's, because
    /// that is what freshness is judged against (`kols_core::PresenceBeat`).
    pub fn record_beat(
        &self,
        who: &str,
        state: kols_core::Beat,
        heard_at: i64,
    ) -> Result<(), StoreError> {
        let mut heard: std::collections::BTreeMap<String, (String, i64)> = self
            .beats()
            .into_iter()
            .map(|(who, state, at)| (who, (state.name().to_owned(), at)))
            .collect();
        heard.insert(who.to_owned(), (state.name().to_owned(), heard_at));
        let text = heard
            .iter()
            .map(|(who, (state, at))| format!("{who} {state} {at}"))
            .collect::<Vec<_>>()
            .join("\n");
        write_atomically(&self.root, self.root.join("beats"), text.as_bytes())?;
        Ok(())
    }

    /// The beats this node has heard, with when it heard each.
    ///
    /// Freshness is **not** applied here: this returns observations, and how old
    /// an observation may be before it stops meaning anything is a question for
    /// whoever is answering it. A store that filtered would make "heard nothing
    /// recently" and "heard nothing ever" indistinguishable at exactly the layer
    /// that still has both.
    pub fn beats(&self) -> Vec<(String, kols_core::Beat, i64)> {
        let Ok(text) = fs::read_to_string(self.root.join("beats")) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let who = parts.next()?.to_owned();
                let state = kols_core::Presence::from_name(parts.next()?)?.to_beat()?;
                let at = parts.next()?.parse().ok()?;
                Some((who, state, at))
            })
            .collect()
    }

    /// Forgets every beat heard, which is what a daemon does when it starts.
    ///
    /// **Presence is ephemeral and dropped on restart** (`design/01` §9). It
    /// reaches the interface through a file only because the daemon and the
    /// executor are different processes here; that is a transport detail, and
    /// leaving yesterday's roster on disk would turn it into a claim.
    pub fn forget_beats(&self) -> Result<(), StoreError> {
        match fs::remove_file(self.root.join("beats")) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(StoreError::Io(err)),
        }
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
            self.did(1, 0, 0);
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
        self.write_segment_mark(cid, "link", &raw)?;
        // What makes the cached answer above safe to hold: the one place a link
        // appears is the one place the tally moves.
        append_to(self.segment_tally_path(), &[0])
    }

    /// Every segment this node holds, as the CIDs its links were written under.
    ///
    /// Read from the `.link` marks rather than from the chunk store, because a
    /// link is written exactly when a segment's records were stored — so this is
    /// the set of segments this node has actually absorbed, not the set of
    /// chunks it happens to have bytes for.
    pub fn segments(&self) -> Vec<Cid> {
        let Ok(entries) = fs::read_dir(self.root.join("segments")) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .inspect(|_| self.did(1, 0, 0))
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let hex = name.strip_suffix(".link")?;
                let bytes: [u8; 32] = intranet_crypto::from_hex(hex)?.try_into().ok()?;
                Some(Cid::from_hash(Hash::from_bytes(bytes)))
            })
            .collect()
    }

    /// Records that a member asked for history older than `before` in a channel.
    ///
    /// **A want rather than a fetch, because the two live on different sides.**
    /// The executor answers commands and holds no node; the daemon holds the node
    /// and answers to nobody. So asking is a durable note one writes and the
    /// other reads on its next tick — the same shape the waiting room already
    /// uses in the opposite direction.
    ///
    /// One outstanding want per channel. A member scrolling repeatedly is asking
    /// for the same thing further back, not for several different things.
    pub fn want_history(&self, channel: &ChannelId, before: Hlc) -> Result<(), StoreError> {
        let dir = self.root.join("wants");
        fs::create_dir_all(&dir)?;
        write_atomically(
            &self.root,
            dir.join(to_hex(channel.as_bytes())),
            &{
                let mut raw = before.wall_millis.to_be_bytes().to_vec();
                raw.extend_from_slice(&before.counter.to_be_bytes());
                raw
            },
        )
    }

    /// Channels a member has asked for older history in.
    pub fn wants(&self) -> Vec<(ChannelId, Hlc)> {
        let Ok(entries) = fs::read_dir(self.root.join("wants")) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .inspect(|_| self.did(1, 0, 0))
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let bytes: [u8; 32] = intranet_crypto::from_hex(&name)?.try_into().ok()?;
                let raw = fs::read(entry.path()).ok()?;
                let (wall, counter) = raw.split_at_checked(8)?;
                let before = Hlc::new(
                    i64::from_be_bytes(wall.try_into().ok()?),
                    u32::from_be_bytes(counter.try_into().ok()?),
                );
                Some((ChannelId::from_bytes(bytes), before))
            })
            .collect()
    }

    /// Forgets a want, once it is satisfied or cannot be.
    pub fn forget_want(&self, channel: &ChannelId) -> Result<(), StoreError> {
        match fs::remove_file(self.root.join("wants").join(to_hex(channel.as_bytes()))) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(StoreError::Io(err)),
        }
    }

    /// Marks a segment as one a member actually asked to have here.
    ///
    /// **This is the pin, and without it the ceiling eats its own tail.**
    /// Shedding drops the oldest cached history first, which is exactly what
    /// somebody scrolling back has just asked for — so a member at their ceiling
    /// would fetch a page and have it thrown away before they read it, forever.
    /// A segment somebody asked for is not cold, whatever its age says.
    pub fn mark_wanted(&self, cid: &Cid, now: i64) -> Result<(), StoreError> {
        self.write_segment_mark(cid, "wanted", &now.to_be_bytes())
    }

    /// When a member last asked to have this segment here, if they ever did.
    pub fn wanted_at(&self, cid: &Cid) -> Option<i64> {
        let raw = fs::read(self.segment_path(cid, "wanted")).ok()?;
        Some(i64::from_be_bytes(raw.try_into().ok()?))
    }

    /// Notes that this node may be the last holder of something it must give up.
    ///
    /// **Persisted, because the grace window is measured in days and a process
    /// is not.** A clock kept in memory restarts with the node, so an
    /// installation that is restarted daily would hold at-risk content forever
    /// and never tell anybody it was stuck — which is the failure mode this
    /// window exists to make impossible.
    pub fn mark_at_risk(&self, cid: &Cid, now: i64) -> Result<(), StoreError> {
        if self.at_risk_since(cid).is_some() {
            // The window starts when the object first became at risk, not when
            // it was last looked at — otherwise a node that checks every tick
            // resets the clock every tick and the deadline never arrives.
            return Ok(());
        }
        self.write_segment_mark(cid, "at-risk", &now.to_be_bytes())
    }

    /// When this object was first seen to be the last known copy.
    pub fn at_risk_since(&self, cid: &Cid) -> Option<i64> {
        let raw = fs::read(self.segment_path(cid, "at-risk")).ok()?;
        Some(i64::from_be_bytes(raw.try_into().ok()?))
    }

    /// Forgets that an object was at risk, once somebody else holds it.
    pub fn clear_at_risk(&self, cid: &Cid) -> Result<(), StoreError> {
        match fs::remove_file(self.segment_path(cid, "at-risk")) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(StoreError::Io(err)),
        }
    }

    /// Records that content was given up, and when.
    ///
    /// **A member who was told and did nothing has made a choice; a member who
    /// was never told has had one made for them.** This is the difference,
    /// written down: an append-only note of what this node let go, so the
    /// question "what happened to that" has an answer that is not a shrug.
    pub fn record_dropped(&self, cid: &Cid, now: i64) -> Result<(), StoreError> {
        let path = self.root.join("dropped");
        let mut log = fs::read_to_string(&path).unwrap_or_default();
        log.push_str(&format!("{now} {}\n", to_hex(cid.hash().as_bytes())));
        write_atomically(&self.root, path, log.as_bytes())
    }

    /// What this node has given up, newest last.
    pub fn dropped(&self) -> Vec<(i64, String)> {
        fs::read_to_string(self.root.join("dropped"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let (at, cid) = line.split_once(' ')?;
                Some((at.parse().ok()?, cid.to_owned()))
            })
            .collect()
    }

    /// Whether any held chain runs back into history this node does not hold.
    ///
    /// True when some segment names a predecessor there is no link for — which is
    /// exactly the shape of a walk that stopped, whether because the chain is
    /// still arriving or because the ceiling stopped it.
    ///
    /// **This is the difference between a short channel and a bounded one**, and
    /// nothing else on screen distinguishes them. A member at their ceiling sees
    /// fewer messages than another member of the same network, and without this
    /// they would have no reason to think anything but that the network is quiet.
    pub fn history_incomplete(&self, channel: &ChannelId) -> bool {
        // **One `stat` when nothing has been absorbed**, which is the case on
        // every tick of every open channel. The answer costs a `.link` and a
        // `.channel` read per held segment, and segments accumulate for as long
        // as a channel has history — so asking it afresh each time is the same
        // unbounded shape as listing the records was.
        //
        // It is sound to cache against this one number because the answer
        // depends only on which `.link` marks exist, and those are written in
        // exactly one place. Shedding does not remove them, deliberately: a
        // member who gave up a servable copy has not lost history, and the links
        // still describe a whole chain.
        let tally = self.segment_tally();
        let mut seen = self
            .segments_seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((at, answers)) = seen.as_ref()
            && *at == tally
            && let Some(answer) = answers.get(channel)
        {
            return *answer;
        }

        let held: std::collections::BTreeSet<Cid> = self.segments().into_iter().collect();
        // The listing itself is charged by `segments`; this is the two mark
        // reads per segment that answering the question costs.
        self.did(0, 2 * held.len() as u64, 0);
        let answer = held
            .iter()
            .filter(|cid| self.segment_channel(cid).is_none_or(|held| held == *channel))
            .any(|cid| {
                self.segment_link(cid)
                    .and_then(|(_, previous)| previous)
                    .is_some_and(|previous| !held.contains(&previous))
            });

        match seen.as_mut() {
            Some((at, answers)) if *at == tally => {
                answers.insert(*channel, answer);
            }
            _ => {
                *seen = Some((tally, [(*channel, answer)].into_iter().collect()));
            }
        }
        answer
    }

    /// Where the segment tally lives.
    fn segment_tally_path(&self) -> PathBuf {
        self.root.join("segments.tally")
    }

    /// How many links have ever been written, in one `stat`.
    fn segment_tally(&self) -> u64 {
        length_of(self.segment_tally_path())
    }

    /// Which channel a segment was absorbed for, when that was recorded.
    ///
    /// **Absent means it counts toward every channel**, which is the answer this
    /// question gave before it was asked per channel at all. The degradation is
    /// one-directional on purpose (`design/09` §4.4): it can call a whole
    /// channel incomplete, which is merely vague and is what already happened,
    /// and it can never call a truncated one whole — which would be the
    /// interface concealing the very thing the notice exists to disclose.
    ///
    /// Its own mark rather than a field on the link, because extending the
    /// link's layout would make a 40-byte mark ambiguous between an old one
    /// carrying a predecessor and a new one carrying a channel. A separate file
    /// is unambiguous by construction and needs no version to read.
    pub fn segment_channel(&self, cid: &Cid) -> Option<ChannelId> {
        let raw = fs::read(self.segment_path(cid, "channel")).ok()?;
        Some(ChannelId::from_bytes(raw.try_into().ok()?))
    }

    /// Records which channel a segment belongs to.
    pub fn mark_segment_channel(
        &self,
        cid: &Cid,
        channel: &ChannelId,
    ) -> Result<(), StoreError> {
        self.write_segment_mark(cid, "channel", channel.as_bytes())
    }

    /// Whether this node holds `cid` on the network's behalf rather than its own.
    ///
    /// The distinction `design/02` §6.4 rests on: duty is what this machine gives
    /// other members, and a cached copy is what it fetched to read. One object
    /// commonly is both, and the reasons are kept apart so that withdrawing a
    /// contribution never drops bytes somebody is still reading.
    pub fn has_duty(&self, cid: &Cid) -> bool {
        self.segment_path(cid, "duty").exists()
    }

    /// Records that this node holds `cid` on the network's behalf, and its weight.
    ///
    /// **The size is written once, at the moment duty is taken, rather than
    /// recomputed.** Summing the tier then costs one small read per object
    /// instead of parsing every manifest and stat-ing every chunk on every tick.
    /// It is safe to fix it here because an object is whole when it is marked: a
    /// segment is only absorbed after its chunks decoded, so there is no state in
    /// which this records a partial weight that would later grow.
    pub fn take_duty(&self, cid: &Cid, bytes: u64) -> Result<(), StoreError> {
        self.write_segment_mark(cid, "duty", &bytes.to_be_bytes())
    }

    /// What this node holds for the network, in bytes.
    ///
    /// Read from the marks rather than the disk, per [`Store::take_duty`]. A mark
    /// that does not parse contributes nothing rather than aborting the sum — a
    /// tier total is a number a member reads, and refusing to produce one because
    /// a single mark is unreadable would replace a slightly wrong figure with no
    /// figure at all.
    pub fn duty_bytes(&self) -> u64 {
        let Ok(entries) = fs::read_dir(self.root.join("segments")) else {
            return 0;
        };
        entries
            .filter_map(Result::ok)
            .inspect(|_| self.did(1, 0, 0))
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.ends_with(".duty"))
            })
            .filter_map(|entry| {
                let raw = fs::read(entry.path()).ok()?;
                Some(u64::from_be_bytes(raw.try_into().ok()?))
            })
            .sum()
    }

    /// What one content object weighs here: its manifest plus the chunks it names.
    ///
    /// `None` when the manifest is not held or does not parse, which is the same
    /// answer as "this node cannot say" and is never zero — a caller must not
    /// read an unknown weight as a free one.
    ///
    /// **No chunk is counted twice across objects**, and that is a property of
    /// the design rather than luck: every segment lives under its own pointer and
    /// therefore its own DEK (`design/01` §3.1.0), and chunk encryption is
    /// deterministic per (chunk, DEK) — so two segments never produce the same
    /// chunk id even for identical text. Summing per object is exact.
    pub fn object_bytes(&self, manifest: &Cid) -> Option<u64> {
        let raw = fs::read(self.chunk_path(manifest)).ok()?;
        let total: u64 = raw.len() as u64;
        let manifest = intranet_storage::Manifest::from_bytes(&raw).ok()?;
        Some(
            manifest
                .chunks
                .iter()
                .filter_map(|cid| fs::metadata(self.chunk_path(cid)).ok())
                .map(|meta| meta.len())
                .sum::<u64>()
                + total,
        )
    }

    /// Every byte this network is costing this disk.
    ///
    /// **Everything under the store, not just `chunks/`** — corrected 2026-09-08,
    /// and the correction roughly doubles the number. This store keeps each
    /// message twice by design: once as a record under `channels/`, which is what
    /// rendering reads, and once inside a segment's chunks, which is what this
    /// node serves and re-verifies from. Counting only the second meant a member
    /// who set a two-gigabyte ceiling could be handed four, which is the one
    /// direction an absolute ceiling must never be wrong in.
    ///
    /// Deliberately not the duty tier, either: this includes what the member
    /// fetched to read, the governance log, and the superseded versions of head
    /// segments. A duty figure must not be inflated by any of that, and a disk is
    /// filled by all of it.
    ///
    /// **Walking a directory tree is not free**, and this is read on the daemon's
    /// tick. Callers are expected to hold the answer for a while rather than ask
    /// per pass; `serve` does.
    pub fn stored_bytes(&self) -> u64 {
        fn walk(path: &Path) -> u64 {
            let Ok(entries) = fs::read_dir(path) else {
                return 0;
            };
            entries
                .filter_map(Result::ok)
                .map(|entry| match entry.metadata() {
                    Ok(meta) if meta.is_dir() => walk(&entry.path()),
                    Ok(meta) => meta.len(),
                    Err(_) => 0,
                })
                .sum()
        }
        walk(&self.root)
    }

    /// Removes a content object's bytes, keeping everything else about it.
    ///
    /// **The records stay.** They are what rendering reads, and they are this
    /// member's own history rather than something held for the network — so
    /// dropping the servable copy costs this node the ability to *serve* the
    /// object and costs the member nothing they can see. That asymmetry is the
    /// reason this is the first thing shed under pressure.
    ///
    /// Safe only for **sealed** segments. A head is republished on every append
    /// and successive versions share every chunk but the tail, so deleting an
    /// old head's chunks would take the current one's with them.
    /// Returns the ids it removed, so the caller can stop announcing them.
    ///
    /// **That return value is not a convenience.** A node that drops bytes and
    /// goes on advertising itself as a provider sends every peer that believes
    /// it on a fetch that fails — and a failed fetch counts against the *serving*
    /// node's reliability with whoever asked (Storage §4.4). Dropping content
    /// and staying quiet about it is worse than not dropping it.
    ///
    /// The **link stays**: it is how a walk knows this segment's place in its
    /// chain without re-fetching, and losing it would make a chain look shorter
    /// than it is. The **records stay** for the same reason spelled out above.
    /// What goes is only the servable copy.
    pub fn forget_object(&self, manifest: &Cid) -> Result<Vec<Cid>, StoreError> {
        let mut gone = Vec::new();
        if let Ok(raw) = fs::read(self.chunk_path(manifest))
            && let Ok(parsed) = intranet_storage::Manifest::from_bytes(&raw)
        {
            for chunk in &parsed.chunks {
                if fs::remove_file(self.chunk_path(chunk)).is_ok() {
                    gone.push(*chunk);
                }
            }
        }
        if fs::remove_file(self.chunk_path(manifest)).is_ok() {
            gone.push(*manifest);
        }
        let _ = fs::remove_file(self.segment_path(manifest, "duty"));
        let _ = fs::remove_file(self.segment_path(manifest, "repair"));
        Ok(gone)
    }

    fn chunk_path(&self, cid: &Cid) -> PathBuf {
        self.root.join("chunks").join(to_hex(cid.hash().as_bytes()))
    }

    /// Withdraws duty for `cid`, leaving the bytes alone.
    ///
    /// **Removing the reason is not removing the object.** If this member also
    /// fetched it to read, it stays held under that reason — which is the whole
    /// point of the split, and the reason lowering a contribution can never take
    /// away somebody's own history.
    pub fn release_duty(&self, cid: &Cid) -> Result<(), StoreError> {
        // The reason a duty was taken goes with the duty. A repair mark left
        // behind would make a later re-adoption look like it had never been
        // given back, and the two marks only ever mean anything together.
        self.clear_repair(cid)?;
        match fs::remove_file(self.segment_path(cid, "duty")) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(StoreError::Io(err)),
        }
    }

    /// Whether this node holds `cid` because **repair** asked it to, rather than
    /// because placement ranked it inside the replication factor.
    ///
    /// Both are duty and both count against the offer; the mark exists because
    /// they are given back under different rules. Placement duty ends the moment
    /// the ledger stops naming this node — somebody else is ranked for it now,
    /// and that somebody is by construction holding it. Repair duty ends only on
    /// evidence that the shortfall it was taken for has actually closed, because
    /// nothing else is ranked for it: this node volunteered precisely because the
    /// nodes that were ranked are not there.
    ///
    /// Without the distinction the next evaluation would release every repaired
    /// object on sight, since a standby is by definition outside the primary set.
    pub fn has_repair(&self, cid: &Cid) -> bool {
        self.segment_path(cid, "repair").exists()
    }

    /// Records that duty for `cid` was taken as repair.
    ///
    /// Written beside the duty mark rather than instead of it, so every existing
    /// reader — the tier total, eviction, shedding — treats a repaired object as
    /// exactly what it is, duty, with no second code path to keep in step.
    pub fn take_repair(&self, cid: &Cid) -> Result<(), StoreError> {
        self.write_segment_mark(cid, "repair", &[])
    }

    /// Forgets that duty for `cid` was repair, leaving the duty itself alone.
    ///
    /// Called when placement catches up and ranks this node for something it had
    /// volunteered for: the object stops being a repair and becomes ordinary
    /// duty, which is a promotion rather than a change of what is held.
    pub fn clear_repair(&self, cid: &Cid) -> Result<(), StoreError> {
        match fs::remove_file(self.segment_path(cid, "repair")) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(StoreError::Io(err)),
        }
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
/// Appends to a file that only ever grows, creating it if absent.
///
/// # The primitive three things here are built on
///
/// A reader has to answer *has anything changed* without looking at everything,
/// and it cannot hold the answer in memory: `serve` opens its **own** `Store` on
/// the same directory, so a counter in one handle never sees the other's writes.
/// The signal has to go through the filesystem and cost one `stat`.
///
/// An append-only file's **length** is that signal. It is monotonic, it is read
/// without opening the file, and — unlike a counter written by
/// read-modify-write — two writers cannot lose one another's update, which is
/// the failure that would matter: a marker that returned to a value a reader had
/// already seen would make that reader skip a record for good.
///
/// The weaker guarantee, stated: this relies on `O_APPEND` making a small write
/// atomic against other appenders. That holds on the local filesystems this runs
/// on and is not promised by POSIX for arbitrary sizes. It is a strictly smaller
/// assumption than the atomic `rename` below, which the whole store already
/// rests on.
fn append_to(path: impl AsRef<Path>, bytes: &[u8]) -> Result<(), StoreError> {
    use std::io::Write;
    if let Some(parent) = path.as_ref().parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

/// How long that file is, or zero when it is not there yet.
///
/// One `stat`, whatever the file holds. Absent reads as zero rather than
/// failing: a store written before this existed has nothing appended yet, and
/// the honest answer for it is *nothing has been marked*, which sends every
/// caller down the rebuild path they would have taken anyway.
fn length_of(path: impl AsRef<Path>) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

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
/// **A holder suspended for longer than this — a laptop asleep — can still have
/// its claim taken over**, and that is correct rather than a defect: from the
/// store's side a sleeping process and a dead one are the same observation, and
/// waiting longer would only move the line. What used to make it a defect was
/// the *waking*, when the first process carried on believing it held a claim it
/// no longer had. [`NodeClaim::beat`] now checks whose claim it is refreshing
/// and reports [`Beat::Lost`], so the outcome is one node stopping rather than
/// two running.
pub const NODE_CLAIM_STALE: i64 = 6_000;

/// The right to run a node for one network.
///
/// Released on drop, and expiring on its own if the holder never gets to drop
/// it — see [`Store::hold_node`] for why both are needed.
pub struct NodeClaim {
    path: PathBuf,
    /// Which holder this is, so [`NodeClaim::beat`] can tell it is still the one.
    token: u64,
}

/// What a heartbeat found out about the claim it was refreshing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beat {
    /// Still ours. Carry on.
    Held,
    /// Somebody else holds this store's claim now, and this node must stop.
    ///
    /// The only way to reach this is to have been suspended past
    /// [`NODE_CLAIM_STALE`] — a laptop asleep — while another process started a
    /// node for the same network. Both would otherwise advance the same MLS
    /// group without seeing each other, which is the failure the claim exists
    /// to prevent and the one with no symptom at the moment it happens.
    Lost,
}

impl NodeClaim {
    /// Says the holder is still running, and checks it still is.
    ///
    /// Called from the node's own loop, so a claim outlives the process holding
    /// it by at most [`NODE_CLAIM_STALE`].
    ///
    /// # Why this reads before it writes
    ///
    /// The expiry above is a wall-clock rule, and wall-clock cannot tell a dead
    /// process from a suspended one: a laptop asleep for a minute looks exactly
    /// like a crash. Its claim goes stale, another process legitimately takes
    /// it over, and on waking the first process would have gone on beating and
    /// running — two nodes, one network, no symptom, until whichever saved its
    /// group state last silently decided the network's key.
    ///
    /// So the beat asks whose claim this is rather than only asserting that
    /// somebody is alive. Losing it is not an error to recover from: this node
    /// has to stop, because the other one is now the holder and is right to be.
    #[must_use]
    pub fn beat(&self) -> Beat {
        if self.owner_on_disk() != Some(self.token) {
            return Beat::Lost;
        }
        // Atomic like every other durable write, and for a sharper reason than
        // most: a half-written heartbeat does not parse, an unparseable one
        // reads as *stale*, and a stale claim is one another process may take
        // over while this one is still running. The window is one tick wide and
        // self-healing, and it is the one direction of failure this file must
        // not have.
        if let Some(root) = self.path.parent() {
            let _ = write_atomically(
                root,
                self.path.join("heartbeat"),
                now_millis().to_string().as_bytes(),
            );
        }
        Beat::Held
    }

    /// Records which holder this is.
    fn write_owner(&self) -> Result<(), StoreError> {
        let root = self
            .path
            .parent()
            .ok_or_else(|| StoreError::Corrupt("a claim with no store above it".to_owned()))?;
        write_atomically(
            root,
            self.path.join("owner"),
            self.token.to_string().as_bytes(),
        )
    }

    /// Whose claim the store currently says this is.
    fn owner_on_disk(&self) -> Option<u64> {
        fs::read_to_string(self.path.join("owner"))
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok())
    }
}

impl Drop for NodeClaim {
    /// Releases the claim, **unless somebody else has taken it over.**
    ///
    /// The check is not tidiness. Without it, a node that lost its claim while
    /// suspended would delete the *successor's* heartbeat and directory on its
    /// way out — turning one recoverable problem into a store that looks
    /// unclaimed while a node is actively running against it, which is the
    /// exact state the claim exists to make impossible.
    fn drop(&mut self) {
        if self.owner_on_disk() != Some(self.token) {
            return;
        }
        let _ = fs::remove_file(self.path.join("heartbeat"));
        let _ = fs::remove_file(self.path.join("owner"));
        let _ = fs::remove_dir(&self.path);
    }
}

/// A value distinguishing this holder from any other.
///
/// Not a pid, for the reason [`Store::hold_node`] gives about pids generally:
/// they are reused, so a stale one can name a live process that is somebody
/// else. This only has to be unlikely to repeat, and it is never compared
/// across machines.
fn claim_token() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_i64(now_millis());
    hasher.write_usize(std::process::id() as usize);
    hasher.finish()
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
