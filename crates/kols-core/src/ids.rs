//! Derived identifiers — `design/08` §7.
//!
//! Every identifier here is derived rather than allocated, which is what lets
//! any member locate any other member's content from public information alone,
//! with no directory to consult and nothing to keep fresh. Each derivation is
//! domain-separated, so no two can collide even given identical inputs.

use intranet_crypto::{Enc, Hash, hash_bytes, to_hex};
use intranet_governance::PointerId;
use intranet_identity::{NetworkId, PerNetworkIdentityId};

const CHANNEL_DOMAIN: &str = "intranet.chat-channel-id.v1";
const CONVERSATION_DOMAIN: &str = "intranet.chat-conversation-id.v1";
const THREAD_DOMAIN: &str = "intranet.chat-thread-id.v1";
const LOG_POINTER_DOMAIN: &str = "intranet.chat-log-pointer.v1";
const MODERATION_POINTER_DOMAIN: &str = "intranet.chat-moderation-pointer.v1";
const TOPIC_DOMAIN: &str = "intranet.chat-topic.v1";

/// A channel's stable identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChannelId([u8; 32]);

impl ChannelId {
    /// Wraps raw identifier bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrows the raw identifier bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Appends this identifier to a canonical encoding.
    pub fn encode(&self, enc: &mut Enc) {
        enc.fixed(&self.0);
    }

    /// Renders the first 8 hex characters, for human-facing output.
    pub fn short(&self) -> String {
        to_hex(&self.0[..4])
    }
}

/// A record's content-addressed identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MessageId([u8; 32]);

impl MessageId {
    /// Wraps raw identifier bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrows the raw identifier bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Appends this identifier to a canonical encoding.
    pub fn encode(&self, enc: &mut Enc) {
        enc.fixed(&self.0);
    }

    /// Renders the first 8 hex characters, for human-facing output.
    pub fn short(&self) -> String {
        to_hex(&self.0[..4])
    }
}

fn derive(domain: &str, build: impl FnOnce(&mut Enc)) -> [u8; 32] {
    let mut enc = Enc::domain(domain);
    build(&mut enc);
    *hash_bytes(&enc.finish()).as_bytes()
}

/// A declared channel's id, in a `server`-profile network.
pub fn server_channel_id(network: &NetworkId, nonce: &[u8; 32]) -> ChannelId {
    ChannelId(derive(CHANNEL_DOMAIN, |e| {
        network.encode(e);
        e.fixed(nonce);
    }))
}

/// The single implied channel of a `conversation`-profile network.
///
/// Derived rather than declared: a conversation has exactly one channel, so
/// there is nothing to name and nothing to record. A `ChannelDefinition` entry
/// in such a network is invalid (`design/03` §4.1).
pub fn conversation_channel_id(network: &NetworkId) -> ChannelId {
    ChannelId(derive(CONVERSATION_DOMAIN, |e| network.encode(e)))
}

/// A thread's channel id, derived from the message it hangs off.
///
/// Threads cost no governance entry precisely because this is derivable — the
/// first reply creates one implicitly (`design/01` §2.2).
pub fn thread_channel_id(parent: &ChannelId, root: &MessageId) -> ChannelId {
    ChannelId(derive(THREAD_DOMAIN, |e| {
        parent.encode(e);
        root.encode(e);
    }))
}

/// Where one author's log for one channel lives.
///
/// This is the derivation that removes the need for a directory: any member can
/// compute where any other member's messages would be, then simply ask for that
/// pointer (`design/01` §3.2).
pub fn author_log_pointer(channel: &ChannelId, author: &PerNetworkIdentityId) -> PointerId {
    PointerId::from_bytes(derive(LOG_POINTER_DOMAIN, |e| {
        channel.encode(e);
        author.encode(e);
    }))
}

/// Where one moderator's redaction log for one channel lives.
pub fn moderation_log_pointer(channel: &ChannelId, moderator: &PerNetworkIdentityId) -> PointerId {
    PointerId::from_bytes(derive(MODERATION_POINTER_DOMAIN, |e| {
        channel.encode(e);
        moderator.encode(e);
    }))
}

/// The gossip topic carrying a channel's live records.
pub fn gossip_topic(channel: &ChannelId) -> String {
    to_hex(&derive(TOPIC_DOMAIN, |e| channel.encode(e)))
}

/// The append-set collection naming who has posted in a channel.
///
/// A best-effort accelerator in front of the complete-but-slow enumeration path
/// (`design/01` §3.2) — a stale or missing entry costs a slower first load, never
/// a lost message, which is the only way an append-set may be relied on.
pub fn participant_index_collection(network: &NetworkId, channel: &ChannelId) -> Hash {
    intranet_storage::collection_id(
        network,
        &format!("chat:authors:{}", to_hex(channel.as_bytes())),
    )
}

/// The append-set collection listing a network's channels.
pub fn channel_browse_collection(network: &NetworkId) -> Hash {
    intranet_storage::collection_id(network, "chat:channels")
}
