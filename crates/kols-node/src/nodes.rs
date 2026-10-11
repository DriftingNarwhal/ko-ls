//! Running a node per network — `design/05` §4, `design/09` §2.
//!
//! A client belongs to several networks and a direct message *is* a network
//! (`03` §4), so "which node is running" stopped being a question with one
//! answer the moment conversations existed. This is the thing that runs more
//! than one.
//!
//! # Why this could not stay singular
//!
//! It did until now: the shell held one handle, described as "the node running
//! for the open network", and switching networks dropped the task for the one
//! being left. That is correct for servers, where the one you are looking at is
//! the one you need — and it makes direct messages impossible rather than slow.
//! A conversation between two people is a network neither of them is *looking
//! at* most of the time, and a node that only runs while its network is on
//! screen can neither receive a message nor be reached to deliver one.
//!
//! It bites a step earlier than that, too. Minting a conversation's invite needs
//! an address, and an address is something only a running node knows (`02`
//! §6.1) — so the node has to be up before the request that offers the
//! conversation can even be composed.
//!
//! # What decides which nodes run
//!
//! **Everything a member has joined is warm unless they say otherwise.** The
//! network in view is hot; a network the member has explicitly set aside is cold;
//! everything else runs. That is a decision taken deliberately over two narrower
//! policies, and the reason is that the narrow ones make a network *silently*
//! unreachable — somebody in a dozen networks would be receiving in one of them
//! and would have no way to tell that was why the others were quiet.
//!
//! Resource use is the member's own to manage, which is the same answer `02`
//! §6.4 already gives for storage and bandwidth: contribution is per network and
//! the client does not decide it on anybody's behalf. A tier is that choice
//! applied to whether a node runs at all.
//!
//! # Cold is polled, not switched off
//!
//! §2's table gives cold a wake latency of "the poll interval", and a member
//! setting: ten minutes by default. So a cold network is **woken periodically to
//! catch up**, not abandoned. This module got that wrong on its first pass —
//! cold meant no node, ever — and the difference matters in a way that reaches
//! other people: a network with no node running serves no content, so a machine
//! that had taken replica duty there stops holding up its end of `05` §5.1 for
//! everybody else in it, without anybody being told.
//!
//! # What a tier does and does not mean
//!
//! Hot and warm are both *continuously running nodes*. The difference is what
//! they are doing: a hot node is connected and subscribed to its channels'
//! topics, a warm one is reachable and quiet.
//!
//! That is worth stating because "warm" sounds cheaper than it is. §2 chose it
//! against a held connection, not against a running node: what a warm network
//! needs is to be *dialable*, so an incoming stream can wake it, and being
//! dialable is what a relay reservation provides. The saving is connections and
//! gossip meshes, not processes.
//!
//! # A conversation sleeps
//!
//! `05` §4 says idle conversations "should be suspended and woken on demand
//! rather than all held live", and D41 (2026-10-10) settles how: a conversation
//! nobody has talked in for [`IDLE_TIMEOUT`] runs no node at all
//! ([`Tier::Asleep`]), and is woken by this member sending in it or by a knock
//! from the other side through the network it was arranged in. An awake one is
//! warm like anything else. Setting one aside is not offered.
//!
//! # The claim still does the enforcing
//!
//! Exactly one process may run a node for a given network, because MLS group
//! state is live (`05` §4). Nothing here weakens that: the claim lives in the
//! *store*, so running fifty nodes for fifty networks trips nothing, and two
//! supervisors racing for the same network still resolve exactly as two
//! processes would.

use std::collections::BTreeMap;

use intranet_identity::NetworkId;

use crate::serve::Sink;
use kols_api::Event;

/// How live a network is being kept — `design/09` §2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// Connected and subscribed. The network in view.
    Hot,
    /// Running and reachable, without the connections a hot node holds.
    ///
    /// The default for everything a member has joined. What it buys is being
    /// **dialable**, so the dial itself is the wake signal (`09` §2). Not for an
    /// asleep conversation, which has no node to dial and is woken by a knock
    /// instead (D41).
    Warm,
    /// Woken on the poll interval rather than held running.
    ///
    /// Only where the member set a network aside. **Not "off"**: §2 gives cold a
    /// wake latency of the poll interval, so a cold network still catches up —
    /// it is slower, not silent. The distinction reaches other people, because a
    /// network with no node running serves nothing and quietly stops holding up
    /// this machine's replica duty (`05` §5.1).
    Cold,
    /// No node at all, and no poll: a direct conversation nobody has talked in
    /// for the idle timeout — D41.
    ///
    /// Unlike cold, nothing wakes it on a schedule. It is woken by this member
    /// sending in it, or by a knock arriving through the network it was
    /// arranged in, whose node is running. Sleeping strands nobody's data, which
    /// is what made "no node" wrong for a cold server: a conversation has two
    /// members, and each holds the whole history.
    Asleep,
}

