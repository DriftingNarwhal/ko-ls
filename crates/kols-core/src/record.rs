//! Records — `design/01` §3.3, encoded per spec 07 §3.3.

use crate::{ChannelId, CoreError, Hlc, MessageId};
use intranet_crypto::{Dec, Enc, Hash, Signature, VerifyingKey, hash_bytes};
use intranet_identity::{DevicePublicKey, PerNetworkIdentity, PerNetworkIdentityId};

/// Domain tag for a record's signed payload.
pub(crate) const RECORD_DOMAIN: &str = "intranet.chat-record.v1";

/// Largest reaction key, a fixed encoding constant rather than network policy.
pub const MAX_REACTION_KEY_BYTES: usize = 64;

/// Largest attachment filename, likewise fixed.
pub const MAX_ATTACHMENT_NAME_BYTES: usize = 255;

/// Default body limit, overridden by the network's `chat:message-max-bytes`.
pub const DEFAULT_MAX_BODY_BYTES: usize = 8 * 1024;

/// Which rate ceiling a record counts against.
///
/// # Why this is a property of the discriminant, not the variant
///
/// Rate ceilings are validity rules, so every node must count the same records —
/// including a node that predates a record kind introduced later. Because the
/// class is a function of the tag's numeric range alone, an old node counts a new
/// kind correctly without understanding it. Had the class been a property of what
/// a variant *means*, two client versions would reach different verdicts on the
/// same records, which is exactly the divergence the rate rule exists to prevent
/// (spec 07 §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecordClass {
    /// Counts against `chat:message-rate-per-minute`.
    Message,
    /// Counts against `chat:reaction-rate-per-minute`.
    Reaction,
    /// Counts against neither; governed by capability instead.
    Control,
    /// Not yet allocated, and refused.
    Reserved,
}

impl RecordClass {
    /// The class a discriminant falls in, knowable without understanding it.
    pub const fn of(tag: u8) -> Self {
        match tag {
            0x01..=0x3F => Self::Message,
            0x40..=0x7F => Self::Reaction,
            0x80..=0xBF => Self::Control,
            _ => Self::Reserved,
        }
    }
}

/// A file published alongside a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// The manifest of the attachment's content object.
    pub manifest_cid: Hash,
    /// Plaintext length, for display before fetching.
    pub byte_len: u64,
    /// Declared media type.
    pub media_type: String,
    /// Filename as the author supplied it.
    pub name: String,
}

/// What a record says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordBody {
    /// A message.
    Message {
        /// The text, exactly as the author wrote it.
        body: String,
        /// The message this replies to, if any.
        reply_to: Option<MessageId>,
        /// Files published with it.
        attachments: Vec<Attachment>,
    },
    /// A revision of one of the author's own messages.
    Edit {
        /// The message being revised.
        target: MessageId,
        /// Its new text.
        body: String,
    },
    /// The author withdrawing one of their own messages.
    Tombstone {
        /// The message being withdrawn.
        target: MessageId,
    },
    /// A reaction to any message.
    Reaction {
        /// The message reacted to.
        target: MessageId,
        /// The reaction itself.
        key: String,
        /// Whether this removes a previous reaction.
        remove: bool,
    },
    /// A pin, or its removal.
    Pin {
        /// The message pinned.
        target: MessageId,
        /// Whether this unpins.
        remove: bool,
    },
    /// A moderator hiding somebody else's message — moderation logs only.
    Redaction {
        /// The message being redacted.
        target: MessageId,
        /// The governance head the moderator's authority is checked against.
        governance_head: Hash,
    },
    /// A kind this build does not know, kept exactly as it arrived.
    ///
    /// Spec 07 §3.7: an unknown kind is retained, counted by class and not
    /// rendered. Refusing it took the whole segment with it, so one record from
    /// a newer client hid every other record its author had in that segment —
    /// and the segments behind it, since the link to the previous one is inside.
    /// Dropping it would make this node re-emit a different record set from the
    /// one it received, which a republished union (spec 07 §2.9) would then
    /// spread. Kept, its id, signature and rate class are the same here as on a
    /// node that understands it, and only what is displayed differs.
    ///
    /// Only a decode produces one: [`UnknownBody`] has no public constructor, so
    /// this build cannot fabricate a kind it does not know. It is not prevented
    /// from re-signing one it received — `Record::create` accepts any body —
    /// and nothing does: the executor builds known kinds only, and `kols-api`
    /// carries no `RecordBody`.
    Unknown(UnknownBody),
}

