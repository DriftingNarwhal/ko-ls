//! What the interface may ask for, and how much a request costs in consent.

use kols_core::{
    Attachment, CategoryId, ChannelChange, ChannelId, Hlc, MessageId, Privacy,
};

/// One thing the interface can ask the core to do.
///
/// Every variant names its target explicitly. There is no "current channel" and
/// no "acting as", because either would be ambient authority — state held on one
/// side of the boundary that the other side's checks depend on. `design/05` §3
/// makes the absence of that the first property of this surface, and it is
/// cheaper to keep than to retrofit.
///
/// # What is deliberately not here yet
///
/// This is P1's vocabulary — text chat in public channels — and it stops there
/// on purpose. Direct messages and search (`00` §5, P2), voice (P3) and stage
/// (P4) each have a command in `design/05` §3 and no code behind them; adding
/// the variants now would put a claim in a type that nothing could serve, which
/// is the same reason `STATUS` §5 refuses to create empty crates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Render a channel, or a page of its history.
    OpenChannel {
        /// Which channel.
        channel: ChannelId,
        /// Page backwards from this reading, or from the head when absent.
        before: Option<Hlc>,
        /// How many messages to return.
        limit: usize,
    },
    /// Write a message.
    SendMessage {
        /// Which channel.
        channel: ChannelId,
        /// The text, as the user typed it.
        body: String,
        /// The message this replies to, if any.
        reply_to: Option<MessageId>,
        /// Files published alongside it.
        attachments: Vec<Attachment>,
    },
    /// Revise one of the user's own messages.
    EditMessage {
        /// Which channel.
        channel: ChannelId,
        /// The message being revised.
        target: MessageId,
        /// Its new text.
        body: String,
    },
    /// Withdraw one of the user's own messages.
    DeleteMessage {
        /// Which channel.
        channel: ChannelId,
        /// The message being withdrawn.
        target: MessageId,
    },
    /// React to a message, or take a reaction back.
    React {
        /// Which channel.
        channel: ChannelId,
        /// The message reacted to.
        target: MessageId,
        /// The reaction itself.
        key: String,
        /// Whether this removes a previous reaction.
        remove: bool,
    },
    /// Pin a message, or unpin it.
    Pin {
        /// Which channel.
        channel: ChannelId,
        /// The message pinned.
        target: MessageId,
        /// Whether this unpins.
        remove: bool,
    },
    /// Define a new channel.
    CreateChannel {
        /// Its name.
        name: String,
        /// The category it belongs to, if any.
        category: Option<CategoryId>,
        /// Whether it is restricted to a roster.
        privacy: Privacy,
        /// A short description.
        topic: String,
    },
    /// Change an existing channel's definition.
    UpdateChannel {
        /// Which channel.
        channel: ChannelId,
        /// What about it changes.
        change: ChannelChange,
    },
    /// Claim a display name in this network.
    ///
    /// Carries no identity, and that is the security property rather than an
    /// economy: the claim binds whoever the executor is acting as, so there is
    /// no field in which to name somebody else (spec 07 §3.9).
    SetName {
        /// The name, exactly as the member typed it.
        name: String,
    },
    /// Mint an invite somebody can redeem to join this network.
    ///
    /// Signs a credential rather than writing to the log, which is why it is
    /// still a signed action: what it produces travels on its own and is
    /// verified by whoever receives it (Core §5.6).
    CreateInvite {
        /// How many identities may be admitted with it.
        uses: u32,
        /// How long it stays valid.
        valid_for_hours: i64,
    },
    /// Replace the relays this network designates as entry points.
    ///
    /// Core §5.5: a hosted relay is not a member and cannot advertise itself,
    /// so replayed policy is the only thing that carries a newly deployed one to
    /// members who already joined.
    SetBootstrapRelays {
        /// Multiaddrs, replacing the current set outright.
        ///
        /// Replacing rather than appending, because "which relays does this
        /// network use" is one answer and a command that only ever added would
        /// have no way to retire a relay that is gone.
        relays: Vec<String>,
    },
    /// Admit an identity to the network.
    AdmitMember {
        /// Who.
        identity: intranet_identity::PerNetworkIdentityId,
    },
    /// Remove an identity from the network.
    RevokeMember {
        /// Who.
        identity: intranet_identity::PerNetworkIdentityId,
    },
}