impl Tier {
    /// Whether a node runs *continuously* at this tier.
    ///
    /// False for cold, which runs in bursts on the poll rather than not at all —
    /// so this answers "is it held up", not "does it ever run".
    pub fn runs(self) -> bool {
        matches!(self, Self::Hot | Self::Warm)
    }

    /// A label for reports and tests.
    pub fn label(self) -> &'static str {
        match self {
            Self::Hot => "hot",
            Self::Warm => "warm",
            Self::Cold => "cold",
            Self::Asleep => "asleep",
        }
    }
}

/// How long a cold network waits between polls — `design/09` §2.
///
/// §2 makes this a member setting defaulting to ten minutes. The default is here;
/// where a member has changed it is the shell's to pass.
pub const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(600);

/// How long a polled network is left running before it is put back to sleep.
///
/// Long enough to connect, sync governance and take what is new; short enough
/// that a cold network is not a warm one with extra steps. A poll that ended
/// before the node had connected would be a poll that never caught anything up,
/// which is the failure worth avoiding — it would look like working software and
/// deliver nothing.
pub const POLL_SETTLE: std::time::Duration = std::time::Duration::from_secs(45);

/// How long a conversation stays awake after the last message either way — D41.
///
/// The user's default, "an hour or something", meant to become a setting
/// (`design/09` §2). Until it is one, this is the value.
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3600);

/// Whether a network is a conversation that has gone to sleep — D41.
///
/// Asleep once nothing has been sent or received in it, and it has not been
/// woken, for `timeout`. A conversation where nothing has ever happened is
/// asleep too: creating or accepting one marks it awake, so this is only a
/// conversation from before that, which is the safe direction to read it.
/// An activity time in the future reads as now rather than as asleep.
pub fn is_asleep(
    profile: kols_core::NetworkProfile,
    last_activity: Option<std::time::SystemTime>,
    now: std::time::SystemTime,
    timeout: std::time::Duration,
) -> bool {
    profile == kols_core::NetworkProfile::Conversation
        && last_activity.is_none_or(|at| now.duration_since(at).is_ok_and(|idle| idle >= timeout))
}

/// Whether the network at `store` should be asleep now — [`is_asleep`], plus
/// the one thing that must keep a quiet conversation awake.
///
/// **A conversation still waiting for its second member never sleeps.** Its
/// request carries this node's addresses, and accepting it is a redeem the
/// issuing node must answer and admit (spec 07 §6.2). A request delivered to
/// somebody who was away for longer than the idle timeout would otherwise name
/// a node that is not running, and accepting would time out. The membership is
/// replayed only when the conversation would otherwise sleep, so an active one
/// costs nothing extra.
pub fn should_sleep(
    profile: kols_core::NetworkProfile,
    store: &crate::store::Store,
    now: std::time::SystemTime,
) -> bool {
    is_asleep(profile, store.last_activity(), now, IDLE_TIMEOUT)
        && store.state().is_ok_and(|state| {
            state
                .groups
                .get(&intranet_governance::GroupId::everyone())
                .is_some_and(|everyone| everyone.members.len() >= 2)
        })
}

