//! Presence — `design/01` §9, and the one rule `design/09` §4.1 exists to enforce.
//!
//! # What presence is, and the claim it must never make
//!
//! With no server, *offline* and *I have not heard from them* are the same
//! observation. Discord can assert the first because every client is connected
//! to it; this cannot, because a member may be perfectly online and simply not
//! reachable from here. So a beat is **positive evidence only**: hearing one
//! says somebody was there, and hearing none says nothing at all. Every type in
//! this module is shaped so that the absence of a beat has no representation —
//! there is no `Offline` variant to reach for, because there is no observation
//! that would justify one.
//!
//! # Invisible is an absence, not a state
//!
//! `01` §9 makes `invisible` a real per-user setting rather than a courtesy,
//! and the wire is where that has to be honoured: a beat saying "invisible"
//! would tell every member of the network that this node is running and hiding,
//! which is most of what the setting exists to withhold. So invisible is not on
//! the wire at all. It is a local choice to **publish nothing**, and a member
//! who has chosen it is indistinguishable from one whose laptop is shut — which
//! is the only implementation of the setting that means what it says.
//!
//! That asymmetry is why [`Presence`] and [`Beat`] are different types rather
//! than one enum used twice.
//!
//! # Why a beat is signed as well as sealed
//!
//! The seal keeps presence inside the network; it does not say who sent what.
//! Gossip carries a payload to every subscriber and any of them can republish
//! it, so without a signature one member could announce another as present — or
//! keep announcing them long after they left. The signature is over the state
//! *and* the time, so a replayed beat cannot be re-dated either.

use crate::CoreError;
use intranet_crypto::{Dec, Enc, Hash, Signature, VerifyingKey, keyed_hash};
use intranet_identity::{PerNetworkIdentity, PerNetworkIdentityId};
use intranet_storage::{Dek, EpochKey};

const PRESENCE_KEY_DOMAIN: &str = "intranet.chat-presence-key.v1";
const PRESENCE_DOMAIN: &str = "intranet.chat-presence.v1";
const PRESENCE_PAYLOAD_DOMAIN: &str = "intranet.chat-presence-payload.v1";

/// How often a node says it is here, in milliseconds.
///
/// Thirty seconds, from `01` §9.
pub const BEAT_MILLIS: i64 = 30 * 1000;

/// How long a beat is treated as current, in milliseconds.
///
/// Ninety seconds — three beats, so two may be lost to an ordinary gossip
/// hiccup before somebody stops being shown as here. Short enough that a closed
/// laptop stops looking present within about a minute and a half, which is the
/// bound worth stating, since what replaces *here* is **no signal** rather than
/// a claim about where they went.
pub const FRESH_MILLIS: i64 = 3 * BEAT_MILLIS;

/// What a member is telling the network about themselves.
///
/// Deliberately coarse (`01` §9). Anything finer is either a guess this client
/// cannot make or a detail a member did not agree to publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beat {
    /// Here and using the application.
    Here,
    /// Here, but nothing has happened on this machine for a while.
    ///
    /// A claim about *input*, which this node can observe, rather than about
    /// attention, which it cannot.
    Idle,
    /// Here and asking not to be disturbed.
    Busy,
}

impl Beat {
    /// The single byte this is on the wire.
    ///
    /// Explicit rather than derived from declaration order, so reordering the
    /// variants cannot silently change what an older peer reads.
    pub fn code(self) -> u8 {
        match self {
            Self::Here => 1,
            Self::Idle => 2,
            Self::Busy => 3,
        }
    }

    /// Reads a state off the wire, refusing anything it does not know.
    ///
    /// An unknown code is **not** rounded down to `Here`: a future client
    /// inventing a state must not have it read as a claim it did not make.
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Here),
            2 => Some(Self::Idle),
            3 => Some(Self::Busy),
            _ => None,
        }
    }

    /// The name this goes by at the boundary and in the interface.
    pub fn name(self) -> &'static str {
        match self {
            Self::Here => "here",
            Self::Idle => "idle",
            Self::Busy => "busy",
        }
    }
}

/// What *this* member has chosen, which is a larger set than what goes out.
///
/// [`Presence::Invisible`] has no [`Beat`] because it is the choice to send
/// nothing — see the module note. Keeping the two apart in the type system is
/// what stops an invisible member being broadcast as invisible, which is the
/// mistake this design is most likely to make by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// Broadcast this state.
    Show(Beat),
    /// Broadcast nothing at all.
    Invisible,
}

impl Presence {
    /// The state to put on the wire, or `None` while invisible.
    pub fn to_beat(self) -> Option<Beat> {
        match self {
            Self::Show(beat) => Some(beat),
            Self::Invisible => None,
        }
    }

    /// The name this goes by at the boundary and in the interface.
    pub fn name(self) -> &'static str {
        match self {
            Self::Show(beat) => beat.name(),
            Self::Invisible => "invisible",
        }
    }

    /// Reads a choice by name, refusing anything unknown.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "here" => Some(Self::Show(Beat::Here)),
            "idle" => Some(Self::Show(Beat::Idle)),
            "busy" => Some(Self::Show(Beat::Busy)),
            "invisible" => Some(Self::Invisible),
            _ => None,
        }
    }
}

