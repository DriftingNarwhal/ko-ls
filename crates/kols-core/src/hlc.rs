//! Hybrid logical clocks — `design/01` §4, `design/08` §4.1.

use intranet_crypto::{Dec, DecodeError, Enc};

/// A hybrid logical clock reading.
///
/// # Why not a plain timestamp
///
/// Ordering messages by wall clock alone means a reply can sort before the
/// message it answers, whenever the replier's clock runs behind. An HLC advances
/// against the highest clock the author has *observed* in that channel, so
/// causality that was actually witnessed is preserved regardless of skew, while
/// genuinely concurrent messages still sort by something every node agrees on.
///
/// What it does not do is establish a true order between two people typing at
/// once. Nothing without a central sequencer can, and `design/01` §4 says so
/// rather than implying otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hlc {
    /// Milliseconds since the Unix epoch, signed to match `intranet_crypto::Timestamp`.
    pub wall_millis: i64,
    /// Disambiguator for events sharing a millisecond, monotonic per author per channel.
    pub counter: u32,
}

impl Hlc {
    /// Builds a reading.
    pub const fn new(wall_millis: i64, counter: u32) -> Self {
        Self {
            wall_millis,
            counter,
        }
    }

    /// The reading an author should stamp on its next record.
    ///
    /// `now` is this node's wall clock; `observed` is the highest reading the
    /// author has seen in this channel, including its own. The counter advances
    /// instead of the clock whenever the clock has not moved past what was
    /// already observed, which is what keeps the result strictly increasing per
    /// author even when the local clock stalls or steps backwards.
    pub fn next(now: i64, observed: Option<Hlc>) -> Self {
        match observed {
            Some(prev) if now <= prev.wall_millis => Self {
                wall_millis: prev.wall_millis,
                counter: prev.counter.saturating_add(1),
            },
            _ => Self {
                wall_millis: now,
                counter: 0,
            },
        }
    }

    /// Appends this reading to a canonical encoding.
    pub fn encode(&self, enc: &mut Enc) {
        enc.i64(self.wall_millis).u32(self.counter);
    }

    /// Reads a reading.
    pub fn decode(dec: &mut Dec<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            wall_millis: dec.i64()?,
            counter: dec.u32()?,
        })
    }
}