/// The body of a record whose kind this build does not know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownBody {
    kind: u8,
    bytes: Vec<u8>,
}

impl UnknownBody {
    /// The discriminant it arrived with, in a non-reserved class range.
    pub const fn kind(&self) -> u8 {
        self.kind
    }

    /// The body exactly as encoded, between the discriminant and the signature.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl RecordBody {
    /// This body's discriminant.
    pub const fn tag(&self) -> u8 {
        match self {
            Self::Message { .. } => 0x01,
            Self::Edit { .. } => 0x02,
            Self::Tombstone { .. } => 0x03,
            Self::Reaction { .. } => 0x40,
            Self::Pin { .. } => 0x41,
            Self::Redaction { .. } => 0x80,
            Self::Unknown(unknown) => unknown.kind,
        }
    }

    /// Which rate ceiling this counts against.
    pub const fn class(&self) -> RecordClass {
        RecordClass::of(self.tag())
    }
}

/// One signed contribution to one channel by one author.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The channel this belongs to.
    ///
    /// A channel id already commits to its network (every derivation in
    /// [`crate::ids`] takes the network id), so a record cannot be replayed into
    /// another network and carrying the network id as well would restate a fixed
    /// fact in every record (spec 07 §3.3).
    pub channel: ChannelId,
    /// The identity that wrote it.
    pub author: PerNetworkIdentityId,
    /// The device that signed it.
    ///
    /// Present from v1 although v1 ships single-device: adding an authenticated
    /// field later means a second record encoding supported forever, and 32
    /// bytes now is cheaper than that.
    pub device: DevicePublicKey,
    /// When the author says they wrote it.
    pub hlc: Hlc,
    /// What it says.
    pub body: RecordBody,
    /// The signing device's signature over everything above.
    pub signature: Signature,
}

impl Record {
    /// Builds and signs a record.
    ///
    /// Signing is by the identity's own key here; once device enrollment ships,
    /// the signer becomes the device key and `device` names it. The encoding
    /// does not change, which is the point of carrying the field from the start.
    pub fn create(
        author: &PerNetworkIdentity,
        channel: ChannelId,
        hlc: Hlc,
        body: RecordBody,
    ) -> Self {
        let author_id = author.id();
        let device = DevicePublicKey::from_verifying_key(*author_id.verifying_key());
        let payload = Self::payload(&channel, &author_id, &device, &hlc, &body);
        Self {
            channel,
            author: author_id,
            device,
            hlc,
            body,
            signature: author.sign(&payload),
        }
    }

    /// The bytes the signature covers.
    fn payload(
        channel: &ChannelId,
        author: &PerNetworkIdentityId,
        device: &DevicePublicKey,
        hlc: &Hlc,
        body: &RecordBody,
    ) -> Enc {
        let mut e = Enc::domain(RECORD_DOMAIN);
        channel.encode(&mut e);
        author.encode(&mut e);
        device.encode(&mut e);
        hlc.encode(&mut e);
        e.variant(body.tag());
        match body {
            RecordBody::Message {
                body,
                reply_to,
                attachments,
            } => {
                e.str(body);
                e.option(reply_to.as_ref(), |e, id| id.encode(e));
                e.seq(attachments.iter(), |e, a| {
                    e.fixed(a.manifest_cid.as_bytes());
                    e.u64(a.byte_len);
                    e.str(&a.media_type);
                    e.str(&a.name);
                });
            }
            RecordBody::Edit { target, body } => {
                target.encode(&mut e);
                e.str(body);
            }
            RecordBody::Tombstone { target } => target.encode(&mut e),
            RecordBody::Reaction {
                target,
                key,
                remove,
            } => {
                target.encode(&mut e);
                e.str(key);
                e.bool(*remove);
            }
            RecordBody::Pin { target, remove } => {
                target.encode(&mut e);
                e.bool(*remove);
            }
            RecordBody::Redaction {
                target,
                governance_head,
            } => {
                target.encode(&mut e);
                e.fixed(governance_head.as_bytes());
            }
            // Unframed, because that is how it arrived: the body runs from the
            // discriminant to the signature, and re-emitting it any other way
            // would change the record's id.
            RecordBody::Unknown(unknown) => {
                for &byte in &unknown.bytes {
                    e.u8(byte);
                }
            }
        }
        e
    }

