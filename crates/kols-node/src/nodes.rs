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
//! [`Tier`], which is `design/09` §2's policy and had been unwritten since that
//! section was drafted. The short of it: **the network in view is hot, every
//! conversation is warm whenever the application is running, and everything else
//! is cold.** §2 says the second part in as many words — the cold poll interval
//! "never applies to direct messages, which are warm whenever the application is
//! running" — and it is the whole reason this module exists.
//!
//! # What a tier does and does not mean
//!
//! Hot and warm are both *running nodes*. The difference is what they are doing:
//! a hot node is connected and subscribed to its channels' topics, a warm one is
//! reachable and quiet. Cold is the only tier that is not a process at all.
//!
//! That is worth stating because "warm" sounds cheaper than it is. §2 chose it
//! against a held connection, not against a running node: what a warm network
//! needs is to be *dialable*, so an incoming stream can wake it, and being
//! dialable is what a relay reservation provides. The saving is connections and
//! gossip meshes, not processes.
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
    /// Every conversation, whenever the application is running, plus recently
    /// used servers. What this buys is being **dialable**, so the dial itself is
    /// the wake signal and there is no wake-up message to design (`09` §2).
    Warm,
    /// Not running at all.
    Cold,
}

impl Tier {
    /// Whether a node runs at this tier.
    ///
    /// The line this whole module turns on, and it falls between warm and cold
    /// rather than between hot and warm: a warm node is a running process that
    /// holds fewer connections, not an absent one.
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

/// What tier a network should be kept at — `design/09` §2's policy, applied.
///
/// `in_view` is the network the member is looking at, if any.
///
/// **A conversation is warm whether or not it is in view**, which is the rule
/// that makes direct messages work at all: a conversation nobody is looking at
/// still has to be reachable, or a message can neither arrive nor be delivered.
/// §2 states it directly, and it is the one place this policy is not a matter of
/// taste.
pub fn tier_for(network: &NetworkId, is_conversation: bool, in_view: Option<&NetworkId>) -> Tier {
    if in_view == Some(network) {
        return Tier::Hot;
    }
    if is_conversation {
        return Tier::Warm;
    }
    Tier::Cold
}

/// A node this supervisor started.
struct Running {
    tier: Tier,
    handle: tokio::task::JoinHandle<()>,
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
    /// Whether it is a conversation, which decides its tier when out of view.
    pub is_conversation: bool,
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
    ) {
        let mut wanted: BTreeMap<NetworkId, (&Spec, Tier)> = BTreeMap::new();
        for spec in specs {
            let tier = tier_for(&spec.network, spec.is_conversation, in_view);
            if tier.runs() {
                wanted.insert(spec.network, (spec, tier));
            }
        }

        // Stop first, so a network moving out of the running set has released
        // its claim before anything else asks for one.
        let stopping: Vec<NetworkId> = self
            .running
            .keys()
            .filter(|network| !wanted.contains_key(*network))
            .copied()
            .collect();
        for network in stopping {
            self.stop(&network);
        }

        for (network, (spec, tier)) in wanted {
            match self.running.get_mut(&network) {
                // Already running. Hot and warm are both running nodes, so a
                // tier change is a label rather than a restart.
                Some(run) => run.tier = tier,
                None => self.start(spec, tier, events, seal_bytes),
            }
        }
    }

    /// Stops one network's node, releasing its claim.
    pub fn stop(&mut self, network: &NetworkId) {
        if let Some(run) = self.running.remove(network) {
            run.handle.abort();
        }
    }

    /// Stops everything.
    pub fn stop_all(&mut self) {
        let networks: Vec<NetworkId> = self.running.keys().copied().collect();
        for network in networks {
            self.stop(&network);
        }
    }

    fn start(&mut self, spec: &Spec, tier: Tier, events: &TaggedSink, seal_bytes: usize) {
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

        self.running.insert(network, Running { tier, handle });
    }
}

impl Drop for Nodes {
    fn drop(&mut self) {
        self.stop_all();
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
    fn a_conversation_is_warm_even_when_nobody_is_looking_at_it() {
        // The rule the whole feature rests on. A conversation nobody has open
        // still has to be reachable, or a message can neither arrive nor be
        // delivered — `design/09` §2 says the cold poll "never applies to direct
        // messages, which are warm whenever the application is running".
        let conversation = net(2);
        assert_eq!(tier_for(&conversation, true, Some(&net(1))), Tier::Warm);
        assert_eq!(tier_for(&conversation, true, None), Tier::Warm);
    }

    #[test]
    fn a_server_nobody_is_looking_at_is_cold() {
        assert_eq!(tier_for(&net(3), false, Some(&net(1))), Tier::Cold);
        assert_eq!(tier_for(&net(3), false, None), Tier::Cold);
    }

    #[test]
    fn a_conversation_in_view_is_hot_rather_than_merely_warm() {
        // Being a conversation raises the floor; it does not cap the ceiling.
        let conversation = net(2);
        assert_eq!(tier_for(&conversation, true, Some(&conversation)), Tier::Hot);
    }

    #[test]
    fn both_running_tiers_run_and_only_cold_does_not() {
        // The line falls between warm and cold, not between hot and warm: a warm
        // node is a running process holding fewer connections, not an absent one.
        assert!(Tier::Hot.runs());
        assert!(Tier::Warm.runs());
        assert!(!Tier::Cold.runs());
    }

    #[test]
    fn a_supervisor_starting_nothing_reports_everything_cold() {
        let nodes = Nodes::new();
        assert_eq!(nodes.tier(&net(1)), Tier::Cold);
        assert!(nodes.running().is_empty());
    }
}