/// What tier a network should be kept at — `design/09` §2's policy, applied.
///
/// `in_view` is the network the member is looking at, if any. `set_aside` is
/// whether the member has asked for this one to be kept cold. `asleep` is
/// [`is_asleep`]'s answer, and wins over being in view: reading what somebody
/// sent is not a reason to hold a connection open to them (the user,
/// 2026-10-10), and the history being read is already on this machine.
///
/// **Everything joined is warm unless the member says otherwise**, which is the
/// decision taken over two narrower policies. The narrow ones save resources by
/// making some networks silently unreachable, and "silently" is the objection: a
/// member in a dozen networks would be receiving in one and have no way to tell
/// that was why the rest were quiet.
///
/// A conversation that is awake is warm like anything else. Setting one aside
/// is not offered (`09` §1.10, D41); this function would honour it like a
/// server's if a caller passed it.
pub fn tier_for(
    network: &NetworkId,
    set_aside: bool,
    asleep: bool,
    in_view: Option<&NetworkId>,
) -> Tier {
    if asleep {
        return Tier::Asleep;
    }
    if in_view == Some(network) {
        return Tier::Hot;
    }
    if set_aside {
        return Tier::Cold;
    }
    Tier::Warm
}

/// A node this supervisor started.
struct Running {
    tier: Tier,
    handle: tokio::task::JoinHandle<()>,
    /// When it was started, which is what bounds a poll.
    since: std::time::Instant,
}

/// The nodes a client is running, one per network.
///
/// Dropping this aborts every task, which is the whole shutdown protocol — the
/// same one the single-node shell already had, applied to a set. Each task's
/// abort drops its node, which drops its store claim and releases its relay
/// reservation rather than leaving both to expire (`05` §1.1).
pub struct Nodes {
    running: BTreeMap<NetworkId, Running>,
    /// When each cold network was last woken.
    ///
    /// Kept for networks that are not running, which is the whole point: a poll
    /// schedule has to survive the node it starts and stops.
    polled: BTreeMap<NetworkId, std::time::Instant>,
    /// The same moments by the wall clock, for saying *when* rather than *how
    /// long ago*. Kept apart from `polled` because a monotonic instant does not
    /// advance while a laptop sleeps on every platform, and an age computed from
    /// one after a sleep says a poll an hour old happened two minutes ago — the
    /// overstated freshness `design/09` §1.3 warns a row against.
    checked: BTreeMap<NetworkId, std::time::SystemTime>,
    /// The runtime nodes are spawned onto.
    ///
    /// # Held rather than taken from the ambient context, and that is a fix
    ///
    /// The first version called `tokio::spawn`, which finds the runtime *entered
    /// on the calling thread* — and **panics** when there is none rather than
    /// returning an error. That is fine from an async caller and fatal from a
    /// synchronous one, which is exactly what a Tauri command is: selecting a
    /// network took the whole window down, every time, with "there is no reactor
    /// running".
    ///
    /// Nothing in a test caught it because every test here reconciles inside
    /// `block_on`, which enters a runtime and makes the ambient lookup succeed.
    /// So the dependency was invisible in the one place it was exercised and
    /// fatal in the one place it was used.
    ///
    /// A held handle removes the question: a caller supplies the runtime once,
    /// where it is known, and every spawn after that works from any thread.
    runtime: tokio::runtime::Handle,
}

/// Where a supervised node's events go, tagged with the network they came from.
///
/// A single sink is not enough once several nodes are reporting: `Event` says
/// what happened and not *where*, which was unambiguous while one node ran and
/// is not now. A consumer that could not tell a record in a conversation from a
/// record in a server would render one into the other.
pub type TaggedSink = std::sync::Arc<dyn Fn(&NetworkId, &[Event]) + Send + Sync>;

/// What a supervised node needs in order to start.
pub struct Spec {
    /// The network's id.
    pub network: NetworkId,
    /// Where its store lives.
    pub root: std::path::PathBuf,
    /// Whether the member has asked for this network to be kept cold.
    pub set_aside: bool,
    /// Whether this is a conversation asleep under D41 ([`is_asleep`]).
    pub asleep: bool,
}

impl Nodes {
    /// A supervisor running nothing, spawning onto `runtime`.
    pub fn new(runtime: tokio::runtime::Handle) -> Self {
        Self {
            running: BTreeMap::new(),
            polled: BTreeMap::new(),
            checked: BTreeMap::new(),
            runtime,
        }
    }

    /// A supervisor spawning onto the runtime the caller is already inside.
    ///
    /// For an async caller that has one. **Panics** where there is none, which is
    /// the honest behaviour for a constructor: failing here names the problem at
    /// the point it can be fixed, rather than at the first reconcile — which is
    /// how this went out as a crash on selecting a network.
    pub fn here() -> Self {
        Self::new(tokio::runtime::Handle::current())
    }

