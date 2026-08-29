//! The gate every command crosses — `design/05` §3, property 1.

use crate::{Command, Sensitivity};
use intranet_governance::{Capability, GovernanceState};
use intranet_identity::PerNetworkIdentityId;
use kols_core::{
    CategoryChange, MAX_CATEGORY_NAME_BYTES,
    Authority, CategoryId, ChannelChange, ChannelId, ChatPolicy, MAX_CHANNEL_NAME_BYTES,
    MAX_CHANNEL_TOPIC_BYTES, MAX_REACTION_KEY_BYTES, NameRefusal, Names, Placement, holds,
    holds_in_scope,
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
    /// Who holds which display name, as replay understands it.
    ///
    /// A second replay product alongside `channels`, and here for the same
    /// reason: whether a name is free is a question about the whole network,
    /// and letting the interface answer it would let the interface choose the
    /// answer.
    pub names: &'a Names,
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
    /// A display name could not be claimed.
    Name(NameRefusal),
    /// Channel structure was asked for in a network that has no channels.
    ///
    /// A `conversation`-profile network has exactly one implied channel and a
    /// channel entry in it is invalid on replay (`design/03` §4.1), so refusing
    /// here saves writing an entry every node would reject.
    NotAServer,
    /// A verb outside the vocabulary `design/02` §2.2 defines.
    ///
    /// Refused rather than written, because an unregistered extension name is
    /// refused by the protocol at replay (Core §2.2.1) — so a mistyped verb
    /// produces a grant that resolves for nobody, and an absent grant and a
    /// misspelled one look identical from every side.
    UnknownVerb(String),
    /// A governance-tier verb was aimed at `everyone`.
    ///
    /// Core §2.4 hardcodes this: `everyone` may never hold a governance-tier
    /// capability under *any* configuration, because admission would otherwise
    /// confer governance power. The protocol refuses it too — this refuses it
    /// before an entry every node would reject is signed and replayed forever.
    EveryoneCeiling {
        /// The verb that cannot go there.
        verb: String,
    },
    /// A role's capability set is unrestricted, so it cannot be edited a verb at a time.
    ///
    /// `Founders` holds [`intranet_governance::CapabilitySet::All`] — every
    /// capability, including ones defined later (Core §2.3). Withdrawing one
    /// verb from it means replacing `All` with an explicit set, which is a
    /// different and much larger act than the checkbox that asked for it: it
    /// would silently drop every capability nobody happened to enumerate.
    Unrestricted {
        /// Which role.
        group: String,
    },
    /// A setting was given a negative value.
    ///
    /// Refused rather than written, because every reader of these falls back to
    /// the default on a value it cannot use — so a negative would be replayed
    /// forever and read as the number it was meant to replace.
    Negative {
        /// Which setting.
        field: &'static str,
    },
    /// Auto-admit was asked for on a network whose governance is member-vote.
    ///
    /// Core §2.6's one incompatible pairing: admission cannot be both automatic
    /// and deliberated. The protocol refuses it on replay; this refuses it
    /// before a governance entry is spent finding that out.
    IncoherentAdmission,
    /// No role by that name.
    NoSuchRole(String),
    /// A role by that name already exists.
    RoleExists(String),
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
            Self::Name(refusal) => write!(f, "{refusal}"),
            Self::NotAServer => {
                write!(f, "this network is a conversation, which has no channels to manage")
            }
            Self::UnknownVerb(verb) => write!(
                f,
                "{verb:?} is not one of this application's permissions, and granting it \
                 would produce an entry no node resolves"
            ),
            Self::EveryoneCeiling { verb } => write!(
                f,
                "chat:{verb} is governance-tier, and everyone may never hold one — \
                 otherwise being admitted would itself confer it. Put it on a role instead"
            ),
            Self::Unrestricted { group } => write!(
                f,
                "{group} holds every capability there is, so there is no set to take one \
                 out of. Narrowing it means replacing that with an explicit list"
            ),
            Self::Negative { field } => write!(
                f,
                "{field} cannot be negative — every reader would fall back to the default, \
                 so the change would be recorded forever and do nothing"
            ),
            Self::IncoherentAdmission => write!(
                f,
                "this network decides admission by member vote, so it cannot also admit \
                 automatically. Change the governance model first, or keep explicit intake"
            ),
            Self::NoSuchRole(group) => write!(f, "there is no role called {group:?} here"),
            Self::RoleExists(group) => write!(f, "a role called {group:?} already exists here"),
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

        Command::CreateCategory {
            name: category_name,
            ..
        } => {
            if !policy.allows_channel_definitions() {
                return Err(Refusal::NotAServer);
            }
            // Network-wide only. Nothing encloses a category, so there is no
            // narrower grant that could authorize creating one, and scoping it to
            // the category being created would be circular (spec 07 §1.8).
            require(
                holds_in_scope(actor.state, &actor.identity, "manage-channel", None),
                name,
                "chat:manage-channel:*, which is the only scope that can create a category",
            )?;
            if category_name.trim().is_empty() {
                return Err(Refusal::Empty("category name"));
            }
            bound("category name", category_name.len(), MAX_CATEGORY_NAME_BYTES)?;
        }

        Command::UpdateCategory { category, change } => {
            if !policy.allows_channel_definitions() {
                return Err(Refusal::NotAServer);
            }
            require(
                holds_in_scope(
                    actor.state,
                    &actor.identity,
                    "manage-channel",
                    Some(category),
                ),
                name,
                "chat:manage-channel, at the category or the network",
            )?;
            if let CategoryChange::Rename(category_name) = change {
                if category_name.trim().is_empty() {
                    return Err(Refusal::Empty("category name"));
                }
                bound("category name", category_name.len(), MAX_CATEGORY_NAME_BYTES)?;
            }
            // A position has no range to violate, for the reason `check_change`
            // gives of a channel's, and deleting carries no value at all.
        }

        Command::SetName { name } => {
            require(
                holds_in_scope(actor.state, &actor.identity, "set-name", None),
                "set-name",
                "chat:set-name, which a network grants to everyone at genesis",
            )?;
            // Both halves of spec 07 §3.9 in one call: whether the name can be
            // normalized at all, and whether somebody else already holds the key
            // it normalizes to. Refused here rather than after a log entry every
            // node would ignore.
            actor
                .names
                .claimable(&actor.identity, name)
                .map_err(Refusal::Name)?;
        }

        Command::CreateInvite { uses, .. } => {
            require(
                actor
                    .state
                    .identity_holds(&actor.identity, &Capability::ApproveNode),
                name,
                "approve-node",
            )?;
            if *uses == 0 {
                return Err(Refusal::Empty("uses"));
            }
        }

        Command::SetBootstrapRelays { relays } => {
            require(
                actor
                    .state
                    .identity_holds(&actor.identity, &Capability::DefinePolicy),
                name,
                "define-policy",
            )?;
            if relays.len() > intranet_governance::MAX_BOOTSTRAP_RELAYS {
                return Err(Refusal::TooMany {
                    field: "relays",
                    actual: relays.len(),
                    limit: intranet_governance::MAX_BOOTSTRAP_RELAYS,
                });
            }
            for relay in relays {
                if relay.trim().is_empty() {
                    return Err(Refusal::Empty("relay address"));
                }
                bound(
                    "relay address",
                    relay.len(),
                    intranet_governance::MAX_RELAY_ADDRESS_BYTES,
                )?;
            }
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

        Command::SetNetworkName { name: network_name } => {
            require(
                actor
                    .state
                    .identity_holds(&actor.identity, &Capability::DefinePolicy),
                name,
                "define-policy",
            )?;
            // Empty is legitimate and means unnamed (spec 07 §1.7: a network
            // with no name declared has no name, and clients must not invent
            // one), so only the ceiling is checked.
            bound(
                "network name",
                network_name.len(),
                kols_core::MAX_NETWORK_NAME_BYTES,
            )?;
        }

        Command::SetChatSetting { setting, value } => {
            require(
                actor
                    .state
                    .identity_holds(&actor.identity, &Capability::DefinePolicy),
                name,
                "define-policy",
            )?;
            // **Negative is refused rather than stored**, and the reason is that
            // storing it does nothing visible. Every reader of these falls back
            // to the default on a value it cannot use — `ChatPolicy::non_negative`
            // for the sizes, the retention sentinel for the windows — so a
            // negative would be written, replayed by every joiner forever, and
            // read as the number it was meant to replace. A setting that reports
            // success and changes nothing is worse than a refusal.
            //
            // Zero is *not* refused, because it means something for every one of
            // these — and not the same something (`ChatSetting::zero_means`).
            // Refusing it would take away a network's only way to say "no
            // attachments" or "no ceiling".
            if *value < 0 {
                return Err(Refusal::Negative {
                    field: setting.key(),
                });
            }
        }

        Command::SetAdmissionMode { mode } => {
            require(
                actor
                    .state
                    .identity_holds(&actor.identity, &Capability::DefinePolicy),
                name,
                "define-policy",
            )?;
            // Core §2.6: auto-admit says a valid invite grants membership
            // immediately, member-vote says a quorum decides it, and a network
            // cannot do both. The protocol refuses this pairing on replay, so
            // this is not the enforcement — it is refusing to spend a governance
            // entry, replayed by every joiner forever, on a change that will be
            // rejected. Refused where policy is set rather than where a joiner
            // is turned away, which is §2.6's own point.
            if *mode == intranet_governance::AdmissionMode::AutoAdmit
                && matches!(
                    actor.state.policy.governance_model,
                    intranet_governance::GovernanceModel::MemberVote { .. }
                )
            {
                return Err(Refusal::IncoherentAdmission);
            }
        }

        Command::CreateRole { group } => {
            require(
                actor
                    .state
                    .identity_holds(&actor.identity, &Capability::DefineGroup),
                name,
                "define-group",
            )?;
            if group.as_str().trim().is_empty() {
                return Err(Refusal::Empty("role name"));
            }
            bound("role name", group.as_str().len(), MAX_ROLE_NAME_BYTES)?;
            // `DefineGroup` both creates and redefines, so without this a
            // "create" would silently replace an existing role's capability set
            // with an empty one — the destructive reading of a button that says
            // it adds something.
            if actor.state.groups.contains_key(group) {
                return Err(Refusal::RoleExists(group.to_string()));
            }
        }

        Command::SetPermission {
            group, verb, grant, ..
        } => {
            require(
                actor
                    .state
                    .identity_holds(&actor.identity, &Capability::DefineGroup),
                name,
                "define-group",
            )?;
            if !kols_core::is_verb(verb) {
                return Err(Refusal::UnknownVerb(verb.clone()));
            }
            let target = actor
                .state
                .groups
                .get(group)
                .ok_or_else(|| Refusal::NoSuchRole(group.to_string()))?;
            if matches!(
                target.capabilities,
                intranet_governance::CapabilitySet::All
            ) {
                return Err(Refusal::Unrestricted {
                    group: group.to_string(),
                });
            }
            // Core §2.4's hardcoded ceiling, applied before anything is signed.
            // Only granting is checked: taking a governance-tier verb *off*
            // `everyone` is the repair for a network that somehow has one, and
            // refusing that would make the invariant unfixable.
            if *grant
                && group.is_everyone()
                && kols_core::capabilities::VERBS
                    .iter()
                    .any(|(candidate, tier)| {
                        candidate == verb && *tier == intranet_governance::Tier::Governance
                    })
            {
                return Err(Refusal::EveryoneCeiling { verb: verb.clone() });
            }
        }

        Command::SetRoleMember { group, .. } => {
            // The dynamic-tier capability (Core §2.4): whether this is a
            // governance-tier act depends on what the *target* role holds, and
            // `identity_holds` resolves that against replayed state rather than
            // against anything the interface supplied.
            require(
                actor.state.identity_holds(
                    &actor.identity,
                    &Capability::manage_membership(group.clone()),
                ),
                name,
                "manage-membership for that role",
            )?;
            if !actor.state.groups.contains_key(group) {
                return Err(Refusal::NoSuchRole(group.to_string()));
            }
        }
    }

    Ok(Authorized(command))
}

/// The longest a role's name may be.
///
/// A group name is replayed by every joiner and rendered in chrome, so it is
/// bounded for the reason spec 07 §1.7 bounds a network's: an unbounded one is
/// both replayed bloat and a denial-of-display. The protocol sets no ceiling of
/// its own, so this is the client's and is stated rather than assumed.
pub const MAX_ROLE_NAME_BYTES: usize = 64;

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
        //
        // A position has no range to violate either — every `u32` is a legitimate
        // one, and two channels sharing a position is explicitly not an error
        // (spec 07 §1.6), because concurrent managers can produce it and the
        // tie-break is what keeps readers agreeing rather than a refusal.
        ChannelChange::Recategorise(_)
        | ChannelChange::Archive
        | ChannelChange::Delete
        | ChannelChange::SetPosition(_) => Ok(()),
    }
}

/// Convenience for the common case: a placement map built from replayed state.
pub type PlacementMap = std::collections::BTreeMap<ChannelId, Placement>;

/// Builds a placement from a channel and its category.
pub const fn placement(channel: ChannelId, category: Option<CategoryId>) -> Placement {
    Placement { channel, category }
}
