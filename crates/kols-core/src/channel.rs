//! Channel structure as `chat`-namespace application entries — spec 07 §1.3, §3.8.
//!
//! # Why these are payloads rather than entry variants
//!
//! `design/06` §2 asked the protocol for four chat-shaped `EntryBody` variants.
//! What landed instead was one generic application entry (Core §2.7.2) carrying
//! a namespace, a kind, a declared capability and opaque bytes — because four
//! chat variants in the core vocabulary would have shaped the platform around one
//! application, which Core §0 rules out. These are those bytes.
//!
//! # What the generic form moved onto readers
//!
//! Two obligations that a chat-shaped variant would have discharged in the
//! protocol now belong to every conformant client, and both are here:
//!
//! 1. **The declared capability must be the right one** ([`ChannelEntry::required`]).
//!    The protocol verified that the author holds what the entry *declared*; only
//!    something that understands `chat` knows what it *should* have declared.
//!    Without this check, an author holding `chat:post:*` — the most ordinary
//!    grant a network issues — could mint channel structure by declaring that
//!    instead of a capability they do not hold.
//! 2. **A channel entry is invalid in a `conversation` network** ([`admit`]).
//!    That profile has exactly one implied channel (§1.2), and the protocol
//!    carries `chat` payloads without decoding them, so it cannot enforce this
//!    and does not claim to.
//!
//! Neither check is a second authorization pass. Both close a gap a generic
//! carrier cannot see, and a client skipping them would accept structure every
//! conformant client refuses.
//!
//! # Unknown discriminants are refused, not skipped
//!
//! Unlike a record kind, which `design/08` §9 requires be retained and counted
//! without being rendered, a channel entry carries *structure* — a reader either
//! applies it correctly or must not apply it at all. Skipping one silently would
//! leave two nodes with different channel state, which is the divergence replay
//! exists to prevent.

use std::collections::BTreeMap;
use crate::{CategoryId, ChannelId, CoreError, NetworkProfile};
use intranet_crypto::{Dec, Enc, Hash, VerifyingKey, to_hex};
use intranet_governance::{Capability, EntryBody, GovernanceState, is_valid_app_entry_name};
use intranet_identity::PerNetworkIdentityId;

/// Domain tag for a channel entry payload — spec 07 §3.2.
pub(crate) const CHANNEL_ENTRY_DOMAIN: &str = "intranet.chat-channel-entry.v1";

/// The namespace every entry in this module belongs to.
pub const CHAT_NAMESPACE: &str = "chat";

/// Largest channel name, in bytes of UTF-8.
pub const MAX_CHANNEL_NAME_BYTES: usize = 256;

/// Largest channel topic, in bytes of UTF-8.
pub const MAX_CHANNEL_TOPIC_BYTES: usize = 1024;

/// Largest rotation reason, in bytes of UTF-8.
pub const MAX_ROTATION_REASON_BYTES: usize = 256;

/// What a channel is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChannelKind {
    /// Text chat.
    Text,
    /// A voice room.
    Voice,
    /// A stage, where a speaker set broadcasts to listeners.
    Stage,
}

/// Whether a channel's content is readable by the whole network.
///
/// Private is enforced by key possession, not by this flag: the flag records
/// intent and drives whether a channel gets its own MLS group (`design/03` §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Privacy {
    /// Every member may read it.
    Public,
    /// Only the channel's roster may read it.
    Private,
}

/// Adding or removing one identity from a private channel's roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MembershipAction {
    /// Add them.
    Add,
    /// Remove them.
    Remove,
}

/// What a `channel-update` changes.
///
/// One change per entry rather than a struct of optional fields: replay applies
/// entries in order, and an entry that could carry several changes at once would
/// need a rule for what a partially-understood one means. One change is either
/// understood and applied, or refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelChange {
    /// A new name.
    Rename(String),
    /// A new category, or none.
    Recategorise(Option<CategoryId>),
    /// A new topic.
    SetTopic(String),
    /// A new per-channel slowmode, in seconds. Zero is off.
    SetSlowmode(u32),
    /// Archived: readable, not writable.
    Archive,
    /// Deleted: hidden from listings.
    ///
    /// Hidden, not erased. Records already fetched stay fetched, exactly as
    /// `design/01` §6 says of message deletion, and the UI must not imply more.
    Delete,
    /// A new position among its siblings — spec 07 §1.6.
    ///
    /// A change rather than a field on the definition, and deliberately: adding
    /// it to the definition body would re-encode every channel definition
    /// already written, and "never positioned" has to stay distinct from
    /// "positioned at zero" so that a new channel sorts last rather than first.
    SetPosition(u32),
}

