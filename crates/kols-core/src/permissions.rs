//! Permission resolution — `design/02` §3.
//!
//! Every question here is answered by replaying the governance log, never by
//! asking a peer or trusting a cached claim. The resolution order is fixed and
//! one level deep: channel override, then category default, then network
//! default, then denied.

use crate::ChannelId;
use intranet_crypto::{Hash, to_hex};
use intranet_governance::{Capability, ContentType, GovernanceLog, GovernanceState, LogEntry};
use intranet_identity::PerNetworkIdentityId;
use std::cell::RefCell;
use std::collections::BTreeMap;

/// A category, the scope permissions are expected to bind at.
///
/// Binding per channel is the obvious design and does not scale: a group with
/// rights on 300 channels holds 300 capability entries, every one of them
/// replayed by every node. Categories keep that to roughly one per role
/// (`design/02` §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CategoryId([u8; 32]);

impl CategoryId {
    /// Wraps raw identifier bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrows the raw identifier bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Where a channel sits, for permission resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// The channel itself.
    pub channel: ChannelId,
    /// Its category, if it has one.
    pub category: Option<CategoryId>,
}

/// One of the three scopes a chat capability can be granted at — `design/02` §3.
///
/// # Why this is a type rather than three functions
///
/// A grant and a resolution have to agree on a capability's *name*, byte for
/// byte: `identity_holds` compares `Capability::Extension(String)` values, so a
/// writer that spelled a scope differently from the reader would produce a grant
/// that resolves for nobody — and nothing would say so, because an absent grant
/// and a misspelled one are the same observation (`design/02` §3: denials are
/// absent grants).
///
/// Naming used to live in three private helpers here, which was safe only while
/// this module was the only thing that built one. Granting is now a command
/// (`design/05` §3's `SetPermission`), so a second caller exists and the two
/// must not be able to drift. [`Scope::capability`] is the one construction, used
/// by the writer and by every resolution below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// The whole network — `chat:<verb>:*`.
    Network,
    /// One category, and every channel that names it — `chat:<verb>:cat:<id>`.
    ///
    /// The scope `design/02` §4 expects a grant to bind at: a group with rights
    /// on 300 channels otherwise holds 300 capability entries, every one of them
    /// replayed by every node.
    Category(CategoryId),
    /// One channel — `chat:<verb>:<id>`. The override, not the default.
    Channel(ChannelId),
}

impl Scope {
    /// The capability `verb` at this scope.
    ///
    /// The only place a chat capability name is built. Resolution reads what
    /// this writes.
    pub fn capability(&self, verb: &str) -> Capability {
        Capability::extension(self.name(verb))
    }

    /// The capability's name, as the registry and the log carry it.
    pub fn name(&self, verb: &str) -> String {
        match self {
            Self::Network => format!("chat:{verb}:*"),
            Self::Category(id) => format!("chat:{verb}:cat:{}", to_hex(id.as_bytes())),
            Self::Channel(id) => format!("chat:{verb}:{}", to_hex(id.as_bytes())),
        }
    }
}

/// Whether `identity` holds `verb` for this placement.
///
/// Denials are the absence of a grant, never a negative grant: a "deny"
/// capability would need precedence rules over the union-of-groups model the
/// protocol defines, which is the sprawl Core §2.1 exists to prevent. To exclude
/// someone from a broadly-granted category, the channel carries an override
/// binding the narrower group.
pub fn holds(
    state: &GovernanceState,
    identity: &PerNetworkIdentityId,
    verb: &str,
    placement: &Placement,
) -> bool {
    if state.identity_holds(identity, &Scope::Channel(placement.channel).capability(verb)) {
        return true;
    }
    holds_in_scope(state, identity, verb, placement.category.as_ref())
}

/// Whether `identity` holds `verb` at category or network scope.
///
/// The same resolution as [`holds`] with its first step removed, for the
/// questions asked about a channel that does not exist yet. Creating one is the
/// case: a definition can only ever be authorized by a category or network
/// grant, because the channel's id is minted by the entry that creates it and
/// nobody could hold a grant naming it beforehand.
///
/// Separate from [`holds`] rather than reached by passing a placeholder channel,
/// so a caller cannot invent an id to ask about and have the answer quietly
/// depend on it.
pub fn holds_in_scope(
    state: &GovernanceState,
    identity: &PerNetworkIdentityId,
    verb: &str,
    category: Option<&CategoryId>,
) -> bool {
    if let Some(category) = category
        && state.identity_holds(identity, &Scope::Category(*category).capability(verb))
    {
        return true;
    }
    state.identity_holds(identity, &Scope::Network.capability(verb))
}

