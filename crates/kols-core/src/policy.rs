//! Reading the chat settings out of network policy — spec 07 §4.3, §1.2.
//!
//! Every value here is a network policy value (Core §2.6.2), not a local
//! setting, for two independent reasons either of which would be sufficient.
//!
//! **They are validity rules.** A record past a rate ceiling is *refused* by
//! readers, so a local limit would mean two members rendering different history
//! from the same records — the cross-node divergence the whole design avoids.
//!
//! **They spend other people's resources.** At replication factor 3 a 25 MiB
//! attachment costs 75 MiB network-wide. The person accountable for a network's
//! health is the one who should set that.
//!
//! Absent means the default below, never a refusal — Core §2.6.2 draws that
//! distinction deliberately against the capability registry, where an absent
//! entry *is* refused.

use intranet_governance::{NetworkPolicy, PolicyValue};

/// What kind of network this is — spec 07 §1.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkProfile {
    /// Channels, categories, roles.
    Server,
    /// One implied channel and nothing else.
    Conversation,
}

/// Key names, as they appear in policy. Public because a genesis builder needs
/// them and guessing at a string is how a typo becomes a silent default.
pub mod keys {
    /// `server` or `conversation`.
    pub const PROFILE: &str = "chat:network-profile";
    /// Messages, edits and withdrawals per author per channel per minute.
    pub const MESSAGE_RATE: &str = "chat:message-rate-per-minute";
    /// Reactions and pins per author per channel per minute.
    pub const REACTION_RATE: &str = "chat:reaction-rate-per-minute";
    /// Largest message or edit body, in bytes.
    pub const MESSAGE_MAX_BYTES: &str = "chat:message-max-bytes";
    /// Largest single attachment, in bytes.
    pub const ATTACHMENT_MAX_BYTES: &str = "chat:attachment-max-bytes";
    /// Most attachments on one message.
    pub const ATTACHMENT_MAX_COUNT: &str = "chat:attachment-max-count";
    /// Largest publishable segment, in bytes.
    pub const SEGMENT_MAX_BYTES: &str = "chat:segment-max-bytes";
    /// How far ahead of local time a record may claim to be.
    pub const MAX_FUTURE_SKEW_MILLIS: &str = "chat:max-future-skew-millis";
    /// The largest slowmode a channel manager may set.
    pub const SLOWMODE_MAX_SECONDS: &str = "chat:slowmode-max-seconds";
    /// Days a message segment stays maintained. Zero or absent means forever.
    pub const RETAIN_MESSAGES_DAYS: &str = "chat:retain-messages-days";
    /// Days an attachment stays maintained. Zero or absent means forever.
    pub const RETAIN_ATTACHMENTS_DAYS: &str = "chat:retain-attachments-days";
}

/// The shipped defaults, from spec 07 §4.3.
///
/// Deliberately generous: 30 messages a minute is one every two seconds, which
/// no human sustains and every flood exceeds. These bound abuse; they do not
/// pace conversation, which is per-channel slowmode's job.
pub mod defaults {
    /// See [`super::keys::MESSAGE_RATE`].
    pub const MESSAGE_RATE: i64 = 30;
    /// See [`super::keys::REACTION_RATE`].
    pub const REACTION_RATE: i64 = 60;
    /// See [`super::keys::MESSAGE_MAX_BYTES`].
    pub const MESSAGE_MAX_BYTES: i64 = 8 * 1024;
    /// See [`super::keys::ATTACHMENT_MAX_BYTES`].
    pub const ATTACHMENT_MAX_BYTES: i64 = 25 * 1024 * 1024;
    /// See [`super::keys::ATTACHMENT_MAX_COUNT`].
    pub const ATTACHMENT_MAX_COUNT: i64 = 10;
    /// See [`super::keys::SEGMENT_MAX_BYTES`].
    pub const SEGMENT_MAX_BYTES: i64 = 8 * 1024 * 1024;
    /// See [`super::keys::MAX_FUTURE_SKEW_MILLIS`].
    pub const MAX_FUTURE_SKEW_MILLIS: i64 = 300_000;
    /// See [`super::keys::SLOWMODE_MAX_SECONDS`].
    pub const SLOWMODE_MAX_SECONDS: i64 = 21_600;
    /// See [`super::keys::RETAIN_MESSAGES_DAYS`] — forever.
    pub const RETAIN_MESSAGES_DAYS: i64 = 0;
    /// See [`super::keys::RETAIN_ATTACHMENTS_DAYS`] — forever.
    pub const RETAIN_ATTACHMENTS_DAYS: i64 = 0;
}