impl ChannelChange {
    const fn tag(&self) -> u8 {
        match self {
            Self::Rename(_) => 0x01,
            Self::Recategorise(_) => 0x02,
            Self::SetTopic(_) => 0x03,
            Self::SetSlowmode(_) => 0x04,
            Self::Archive => 0x05,
            Self::Delete => 0x06,
            Self::SetPosition(_) => 0x07,
        }
    }
}

/// The body of a channel entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelEntryBody {
    /// A channel is created.
    Definition {
        /// Human-readable name, unique within the network at definition time.
        name: String,
        /// The category it sits in, which is the scope permissions bind at.
        category: Option<CategoryId>,
        /// What it is for.
        kind: ChannelKind,
        /// Whether the network at large may read it.
        privacy: Privacy,
        /// A short description.
        topic: String,
        /// Seconds between one author's messages. Zero is off.
        slowmode: u32,
    },
    /// A channel's definition changes.
    Update {
        /// What changes.
        change: ChannelChange,
    },
    /// A private channel's roster changes.
    Membership {
        /// Whether they are being added or removed.
        action: MembershipAction,
        /// Whose membership changes.
        identity: PerNetworkIdentityId,
    },
    /// A private channel's key rotates.
    ///
    /// The channel analogue of `EpochRotation`, inheriting its discipline:
    /// tentative until finality, prior channel-epoch secrets retained until then.
    Rotation {
        /// The MLS commit this anchors.
        commit_ref: Hash,
        /// Why it rotated, for the audit trail.
        reason: String,
    },
}

impl ChannelEntryBody {
    /// This body's discriminant — spec 07 §3.8.
    pub const fn tag(&self) -> u8 {
        match self {
            Self::Definition { .. } => 0x01,
            Self::Update { .. } => 0x02,
            Self::Membership { .. } => 0x03,
            Self::Rotation { .. } => 0x04,
        }
    }

    /// The `kind` string this body travels under in an application entry.
    ///
    /// Paired with the discriminant deliberately: the string is what the
    /// protocol carries and what a non-chat reader sees, the discriminant is what
    /// the payload commits to, and [`ChannelEntry::decode`] refuses a pair that
    /// disagrees. Either alone would let a mismatch through.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Definition { .. } => "channel-definition",
            Self::Update { .. } => "channel-update",
            Self::Membership { .. } => "channel-membership",
            Self::Rotation { .. } => "channel-rotation",
        }
    }

    /// The verb whose capability this kind requires — spec 07 §3.8.
    ///
    /// Creating is ordinary and changing is governance, and the split is the
    /// point: a definition grants nobody access to anything, because a new
    /// private channel has an empty roster until a `channel-membership` entry
    /// adds someone — and that entry is governance-tier. The tier follows what
    /// an action can widen, not how consequential it sounds.
    pub const fn required_verb(&self) -> &'static str {
        match self {
            Self::Definition { .. } => "create-channel",
            Self::Update { .. } | Self::Membership { .. } | Self::Rotation { .. } => {
                "manage-channel"
            }
        }
    }
}

/// One piece of channel structure, as carried in a `chat` application entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelEntry {
    /// The channel this concerns.
    ///
    /// Inside the payload rather than only in the entry's envelope, so the
    /// governance entry's hash covers it and nothing relaying the entry can
    /// change which channel it applies to.
    pub channel: ChannelId,
    /// What it says.
    pub body: ChannelEntryBody,
}

impl ChannelEntry {
    /// Builds an entry.
    pub const fn new(channel: ChannelId, body: ChannelEntryBody) -> Self {
        Self { channel, body }
    }

