//! The gate every command crosses — `design/05` §3, property 1.

use crate::{Command, Sensitivity};
use intranet_governance::{Capability, GovernanceState};
use intranet_identity::PerNetworkIdentityId;
use kols_core::{
    Authority, CategoryId, ChannelChange, ChannelId, ChatPolicy, MAX_CHANNEL_NAME_BYTES,
    MAX_CHANNEL_TOPIC_BYTES, MAX_REACTION_KEY_BYTES, Placement, holds, holds_in_scope,
};

/// Where replayed state says a channel sits.
///
/// A lookup rather than a field on [`Command`], because a channel's category
/// decides which capability authorizes an action on it, and letting the
/// interface supply it would let the interface choose its own answer.
pub trait Channels {
    /// This channel's placement, or `None` if replay does not know it.
    fn placement(&self, channel: &ChannelId) -> Option<Placement>;
}

impl Channels for std::collections::BTreeMap<ChannelId, Placement> {
    fn placement(&self, channel: &ChannelId) -> Option<Placement> {
        self.get(channel).copied()
    }
}

impl<T: Channels + ?Sized> Channels for &T {
    fn placement(&self, channel: &ChannelId) -> Option<Placement> {
        (**self).placement(channel)
    }
}

/// Who is asking, and what the network currently says.
///
/// Carries both an [`Authority`] and the [`GovernanceState`] it was built from,
/// which looks redundant and is not. The chat verbs go through `Authority`
/// because that trait is where the one distinction the resolution actually has —
/// moderation authority *as of* a governance head rather than now — is visible
/// in the type (`design/01` §6). The protocol's own capabilities have no such
/// distinction and are asked of the state directly.
pub struct Actor<'a, A: Authority, C: Channels> {
    /// The identity acting. Never supplied by the interface.
    pub identity: PerNetworkIdentityId,
    /// Resolution for the chat capability vocabulary.
    pub authority: &'a A,
    /// Replayed state, for the protocol's own capabilities and for policy.
    pub state: &'a GovernanceState,
    /// Where replay says each channel sits.
    pub channels: &'a C,
}

/// Why a command was refused.
///
/// Refusals are values rather than strings so the interface can react to the
/// kind — "you are going too fast" wants different chrome from "you cannot post
/// here" — and so the reason survives crossing the boundary intact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Replay knows no such channel.
    NoSuchChannel(ChannelId),
    /// The actor does not hold what this command needs.
    NotPermitted {
        /// Which command.
        command: &'static str,
        /// What would have authorized it, in the vocabulary of `design/02` §2.2.
        needs: &'static str,
    },
    /// A field that must carry something carried nothing.
    Empty(&'static str),
    /// A field exceeded the bound that applies to it.
    TooLarge {
        /// Which field.
        field: &'static str,
        /// How large it was.
        actual: usize,
        /// The largest it may be.
        limit: usize,
    },
    /// More of something than this network permits.
    TooMany {
        /// Which field.
        field: &'static str,
        /// How many there were.
        actual: usize,
        /// The most there may be.
        limit: usize,
    },
    /// Channel structure was asked for in a network that has no channels.
    ///
    /// A `conversation`-profile network has exactly one implied channel and a
    /// channel entry in it is invalid on replay (`design/03` §4.1), so refusing
    /// here saves writing an entry every node would reject.
    NotAServer,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchChannel(id) => {
                write!(f, "no channel {}", &intranet_crypto::to_hex(id.as_bytes())[..12])
            }
            Self::NotPermitted { command, needs } => {
                write!(f, "{command} needs {needs}, which you do not hold")
            }
            Self::Empty(field) => write!(f, "{field} is empty"),
            Self::TooLarge {
                field,
                actual,
                limit,
            } => write!(f, "{field} is {actual} bytes, and the ceiling here is {limit}"),
            Self::TooMany {
                field,
                actual,
                limit,
            } => write!(f, "{actual} {field}, and the ceiling here is {limit}"),
            Self::NotAServer => {
                write!(f, "this network is a conversation, which has no channels to manage")
            }
        }
    }
}

impl std::error::Error for Refusal {}