    /// The record's complete canonical bytes, signature included.
    ///
    /// This is what a segment embeds and what gossip carries, so a record read
    /// from storage and the same record received live are byte-identical.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut e = Self::payload(
            &self.channel,
            &self.author,
            &self.device,
            &self.hlc,
            &self.body,
        );
        e.fixed(self.signature.as_bytes());
        e.finish()
    }

    /// Where this record sits in its channel's merged order.
    ///
    /// The pair a page boundary is cut on, and the key
    /// [`crate::ChannelView`] merges on — one definition rather than two that
    /// agree today (`design/09` §4.4).
    pub fn cursor(&self) -> crate::Cursor {
        crate::Cursor::new(self.hlc, self.id())
    }

    /// The record's identifier.
    ///
    /// Hashed over payload *and* signature, matching `AppendSetEntry::entry_id`
    /// — so the id commits to the exact bytes delivered rather than to equal
    /// field values. Ed25519 is deterministic, so this is stable.
    pub fn id(&self) -> MessageId {
        MessageId::from_bytes(*hash_bytes(&self.canonical_bytes()).as_bytes())
    }

    /// Verifies the signature against the signing device's key.
    ///
    /// This answers only "were these bytes signed by that device". Whether the
    /// device is certified for `author`, whether `author` may post here, and
    /// whether the record is within rate are separate checks against replayed
    /// governance state, which this crate deliberately does not perform.
    pub fn verify_signature(&self) -> Result<(), CoreError> {
        let payload = Self::payload(
            &self.channel,
            &self.author,
            &self.device,
            &self.hlc,
            &self.body,
        );
        self.device
            .verifying_key()
            .verify(&payload, &self.signature)
            .map_err(|_| CoreError::BadSignature)
    }

    /// Reads a record from its canonical bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        let mut dec = Dec::domain(bytes, RECORD_DOMAIN)?;
        let channel = ChannelId::from_bytes(dec.fixed::<32>()?);
        let author = PerNetworkIdentityId::from_verifying_key(
            VerifyingKey::from_bytes(dec.fixed::<32>()?).map_err(|_| CoreError::BadSignature)?,
        );
        let device = DevicePublicKey::from_verifying_key(
            VerifyingKey::from_bytes(dec.fixed::<32>()?).map_err(|_| CoreError::BadSignature)?,
        );
        let hlc = Hlc::decode(&mut dec)?;
        let tag = dec.variant()?;
        let body = match tag {
            0x01 => RecordBody::Message {
                body: dec.str()?.to_owned(),
                reply_to: dec
                    .option(|d| Ok::<_, CoreError>(MessageId::from_bytes(d.fixed::<32>()?)))?,
                attachments: dec.seq(|d| {
                    Ok::<_, CoreError>(Attachment {
                        manifest_cid: Hash::from_bytes(d.fixed::<32>()?),
                        byte_len: d.u64()?,
                        media_type: d.str()?.to_owned(),
                        name: d.str()?.to_owned(),
                    })
                })?,
            },
            0x02 => RecordBody::Edit {
                target: MessageId::from_bytes(dec.fixed::<32>()?),
                body: dec.str()?.to_owned(),
            },
            0x03 => RecordBody::Tombstone {
                target: MessageId::from_bytes(dec.fixed::<32>()?),
            },
            0x40 => RecordBody::Reaction {
                target: MessageId::from_bytes(dec.fixed::<32>()?),
                key: dec.str()?.to_owned(),
                remove: dec.bool()?,
            },
            0x41 => RecordBody::Pin {
                target: MessageId::from_bytes(dec.fixed::<32>()?),
                remove: dec.bool()?,
            },
            0x80 => RecordBody::Redaction {
                target: MessageId::from_bytes(dec.fixed::<32>()?),
                governance_head: Hash::from_bytes(dec.fixed::<32>()?),
            },
            other if RecordClass::of(other) == RecordClass::Reserved => {
                return Err(CoreError::UnknownKind(other));
            }
            // Spec 07 §3.7. A body this build cannot parse still has a known
            // extent — a record is delivered whole, and the signature is the
            // last 64 bytes of it — so it can be kept and verified unread.
            other => {
                let start = bytes.len() - dec.remaining();
                let end = bytes
                    .len()
                    .checked_sub(64)
                    .filter(|&end| end >= start)
                    .ok_or(intranet_crypto::DecodeError::UnexpectedEnd {
                        needed: 64,
                        remaining: dec.remaining(),
                    })?;
                let signature = Signature::from_bytes(
                    bytes[end..].try_into().expect("exactly 64 bytes remain"),
                );
                return Ok(Self {
                    channel,
                    author,
                    device,
                    hlc,
                    body: RecordBody::Unknown(UnknownBody {
                        kind: other,
                        bytes: bytes[start..end].to_vec(),
                    }),
                    signature,
                });
            }
        };
        let signature = Signature::from_bytes(dec.fixed::<64>()?);
        dec.finish()?;
        Ok(Self {
            channel,
            author,
            device,
            hlc,
            body,
            signature,
        })
    }

    /// Checks a record's field bounds.
    ///
    /// The body limit is spec 07 §4.3's `chat:message-max-bytes`, and the
    /// reaction-key and attachment-name limits are the two fixed field bounds
    /// the same section states. `max_body` comes from the network's policy; the
    /// other two are fixed, because nothing is gained by letting them vary and
    /// every policy value is one more thing two nodes can briefly disagree on.
    ///
    /// Only the reader half is here. The writer's gate (`kols-api`'s
    /// `authorize`) bounds the body and the reaction key but not an attachment's
    /// name, which no shipped path builds yet: every `SendMessage` the shell and
    /// the harness construct carries no attachments.
    ///
    /// An unknown kind's body is checked against nothing, as spec 07 §3.7
    /// requires: which of its fields a limit applies to is part of what this
    /// build does not know. §3.7 bounds it through the segment carrying it, by
    /// `chat:segment-max-bytes`, and **this reader does not enforce that
    /// ceiling yet.** `ChatPolicy::segment_max_bytes` has no caller outside the
    /// tests, so a segment of any byte size is fetched and read, though
    /// `MAX_RECORDS_PER_SEGMENT` still bounds how many records it holds. That is
    /// a divergence from spec 07 §2.2 for every kind, not only unknown ones
    /// (`STATUS.md` O27).
    pub fn check_bounds(&self, max_body: usize) -> Result<(), CoreError> {
        let too_large = |field, actual, limit| {
            Err(CoreError::TooLarge {
                field,
                actual,
                limit,
            })
        };
        match &self.body {
            RecordBody::Message {
                body, attachments, ..
            } => {
                if body.len() > max_body {
                    return too_large("body", body.len(), max_body);
                }
                for a in attachments {
                    if a.name.len() > MAX_ATTACHMENT_NAME_BYTES {
                        return too_large(
                            "attachment name",
                            a.name.len(),
                            MAX_ATTACHMENT_NAME_BYTES,
                        );
                    }
                }
            }
            RecordBody::Edit { body, .. } => {
                if body.len() > max_body {
                    return too_large("body", body.len(), max_body);
                }
            }
            RecordBody::Reaction { key, .. } if key.len() > MAX_REACTION_KEY_BYTES => {
                return too_large("reaction key", key.len(), MAX_REACTION_KEY_BYTES);
            }
            _ => {}
        }
        Ok(())
    }
}