    /// Which networks are running, and at what tier.
    pub fn running(&self) -> Vec<(NetworkId, Tier)> {
        self.running
            .iter()
            .map(|(network, run)| (*network, run.tier))
            .collect()
    }

    /// Whether a node for `network` is running right now.
    ///
    /// Not the same as being in the running set: a node whose task ended — a
    /// refused store claim, a startup failure — stays in the set until the next
    /// reconcile, and anything it wrote while it ran is now a statement about
    /// the past. This is what a reader of a node's live state asks first.
    pub fn is_live(&self, network: &NetworkId) -> bool {
        self.running
            .get(network)
            .is_some_and(|run| !run.handle.is_finished())
    }

    /// When a cold network was last woken for a poll, if it has been since this
    /// supervisor started. A monotonic instant, for scheduling; it says how long
    /// ago and nothing about the wall clock.
    pub fn last_polled(&self, network: &NetworkId) -> Option<std::time::Instant> {
        self.polled.get(network).copied()
    }

    /// When this machine last looked at a cold network, by the wall clock —
    /// `design/09` §1.3's *when it last looked*: the start of its last poll, or
    /// the moment it stopped running when the member set it aside.
    ///
    /// It does not survive a restart; a cold network is polled on the first
    /// reconcile after one, so "not yet" is a short state.
    pub fn last_checked(&self, network: &NetworkId) -> Option<std::time::SystemTime> {
        self.checked.get(network).copied()
    }

    /// The tier a network is currently being kept at.
    ///
    /// `Cold` for one that is not running, which is the same answer as "not
    /// running" by construction — there is no state in which a cold network has
    /// a task.
    pub fn tier(&self, network: &NetworkId) -> Tier {
        self.running.get(network).map_or(Tier::Cold, |run| run.tier)
    }

    /// Brings the running set in line with what `specs` and `in_view` ask for.
    ///
    /// Starts what should be running and is not, stops what is running and
    /// should not be, and leaves everything else alone — including a node whose
    /// tier changed between hot and warm, because both are running nodes and
    /// restarting one to change a label would drop its connections for nothing.
    ///
    /// **Idempotent on purpose.** A caller may run this on every tick without
    /// having to remember what it did last time, which is what keeps the policy
    /// in one place rather than smeared across every event that might change it.
    pub fn reconcile(
        &mut self,
        specs: &[Spec],
        in_view: Option<&NetworkId>,
        events: &TaggedSink,
        seal_bytes: usize,
        now: std::time::Instant,
    ) {
        let mut wanted: BTreeMap<NetworkId, (&Spec, Tier)> = BTreeMap::new();
        let mut polling: BTreeMap<NetworkId, &Spec> = BTreeMap::new();
        // Networks that were running, not on a poll, when the member set them
        // aside: they were looking right up to now, which is as good as a poll.
        let mut just_set_aside: Vec<NetworkId> = Vec::new();

        for spec in specs {
            match tier_for(&spec.network, spec.set_aside, spec.asleep, in_view) {
                tier if tier.runs() => {
                    wanted.insert(spec.network, (spec, tier));
                }
                // Neither wanted nor polled, so a running one is stopped below.
                // Its poll schedule is dropped: it is woken by a send or a
                // knock, never by the clock.
                Tier::Asleep => {
                    self.polled.remove(&spec.network);
                }
                // **Cold is a schedule, not an absence.** A network the member
                // set aside is woken when its poll comes round and put back to
                // sleep once it has had time to catch up.
                _ => {
                    let due = self
                        .polled
                        .get(&spec.network)
                        .is_none_or(|last| now.duration_since(*last) >= POLL_INTERVAL);
                    let settled = self
                        .running
                        .get(&spec.network)
                        .is_some_and(|run| now.duration_since(run.since) >= POLL_SETTLE);
                    if self.running.contains_key(&spec.network) {
                        // Mid-poll. Leave it alone until it has settled.
                        if !settled {
                            wanted.insert(spec.network, (spec, Tier::Cold));
                        } else if !self.polled.contains_key(&spec.network) {
                            // Not a poll at all: a running network the member
                            // has just set aside. Without this it read as never
                            // checked, and was woken again on the next tick.
                            just_set_aside.push(spec.network);
                        }
                    } else if due {
                        polling.insert(spec.network, spec);
                    }
                }
            }
        }

        for network in just_set_aside {
            self.polled.insert(network, now);
            self.checked.insert(network, std::time::SystemTime::now());
        }

        // Stop first, so a network leaving the running set has released its
        // claim before anything else asks for one — including a poll that has
        // settled and is going back to sleep.
        let stopping: Vec<NetworkId> = self
            .running
            .keys()
            .filter(|network| !wanted.contains_key(*network) && !polling.contains_key(*network))
            .copied()
            .collect();
        for network in stopping {
            // Dropped rather than awaited: this is the tier reconciler lowering
            // a node it is about to start again, not a caller that needs the
            // claim released before it touches the store.
            let _ = self.stop(&network);
        }

        for (network, (spec, tier)) in wanted {
            match self.running.get_mut(&network) {
                // Already running. Hot and warm are both continuously running,
                // so a change between them is a label rather than a restart.
                Some(run) => run.tier = tier,
                None => self.start(spec, tier, events, seal_bytes, now),
            }
        }

        for (network, spec) in polling {
            self.polled.insert(network, now);
            self.checked.insert(network, std::time::SystemTime::now());
            self.start(spec, Tier::Cold, events, seal_bytes, now);
        }
    }

