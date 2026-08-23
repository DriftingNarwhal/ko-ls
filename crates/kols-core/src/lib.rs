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
//! Every encoding here implements `design/08-record-encoding.md`, which is the
//! contract rather than a description of this code. Three things depend on the
//! exact bytes: a record's id is the hash of them, signatures verify against
//! them, and the final merge tie-break compares them. A change that alters an
//! encoding is a wire break, and the test vectors in `tests/` exist to make that
//! fail loudly rather than be re-blessed casually.

#![deny(missing_docs)]

pub mod capabilities;
mod channel;
mod hlc;
mod log;
mod permissions;
mod policy;
mod ids;
mod limits;
mod live;
mod names;
mod record;
mod segment;
mod view;

pub use channel::{
    CHAT_NAMESPACE, ChannelChange, ChannelEntry, ChannelEntryBody, ChannelKind, ChannelRefusal,
    MAX_CHANNEL_NAME_BYTES, MAX_CHANNEL_TOPIC_BYTES, MAX_ROTATION_REASON_BYTES, MembershipAction,
    CategoryChange, EntrySubject, MAX_CATEGORY_NAME_BYTES, Privacy, SidebarCategory,
    SidebarChannel, SidebarRow, admit, sidebar_order,
};
pub use hlc::Hlc;
pub use live::{LivePayload, channel_content_key};
pub use names::{
    MAX_NAME_BYTES, MAX_NAME_GRAPHEMES, NAME_CLAIM_DOMAIN, NAME_CLAIM_KIND, NAME_CLAIM_TAG,
    NameClaim, NameRefusal, Names, name_key, replay_names,
};
pub use log::{AuthorLog, CHAT_LOG_CONTENT_TYPE, Published, publish_head_index};
pub use ids::{
    ChannelId, MessageId, author_log_pointer, author_segment_pointer, channel_browse_collection,
    conversation_channel_id, gossip_topic, moderation_log_pointer, participant_index_collection,
    server_channel_id, thread_channel_id,
};
pub use record::{
    Attachment, DEFAULT_MAX_BODY_BYTES, MAX_ATTACHMENT_NAME_BYTES, MAX_REACTION_KEY_BYTES,
    Record, RecordBody, RecordClass,
};
pub use permissions::{Authority, CategoryId, Placement, StateAuthority, holds, holds_in_scope};
pub use policy::{
    ChatPolicy, MAX_NETWORK_NAME_BYTES, NetworkProfile, Retention, conversation_genesis_values,
    defaults, keys,
};
pub use segment::{MAX_RECORDS_PER_SEGMENT, Segment};
pub use limits::{RATE_WINDOW_MILLIS, ReaderLimits, Withheld, withheld};
pub use view::{ChannelView, Rejection, RenderedMessage};

/// What can go wrong building or reading a record.
///
/// Not `Clone`, because `StorageError` is not — and wrapping it in something
/// cloneable (a `String`, an `Arc`) to win a derive would throw away the
/// structured refusal the storage layer took care to produce.
#[derive(Debug, PartialEq, Eq)]
pub enum CoreError {
    /// The bytes were not a valid encoding of the expected type.
    Decode(intranet_crypto::DecodeError),
    /// A record's signature did not verify against its author's device key.
    BadSignature,
    /// A channel entry's payload discriminant disagreed with its declared kind.
    ///
    /// The two come from different places — the `kind` string from the entry
    /// envelope the protocol reads, the discriminant from bytes it never decodes
    /// — so a disagreement means one was changed by something that could not
    /// change both.
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
    UnknownChannelField(&'static str, u8),
    /// An entry's subject disagrees with what its body is about.
    ///
    /// Unreachable from a decode, where the discriminant chooses both. Reachable
    /// from a value built in memory, which is what this catches — before it is
    /// signed, rather than at every reader that would have to refuse it.
    SubjectMismatch {
        /// The kind whose subject was wrong.
        kind: &'static str,
    },
    /// A record carried a discriminant this build does not recognise.
    ///
    /// Not an error at the reader level — `design/08` §9 requires unknown kinds
    /// be retained and counted rather than rejected — but the decoder still has
    /// to say so, since it cannot produce a typed body it does not know.
    UnknownKind(u8),
    /// A record's clock did not strictly increase within its author's log.
    NonMonotonicClock {
        /// The previous record's reading.
        previous: crate::Hlc,
        /// The reading offered.
        offered: crate::Hlc,
    },
    /// The storage layer refused the publish.
    Storage(intranet_storage::StorageError),
    /// A field exceeded the bound that applies to it.
    TooLarge {
        /// Which field.
        field: &'static str,
        /// How many bytes it carried.
        actual: usize,
        /// The largest it may be.
        limit: usize,
    },
}

impl From<intranet_crypto::DecodeError> for CoreError {
    fn from(err: intranet_crypto::DecodeError) -> Self {
        Self::Decode(err)
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(err) => write!(f, "decode failed: {err}"),
            Self::BadSignature => write!(f, "signature did not verify"),
            Self::UnknownKind(tag) => write!(f, "unknown record kind {tag:#04x}"),
            Self::ChannelKindMismatch { declared, encoded } => write!(
                f,
                "entry declared kind {declared} but its payload encodes {encoded}"
            ),
            Self::UnknownChannelField(field, tag) => {
                write!(f, "unknown {field} discriminant {tag:#04x}")
            }
            Self::SubjectMismatch { kind } => {
                write!(f, "a {kind} entry names the wrong kind of subject")
            }
            Self::NonMonotonicClock { previous, offered } => write!(
                f,
                "clock did not advance: previous {previous:?}, offered {offered:?}"
            ),
            Self::Storage(err) => write!(f, "storage refused the publish: {err}"),
            Self::TooLarge {
                field,
                actual,
                limit,
            } => write!(f, "{field} is {actual} bytes, limit is {limit}"),
        }
    }
}

impl std::error::Error for CoreError {}
