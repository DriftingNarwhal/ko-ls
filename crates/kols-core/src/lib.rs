//! Domain model for ko-ls — records, canonical encoding, and derived identifiers.
//!
//! # Scope
//!
//! This crate is deliberately I/O-free and deterministic. Merge ordering, record
//! encoding, identifier derivation and permission resolution are exactly the code
//! that must produce identical answers on every node, and pure functions over
//! explicit inputs are how that stays testable (`design/05` §2).
//!
//! # Encoding is normative
//!
//! Every encoding here implements `distributed-intranet/specs/07` §3, which is
//! the contract rather than a description of this code (`design/08` holds this
//! crate's obligations against it). Three things depend on the
//! exact bytes: a record's id is the hash of them, signatures verify against
//! them, and the final merge tie-break compares them. A change that alters an
//! encoding is a wire break, and the test vectors in `tests/` exist to make that
//! fail loudly rather than be re-blessed casually.

#![deny(missing_docs)]

pub mod capabilities;
mod channel;
mod cursor;
mod dm;
mod hlc;
mod ids;
mod limits;
mod live;
mod log;
mod names;
mod permissions;
mod policy;
mod presence;
mod record;
mod segment;
mod view;

pub use channel::{
    CHAT_NAMESPACE, CategoryChange, ChannelChange, ChannelEntry, ChannelEntryBody, ChannelKind,
    ChannelRefusal, EntrySubject, MAX_CATEGORY_NAME_BYTES, MAX_CHANNEL_NAME_BYTES,
    MAX_CHANNEL_TOPIC_BYTES, MAX_ROTATION_REASON_BYTES, MembershipAction, Privacy, SidebarCategory,
    SidebarChannel, SidebarRow, admit, sidebar_order,
};
pub use cursor::{Cursor, Window};
pub use dm::{DM_KIND, DM_KNOCK_KIND, DM_NAMESPACE, DmInvite, DmKnock};
pub use hlc::Hlc;
pub use ids::{
    ChannelId, MessageId, author_log_pointer, author_segment_pointer, category_id,
    channel_browse_collection, conversation_channel_id, gossip_topic, moderation_log_pointer,
    participant_index_collection, presence_topic, server_channel_id, thread_channel_id,
};
pub use limits::{RATE_WINDOW_MILLIS, ReaderLimits, Withheld, withheld};
pub use live::{LivePayload, channel_content_key};
pub use log::{AuthorLog, CHAT_LOG_CONTENT_TYPE, Published, publish_head_index};
pub use names::{
    MAX_NAME_BYTES, MAX_NAME_GRAPHEMES, NAME_CLAIM_DOMAIN, NAME_CLAIM_KIND, NAME_CLAIM_TAG,
    NameClaim, NameRefusal, Names, name_key, replay_names,
};
pub use permissions::{
    Authority, CategoryId, LogAuthority, Placement, Scope, StateAuthority, holds, holds_in_scope,
    is_verb,
};
pub use policy::{
    ChatPolicy, ChatSetting, MAX_NETWORK_NAME_BYTES, NetworkProfile, Retention, Unit, ZeroMeaning,
    conversation_genesis_values, defaults, keys,
};
pub use presence::{
    BEAT_MILLIS, Beat, FRESH_MILLIS, Presence, PresenceBeat, SealedBeat, presence_key,
};
pub use record::{
    Attachment, DEFAULT_MAX_BODY_BYTES, MAX_ATTACHMENT_NAME_BYTES, MAX_REACTION_KEY_BYTES, Record,
    RecordBody, RecordClass, UnknownBody,
};
pub use segment::{MAX_RECORDS_PER_SEGMENT, Segment, framed};
pub use view::{ChannelView, Rejection, RenderedMessage};

/// What can go wrong building or reading a record.
///
/// Not `Clone`, because `StorageError` is not — and wrapping it in something
/// cloneable (a `String`, an `Arc`) to win a derive would throw away the
/// structured refusal the storage layer took care to produce.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    /// The bytes were not a valid encoding of the expected type.
    #[error("decode failed: {0}")]
    Decode(#[from] intranet_crypto::DecodeError),
    /// A record's signature did not verify against its author's device key.
    #[error("signature did not verify")]
    BadSignature,
    /// A channel entry's payload discriminant disagreed with its declared kind.
    ///
    /// The two come from different places — the `kind` string from the entry
    /// envelope the protocol reads, the discriminant from bytes it never decodes
    /// — so a disagreement means one was changed by something that could not
    /// change both.
    #[error("entry declared kind {declared} but its payload encodes {encoded}")]
    ChannelKindMismatch {
        /// The `kind` the entry travelled under.
        declared: String,
        /// The kind its payload actually encodes.
        encoded: &'static str,
    },
    /// A channel entry carried a discriminant this build does not recognise.
    ///
    /// Refused rather than skipped, unlike an unknown *record* kind: a channel
    /// entry carries structure, and applying part of it would leave two nodes
    /// with different channel state.
    #[error("unknown {0} discriminant {1:#04x}")]
    UnknownChannelField(&'static str, u8),
    /// An entry's subject disagrees with what its body is about.
    ///
    /// Unreachable from a decode, where the discriminant chooses both. Reachable
    /// from a value built in memory, which is what this catches — before it is
    /// signed, rather than at every reader that would have to refuse it.
    #[error("a {kind} entry names the wrong kind of subject")]
    SubjectMismatch {
        /// The kind whose subject was wrong.
        kind: &'static str,
    },
    /// A record carried a discriminant from the reserved range.
    ///
    /// `0xC0`–`0xFF`, which spec 07 §3.3 refuses, and `0x00`, which no class
    /// range covers. An unallocated
    /// discriminant in a class range is not an error: it decodes as
    /// [`crate::RecordBody::Unknown`] and is retained (§3.7).
    #[error("reserved record kind {0:#04x}")]
    UnknownKind(u8),
    /// A record's clock did not strictly increase within its author's log.
    #[error("clock did not advance: previous {previous:?}, offered {offered:?}")]
    NonMonotonicClock {
        /// The previous record's reading.
        previous: crate::Hlc,
        /// The reading offered.
        offered: crate::Hlc,
    },
    /// A direct-message request's proof did not link the pair it should.
    ///
    /// Distinct from [`Self::BadSignature`], and the distinction is the point: the
    /// signatures verified, so somebody really made this statement — it is simply
    /// about a different pair than the request claims. Core §1.2 names that as the
    /// forgery worth guarding against, since a broken signature is caught by
    /// decoding and this is not.
    #[error("the request's proof is valid but links a different pair than it claims")]
    UnlinkedInvite,
    /// The storage layer refused the publish.
    #[error("storage refused the publish: {0}")]
    Storage(intranet_storage::StorageError),
    /// A field exceeded the bound that applies to it.
    #[error("{field} is {actual} bytes, limit is {limit}")]
    TooLarge {
        /// Which field.
        field: &'static str,
        /// How many bytes it carried.
        actual: usize,
        /// The largest it may be.
        limit: usize,
    },
}