    /// The narrowest capability that authorizes this entry.
    ///
    /// **Not the one an entry must declare.** There is no single such
    /// capability: the protocol checks that the author holds exactly what the
    /// entry declared, so a valid declaration must name something the author
    /// actually has — which depends on whether they were granted this channel,
    /// its category, or the network. [`Self::acceptable`] is the set, and
    /// [`Self::declaration`] picks from it. This is the narrowest member, useful
    /// for saying what was expected when a declaration is refused.
    ///
    /// The channel-scoped form is deliberately unusable for a *definition*
    /// without a category, and this is worth knowing rather than discovering: an
    /// extension capability must be registered by exact name before it resolves
    /// (Core §2.2), and a new channel's id does not exist until the entry
    /// creating it does — so nobody can have registered it. A definition is
    /// authorized by its category's scope or the network-wide form in practice,
    /// which is what `kols-core::capabilities` registers.
    pub fn required(&self) -> Capability {
        let verb = self.body.required_verb();
        match &self.body {
            ChannelEntryBody::Definition {
                category: Some(category),
                ..
            } => Capability::extension(format!(
                "chat:{verb}:cat:{}",
                to_hex(category.as_bytes())
            )),
            _ => Capability::extension(format!(
                "chat:{verb}:{}",
                to_hex(self.channel.as_bytes())
            )),
        }
    }

    /// Every capability that legitimately authorizes this entry.
    ///
    /// Scope resolution is one level deep (`design/02` §3): a channel override,
    /// then its category, then the network-wide form. An entry may declare any of
    /// them, so the check is membership in this set rather than equality with one
    /// name — while still refusing a capability for a *different* channel, which
    /// exact-name matching gives directly.
    pub fn acceptable(&self, category: Option<&CategoryId>) -> Vec<Capability> {
        let verb = self.body.required_verb();
        let mut out = vec![Capability::extension(format!(
            "chat:{verb}:{}",
            to_hex(self.channel.as_bytes())
        ))];
        // A definition's category comes from the entry itself: the channel does
        // not exist in replayed state yet, so there is nothing else to ask. Every
        // other kind takes it from replayed state, since the entry does not
        // restate where the channel lives.
        let category = match &self.body {
            ChannelEntryBody::Definition { category, .. } => category.as_ref(),
            _ => category,
        };
        if let Some(category) = category {
            out.push(Capability::extension(format!(
                "chat:{verb}:cat:{}",
                to_hex(category.as_bytes())
            )));
        }
        out.push(Capability::extension(format!("chat:{verb}:*")));
        out
    }