/// Whether `verb` is one this application defines — `design/02` §2.2.
///
/// Checked before a grant is written rather than only when it is read. An
/// unregistered extension name is refused outright by the protocol (Core §2.2.1),
/// so granting `chat:pots:*` would produce an entry that resolves for nobody and
/// reports no error — the failure mode a typo in a free-text field produces, and
/// the reason the interface names verbs rather than accepting them.
pub fn is_verb(verb: &str) -> bool {
    crate::capabilities::VERBS.iter().any(|(name, _)| *name == verb)
}

/// What a reader needs to answer about the identities behind records.
///
/// A trait rather than a bare `GovernanceState` for one reason: redaction
/// authority is a question about state *at a point in the chain*, not about
/// state now (`design/01` §6), and that distinction should be visible in the
/// type rather than quietly collapsed.
pub trait Authority {
    /// Whether this identity currently belongs to the network at all.
    fn is_member(&self, identity: &PerNetworkIdentityId) -> bool;

    /// Whether this identity may write records into this channel.
    fn may_post(&self, identity: &PerNetworkIdentityId, placement: &Placement) -> bool;

    /// Whether this identity held moderation authority as of `head`.
    fn may_moderate_at(
        &self,
        identity: &PerNetworkIdentityId,
        placement: &Placement,
        head: &Hash,
    ) -> bool;

    /// Whether this identity may moderate here **now**.
    ///
    /// Separate from [`Authority::may_moderate_at`] because the two answer
    /// different questions. A redaction cites the governance head its author
    /// observed, so it is judged as of that moment and keeps standing when its
    /// author is later demoted (`design/01` §6). A pin cites nothing — it is a
    /// claim about the present, and a demoted moderator's pins should stop
    /// holding when their authority does.
    fn may_moderate_now(&self, identity: &PerNetworkIdentityId, placement: &Placement) -> bool;
}

/// [`Authority`] over one replayed state, which **cannot judge a redaction.**
///
/// Every present-tense question — membership, posting, moderating *now* — is a
/// question about one state, and this answers all of them. A redaction is not:
/// it cites the governance head its author observed and is judged as of that
/// moment (`design/01` §6), which needs the log.
///
/// So [`Authority::may_moderate_at`] here **refuses**, rather than quietly
/// answering the present-tense question instead. It used to answer it, which
/// meant demoting a moderator retroactively invalidated redactions that should
/// have stood — spec 07 §9 Q1, and O3. Refusing is the fail-closed direction of
/// `design/00` §2's second principle: an unverifiable claim does not get to
/// remove content, so the message stays visible rather than being hidden on an
/// authority nobody checked.
///
/// **Use [`LogAuthority`] to render.** This is for callers that hold no log and
/// ask nothing about a head.
pub struct StateAuthority<'a> {
    /// The replayed state to answer from.
    pub state: &'a GovernanceState,
}

/// [`Authority`] with the log behind it, which is what a reader needs.
///
/// The only implementation that can answer `design/01` §6's actual question:
/// did this moderator hold authority *at the head their redaction cites*. That
/// is a question about a point in the chain, and Core §2.7 is explicit that
/// recomputing state at a point is what the log is for.
///
/// # Why a demotion must not reach backwards
///
/// A moderator who hides a message and is later demoted did a legitimate thing.
/// Answering from current state un-hides every message they ever redacted, at
/// the moment their role changes — so a routine permissions edit silently
/// restores content somebody removed, which is both wrong and invisible. Judging
/// each redaction as of its own cited head keeps it standing, and stops them
/// making new ones, which is exactly the difference asked for.
///
/// # The cache is not an optimisation detail
///
/// A channel's redactions commonly cite a handful of distinct heads between
/// them, and replaying per record would be quadratic in a busy moderated
/// channel. Replayed states are memoised per head. This stays pure — same
/// inputs, same answers, no I/O — which is what `kols-core` requires.
pub struct LogAuthority<'a> {
    /// Current state, for every question that is about now.
    pub state: &'a GovernanceState,
    /// The log, for the questions that are about a point in the chain.
    pub log: &'a GovernanceLog,
    /// Memoised replays, keyed by the head they were replayed to.
    replayed: RefCell<BTreeMap<Hash, Option<GovernanceState>>>,
}