    /// Stops one network's node, handing back the task so a caller can await it.
    ///
    /// **Aborting is a request, not a stop** — the same reason [`Self::stop_all`]
    /// returns its tasks, and this had the bug that one documents. A claim is
    /// released when the serving future is *dropped*, and `abort` only asks the
    /// task to stop at its next await point, so a caller that carried on
    /// immediately was acting against a node that was still holding the store.
    ///
    /// Which is not theoretical for one caller in particular: deleting a
    /// network's store is exactly what must not happen while its node holds the
    /// key group, and the workspace refuses it on precisely that ground.
    #[must_use = "await the task, or the claim it holds is not released"]
    pub fn stop(&mut self, network: &NetworkId) -> Option<tokio::task::JoinHandle<()>> {
        let run = self.running.remove(network)?;
        run.handle.abort();
        Some(run.handle)
    }

    /// Stops everything, handing back the tasks so a caller can await them.
    ///
    /// **Aborting is a request, not a stop**, and the difference is the whole
    /// reason this returns anything: a claim is released when the serving future
    /// is *dropped*, and `abort` only asks the task to stop at its next await
    /// point. A caller that returned here immediately would leave every store
    /// claimed and every relay reservation held, which is exactly the state
    /// stopping was supposed to avoid — and the next launch would sit waiting
    /// out the staleness window on each of them.
    #[must_use = "the tasks must be awaited, or the claims they hold are not released"]
    pub fn stop_all(&mut self) -> Vec<tokio::task::JoinHandle<()>> {
        let networks: Vec<NetworkId> = self.running.keys().copied().collect();
        let mut stopping = Vec::new();
        for network in networks {
            if let Some(run) = self.running.remove(&network) {
                run.handle.abort();
                stopping.push(run.handle);
            }
        }
        stopping
    }

    fn start(
        &mut self,
        spec: &Spec,
        tier: Tier,
        events: &TaggedSink,
        seal_bytes: usize,
        now: std::time::Instant,
    ) {
        let network = spec.network;
        let root = spec.root.clone();
        let tagged = events.clone();

        // Each node reports through a sink of its own that knows which network
        // it is. `Event` carries no network id — it did not need one while a
        // single node ran — and adding the tag here rather than to the type
        // keeps `design/05` §3's vocabulary unchanged for consumers that still
        // only ever look at one.
        let sink: Sink = std::sync::Arc::new(move |batch: &[Event]| {
            tagged(&network, batch);
        });

        let handle = self.runtime.spawn(async move {
            // A refusal here is ordinary rather than exceptional: another
            // process may hold this network's claim, which is exactly what the
            // claim is for. It stops this node and leaves every other one
            // running, which is the behaviour the single-node shell could not
            // have — there, one refusal was the whole client.
            let _ = crate::serve::serve(
                root,
                "",
                &[],
                seal_bytes,
                true,
                crate::serve::LIVE_WINDOW_MILLIS,
                &crate::serve::Output {
                    events: &sink,
                    report: &crate::quiet(),
                },
            )
            .await;
        });

        self.running.insert(
            network,
            Running {
                tier,
                handle,
                since: now,
            },
        );
    }
}