    /// The payload's canonical bytes — spec 07 §3.8.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Enc::domain(CHANNEL_ENTRY_DOMAIN);
        self.channel.encode(&mut e);
        e.variant(self.body.tag());
        match &self.body {
            ChannelEntryBody::Definition {
                name,
                category,
                kind,
                privacy,
                topic,
                slowmode,
            } => {
                e.str(name);
                e.option(category.as_ref(), |e, c| {
                    e.fixed(c.as_bytes());
                });
                e.variant(match kind {
                    ChannelKind::Text => 0x01,
                    ChannelKind::Voice => 0x02,
                    ChannelKind::Stage => 0x03,
                });
                e.variant(match privacy {
                    Privacy::Public => 0x01,
                    Privacy::Private => 0x02,
                });
                e.str(topic);
                e.u32(*slowmode);
            }
            ChannelEntryBody::Update { change } => {
                e.variant(change.tag());
                match change {
                    ChannelChange::Rename(name) => {
                        e.str(name);
                    }
                    ChannelChange::Recategorise(category) => {
                        e.option(category.as_ref(), |e, c| {
                            e.fixed(c.as_bytes());
                        });
                    }
                    ChannelChange::SetTopic(topic) => {
                        e.str(topic);
                    }
                    ChannelChange::SetSlowmode(seconds) => {
                        e.u32(*seconds);
                    }
                    ChannelChange::SetPosition(position) => {
                        e.u32(*position);
                    }
                    ChannelChange::Archive | ChannelChange::Delete => {}
                }
            }
            ChannelEntryBody::Membership { action, identity } => {
                e.variant(match action {
                    MembershipAction::Add => 0x01,
                    MembershipAction::Remove => 0x02,
                });
                identity.encode(&mut e);
            }
            ChannelEntryBody::Rotation { commit_ref, reason } => {
                e.fixed(commit_ref.as_bytes());
                e.str(reason);
            }
        }
        e.finish()
    }

    /// Reads an entry from a payload, checking it against the `kind` it travelled under.
    ///
    /// The `kind` string and the payload's discriminant are checked against each
    /// other because they come from different places — the string from the entry's
    /// envelope, which the protocol reads, and the discriminant from bytes the
    /// protocol never decodes. A disagreement means one of them was changed by
    /// something that could not change both.
    pub fn decode(kind: &str, payload: &[u8]) -> Result<Self, CoreError> {
        let entry = Self::decode_payload(payload)?;
        if entry.body.kind() != kind {
            return Err(CoreError::ChannelKindMismatch {
                declared: kind.to_owned(),
                encoded: entry.body.kind(),
            });
        }
        Ok(entry)
    }

    /// Reads an entry from a payload alone.
    pub fn decode_payload(payload: &[u8]) -> Result<Self, CoreError> {
        let mut d = Dec::domain(payload, CHANNEL_ENTRY_DOMAIN)?;
        let channel = ChannelId::from_bytes(d.fixed::<32>()?);
        let tag = d.variant()?;
        let body = match tag {
            0x01 => ChannelEntryBody::Definition {
                name: d.str()?.to_owned(),
                category: d.option(|d| {
                    Ok::<_, CoreError>(CategoryId::from_bytes(d.fixed::<32>()?))
                })?,
                kind: match d.variant()? {
                    0x01 => ChannelKind::Text,
                    0x02 => ChannelKind::Voice,
                    0x03 => ChannelKind::Stage,
                    other => return Err(CoreError::UnknownChannelField("channel kind", other)),
                },
                privacy: match d.variant()? {
                    0x01 => Privacy::Public,
                    0x02 => Privacy::Private,
                    other => return Err(CoreError::UnknownChannelField("privacy", other)),
                },
                topic: d.str()?.to_owned(),
                slowmode: d.u32()?,
            },
            0x02 => ChannelEntryBody::Update {
                change: match d.variant()? {
                    0x01 => ChannelChange::Rename(d.str()?.to_owned()),
                    0x02 => ChannelChange::Recategorise(d.option(|d| {
                        Ok::<_, CoreError>(CategoryId::from_bytes(d.fixed::<32>()?))
                    })?),
                    0x03 => ChannelChange::SetTopic(d.str()?.to_owned()),
                    0x04 => ChannelChange::SetSlowmode(d.u32()?),
                    0x05 => ChannelChange::Archive,
                    0x06 => ChannelChange::Delete,
                    0x07 => ChannelChange::SetPosition(d.u32()?),
                    other => return Err(CoreError::UnknownChannelField("channel change", other)),
                },
            },
            0x03 => ChannelEntryBody::Membership {
                action: match d.variant()? {
                    0x01 => MembershipAction::Add,
                    0x02 => MembershipAction::Remove,
                    other => {
                        return Err(CoreError::UnknownChannelField("membership action", other));
                    }
                },
                identity: PerNetworkIdentityId::from_verifying_key(
                    VerifyingKey::from_bytes(d.fixed::<32>()?)
                        .map_err(|_| CoreError::UnknownChannelField("identity", 0))?,
                ),
            },
            0x04 => ChannelEntryBody::Rotation {
                commit_ref: Hash::from_bytes(d.fixed::<32>()?),
                reason: d.str()?.to_owned(),
            },
            other => return Err(CoreError::UnknownChannelField("channel entry kind", other)),
        };
        d.finish()?;
        Ok(Self { channel, body })
    }

    /// Checks field bounds a network cannot loosen.
    ///
    /// Separate from decoding because these are validity rules over a
    /// well-formed value, and separate from network policy because they bound the
    /// governance log, which every joiner replays in full and which never shrinks.
    pub fn check_bounds(&self) -> Result<(), CoreError> {
        let too_large = |field: &'static str, actual: usize, limit: usize| CoreError::TooLarge {
            field,
            actual,
            limit,
        };
        let check = |field: &'static str, s: &str, limit: usize| {
            if s.len() > limit {
                Err(too_large(field, s.len(), limit))
            } else {
                Ok(())
            }
        };
        match &self.body {
            ChannelEntryBody::Definition { name, topic, .. } => {
                check("channel name", name, MAX_CHANNEL_NAME_BYTES)?;
                check("channel topic", topic, MAX_CHANNEL_TOPIC_BYTES)?;
            }
            ChannelEntryBody::Update { change } => match change {
                ChannelChange::Rename(name) => {
                    check("channel name", name, MAX_CHANNEL_NAME_BYTES)?;
                }
                ChannelChange::SetTopic(topic) => {
                    check("channel topic", topic, MAX_CHANNEL_TOPIC_BYTES)?;
                }
                _ => {}
            },
            ChannelEntryBody::Rotation { reason, .. } => {
                check("rotation reason", reason, MAX_ROTATION_REASON_BYTES)?;
            }
            ChannelEntryBody::Membership { .. } => {}
        }
        Ok(())
    }
}