impl<'a> LogAuthority<'a> {
    /// Builds an authority over a state and the log that produced it.
    pub fn new(state: &'a GovernanceState, log: &'a GovernanceLog) -> Self {
        Self {
            state,
            log,
            replayed: RefCell::new(BTreeMap::new()),
        }
    }

    /// The ancestry of `head`, oldest first, or `None` if it is not held here.
    ///
    /// Walked rather than taken from the log, because what is wanted is the
    /// chain *to this head* and the log's own helper answers about the canonical
    /// chain — which is a different question the moment a head sits on a branch
    /// this node has not reconciled onto.
    fn chain_to(&self, head: &Hash) -> Option<Vec<&'a LogEntry>> {
        let mut chain = Vec::new();
        let mut cursor = Some(*head);
        while let Some(hash) = cursor {
            let entry = self.log.get(&hash)?;
            cursor = entry.parent;
            chain.push(entry);
        }
        chain.reverse();
        Some(chain)
    }

    /// Replays to `head`, memoised.
    ///
    /// `None` means this node cannot judge that head — it has not seen it, or
    /// the chain behind it does not replay. Both are the same answer to the
    /// caller and both fail closed.
    fn state_at(&self, head: &Hash, ask: impl Fn(&GovernanceState) -> bool) -> bool {
        let mut cache = self.replayed.borrow_mut();
        let entry = cache.entry(*head).or_insert_with(|| {
            self.chain_to(head)
                .and_then(|chain| GovernanceState::replay(chain).ok())
        });
        entry.as_ref().is_some_and(ask)
    }
}

impl Authority for StateAuthority<'_> {
    fn is_member(&self, identity: &PerNetworkIdentityId) -> bool {
        self.state.is_member(identity)
    }

    fn may_post(&self, identity: &PerNetworkIdentityId, placement: &Placement) -> bool {
        posts(self.state, identity, placement)
    }

    /// Always false: this authority holds no log and cannot judge a head.
    ///
    /// See the type's own documentation for why refusing beats answering the
    /// present-tense question in its place.
    fn may_moderate_at(
        &self,
        _identity: &PerNetworkIdentityId,
        _placement: &Placement,
        _head: &Hash,
    ) -> bool {
        false
    }

    fn may_moderate_now(&self, identity: &PerNetworkIdentityId, placement: &Placement) -> bool {
        moderates(self.state, identity, placement)
    }
}

impl Authority for LogAuthority<'_> {
    fn is_member(&self, identity: &PerNetworkIdentityId) -> bool {
        self.state.is_member(identity)
    }

    fn may_post(&self, identity: &PerNetworkIdentityId, placement: &Placement) -> bool {
        posts(self.state, identity, placement)
    }

    fn may_moderate_at(
        &self,
        identity: &PerNetworkIdentityId,
        placement: &Placement,
        head: &Hash,
    ) -> bool {
        self.state_at(head, |state| moderates(state, identity, placement))
    }

    fn may_moderate_now(&self, identity: &PerNetworkIdentityId, placement: &Placement) -> bool {
        moderates(self.state, identity, placement)
    }
}

/// Whether `identity` may write records into this channel, in `state`.
///
/// Both gates: the network-level right to write a log at all, and the
/// channel-level right to write *this* one.
fn posts(state: &GovernanceState, identity: &PerNetworkIdentityId, placement: &Placement) -> bool {
    state.identity_holds(
        identity,
        &Capability::publish(ContentType::new(crate::CHAT_LOG_CONTENT_TYPE)),
    ) && holds(state, identity, "post", placement)
}

/// Whether `identity` holds moderation authority here, in `state`.
///
/// One implementation, called with *different states* by the two questions that
/// ask it — `may_moderate_now` against current state, `may_moderate_at` against
/// the state as of a cited head. Two copies would be two chances to answer the
/// same question differently, which is the drift this whole distinction exists
/// to prevent.
fn moderates(
    state: &GovernanceState,
    identity: &PerNetworkIdentityId,
    placement: &Placement,
) -> bool {
    state.identity_holds(identity, &Capability::ModerateContent)
        || holds(state, identity, "moderate", placement)
}
