//! Permission resolution — `design/02` §3.
//!
//! Every question here is answered by replaying the governance log, never by
//! asking a peer or trusting a cached claim. The resolution order is fixed and
//! one level deep: channel override, then category default, then network
//! default, then denied.

use crate::ChannelId;
use intranet_crypto::{Hash, to_hex};
use intranet_governance::{Capability, ContentType, GovernanceState};
use intranet_identity::PerNetworkIdentityId;

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

/// [`Authority`] over one replayed state.
///
/// **Flagged:** `may_moderate_at` ignores `head` and answers from current state.
/// `design/01` §6 requires the check be made against state *as of* the head the
/// moderator cited, which needs the log rather than one state snapshot. The
/// difference shows only when a moderator is demoted after acting: this
/// implementation retroactively invalidates their past redactions, where the
/// design says they should stand. Correct once a log-backed authority exists;
/// recorded here rather than silently approximated.
pub struct StateAuthority<'a> {
    /// The replayed state to answer from.
    pub state: &'a GovernanceState,
}

impl Authority for StateAuthority<'_> {
    fn is_member(&self, identity: &PerNetworkIdentityId) -> bool {
        self.state.is_member(identity)
    }

    fn may_post(&self, identity: &PerNetworkIdentityId, placement: &Placement) -> bool {
        // Both gates: the network-level right to write a log at all, and the
        // channel-level right to write *this* one.
        self.state.identity_holds(
            identity,
            &Capability::publish(ContentType::new(crate::CHAT_LOG_CONTENT_TYPE)),
        ) && holds(self.state, identity, "post", placement)
    }

    fn may_moderate_at(
        &self,
        identity: &PerNetworkIdentityId,
        placement: &Placement,
        _head: &Hash,
    ) -> bool {
        self.may_moderate_now(identity, placement)
    }

    fn may_moderate_now(&self, identity: &PerNetworkIdentityId, placement: &Placement) -> bool {
        self.state
            .identity_holds(identity, &Capability::ModerateContent)
            || holds(self.state, identity, "moderate", placement)
    }
}