/// Why a channel entry was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelRefusal {
    /// The log entry was not an application entry at all.
    NotAnAppEntry,
    /// The payload was not a well-formed channel entry.
    Malformed(String),
    /// The network's profile has no channels to define.
    ///
    /// A `conversation` network has exactly one implied channel (spec 07 §1.2).
    /// The protocol cannot enforce this — it carries `chat` payloads without
    /// decoding them — so every conformant reader must.
    NotAServer,
    /// The entry declared a capability that does not authorize its kind.
    ///
    /// The protocol checked the author holds what was declared; this checks that
    /// what was declared is what this kind requires. Without it, an author with
    /// any registered `chat` capability could mint channel structure.
    WrongCapability {
        /// What the entry declared.
        declared: Capability,
        /// What its kind requires, at the scope it should have named.
        required: Capability,
    },
    /// The entry was not in the `chat` namespace.
    ForeignNamespace(String),
}

impl std::fmt::Display for ChannelRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnAppEntry => write!(f, "not an application entry"),
            Self::Malformed(why) => write!(f, "malformed channel entry: {why}"),
            Self::NotAServer => {
                write!(f, "a conversation-profile network has no channels to define")
            }
            Self::WrongCapability { declared, required } => write!(
                f,
                "entry declared {declared:?} but its kind requires {required:?}"
            ),
            Self::ForeignNamespace(namespace) => {
                write!(f, "namespace {namespace} is not chat")
            }
        }
    }
}

/// Decides whether a channel entry may be applied — spec 07 §1.2, §3.8.
///
/// The two checks E2's generalised form moved onto readers, in one place so
/// neither can be forgotten. `category` is the category the entry's channel is
/// known to sit in, from replayed state, and is what allows a category-scoped
/// grant to authorize an entry against a channel inside it.
///
/// What this deliberately does **not** do is ask whether the author holds the
/// capability. The protocol already verified that against the entry's declaration
/// before it was accepted into the log; repeating it here would suggest the
/// protocol's check was insufficient, when the actual gap is narrower and is what
/// [`ChannelRefusal::WrongCapability`] names.
pub fn admit(
    entry: &ChannelEntry,
    namespace: &str,
    declared: &Capability,
    profile: NetworkProfile,
    category: Option<&CategoryId>,
) -> Result<(), ChannelRefusal> {
    if namespace != CHAT_NAMESPACE {
        return Err(ChannelRefusal::ForeignNamespace(namespace.to_owned()));
    }
    if profile != NetworkProfile::Server {
        return Err(ChannelRefusal::NotAServer);
    }
    if !entry.acceptable(category).contains(declared) {
        return Err(ChannelRefusal::WrongCapability {
            declared: declared.clone(),
            required: entry.required(),
        });
    }
    Ok(())
}

