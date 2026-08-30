//! A directory of networks — `design/09` §1.
//!
//! # Why this exists
//!
//! A store is one network. A person belongs to several, and a direct message is
//! a network too (`design/03` §4), so "which network" is a question the client
//! has to be able to ask before it can do anything else. Until now `$KOLS_HOME`
//! *was* a store, which works for a terminal that is told which one to use with
//! `--home` and does not work for a window that has to offer a choice.
//!
//! # What it deliberately is not
//!
//! Not a node manager. Each network is a separate node with a separate peer id —
//! forced rather than chosen, since `keypair_for` derives the libp2p keypair from
//! the per-network identity, and one swarm across several networks would mean one
//! peer id correlating identities Core §1.2 keeps unlinkable (`design/09` §1).
//! Which of them are *running* is `design/09` §2's hot/warm/cold question and is
//! not answered here.
//!
//! # Creating a network lives here rather than in the terminal
//!
//! It was in `kols init`, which meant the window could not do it without a second
//! copy of three things that are easy to get subtly wrong: the genesis policy, the
//! chat capability registrations, and the check that replay produces a member.
//! One path, two front ends.

use crate::network;
use crate::store::Store;
use intranet_crypto::to_hex;
use intranet_identity::NetworkId;
use std::path::{Path, PathBuf};

/// A network this client knows about.
#[derive(Debug, Clone)]
pub struct Known {
    /// Its id, as hex.
    pub id: String,
    /// The local label its creator or joiner gave it.
    pub label: String,
    /// Where its store lives.
    pub path: PathBuf,
    /// Whether this node holds an epoch key for it.
    ///
    /// A network without one is joined and unreadable — the ordinary state
    /// between being admitted and being keyed in, which the interface should
    /// show as waiting rather than as broken.
    pub keyed: bool,
}