/// How much consent a command needs before it runs.
///
/// App Hosting §3.3 is the requirement this exists to meet: **any signed action
/// on the user's behalf must go through a platform-level permission prompt** and
/// must never be directly executable by app code. So the line that matters is
/// whether a command puts the user's signature on something, and everything
/// above [`Sensitivity::Local`] does.
///
/// The split above that line follows the **tier of the capability the command
/// needs**, which `design/02` §2.2 already assigns — not a judgement about how
/// consequential an action feels. That is the same rule spec 07 §3.8 settled for
/// channel entries: the tier follows what an action can widen. It is why
/// [`Command::Pin`] is [`Sensitivity::Governs`] and [`Command::SendMessage`] is
/// not, despite pinning being the smaller act: pinning needs `chat:moderate`,
/// which is governance-tier, and posting needs `chat:post`, which is ordinary.
///
/// **A finer class is never a weaker one.** A consent decorator satisfies §3.3
/// by prompting for everything that is not `Local`; `Governs` exists so it can
/// prompt *differently*, never so it can prompt less.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sensitivity {
    /// Reads. Nothing is signed and nothing leaves this node on the user's behalf.
    Local,
    /// Signs something, under a capability `design/02` §2.2 tiers as ordinary.
    Signs,
    /// Signs something, under a governance-tier capability.
    Governs,
}

impl Command {
    /// How much consent this command needs.
    ///
    /// Total by construction — the match is exhaustive and has no wildcard arm,
    /// so a new command cannot be added without classifying it. A default arm
    /// here would silently make the next variant `Local`, which is the one
    /// mistake this classification cannot survive.
    pub const fn sensitivity(&self) -> Sensitivity {
        match self {
            Self::OpenChannel { .. } => Sensitivity::Local,

            Self::SendMessage { .. }
            | Self::EditMessage { .. }
            | Self::DeleteMessage { .. }
            | Self::React { .. }
            | Self::CreateChannel { .. }
            | Self::SetName { .. } => Sensitivity::Signs,

            Self::Pin { .. }
            | Self::CreateInvite { .. }
            | Self::SetBootstrapRelays { .. }
            | Self::UpdateChannel { .. }
            | Self::AdmitMember { .. }
            | Self::RevokeMember { .. } => Sensitivity::Governs,
        }
    }

    /// The chat verb this command needs, if it needs one.
    ///
    /// `None` for the two that are gated on the protocol's own capabilities
    /// rather than the chat vocabulary — admission and removal are the
    /// network's business, not a channel's, and `approve-node` and
    /// `revoke-node` are governance-tier by the protocol's own definition
    /// rather than by anything `design/02` assigns.
    ///
    /// This is the mapping [`Sensitivity`] is derived from, exposed so the two
    /// can be checked against `kols_core::capabilities::VERBS` rather than
    /// against somebody's recollection of it.
    pub const fn verb(&self) -> Option<&'static str> {
        match self {
            Self::OpenChannel { .. } => Some("read"),
            Self::SendMessage { .. }
            | Self::EditMessage { .. }
            | Self::DeleteMessage { .. }
            | Self::React { .. } => Some("post"),
            Self::Pin { .. } => Some("moderate"),
            Self::CreateChannel { .. } => Some("create-channel"),
            Self::SetName { .. } => Some("set-name"),
            Self::UpdateChannel { .. } => Some("manage-channel"),
            Self::CreateInvite { .. }
            | Self::SetBootstrapRelays { .. }
            | Self::AdmitMember { .. }
            | Self::RevokeMember { .. } => None,
        }
    }

    /// A short name for this command, for refusals and logs.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::OpenChannel { .. } => "open-channel",
            Self::SendMessage { .. } => "send-message",
            Self::EditMessage { .. } => "edit-message",
            Self::DeleteMessage { .. } => "delete-message",
            Self::React { .. } => "react",
            Self::Pin { .. } => "pin",
            Self::CreateChannel { .. } => "create-channel",
            Self::SetName { .. } => "set-name",
            Self::UpdateChannel { .. } => "update-channel",
            Self::CreateInvite { .. } => "create-invite",
            Self::SetBootstrapRelays { .. } => "set-bootstrap-relays",
            Self::AdmitMember { .. } => "admit-member",
            Self::RevokeMember { .. } => "revoke-member",
        }
    }
}