impl ChannelEntry {
    /// The capability this author should declare, from what they actually hold.
    ///
    /// Narrowest first — channel, then category, then network-wide — so an entry
    /// claims the least authority that authorizes it. `None` means the author
    /// holds nothing that would authorize this entry, and publishing it would
    /// produce a log entry every replaying node refuses.
    ///
    /// # Why this cannot be a constant
    ///
    /// The protocol verifies the author holds *exactly* what the entry declared.
    /// A declaration is therefore only valid if the author has that specific
    /// name, and which name that is depends on how they were granted — a member
    /// with `chat:create-channel:*` and a member with a category grant must
    /// declare different things for the same entry. An earlier version of this
    /// module declared a fixed capability per kind, which made channel creation
    /// work only for Founders: everyone else declared a channel-scoped name they
    /// did not hold, and which could not have been registered in advance because
    /// the channel id did not exist yet.
    pub fn declaration(
        &self,
        state: &GovernanceState,
        author: &PerNetworkIdentityId,
        category: Option<&CategoryId>,
    ) -> Option<Capability> {
        self.acceptable(category)
            .into_iter()
            .find(|capability| state.identity_holds(author, capability))
    }

    /// Packages this entry as an application entry body for the governance log.
    ///
    /// The declaration is chosen from what `author` holds rather than taken from
    /// the caller, so a client cannot publish an entry declaring a capability its
    /// kind does not need — the mistake the reader side of §3.8 exists to catch,
    /// made unavailable here rather than merely detected there.
    ///
    /// Refuses when the author holds nothing that authorizes the entry, which is
    /// better than producing a log entry every replaying node rejects.
    pub fn to_app_entry(
        &self,
        state: &GovernanceState,
        author: &PerNetworkIdentityId,
        category: Option<&CategoryId>,
    ) -> Result<EntryBody, ChannelRefusal> {
        let required = self.declaration(state, author, category).ok_or_else(|| {
            ChannelRefusal::WrongCapability {
                declared: Capability::extension(String::new()),
                required: self.required(),
            }
        })?;
        Ok(EntryBody::AppEntry {
            namespace: CHAT_NAMESPACE.to_owned(),
            kind: self.body.kind().to_owned(),
            required,
            payload: self.encode(),
        })
    }

    /// Reads a channel entry out of a governance log entry, applying every check.
    ///
    /// This is the only way to obtain a [`ChannelEntry`] from replayed state, and
    /// that is deliberate. Both obligations E2's generalised form moved onto
    /// readers — the declared capability must be the right one, and a channel
    /// entry is invalid in a `conversation` network — are on this path, so a
    /// client cannot reach a usable value while skipping either. Decoding a
    /// payload directly is still possible ([`Self::decode_payload`]) but yields
    /// only bytes-to-value, and names itself accordingly.
    ///
    /// `category` is where replayed state says this entry's channel sits, which
    /// is what lets a category-scoped grant authorize an entry against a channel
    /// inside it. A definition takes its category from the entry instead, since
    /// the channel does not exist yet.
    pub fn read(
        body: &EntryBody,
        profile: NetworkProfile,
        category: Option<&CategoryId>,
    ) -> Result<Self, ChannelRefusal> {
        let EntryBody::AppEntry {
            namespace,
            kind,
            required,
            payload,
        } = body
        else {
            return Err(ChannelRefusal::NotAnAppEntry);
        };
        if !is_valid_app_entry_name(namespace, kind) {
            return Err(ChannelRefusal::Malformed(
                "namespace or kind is not well-formed".to_owned(),
            ));
        }
        if namespace != CHAT_NAMESPACE {
            return Err(ChannelRefusal::ForeignNamespace(namespace.clone()));
        }
        let entry =
            Self::decode(kind, payload).map_err(|err| ChannelRefusal::Malformed(err.to_string()))?;
        entry
            .check_bounds()
            .map_err(|err| ChannelRefusal::Malformed(err.to_string()))?;
        admit(&entry, namespace, required, profile, category)?;
        Ok(entry)
    }
}

/// A channel, as the sidebar needs it in order to sort — spec 07 §1.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarChannel {
    /// Which channel.
    pub id: ChannelId,
    /// The category it sits in, if any.
    pub category: Option<CategoryId>,
    /// Its position among its siblings. `None` means never positioned.
    pub position: Option<u32>,
}

