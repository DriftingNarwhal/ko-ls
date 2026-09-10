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
//! # A conversation is no longer a special case
//!
//! §2 says a conversation is warm whenever the application runs, and `05` §4
//! says idle conversations "should be suspended and woken on demand rather than
//! all held live" — a tension nobody had to resolve while neither was built.
//! Defaulting *everything* to warm resolves it without choosing a side: a
//! conversation is warm because every joined network is, and a member who wants
//! one quiet sets it aside exactly as they would a server.
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
    /// **dialable**, so the dial itself is the wake signal and there is no
    /// wake-up message to design (`09` §2).
    Warm,
    /// Woken on the poll interval rather than held running.
    ///
    /// Only where the member set a network aside. **Not "off"**: §2 gives cold a
    /// wake latency of the poll interval, so a cold network still catches up —
    /// it is slower, not silent. The distinction reaches other people, because a
    /// network with no node running serves nothing and quietly stops holding up
    /// this machine's replica duty (`05` §5.1).
    Cold,
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

/// What tier a network should be kept at — `design/09` §2's policy, applied.
///
/// `in_view` is the network the member is looking at, if any. `set_aside` is
/// whether the member has asked for this one to be kept cold.
///
/// **Everything joined is warm unless the member says otherwise**, which is the
/// decision taken over two narrower policies. The narrow ones save resources by
/// making some networks silently unreachable, and "silently" is the objection: a
/// member in a dozen networks would be receiving in one and have no way to tell
/// that was why the rest were quiet.
///
/// Note there is no case for conversations. §2 says a conversation is warm
/// whenever the application runs and `05` §4 says idle ones should be suspended;
/// defaulting everything to warm satisfies the first without having to settle the
/// second, and a member who wants a conversation quiet sets it aside exactly as
/// they would a server.
pub fn tier_for(network: &NetworkId, set_aside: bool, in_view: Option<&NetworkId>) -> Tier {
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
#[derive(Default)]
pub struct Nodes {
    running: BTreeMap<NetworkId, Running>,
    /// When each cold network was last woken.
    ///
    /// Kept for networks that are not running, which is the whole point: a poll
    /// schedule has to survive the node it starts and stops.
    polled: BTreeMap<NetworkId, std::time::Instant>,
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
}

impl Nodes {
    /// A supervisor running nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Which networks are running, and at what tier.
    pub fn running(&self) -> Vec<(NetworkId, Tier)> {
        self.running
            .iter()
            .map(|(network, run)| (*network, run.tier))
            .collect()
    }

    /// The tier a network is currently being kept at.
    ///
    /// `Cold` for one that is not running, which is the same answer as "not
    /// running" by construction — there is no state in which a cold network has
    /// a task.
    pub fn tier(&self, network: &NetworkId) -> Tier {
        self.running
            .get(network)
            .map_or(Tier::Cold, |run| run.tier)
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

        for spec in specs {
            match tier_for(&spec.network, spec.set_aside, in_view) {
                tier if tier.runs() => {
                    wanted.insert(spec.network, (spec, tier));
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
                        }
                    } else if due {
                        polling.insert(spec.network, spec);
                    }
                }
            }
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
            self.stop(&network);
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
            self.start(spec, Tier::Cold, events, seal_bytes, now);
        }
    }

    /// Stops one network's node, releasing its claim.
    pub fn stop(&mut self, network: &NetworkId) {
        if let Some(run) = self.running.remove(network) {
            run.handle.abort();
        }
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

        let handle = tokio::spawn(async move {
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
        assert_eq!(tier_for(&server, false, Some(&server)), Tier::Hot);
    }

    #[test]
    fn everything_joined_is_warm_by_default() {
        // The decision this policy turns on. A member in a dozen networks
        // receives in all of them, because the alternative makes the other
        // eleven silently unreachable — and "silently" is the objection, not the
        // resource use.
        assert_eq!(tier_for(&net(2), false, Some(&net(1))), Tier::Warm);
        assert_eq!(tier_for(&net(3), false, None), Tier::Warm);
    }

    #[test]
    fn only_a_network_the_member_set_aside_is_cold() {
        assert_eq!(tier_for(&net(4), true, Some(&net(1))), Tier::Cold);
        assert_eq!(tier_for(&net(4), true, None), Tier::Cold);
    }

    #[test]
    fn looking_at_a_network_outranks_having_set_it_aside() {
        // Opening something you had set aside is as clear a statement as setting
        // it aside was, and the later one wins.
        let aside = net(5);
        assert_eq!(tier_for(&aside, true, Some(&aside)), Tier::Hot);
    }

    #[test]
    fn a_conversation_needs_no_case_of_its_own() {
        // `09` §2 wants conversations warm whenever the application runs; `05`
        // §4 wants idle ones suspended. Defaulting everything to warm satisfies
        // the first without settling the second, and a member who wants one
        // quiet sets it aside exactly as they would a server. There is no
        // `is_conversation` argument here, and that absence is the resolution.
        assert_eq!(tier_for(&net(6), false, None), Tier::Warm);
        assert_eq!(tier_for(&net(6), true, None), Tier::Cold);
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
        assert!(POLL_INTERVAL > POLL_SETTLE, "a poll sleeps longer than it wakes");
    }

    #[test]
    fn a_supervisor_starting_nothing_reports_everything_cold() {
        let nodes = Nodes::new();
        assert_eq!(nodes.tier(&net(1)), Tier::Cold);
        assert!(nodes.running().is_empty());
    }
}