/// How long content stays maintained — `design/01` §8's first axis.
///
/// # Why two of these rather than one
///
/// Text and attachments differ in cost by orders of magnitude, and a single
/// window has to be wrong for one of them. A message is capped at 8 KiB and rate
/// limits cap the flow, so a million of them is a few gigabytes network-wide —
/// years of a busy network, and cheap. One attachment may be 25 MiB, ten to a
/// message: a single heavy week outweighs all of that text. A network that wants
/// to bound what it spends on other people's disks almost always means the
/// attachments, and taking the scrollback with them is a cost it did not intend.
///
/// # Retention is a decision to stop maintaining, not to delete
///
/// Content past its window stops being replicated and, decisively, stops being
/// re-wrapped on epoch rotation. Storage §5.2 already has content with no live
/// wrapping simply going dark, so this needs no new mechanism — only the
/// decision to stop refreshing. It is emphatically **not** deletion: a member who
/// already fetched a segment and its DEK keeps both forever, and no rotation
/// takes that back (Core §3.1). Dropping content makes it unavailable to those
/// who did not already hold it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    /// Kept and re-wrapped for as long as the network exists.
    Forever,
    /// Maintained for this many days, then allowed to go dark.
    Days(u32),
}

impl Retention {
    /// Whether something this many days old is still maintained.
    pub const fn covers(&self, age_days: u32) -> bool {
        match self {
            Self::Forever => true,
            Self::Days(window) => age_days <= *window,
        }
    }
}

/// A typed view over a network's chat settings.
#[derive(Debug, Clone, Copy)]
pub struct ChatPolicy<'a> {
    policy: &'a NetworkPolicy,
}

impl<'a> ChatPolicy<'a> {
    /// Reads the chat settings out of a network's policy.
    pub const fn of(policy: &'a NetworkPolicy) -> Self {
        Self { policy }
    }

    /// This network's profile.
    ///
    /// **A network with no profile declared is a `server`.** That is the safe
    /// reading rather than the strict one: it permits channel entries rather
    /// than retroactively invalidating history a node legitimately holds
    /// (spec 07 §1.2). An unrecognised value reads as `server` for the same
    /// reason.
    pub fn profile(&self) -> NetworkProfile {
        match self.policy.app_policy_text(keys::PROFILE, "server") {
            "conversation" => NetworkProfile::Conversation,
            _ => NetworkProfile::Server,
        }
    }

    /// Whether a channel may be declared in this network at all.
    ///
    /// False for a conversation, where the single channel is derived and a
    /// `ChannelDefinition` entry is invalid on replay.
    pub fn allows_channel_definitions(&self) -> bool {
        matches!(self.profile(), NetworkProfile::Server)
    }

    /// Message-class records per author per channel per minute.
    pub fn message_rate_per_minute(&self) -> i64 {
        self.policy
            .app_policy_int(keys::MESSAGE_RATE, defaults::MESSAGE_RATE)
    }

    /// Reaction-class records per author per channel per minute.
    pub fn reaction_rate_per_minute(&self) -> i64 {
        self.policy
            .app_policy_int(keys::REACTION_RATE, defaults::REACTION_RATE)
    }

    /// Largest message or edit body, in bytes.
    pub fn message_max_bytes(&self) -> usize {
        self.non_negative(keys::MESSAGE_MAX_BYTES, defaults::MESSAGE_MAX_BYTES)
    }