/// A category, as the sidebar needs it in order to sort — spec 07 §1.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarCategory {
    /// Which category.
    pub id: CategoryId,
    /// Its position among the other categories. `None` means never positioned.
    pub position: Option<u32>,
}

/// One row of the computed default order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarRow {
    /// A channel in no category. These come before every category.
    Channel(ChannelId),
    /// A category, and the channels inside it in their own order.
    Category {
        /// Which category.
        id: CategoryId,
        /// Its channels, ordered.
        channels: Vec<ChannelId>,
    },
}

/// Orders siblings: positioned before unpositioned, then by position, then by id.
///
/// Ties are **not** an error and must never be refused. Two managers may set the
/// same position concurrently and the log has no way to stop them, so refusing
/// the second entry would mean refusing something valid after the fact. The
/// tie-break is what keeps every reader's answer identical instead — the same
/// move spec 07 §2.9 already makes for concurrent pointer versions.
fn cmp_siblings(a: (Option<u32>, &[u8; 32]), b: (Option<u32>, &[u8; 32])) -> std::cmp::Ordering {
    a.0.is_none()
        .cmp(&b.0.is_none())
        .then_with(|| a.0.unwrap_or(0).cmp(&b.0.unwrap_or(0)))
        .then_with(|| a.1.cmp(b.1))
}

/// The network's default sidebar order — spec 07 §1.6.
///
/// Two-level, and total: uncategorised channels first, then categories by
/// position, then each category's channels within it. Positions compare only
/// among siblings, so a channel's position says nothing about where its category
/// sits and two channels in different categories are never compared at all.
///
/// **Uncategorised sorts first, not last**, and that is normative rather than
/// taste: a channel whose category is removed then has somewhere obvious to
/// appear. Sorting that group last would make recategorising to no category
/// indistinguishable from deletion on any network with more than a screenful.
///
/// **A channel may name a category with no definition**, which is not an error
/// (spec 07 §1.8). It sorts as a category with no position, and what a client
/// labels it is a client's decision rather than replayed state.
///
/// This is the *default*. A member's own arrangement overrides all of it, is
/// carried by no entry and reaches nobody, so it has no business in here.
pub fn sidebar_order(
    channels: &[SidebarChannel],
    categories: &[SidebarCategory],
) -> Vec<SidebarRow> {
    // BTreeMap rather than a hashed one: this order is normative, and a
    // hash-ordered collection would make it depend on a seed.
    let mut grouped: BTreeMap<CategoryId, Vec<&SidebarChannel>> = BTreeMap::new();
    let mut loose: Vec<&SidebarChannel> = Vec::new();
    for channel in channels {
        match channel.category {
            Some(category) => grouped.entry(category).or_default().push(channel),
            None => loose.push(channel),
        }
    }

    let mut positions: BTreeMap<CategoryId, Option<u32>> = BTreeMap::new();
    for category in categories {
        positions.insert(category.id, category.position);
    }
    // A category nothing defined still sorts, because a channel names it.
    for category in grouped.keys() {
        positions.entry(*category).or_insert(None);
    }

    loose.sort_by(|a, b| {
        cmp_siblings((a.position, a.id.as_bytes()), (b.position, b.id.as_bytes()))
    });

    let mut ordered: Vec<(CategoryId, Option<u32>)> = positions.into_iter().collect();
    ordered.sort_by(|a, b| cmp_siblings((a.1, a.0.as_bytes()), (b.1, b.0.as_bytes())));

    let mut rows: Vec<SidebarRow> = loose.iter().map(|c| SidebarRow::Channel(c.id)).collect();
    for (id, _) in ordered {
        let mut inner = grouped.remove(&id).unwrap_or_default();
        inner.sort_by(|a, b| {
            cmp_siblings((a.position, a.id.as_bytes()), (b.position, b.id.as_bytes()))
        });
        rows.push(SidebarRow::Category {
            id,
            channels: inner.into_iter().map(|c| c.id).collect(),
        });
    }
    rows
}