/// A command that has been through [`authorize`].
///
/// There is no public constructor and no public field, so the only way to hold
/// one is to have passed the check. That is the point: an executor takes an
/// `Authorized` rather than a [`Command`], and "somebody forgot to check" stops
/// being a thing a reviewer has to notice.
///
/// The protocol repo reached the same shape for the same reason — `authorize` on
/// the media relay's guard is the only way to learn a frame's recipients, so a
/// limiter that computes a verdict and never enforces it is not expressible.
///
/// # The property, as a test the compiler runs
///
/// ```compile_fail
/// # fn forge(command: kols_api::Command) {
/// // The field is private, so this is not a thing a caller can write.
/// let _ = kols_api::Authorized(command);
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authorized(Command);

impl Authorized {
    /// The command, for an executor to act on.
    pub const fn command(&self) -> &Command {
        &self.0
    }

    /// Takes the command out.
    pub fn into_command(self) -> Command {
        self.0
    }

    /// How much consent this command needed.
    pub const fn sensitivity(&self) -> Sensitivity {
        self.0.sensitivity()
    }
}

/// Answers "may this actor do this, and is it within bounds?" before anything is signed.
///
/// Two kinds of question, deliberately answered in one place, because both have
/// to be settled before a signature exists and neither is the interface's to
/// decide:
///
/// - **May they?** Resolved by replaying governance state (`design/02` §3),
///   never by trusting that the interface only offered buttons the user was
///   allowed to press. `design/09` §5 puts it plainly: hiding a control is
///   presentation, and this is the enforcement.
/// - **Is it within this network's bounds?** The author's own client enforces
///   the ceilings first (`design/01` §10.2), so somebody who pastes too much
///   text is told, rather than watching every reader silently refuse the record.
///
/// # What this does not check, and where it is checked instead
///
/// **That an edit or withdrawal targets a message the actor wrote.** Authorship
/// is a fact about the record set, not about replayed state, so answering it
/// here would mean handing this function the store. It is enforced where the
/// design puts it — on read, by [`kols_core::ChannelView`], which rejects an
/// edit or tombstone whose target has a different author. Nobody can write into
/// another author's log in the first place, so the worst case is a record every
/// reader ignores rather than one anybody honours.
///
/// **The message rate ceiling.** It is computed over the author's own HLC
/// timestamps (`design/01` §10.2), which means the author's log, which is again
/// the store rather than replayed state.
pub fn authorize<A: Authority, C: Channels>(
    command: Command,
    actor: &Actor<'_, A, C>,
) -> Result<Authorized, Refusal> {
    let policy = ChatPolicy::of(&actor.state.policy);
    let name = command.name();

    match &command {
        Command::OpenChannel { channel, .. } => {
            let placement = placement_of(actor, channel)?;
            require(
                holds(actor.state, &actor.identity, "read", &placement),
                name,
                "chat:read",
            )?;
        }

        Command::SendMessage {
            channel,
            body,
            attachments,
            ..
        } => {
            let placement = placement_of(actor, channel)?;
            require(
                actor.authority.may_post(&actor.identity, &placement),
                name,
                "chat:post, and publish:chat-log alongside it",
            )?;
            if body.trim().is_empty() {
                return Err(Refusal::Empty("message"));
            }
            bound("message", body.len(), policy.message_max_bytes())?;
            if attachments.len() > policy.attachment_max_count() {
                return Err(Refusal::TooMany {
                    field: "attachments",
                    actual: attachments.len(),
                    limit: policy.attachment_max_count(),
                });
            }
            for attachment in attachments {
                bound(
                    "attachment",
                    attachment.byte_len as usize,
                    policy.attachment_max_bytes(),
                )?;
            }
        }

        Command::EditMessage { channel, body, .. } => {
            let placement = placement_of(actor, channel)?;
            require(
                actor.authority.may_post(&actor.identity, &placement),
                name,
                "chat:post, and publish:chat-log alongside it",
            )?;
            if body.trim().is_empty() {
                return Err(Refusal::Empty("message"));
            }
            bound("message", body.len(), policy.message_max_bytes())?;
        }

        Command::DeleteMessage { channel, .. } => {
            let placement = placement_of(actor, channel)?;
            // Withdrawing writes a tombstone into the actor's own log, so it
            // needs the same right as writing anything else there. It needs no
            // moderation capability, and that is structural rather than a
            // decision: nobody can write another author's log (`design/01` §6).
            require(
                actor.authority.may_post(&actor.identity, &placement),
                name,
                "chat:post, and publish:chat-log alongside it",
            )?;
        }

        Command::React {
            channel, key, ..
        } => {
            let placement = placement_of(actor, channel)?;
            require(
                actor.authority.may_post(&actor.identity, &placement),
                name,
                "chat:post, and publish:chat-log alongside it",
            )?;
            if key.is_empty() {
                return Err(Refusal::Empty("reaction"));
            }
            bound("reaction", key.len(), MAX_REACTION_KEY_BYTES)?;
        }

        Command::Pin { channel, .. } => {
            let placement = placement_of(actor, channel)?;
            require(
                holds(actor.state, &actor.identity, "moderate", &placement)
                    || actor
                        .state
                        .identity_holds(&actor.identity, &Capability::ModerateContent),
                name,
                "chat:moderate",
            )?;
        }

        Command::CreateChannel {
            name: channel_name,
            category,
            topic,
            ..
        } => {
            if !policy.allows_channel_definitions() {
                return Err(Refusal::NotAServer);
            }
            // Scope-only resolution: the channel's id is minted by the entry
            // that creates it, so no grant could name it yet.
            require(
                holds_in_scope(
                    actor.state,
                    &actor.identity,
                    "create-channel",
                    category.as_ref(),
                ),
                name,
                "chat:create-channel, at the network or a category",
            )?;
            if channel_name.trim().is_empty() {
                return Err(Refusal::Empty("channel name"));
            }
            bound("channel name", channel_name.len(), MAX_CHANNEL_NAME_BYTES)?;
            bound("channel topic", topic.len(), MAX_CHANNEL_TOPIC_BYTES)?;
        }

        Command::UpdateChannel { channel, change } => {
            if !policy.allows_channel_definitions() {
                return Err(Refusal::NotAServer);
            }
            let placement = placement_of(actor, channel)?;
            require(
                holds(actor.state, &actor.identity, "manage-channel", &placement),
                name,
                "chat:manage-channel",
            )?;
            check_change(change, &policy)?;
        }

        Command::AdmitMember { .. } => require(
            actor
                .state
                .identity_holds(&actor.identity, &Capability::ApproveNode),
            name,
            "approve-node",
        )?,

        Command::RevokeMember { .. } => require(
            actor
                .state
                .identity_holds(&actor.identity, &Capability::RevokeNode),
            name,
            "revoke-node",
        )?,
    }

    Ok(Authorized(command))
}