/// The key presence is sealed under.
///
/// Network-scoped rather than channel-scoped, because presence is about a member
/// and not about a room — and because deriving it per channel would mean a node
/// in eight channels sealing the same fact eight times.
///
/// Bound to the rotation for the same reason the live path's key is: a member
/// removed at a rotation stops being able to open what follows it, and presence
/// is exactly the sort of continuing signal an ex-member should stop receiving.
pub fn presence_key(epoch: &EpochKey, rotation: &Hash) -> Dek {
    let mut context = Enc::domain(PRESENCE_KEY_DOMAIN);
    context.fixed(rotation.as_bytes());
    Dek::from_bytes(*keyed_hash(epoch.expose_for_delivery(), &context.finish()).as_bytes())
}

/// One member saying they are here, signed by them and sealed to the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceBeat {
    /// Who is saying it.
    pub who: PerNetworkIdentityId,
    /// What they are saying.
    pub state: Beat,
    /// When they said it, by their own clock.
    ///
    /// Signed over, so a beat cannot be replayed later under a new date. A
    /// receiver judges freshness against **its own** clock and treats a beat
    /// from the future as one it cannot use — see [`PresenceBeat::fresh_at`].
    pub at: i64,
}

impl PresenceBeat {
    /// The bytes the signature covers.
    fn payload(who: &PerNetworkIdentityId, state: Beat, at: i64) -> Enc {
        let mut e = Enc::domain(PRESENCE_PAYLOAD_DOMAIN);
        who.encode(&mut e);
        e.u8(state.code());
        e.i64(at);
        e
    }

    /// Signs a beat as this identity.
    pub fn create(identity: &PerNetworkIdentity, state: Beat, at: i64) -> (Self, Signature) {
        let who = identity.id();
        let signature = identity.sign(&Self::payload(&who, state, at));
        (Self { who, state, at }, signature)
    }

    /// Seals a signed beat for broadcast.
    pub fn seal(
        identity: &PerNetworkIdentity,
        state: Beat,
        at: i64,
        epoch: &EpochKey,
        rotation: Hash,
    ) -> SealedBeat {
        let (beat, signature) = Self::create(identity, state, at);
        let mut e = Enc::domain(PRESENCE_DOMAIN);
        beat.who.encode(&mut e);
        e.u8(beat.state.code());
        e.i64(beat.at);
        e.fixed(signature.as_bytes());
        SealedBeat {
            rotation,
            sealed: presence_key(epoch, &rotation).seal_chunk(&e.finish()),
        }
    }

    /// Whether a beat heard at `heard_at` is still current at `now`.
    ///
    /// **A beat from the future is not fresh**, which is the one case worth
    /// spelling out: clocks disagree, and treating a far-future timestamp as
    /// current would let one member pin themselves as present indefinitely with
    /// a single beat. Freshness is judged against when this node *heard* it,
    /// which is a clock this node controls; `at` is carried for the signature
    /// and for display, never for this decision.
    pub fn fresh_at(heard_at: i64, now: i64) -> bool {
        now >= heard_at && now.saturating_sub(heard_at) < FRESH_MILLIS
    }
}

/// A sealed beat as it travels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedBeat {
    /// The rotation whose epoch key sealed this.
    pub rotation: Hash,
    /// The sealed beat.
    pub sealed: Vec<u8>,
}

impl SealedBeat {
    /// Opens and verifies a beat.
    ///
    /// Fails closed at every step, and the signature check is not optional here
    /// the way it might look: gossip delivers to every subscriber and any of
    /// them may republish, so without this any member could announce any other
    /// as present, or keep announcing them after they had gone.
    pub fn open(&self, epoch: &EpochKey) -> Result<PresenceBeat, CoreError> {
        let plaintext = presence_key(epoch, &self.rotation)
            .open_chunk(&self.sealed)
            .map_err(|_| CoreError::BadSignature)?;
        let mut d = Dec::domain(&plaintext, PRESENCE_DOMAIN)?;
        let who = PerNetworkIdentityId::from_verifying_key(
            VerifyingKey::from_bytes(d.fixed::<32>()?).map_err(|_| CoreError::BadSignature)?,
        );
        let state = Beat::from_code(d.u8()?).ok_or(CoreError::BadSignature)?;
        let at = d.i64()?;
        let signature = Signature::from_bytes(d.fixed::<64>()?);
        d.finish()?;

        who.verifying_key()
            .verify(&PresenceBeat::payload(&who, state, at), &signature)
            .map_err(|_| CoreError::BadSignature)?;
        Ok(PresenceBeat { who, state, at })
    }

    /// The bytes to publish.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Enc::domain(PRESENCE_DOMAIN);
        e.fixed(self.rotation.as_bytes());
        e.bytes(&self.sealed);
        e.finish()
    }

    /// Reads a sealed beat off the wire.
    pub fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        let mut d = Dec::domain(bytes, PRESENCE_DOMAIN)?;
        let rotation = Hash::from_bytes(d.fixed::<32>()?);
        let sealed = d.bytes()?.to_vec();
        d.finish()?;
        Ok(Self { rotation, sealed })
    }
}