/// A directory holding several networks' stores.
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Opens the workspace at `root`, creating nothing.
    pub const fn at(root: PathBuf) -> Self {
        Self { root }
    }

    /// The default workspace: `$KOLS_HOME`, else `~/.kols`.
    pub fn default_root() -> PathBuf {
        Store::default_root()
    }

    /// Where this workspace lives.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every network this client holds a store for.
    ///
    /// Tolerates one legacy shape: a `$KOLS_HOME` that is *itself* a store rather
    /// than a directory of them, which is what `kols --home` has always meant and
    /// still does. A terminal points at one network; a window points at the
    /// directory holding many.
    pub fn list(&self) -> Vec<Known> {
        if is_store(&self.root) {
            return Store::open(self.root.clone())
                .ok()
                .map(|store| vec![describe(&store)])
                .unwrap_or_default();
        }

        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut networks: Vec<Known> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| is_store(path))
            .filter_map(|path| Store::open(path).ok())
            .map(|store| describe(&store))
            .collect();
        // Stable order, so a list does not reshuffle between renders for reasons
        // nobody can see.
        networks.sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
        networks
    }

    /// Opens one network by the start of its id.
    pub fn open(&self, prefix: &str) -> Result<Store, String> {
        let prefix = prefix.trim().to_ascii_lowercase();
        let matches: Vec<_> = self
            .list()
            .into_iter()
            .filter(|known| known.id.starts_with(&prefix))
            .collect();

        match matches.as_slice() {
            [one] => Store::open(one.path.clone()).map_err(|err| err.to_string()),
            [] => Err(format!("no network here starts with {prefix:?}")),
            many => Err(format!(
                "{} networks start with {prefix:?} — give more of the id",
                many.len()
            )),
        }
    }

    /// Creates a network, with this client as its sole Founder.
    ///
    /// The one place a network comes into being, so the genesis requirements are
    /// met once: `chat-log` on the content-type allowlist, the chat vocabulary
    /// registered, and `everyone` granted what a member needs. Each is silent
    /// when missed — the network looks fine until the first post is refused by
    /// its own author's node.
    pub fn create(&self, label: &str, relays: Vec<String>) -> Result<Store, String> {
        if is_store(&self.root) {
            return Err(format!(
                "{} is already a single network's store. Point at a directory that holds \
                 several, or use `kols --home` to work with that one",
                self.root.display()
            ));
        }
        // A prefix rather than the whole id: a directory name is something a
        // person reads and types, and the store inside it records the id in full.
        //
        // **Built here rather than through `create_at`, because that mints an id
        // of its own.** This used to name the directory after one id and then
        // hand the path to `create_at`, which generated a *second* — so the
        // directory named a network that did not exist and the network inside it
        // lived at a path nothing could derive. Latent until something looked a
        // store up by id, and then it defeated the guarantee `path_for` exists
        // for: joining a network you had already created would fail to find it,
        // make a second store and a second identity, and present as two members
        // who are two strangers.
        let id = NetworkId::from_bytes(crate::random_32()?);
        self.build(self.path_for(&id), id, label, relays)
    }

    /// Where a network's store belongs in this workspace.
    ///
    /// Derived from the id so joining the same network twice lands in the same
    /// place rather than making a second identity in it — which would look like
    /// two memberships and be two strangers.
    pub fn path_for(&self, network: &NetworkId) -> PathBuf {
        self.root.join(&to_hex(network.as_bytes())[..16])
    }

    /// Removes this installation's store for a network, permanently.
    ///
    /// # This is forgetting, not leaving, and the difference is not pedantry
    ///
    /// There is no way to leave a network. Membership is governance state, so
    /// resigning would mean holding `revoke-node` and revoking yourself, and the
    /// protocol has no notion of standing down (`design/02` §5 declines a
    /// hierarchy, which is the same absence seen from the other side). This
    /// deletes the local store: the network stops being one this installation
    /// holds, and the log every other member replays is untouched. To them
    /// nothing happened — you are still a member who has gone quiet.
    ///
    /// **The seed goes with it, and the seed is the identity** (`design/02`
    /// §6.3). Coming back needs the phrase, the network id and a relay, and this
    /// destroys the first of the three, so a later join arrives as a stranger
    /// rather than as the member the log already names. There is no undo and no
    /// recovery service; a caller that does not make that plain to somebody
    /// first has mis-stated what this does.
    ///
    /// Refuses while a node is running for the store, for the reason
    /// [`Store`] holds a claim at all: deleting live MLS state out from under a
    /// running node is the one way to lose a network's key material without any
    /// step reporting a failure.
    pub fn forget(&self, network: &NetworkId) -> Result<(), String> {
        // Found by asking each store what network it holds, rather than by
        // deriving the path from the id. Derivation is correct for anything
        // created or joined from now on, and wrong for every directory named
        // before `create` stopped minting a throwaway id for the name — and a
        // person cannot be asked to care which of those they have.
        let Some(known) = self
            .list()
            .into_iter()
            .find(|known| known.id == to_hex(network.as_bytes()))
        else {
            return Err("no store here for that network".to_owned());
        };
        let path = known.path;
        // Opened only to check the claim: `Store::open` is what knows whether a
        // node is heartbeating, and asking it is cheaper than duplicating the
        // rule and letting the two drift.
        if let Ok(store) = Store::open(path.clone())
            && store.is_being_served()
        {
            return Err(
                "a node is running for that network. Close it or switch away first — \
                 deleting a store while its node holds the key group is how key material \
                 goes missing with nothing reporting it"
                    .to_owned(),
            );
        }
        std::fs::remove_dir_all(&path).map_err(|err| format!("could not remove {path:?}: {err}"))
    }

    /// Creates a network at an exact path.
    ///
    /// What `kols init` uses, since `--home` names one store rather than a
    /// directory of them.
    pub fn create_at(
        &self,
        path: PathBuf,
        label: &str,
        relays: Vec<String>,
    ) -> Result<Store, String> {
        let id = NetworkId::from_bytes(crate::random_32()?);
        self.build(path, id, label, relays)
    }

    fn build(
        &self,
        path: PathBuf,
        id: NetworkId,
        label: &str,
        relays: Vec<String>,
    ) -> Result<Store, String> {
        // The entropy is independent of the id: the id names the network, the
        // entropy derives this member's identity in it. Deriving one from the
        // other would make an identity a function of public information.
        let entropy = crate::random_32()?;
        let store = Store::create(path, id, entropy).map_err(|e| e.to_string())?;
        let founder = store.identity().map_err(|e| e.to_string())?;
        store
            .append_entry(&network::genesis(&founder, id, relays.clone(), label))
            .map_err(|e| e.to_string())?;
        store.set_label(label).map_err(|e| e.to_string())?;
        store.set_relays(&relays).map_err(|e| e.to_string())?;

        // Replayed rather than trusted. A genesis this node cannot replay is a
        // network nobody can join, and finding that out now costs one line.
        let state = store.state().map_err(|e| e.to_string())?;
        if !state.is_member(&founder.id()) {
            return Err("genesis replayed but did not make its founder a member".to_owned());
        }
        Ok(store)
    }
}

/// Whether a directory is a network's store.
///
/// The seed is the test because it is the one file every store has from the
/// moment it exists — a network with no log yet is a store that has attached and
/// not synced, which is an ordinary state.
fn is_store(path: &Path) -> bool {
    path.join("seed").is_file()
}

fn describe(store: &Store) -> Known {
    Known {
        id: to_hex(store.network().as_bytes()),
        label: store.label().unwrap_or_default(),
        path: store.root().to_path_buf(),
        keyed: store.epoch_key().is_ok(),
    }
}
