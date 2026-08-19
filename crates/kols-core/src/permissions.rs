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

fn channel_cap(verb: &str, channel: &ChannelId) -> Capability {
    Capability::extension(format!("chat:{verb}:{}", to_hex(channel.as_bytes())))
}

fn category_cap(verb: &str, category: &CategoryId) -> Capability {
    Capability::extension(format!("chat:{verb}:cat:{}", to_hex(category.as_bytes())))
}

fn network_cap(verb: &str) -> Capability {
    Capability::extension(format!("chat:{verb}:*"))
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
    if state.identity_holds(identity, &channel_cap(verb, &placement.channel)) {
        return true;
    }
    if let Some(category) = &placement.category
        && state.identity_holds(identity, &category_cap(verb, category))
    {
        return true;
    }
    state.identity_holds(identity, &network_cap(verb))
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
        self.state
            .identity_holds(identity, &Capability::ModerateContent)
            || holds(self.state, identity, "moderate", placement)
    }
}
