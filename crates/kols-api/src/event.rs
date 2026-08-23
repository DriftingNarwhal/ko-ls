//! What the node learned — `design/05` §3, the other half of the boundary.
//!
//! Commands go in; events come out. Nothing else crosses.

use intranet_identity::PerNetworkIdentityId;
use kols_core::{ChannelId, Record};

/// How a record reached this node.
///
/// The consumer must not render these differently — order is computed from the
/// merged set, not from arrival (`design/01` §4), so a record that arrived live
/// and the same record read out of a segment months later are byte-identical and
/// sort to the same place.
///
/// It matters for everything *around* rendering. The daemon has always drawn
/// this distinction in its own words, for a reason worth keeping: records off the
/// head segment are the conversation arriving, records off an older one are
/// history being recovered, and a client that pinged for the second would notify
/// somebody about a message from last year.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    /// Pushed over gossip as it was written (`design/01` §7).
    Live,
    /// Read from the head of an author's log — the conversation catching up.
    Head,
    /// Read from sealed segments behind the head — history being recovered.
    Backfill {
        /// How many sealed segments this walk reached.
        ///
        /// Carried because it is the difference between "history is arriving"
        /// and "history arrived": a walk that keeps reporting segments is still
        /// descending a chain, and one that reports few has nearly bottomed out.
        segments: usize,
    },
}

/// Something the node learned, on its way to the interface.
///
/// # Idempotent and re-deliverable, which is a property of the consumer
///
/// `design/05` §3's third property: an event may arrive twice, out of order, or
/// after a gap. That is not a promise the emitter keeps by being careful — it is
/// what the durable path *is*, since the live path may be lossy and the same
/// record legitimately arrives twice, once over gossip and once inside a segment.
///
/// So the obligation falls on whoever consumes these: **merge, never append.**
/// A [`Event::Records`] payload goes into a [`kols_core::ChannelView`], which is
/// a pure function of the record set it has admitted and which deduplicates by
/// record id. Treating the payload as an ordered append instead would produce
/// duplicates on the one delivery pattern the design guarantees.
///
/// # What is deliberately not an event
///
/// This node's transport — which addresses it listens on, which peers it is
/// connected to — is a fact about the machine rather than about the network's
/// content, and a sandboxed build would not be told it at all (App Hosting
/// §3.2's "no ambient host access"). The daemon prints those directly, and they
/// stop at the process boundary rather than crossing this one.
///
/// The same goes for a node's own startup report. What this node *is* when it
/// comes up — whether it holds an epoch key, whether it can key others in — is
/// state, and these are things that happen while it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Records became available in a channel.
    Records {
        /// Which channel.
        channel: ChannelId,
        /// The records, which the consumer merges rather than appends.
        records: Vec<Record>,
        /// How they got here.
        arrival: Arrival,
    },
    /// Governance state advanced, so permissions and channels may have changed.
    ///
    /// Carries a count rather than the entries: replay is the authority on what
    /// the log now says, and an event that carried entries would invite a
    /// consumer to apply them itself and reach a different answer.
    Governance {
        /// How many entries this node took in.
        learned: usize,
    },
    /// The node adopted governance entries written locally by a one-shot command.
    ///
    /// Distinct from [`Event::Governance`] because nothing arrived from anybody:
    /// this is the daemon noticing what its own store gained while it was
    /// running, which is how `kols admit` and the daemon avoid forking the log.
    Adopted {
        /// How many entries.
        entries: usize,
    },
    /// The epoch rotated to exclude members who were removed.
    EpochRotated {
        /// How many members the new epoch excludes.
        excluded: usize,
    },
    /// A member was keyed into the network.
    MemberKeyed {
        /// Who.
        identity: PerNetworkIdentityId,
    },
    /// Somebody presented an invite and this node answered.
    ///
    /// `accepted` covers both outcomes an invite can legitimately have: admitted
    /// outright under auto-admit, or given a waiting-room place under explicit
    /// intake. Both are successful joins (Core §2.4), and a client that treated
    /// the second as a failure would be reporting the network working as
    /// configured as though something had gone wrong.
    JoinAnswered {
        /// Who asked.
        joiner: PerNetworkIdentityId,
        /// Whether the invite was good.
        accepted: bool,
    },
    /// This node's standing with the network's relays, settled at startup.
    ///
    /// Reported on **success as well as failure**, which is the point of it.
    /// Relay trouble already reached a consumer through [`Event::Degraded`]
    /// while success was only ever a `println!`, so a window could show relay
    /// problems and never relay health — and "is my relay working" is the
    /// question two people on separate machines actually have.
    Relay {
        /// The relay a circuit was reserved on, when one was.
        reserved: Option<String>,
        /// How many this network designates.
        ///
        /// Carried so that "designates none" stays distinguishable from
        /// "designates some and none of them worked". Both leave this node
        /// reachable only on its own addresses; only the second is a fault, and
        /// a consumer that could not tell them apart would have to guess which
        /// to say.
        designated: usize,
        /// Why each designated relay did not work, in the order tried.
        ///
        /// Carried because "no circuit" has two causes needing opposite fixes,
        /// and the summary cannot tell them apart: a relay that could not be
        /// reached at all is a network or port problem, and a relay that
        /// answered and returned no address is a relay announcing nothing. Both
        /// reach a consumer as `Degraded` too — and a consumer showing a status
        /// line was left saying "no circuit" while the reason went past in a
        /// different stream.
        failures: Vec<String>,
    },
    /// Something did not work, and the node carried on.
    ///
    /// Not an error return: every one of these is a state the node is expected
    /// to pass through — a key request that arrived before the requester was
    /// admitted, a live payload that failed to open under any held epoch. They
    /// are surfaced rather than swallowed because a node that is quietly failing
    /// at one of them looks exactly like a node with nothing to do.
    Degraded {
        /// What happened, in words a user can act on.
        reason: String,
    },
    /// Reconciliation voided actions after a fork healed — Core §2.7.1 point 5.
    ///
    /// **This exists so that noticing is somebody's job.** When a partition
    /// heals, every entry on the losing branch is treated as if it never
    /// happened. For most kinds that is merely annoying. For a revocation it is
    /// not: the member removed on the losing branch is a fully current member
    /// again on the winning one, entitled to its epoch key, for as long as it
    /// takes somebody to realise — and without this event, nobody is assigned to.
    GovernanceReorg {
        /// This member's own voided actions, which are the ones they can resubmit.
        mine: Vec<VoidedAction>,
        /// How many other members' actions the same reconciliation voided.
        ///
        /// Carried because it is the difference between "your action lost" and
        /// "a partition healed and a lot of things lost", which want different
        /// reactions from a person.
        others: usize,
    },
}

/// One action reconciliation undid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoidedAction {
    /// A short label for what it was.
    pub kind: String,
    /// Whether losing it re-opens a gap rather than merely losing an edit.
    ///
    /// True for revocations, moderation and epoch rotations: each *removed* an
    /// access or a piece of content, so voiding one silently restores what was
    /// taken away. This is the flag Core §2.7.1 expects a client to watch.
    pub security_relevant: bool,
}
