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
use kols_core::NetworkProfile;
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

/// A relay one of this member's other networks already designates.
///
/// What [`Workspace::shared_relays`] found, carrying enough to name the other
/// network to the person being warned — an id alone would tell them a relay is
/// shared with something they cannot identify.
#[derive(Debug, Clone)]
pub struct SharedRelay {
    /// The address as the network being changed names it.
    ///
    /// Not as the other network names it: the same relay is reachable at several
    /// addresses, and echoing back what somebody just typed is what lets them
    /// see which of their own entries is the one being warned about.
    pub relay: String,
    /// The other network's local label, which may be empty.
    pub label: String,
    /// The other network's id, as hex.
    pub id: String,
}

/// The most disk one installation gives every network together, by default.
///
/// A starting value rather than a considered one, and modest on purpose: a
/// member who never opens this setting should find the application costing them
/// something they would not have minded being asked about. Raising it is one
/// number, and the surface says what it is for.
pub const DEFAULT_CEILING: u64 = 2 * 1024 * 1024 * 1024;

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

    /// The most disk this installation may use, across every network.
    ///
    /// # Why this is not per network, and not a `Command`
    ///
    /// A disk does not know how many networks are on it. Per-network offers
    /// bound what each one *gives*, and bound the total at nothing — which stops
    /// being an academic point at P2, where every direct message is its own
    /// network (`03` §4) and a member with thirty conversations would hold thirty
    /// ceilings and no answer to "how much is this costing me".
    ///
    /// It is set outside the `kols-api` vocabulary for the reason creating a
    /// network is (`design/05` §3): that boundary is per network, and this is a
    /// fact about the workspace holding all of them. A command routed through
    /// one network's executor to write a file above it would be borrowing an
    /// authority it does not have.
    pub fn ceiling(&self) -> u64 {
        std::fs::read_to_string(self.root.join("ceiling"))
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(DEFAULT_CEILING)
    }

    /// Sets the most disk this installation may use.
    pub fn set_ceiling(&self, bytes: u64) -> Result<(), String> {
        std::fs::create_dir_all(&self.root).map_err(|err| err.to_string())?;
        std::fs::write(self.root.join("ceiling"), bytes.to_string()).map_err(|err| err.to_string())
    }

    /// What every network here is costing this disk, together.
    ///
    /// Summed across stores rather than tracked, because a total kept as a
    /// running number is a number that drifts: a store removed by hand, a
    /// network forgotten, or a write that failed halfway all leave it wrong in
    /// the direction that matters, which is believing there is room.
    pub fn stored_bytes(&self) -> u64 {
        self.list()
            .into_iter()
            .filter_map(|known| Store::open(known.path).ok())
            .map(|store| store.stored_bytes())
            .sum()
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

    /// Which of `relays` another of this member's networks already designates.
    ///
    /// D29, and the concern is mechanical rather than stylistic: `kad` runs
    /// under libp2p's default protocol name and `PROTOCOL_VERSION` is one string
    /// for every network, so two networks meeting at one relay share a routing
    /// table and their members become mutually discoverable. That is the
    /// correlation Core §1.2 exists to prevent, arrived at without anybody
    /// attacking anything.
    ///
    /// **This reports; it never refuses** (`design/09` §3). Refusing was the
    /// earlier draft and was wrong twice over. It is unenforceable anyway —
    /// nothing stops a founder naming one address in two networks, and the relay
    /// cannot know it is being reused (Core §5.5: it replays no log and holds no
    /// capabilities) — so it would stop the honest case and not the determined
    /// one. And it would block a legitimate configuration: a member relaying on
    /// their own LAN for two of their own networks, where the private hop is the
    /// only way in for both.
    ///
    /// **The client is the only party in a position to notice at all.** The
    /// relay cannot see it, and neither network's other members can. Saying
    /// nothing was the third option and the worst of the three.
    ///
    /// `network` is the network being changed, excluded from the comparison
    /// because its own relays are not somebody else's. `None` is a network that
    /// does not exist yet, which is what creating one with a relay is.
    ///
    /// Compared by **peer id, never by address**. One relay answers at several
    /// addresses — a DNS name and a bare IPv4, TCP beside QUIC — so comparing
    /// the strings would report no overlap in exactly the case this exists for.
    ///
    /// Read from each store's cached relay list (`Store::relays`) rather than by
    /// replaying another network's log. The cache is what this installation
    /// actually knows, and replaying a log this member may not have synced would
    /// answer a question about a network when the question is about a disk.
    pub fn shared_relays(&self, network: Option<&NetworkId>, relays: &[String]) -> Vec<SharedRelay> {
        let mine = network.map(|id| to_hex(id.as_bytes()));
        let others: Vec<(Known, Vec<String>)> = self
            .list()
            .into_iter()
            .filter(|known| Some(&known.id) != mine.as_ref())
            .filter_map(|known| {
                let held = Store::open(known.path.clone()).ok()?.relays();
                (!held.is_empty()).then_some((known, held))
            })
            .collect();

        let mut shared = Vec::new();
        for address in relays {
            // An address naming no peer id is not compared rather than compared
            // as a string: `parse_relay` refuses one at both front ends, so this
            // is unreachable for anything designated through them, and guessing
            // would be worse than declining to answer.
            let Some(host) = relay_host(address) else {
                continue;
            };
            for (known, held) in &others {
                if held.iter().filter_map(|a| relay_host(a)).any(|it| it == host) {
                    shared.push(SharedRelay {
                        relay: address.clone(),
                        label: known.label.clone(),
                        id: known.id.clone(),
                    });
                }
            }
        }
        shared
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
        // **Anchored at the workspace, before any store exists.** The account
        // belongs to the installation rather than to a network, so whoever
        // touches it first must do so from here — a store reaching for one would
        // otherwise put it beside itself (`design/02` §6.3).
        self.ensure_unlocked()?;
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
        self.build(self.path_for(&id), id, label, relays, NetworkProfile::Server)
    }

    /// Creates a `conversation`-profile network — `design/03` §4.1, spec 07 §1.2.
    ///
    /// What a direct message is: one implied channel, no roles, no categories,
    /// and every participant a Founder. Only the first of those is settled at
    /// genesis; adding the other participant is admission and belongs to the DM
    /// flow (E10) rather than here.
    ///
    /// **It designates no relay, and that is D29 rather than an omission.** A
    /// relay is never shared between two of a member's networks, and a fresh
    /// two-person network has no infrastructure of its own — so a conversation
    /// reaches its peer over the connection the shared network already has
    /// (E13, `design/09` §3), never by naming a relay here.
    pub fn create_conversation(&self, label: &str) -> Result<Store, String> {
        // **Anchored at the workspace, before any store exists.** The account
        // belongs to the installation rather than to a network, so whoever
        // touches it first must do so from here — a store reaching for one would
        // otherwise put it beside itself (`design/02` §6.3).
        self.ensure_unlocked()?;
        if is_store(&self.root) {
            return Err(format!(
                "{} is already a single network's store. Point at a directory that holds \
                 several",
                self.root.display()
            ));
        }
        let id = NetworkId::from_bytes(crate::random_32()?);
        self.build(
            self.path_for(&id),
            id,
            label,
            Vec::new(),
            NetworkProfile::Conversation,
        )
    }

    /// Everything a bundle has to carry, for every network on this disk.
    ///
    /// Requires the installation to be unlocked, because it reads seeds — which
    /// is the whole of what it is for.
    pub fn to_bundle(&self) -> Result<Vec<crate::bundle::Entry>, String> {
        self.ensure_unlocked()?;
        Ok(self
            .list()
            .into_iter()
            .filter_map(|known| {
                let store = Store::open(known.path).ok()?;
                Some(crate::bundle::Entry {
                    network: *store.network(),
                    seed: store.entropy(),
                    label: store.label().unwrap_or_default(),
                    // **Without one of these there is nobody to sync from.** A
                    // restored store holds an identity and an empty log; the
                    // history comes back off the network, and a relay is how it
                    // is reached.
                    relays: store.relays(),
                })
            })
            .collect())
    }

    /// Restores networks from a bundle, and reports what it did.
    ///
    /// **A network already here is skipped, never overwritten**, for the reason
    /// a second `init` refuses: the seed at that path cannot be recovered if it
    /// is lost, and importing over it would replace one identity with another
    /// silently. Skipping can leave somebody as the wrong member in that
    /// network, which is visible and fixable; overwriting is neither.
    ///
    /// What is restored is an identity and somewhere to reach the network. The
    /// history is not in the bundle and does not need to be: a returning member
    /// is already named in the governance log, so their own messages come back
    /// off the network like any other history.
    pub fn from_bundle(&self, entries: &[crate::bundle::Entry]) -> Result<Restored, String> {
        self.ensure_unlocked()?;
        let mut restored = Restored::default();
        for entry in entries {
            let path = self.path_for(&entry.network);
            if is_store(&path) {
                restored.skipped.push(entry.label.clone());
                continue;
            }
            match Store::create(path, entry.network, entry.seed) {
                Ok(store) => {
                    let _ = store.set_label(&entry.label);
                    let _ = store.set_relays(&entry.relays);
                    restored.added.push(entry.label.clone());
                }
                // One network that will not restore must not stop the others.
                Err(err) => restored.refused.push(format!("{}: {err}", entry.label)),
            }
        }
        Ok(restored)
    }

    /// How many stores here still hold their seed in the clear.
    ///
    /// What the first run needs in order to say what it is about to protect. It
    /// reads a length rather than opening anything, so it works while the
    /// installation is still locked — which is the only moment it is asked.
    pub fn unprotected(&self) -> usize {
        let roots: Vec<PathBuf> = if is_store(&self.root) {
            vec![self.root.clone()]
        } else {
            std::fs::read_dir(&self.root)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| is_store(path))
                .collect()
        };
        roots
            .iter()
            .filter(|root| {
                std::fs::metadata(root.join("seed")).is_ok_and(|meta| meta.len() == 32)
            })
            .count()
    }

    /// Makes sure this process is unlocked for this workspace.
    ///
    /// The window unlocks by somebody logging in; this is the other path, which
    /// the terminal and the test suite take, and it reads the password from the
    /// environment. There is no third — a code path that opened a seed without a
    /// secret would be a second security posture, and D30 keeps the terminal a
    /// test harness rather than a second interface.
    pub fn ensure_unlocked(&self) -> Result<(), String> {
        let account = crate::account::for_workspace(&self.root).map_err(|err| err.to_string())?;
        self.adopt_all(&account);
        Ok(())
    }

    /// Wraps any seed on this disk that is still in the clear.
    ///
    /// **Runs on every unlock, and is safe to** — a seed already wrapped is left
    /// alone, decided by its length rather than by a marker that could fall out
    /// of step. So the migration `design/02` §6.3 requires is not a one-shot
    /// step somebody has to remember to run, and an installation that skipped a
    /// release still gets it on the next launch.
    ///
    /// Best effort per network. One store that will not open — a disk fault, a
    /// half-written directory — must not stop the others being protected, and
    /// the one that failed is no worse off than before.
    pub fn adopt_all(&self, account: &crate::account::Account) -> usize {
        let mut wrapped = 0;
        let roots: Vec<PathBuf> = if is_store(&self.root) {
            vec![self.root.clone()]
        } else {
            std::fs::read_dir(&self.root)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| is_store(path))
                .collect()
        };
        for root in roots {
            if let Ok(store) = Store::open(root)
                && store.adopt(account).unwrap_or(false)
            {
                wrapped += 1;
            }
        }
        wrapped
    }

    /// Where a network's store lives.
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
    ///
    /// `server` only, and deliberately: a conversation is created by the DM flow
    /// and never by somebody typing a command, so a terminal that could make one
    /// would be a surface for an act that has no meaning outside that flow.
    pub fn create_at(
        &self,
        path: PathBuf,
        label: &str,
        relays: Vec<String>,
    ) -> Result<Store, String> {
        let id = NetworkId::from_bytes(crate::random_32()?);
        self.build(path, id, label, relays, NetworkProfile::Server)
    }

    fn build(
        &self,
        path: PathBuf,
        id: NetworkId,
        label: &str,
        relays: Vec<String>,
        profile: NetworkProfile,
    ) -> Result<Store, String> {
        // The entropy is independent of the id: the id names the network, the
        // entropy derives this member's identity in it. Deriving one from the
        // other would make an identity a function of public information.
        let entropy = crate::random_32()?;
        let store = Store::create(path, id, entropy).map_err(|e| e.to_string())?;
        let founder = store.identity().map_err(|e| e.to_string())?;
        store
            .append_entry(&network::genesis(&founder, id, relays.clone(), label, profile))
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

/// What restoring a bundle did.
#[derive(Debug, Default)]
pub struct Restored {
    /// Networks this machine did not have and now does.
    pub added: Vec<String>,
    /// Networks already here, left exactly as they were.
    pub skipped: Vec<String>,
    /// Networks that would not restore, and why.
    pub refused: Vec<String>,
}

/// Whether a directory is a network's store.
///
/// The seed is the test because it is the one file every store has from the
/// moment it exists — a network with no log yet is a store that has attached and
/// not synced, which is an ordinary state.
fn is_store(path: &Path) -> bool {
    path.join("seed").is_file()
}

/// The peer id of the relay an address names, which is that relay's identity.
///
/// The **first** `/p2p/` component rather than the last, because a circuit
/// address names two peers — `…/p2p/<relay>/p2p-circuit/p2p/<target>` — and the
/// one being designated is the hop you dial. A plain relay address carries only
/// the one, so the same rule is the right answer for both shapes rather than a
/// special case for either.
fn relay_host(address: &str) -> Option<String> {
    address
        .parse::<libp2p::Multiaddr>()
        .ok()?
        .iter()
        .find_map(|part| match part {
            libp2p::multiaddr::Protocol::P2p(peer) => Some(peer.to_string()),
            _ => None,
        })
}

fn describe(store: &Store) -> Known {
    Known {
        id: to_hex(store.network().as_bytes()),
        label: store.label().unwrap_or_default(),
        path: store.root().to_path_buf(),
        keyed: store.epoch_key().is_ok(),
    }
}
