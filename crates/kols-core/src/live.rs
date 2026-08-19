//! The live-delivery payload — spec 07 §5.2, §6.1.
//!
//! # What this path is for, and what it must never become
//!
//! The durable path costs a segment publish, a pointer update and a fetch:
//! seconds, not milliseconds. Chat needs milliseconds, so a record is *also*
//! broadcast as it is written. **Nothing may depend on that broadcast.** Missed
//! payloads arrive with the next segment fetch; duplicates are idempotent
//! because records are content-addressed; out-of-order arrival is irrelevant
//! because order is computed rather than received. A client with this disabled
//! is slower and completely correct, and §6.1 requires conformance be testable
//! that way.
//!
//! That is not a caveat, it is the design. Every guarantee in this system rests
//! on the durable path, so this one can be lossy, out of order or entirely
//! absent without changing what any reader converges on.
//!
//! # Why the payload is sealed at all
//!
//! The transport carries opaque bytes and validates nothing (Core §5.1), so a
//! gossip mesh may include peers this network would not serve content to.
//! Sealing under a key derived from the epoch means a payload is readable only
//! by members who already hold that epoch — the same audience the durable path
//! would serve, reached a second way.
//!
//! # Why it carries its rotation
//!
//! An epoch advances on every membership change, and a payload published either
//! side of one must still open. Carrying the `rotation_ref` lets a receiver pick
//! the key the sender actually used instead of guessing at its current one and
//! failing during the window where the two differ.

use crate::{ChannelId, CoreError, Record};
use intranet_crypto::{Dec, Enc, Hash, keyed_hash};
use intranet_storage::{Dek, EpochKey};

/// Domain tag for a live payload on the wire — spec 07 §3.2.
const LIVE_DOMAIN: &str = "intranet.wire.chat-live.v1";

/// Domain separator for the channel content key — spec 07 §5.2.
const CONTENT_KEY_DOMAIN: &str = "intranet.chat-channel-key.v1";

/// The key a channel's live payloads are sealed under — spec 07 §5.2.
///
/// Derived from an epoch key the member already legitimately holds, so it adds
/// no trust assumption. What it buys is that the public and private paths become
/// one code path with two key sources rather than two code paths — a private
/// channel will substitute its own group's key here and nothing else changes.
///
/// Bound to the channel *and* the rotation, so a payload cannot be replayed into
/// another channel and a key cannot outlive the epoch it came from.
pub fn channel_content_key(epoch: &EpochKey, channel: &ChannelId, rotation: &Hash) -> Dek {
    let mut context = Enc::domain(CONTENT_KEY_DOMAIN);
    channel.encode(&mut context);
    context.fixed(rotation.as_bytes());
    Dek::from_bytes(*keyed_hash(epoch.expose_for_delivery(), &context.finish()).as_bytes())
}

/// One record, sealed for the live path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivePayload {
    /// The channel this belongs to.
    ///
    /// Outside the sealed part because a receiver needs it to choose a key, and
    /// it is not a secret: the topic already names the channel to anybody
    /// carrying it.
    pub channel: ChannelId,
    /// The rotation whose epoch key sealed this.
    pub rotation: Hash,
    /// The sealed record.
    pub sealed: Vec<u8>,
}

impl LivePayload {
    /// Seals a record for broadcast.
    ///
    /// The record's own canonical bytes go in unchanged, signature included, so
    /// what travels live and what a segment carries months later are the same
    /// bytes — which is what makes a record delivered either way independently
    /// verifiable and identically addressed.
    pub fn seal(record: &Record, epoch: &EpochKey, rotation: Hash) -> Self {
        let key = channel_content_key(epoch, &record.channel, &rotation);
        Self {
            channel: record.channel,
            rotation,
            sealed: key.seal_chunk(&record.canonical_bytes()),
        }
    }

    /// Opens a payload with whichever epoch key sealed it.
    ///
    /// Fails closed on every step: a payload whose rotation this node does not
    /// hold, whose seal does not open, or whose contents are not a well-formed
    /// record for the channel it claims. The signature is checked here too,
    /// because a live payload arrives with no pointer vouching for it — unlike a
    /// segment, where the pointer's signature already covers everything inside.
    ///
    /// What this deliberately does **not** check is whether the author may post
    /// here. That is a question about replayed governance state, which this
    /// crate does not hold — see [`crate::Authority`].
    pub fn open(&self, epoch: &EpochKey) -> Result<Record, CoreError> {
        let key = channel_content_key(epoch, &self.channel, &self.rotation);
        let plaintext = key
            .open_chunk(&self.sealed)
            .map_err(|_| CoreError::BadSignature)?;
        let record = Record::decode(&plaintext)?;

        // A record must belong to the channel whose key opened it. Without this
        // a member of one channel could relay somebody's record into another,
        // and the signature would stay genuine throughout — the same lift that
        // spec 07 §3.5 guards against inside a segment.
        if record.channel != self.channel {
            return Err(CoreError::BadSignature);
        }
        record.verify_signature()?;
        Ok(record)
    }

    /// The bytes to publish.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Enc::domain(LIVE_DOMAIN);
        self.channel.encode(&mut e);
        e.fixed(self.rotation.as_bytes());
        e.bytes(&self.sealed);
        e.finish()
    }

    /// Reads a payload off the wire.
    pub fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        let mut d = Dec::domain(bytes, LIVE_DOMAIN)?;
        let channel = ChannelId::from_bytes(d.fixed::<32>()?);
        let rotation = Hash::from_bytes(d.fixed::<32>()?);
        let sealed = d.bytes()?.to_vec();
        d.finish()?;
        Ok(Self {
            channel,
            rotation,
            sealed,
        })
    }
}
