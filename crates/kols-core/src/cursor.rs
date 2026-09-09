//! A position in a channel's merged order — `design/09` §4.4.

use crate::{Hlc, MessageId};
use intranet_crypto::{from_hex, to_hex};

/// Where a reader is in a channel.
///
/// # Why this is a pair and not a clock reading
///
/// Records merge by reading and **then** by record id (`design/01` §4), because
/// two records can carry the same reading: the counter is monotonic per author
/// *and device* (spec 07 §2.6), so two members writing in the same millisecond
/// collide legitimately and neither is wrong.
///
/// A boundary expressed as a reading alone therefore cannot separate such a
/// pair. Asking for everything before it excludes **both** — the one already
/// drawn and the one never drawn — so a message disappears, no control on
/// screen reveals it, and every layer involved is behaving exactly as written.
/// `design/09` §4.4 is where that is argued; this type is the fix.
///
/// # One definition of the order, not two
///
/// [`crate::ChannelView`] keys its records on this, so the order a page is cut
/// on and the order the view merges in are the same code rather than two copies
/// of a comparator that agree today. This is the discipline the rate verdicts
/// follow by asserting against [`crate::withheld`] rather than reimplementing
/// it: an invariant that two pieces of code must share is better held by one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cursor {
    /// The author's clock reading.
    pub hlc: Hlc,
    /// The record's identifier, which breaks a tie between equal readings.
    pub id: MessageId,
}

impl Cursor {
    /// Builds a position.
    pub const fn new(hlc: Hlc, id: MessageId) -> Self {
        Self { hlc, id }
    }

    /// Renders this position as an opaque string.
    ///
    /// **Opaque on purpose.** The interface receives cursors and hands them
    /// back; it never builds one. An interface that could build a cursor could
    /// build a wrong one, and a clock reading is exactly the shape that invites
    /// arithmetic on it — which is how the boundary bug above would come back
    /// through a different door.
    pub fn to_token(self) -> String {
        let mut raw = Vec::with_capacity(44);
        raw.extend_from_slice(&self.hlc.wall_millis.to_be_bytes());
        raw.extend_from_slice(&self.hlc.counter.to_be_bytes());
        raw.extend_from_slice(self.id.as_bytes());
        to_hex(&raw)
    }

    /// Reads a position back from its opaque string.
    ///
    /// Returns `None` for anything that is not one. A cursor arrives from the
    /// interface, so a malformed one is an ordinary input rather than a fault:
    /// the caller falls back to the newest page, which is always a correct
    /// answer to "show me this channel".
    pub fn from_token(token: &str) -> Option<Self> {
        let raw = from_hex(token)?;
        if raw.len() != 44 {
            return None;
        }
        let wall = i64::from_be_bytes(raw[..8].try_into().ok()?);
        let counter = u32::from_be_bytes(raw[8..12].try_into().ok()?);
        let id: [u8; 32] = raw[12..].try_into().ok()?;
        Some(Self::new(Hlc::new(wall, counter), MessageId::from_bytes(id)))
    }
}

/// Which slice of a channel a reader is holding — `design/09` §4.4.
///
/// # Why the interface asks for a range and not a page
///
/// A page is what the store hands over; what the interface holds is everything
/// it has drawn. Making the *range* the unit is what lets one request both
/// extend it and bring it up to date, so scrolling back and the live tick are
/// the same call rather than two that have to agree about what is loaded.
///
/// The four fields describe it completely:
///
/// - `oldest` absent means there is no range yet, and the answer is the newest
///   `back` records — an ordinary open.
/// - `newest` absent means the range runs to **the tail**. That is what makes it
///   *live*: arrivals appear without anything asking for them, and so does
///   backfill landing inside it, which a tail-only read would never show.
/// - `back` and `forward` extend it. They are how far to reach *beyond* what is
///   already loaded, not how much to return.
///
/// Opening a channel *at* a message — a pin, a reply, eventually a search
/// result — is not a fifth shape: it is a range whose ends are both that
/// message, reached from both directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// Where the loaded range starts, or the newest page when absent.
    pub oldest: Option<Cursor>,
    /// Where it ends, or the tail when absent.
    pub newest: Option<Cursor>,
    /// How many further records to reach back beyond `oldest`.
    pub back: usize,
    /// How many further records to reach forward beyond `newest`.
    pub forward: usize,
}

impl Window {
    /// A fresh live range of `back` records, ending at the tail.
    pub const fn opening(back: usize) -> Self {
        Self {
            oldest: None,
            newest: None,
            back,
            forward: 0,
        }
    }

    /// A detached range centred on one message, `reach` records either side.
    pub const fn around(anchor: Cursor, reach: usize) -> Self {
        Self {
            oldest: Some(anchor),
            newest: Some(anchor),
            back: reach,
            forward: reach,
        }
    }

    /// Whether this range follows the tail.
    pub const fn is_live(&self) -> bool {
        self.newest.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> MessageId {
        MessageId::from_bytes([byte; 32])
    }

    #[test]
    fn a_token_round_trips() {
        let cursor = Cursor::new(Hlc::new(1_700_000_000_123, 7), id(0xAB));
        assert_eq!(Cursor::from_token(&cursor.to_token()), Some(cursor));
    }

    #[test]
    fn a_negative_reading_round_trips() {
        // Wall time is signed to match `intranet_crypto::Timestamp`, and a
        // big-endian two's complement encoding is the part of this that is easy
        // to get wrong without ever seeing it: no ordinary clock produces one.
        let cursor = Cursor::new(Hlc::new(-4_000, 1), id(3));
        assert_eq!(Cursor::from_token(&cursor.to_token()), Some(cursor));
    }

    #[test]
    fn rubbish_is_refused_rather_than_guessed() {
        assert_eq!(Cursor::from_token(""), None);
        assert_eq!(Cursor::from_token("zz"), None);
        assert_eq!(Cursor::from_token(&"ab".repeat(43)), None);
    }

    #[test]
    fn the_reading_decides_before_the_id_does() {
        // The whole point: an id may not outrank a reading, or a record would
        // sort by its hash and history would shuffle.
        let early = Cursor::new(Hlc::new(1, 0), id(0xFF));
        let late = Cursor::new(Hlc::new(2, 0), id(0x00));
        assert!(early < late);
    }

    #[test]
    fn the_id_separates_two_records_sharing_a_reading() {
        // The case a bare `Hlc` cannot express, and the reason this type exists.
        let hlc = Hlc::new(5, 2);
        assert!(Cursor::new(hlc, id(1)) < Cursor::new(hlc, id(2)));
    }

    #[test]
    fn the_counter_outranks_the_id_within_a_millisecond() {
        let same = Hlc::new(9, 0);
        let later = Hlc::new(9, 1);
        assert!(Cursor::new(same, id(0xFF)) < Cursor::new(later, id(0x00)));
    }
}