    /// Largest single attachment, in bytes.
    pub fn attachment_max_bytes(&self) -> usize {
        self.non_negative(keys::ATTACHMENT_MAX_BYTES, defaults::ATTACHMENT_MAX_BYTES)
    }

    /// Most attachments on one message.
    pub fn attachment_max_count(&self) -> usize {
        self.non_negative(keys::ATTACHMENT_MAX_COUNT, defaults::ATTACHMENT_MAX_COUNT)
    }

    /// Largest publishable segment, in bytes.
    pub fn segment_max_bytes(&self) -> usize {
        self.non_negative(keys::SEGMENT_MAX_BYTES, defaults::SEGMENT_MAX_BYTES)
    }

    /// How far ahead of local time a record may claim to be.
    pub fn max_future_skew_millis(&self) -> i64 {
        self.policy
            .app_policy_int(keys::MAX_FUTURE_SKEW_MILLIS, defaults::MAX_FUTURE_SKEW_MILLIS)
    }

    /// The largest slowmode a channel manager may set, in seconds.
    pub fn slowmode_max_seconds(&self) -> i64 {
        self.policy
            .app_policy_int(keys::SLOWMODE_MAX_SECONDS, defaults::SLOWMODE_MAX_SECONDS)
    }

    /// How long message segments stay maintained — `design/01` §8.
    ///
    /// **Forever by default**, and the direction of that choice is deliberate.
    /// Retention can be switched on whenever a network wants it; content already
    /// allowed to go dark cannot be brought back. So a network that never thinks
    /// about this keeps its history, which is both what a chat product is
    /// expected to do and the recoverable side of the mistake.
    pub fn retain_messages(&self) -> Retention {
        Self::retention(
            self.policy
                .app_policy_int(keys::RETAIN_MESSAGES_DAYS, defaults::RETAIN_MESSAGES_DAYS),
        )
    }

    /// How long attachments stay maintained — `design/01` §8.
    ///
    /// Separate from messages because the costs are not comparable. A message is
    /// bounded at 8 KiB and its rate is capped, so a million of them is a few
    /// gigabytes network-wide — years of a busy network. One attachment may be
    /// 25 MiB, ten to a message, so a single heavy week outweighs all of that
    /// text. A network bounding what it spends on other members' disks nearly
    /// always means these, and one shared window would make it pay for that in
    /// scrollback it never intended to give up.
    pub fn retain_attachments(&self) -> Retention {
        Self::retention(self.policy.app_policy_int(
            keys::RETAIN_ATTACHMENTS_DAYS,
            defaults::RETAIN_ATTACHMENTS_DAYS,
        ))
    }

    /// Reads a day count as a window, treating anything meaningless as forever.
    ///
    /// Zero and negative both mean forever rather than "expire immediately",
    /// which is the fail-safe reading: a value that arrived corrupted, or from a
    /// client with a different idea of the key, must not quietly start
    /// discarding a network's history.
    const fn retention(days: i64) -> Retention {
        if days <= 0 || days > u32::MAX as i64 {
            Retention::Forever
        } else {
            Retention::Days(days as u32)
        }
    }

    /// Reads a size, refusing to let a negative one become a huge `usize`.
    ///
    /// Policy values are signed, sizes are not, and `-1 as usize` is a limit no
    /// record could ever exceed — a network that set one would silently disable
    /// the bound rather than tighten it. Falling back to the default keeps a
    /// nonsensical setting from being the most permissive one available.
    fn non_negative(&self, key: &str, default: i64) -> usize {
        let value = self.policy.app_policy_int(key, default);
        usize::try_from(value).unwrap_or(default as usize)
    }
}

/// The policy entries a `conversation`-profile network must carry at genesis.
///
/// Only the profile: every other setting has a default, and writing defaults
/// explicitly would freeze today's values into a network that would otherwise
/// pick up a revised default.
pub fn conversation_genesis_values() -> Vec<(String, PolicyValue)> {
    vec![(
        keys::PROFILE.to_owned(),
        PolicyValue::Text("conversation".to_owned()),
    )]
}