impl Drop for Nodes {
    fn drop(&mut self) {
        // Nothing to await here — a `Drop` cannot — so this is the backstop
        // rather than the shutdown path. A caller that wants the claims released
        // promptly awaits [`Nodes::stop_all`] before dropping; one that does not
        // falls back on the six-second expiry, which is what that expiry is for
        // (`05` §1.1).
        let _ = self.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(n: u8) -> NetworkId {
        NetworkId::from_bytes([n; 32])
    }

    #[test]
    fn the_network_in_view_is_hot() {
        let server = net(1);
        assert_eq!(tier_for(&server, false, false, Some(&server)), Tier::Hot);
    }

    #[test]
    fn everything_joined_is_warm_by_default() {
        // The decision this policy turns on. A member in a dozen networks
        // receives in all of them, because the alternative makes the other
        // eleven silently unreachable — and "silently" is the objection, not the
        // resource use.
        assert_eq!(tier_for(&net(2), false, false, Some(&net(1))), Tier::Warm);
        assert_eq!(tier_for(&net(3), false, false, None), Tier::Warm);
    }

    #[test]
    fn only_a_network_the_member_set_aside_is_cold() {
        assert_eq!(tier_for(&net(4), true, false, Some(&net(1))), Tier::Cold);
        assert_eq!(tier_for(&net(4), true, false, None), Tier::Cold);
    }

    #[test]
    fn looking_at_a_network_outranks_having_set_it_aside() {
        // Opening something you had set aside is as clear a statement as setting
        // it aside was, and the later one wins.
        let aside = net(5);
        assert_eq!(tier_for(&aside, true, false, Some(&aside)), Tier::Hot);
    }

    #[test]
    fn an_awake_conversation_is_warm_like_anything_else() {
        // D41 gives a conversation one state of its own, asleep; awake, it is
        // tiered exactly like a server.
        assert_eq!(tier_for(&net(6), false, false, None), Tier::Warm);
        assert_eq!(tier_for(&net(6), true, false, None), Tier::Cold);
    }

    #[test]
    fn cold_is_not_the_same_as_never_running() {
        // The correction. `runs()` answers "is it held up", not "does it ever
        // run" — a cold network is woken on the poll interval, which §2 gives it
        // a wake latency for. Treating cold as off is what the first pass did,
        // and it silently stops a machine serving content it had taken duty for.
        assert!(Tier::Hot.runs());
        assert!(Tier::Warm.runs());
        assert!(!Tier::Cold.runs());
        assert!(
            POLL_INTERVAL > POLL_SETTLE,
            "a poll sleeps longer than it wakes"
        );
    }

    #[test]
    fn a_supervisor_starting_nothing_reports_everything_cold() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime");
        let nodes = Nodes::new(runtime.handle().clone());
        assert_eq!(nodes.tier(&net(1)), Tier::Cold);
        assert!(nodes.running().is_empty());
    }

    #[test]
    fn an_asleep_conversation_runs_nothing_even_in_view() {
        // D41 and the user's point of 2026-10-10: opening a conversation to read
        // it is not a reason to hold a connection to the other person.
        let conversation = net(7);
        let tier = tier_for(&conversation, false, true, Some(&conversation));
        assert_eq!(tier, Tier::Asleep);
        assert!(!tier.runs());
    }

    #[test]
    fn a_conversation_sleeps_after_the_idle_timeout_and_a_server_never_does() {
        use kols_core::NetworkProfile::{Conversation, Server};
        let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(100_000);
        let ago = |secs| Some(now - std::time::Duration::from_secs(secs));

        assert!(!is_asleep(Conversation, ago(10), now, IDLE_TIMEOUT));
        assert!(is_asleep(
            Conversation,
            ago(IDLE_TIMEOUT.as_secs()),
            now,
            IDLE_TIMEOUT
        ));
        assert!(is_asleep(Conversation, None, now, IDLE_TIMEOUT));
        // A clock that moved back reads as activity now, not as long ago.
        assert!(!is_asleep(
            Conversation,
            Some(now + std::time::Duration::from_secs(60)),
            now,
            IDLE_TIMEOUT
        ));
        assert!(!is_asleep(Server, ago(1_000_000), now, IDLE_TIMEOUT));
        assert!(!is_asleep(Server, None, now, IDLE_TIMEOUT));
    }
}