fn placement_of<A: Authority, C: Channels>(
    actor: &Actor<'_, A, C>,
    channel: &ChannelId,
) -> Result<Placement, Refusal> {
    actor
        .channels
        .placement(channel)
        .ok_or(Refusal::NoSuchChannel(*channel))
}

fn require(held: bool, command: &'static str, needs: &'static str) -> Result<(), Refusal> {
    if held {
        Ok(())
    } else {
        Err(Refusal::NotPermitted { command, needs })
    }
}

fn bound(field: &'static str, actual: usize, limit: usize) -> Result<(), Refusal> {
    if actual > limit {
        return Err(Refusal::TooLarge {
            field,
            actual,
            limit,
        });
    }
    Ok(())
}

fn check_change(change: &ChannelChange, policy: &ChatPolicy<'_>) -> Result<(), Refusal> {
    match change {
        ChannelChange::Rename(name) => {
            if name.trim().is_empty() {
                return Err(Refusal::Empty("channel name"));
            }
            bound("channel name", name.len(), MAX_CHANNEL_NAME_BYTES)
        }
        ChannelChange::SetTopic(topic) => {
            bound("channel topic", topic.len(), MAX_CHANNEL_TOPIC_BYTES)
        }
        ChannelChange::SetSlowmode(seconds) => {
            let limit = policy.slowmode_max_seconds();
            if i64::from(*seconds) > limit {
                return Err(Refusal::TooLarge {
                    field: "slowmode",
                    actual: *seconds as usize,
                    limit: limit.max(0) as usize,
                });
            }
            Ok(())
        }
        // Recategorising is bounded by the capability check alone: a category is
        // an id, not a value with a range, and `chat:manage-channel` is what
        // decides whether this actor may move the channel at all.
        ChannelChange::Recategorise(_) | ChannelChange::Archive | ChannelChange::Delete => Ok(()),
    }
}

/// Convenience for the common case: a placement map built from replayed state.
pub type PlacementMap = std::collections::BTreeMap<ChannelId, Placement>;

/// Builds a placement from a channel and its category.
pub const fn placement(channel: ChannelId, category: Option<CategoryId>) -> Placement {
    Placement { channel, category }
}
