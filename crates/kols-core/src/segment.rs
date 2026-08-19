//! Segments — `design/01` §3.1, encoded per `design/08` §6.

use crate::{ChannelId, CoreError, Record};
use intranet_crypto::{Dec, Enc, Hash};
use intranet_identity::PerNetworkIdentityId;
use intranet_storage::Cid;

/// Domain tag for the segment container.
const SEGMENT_DOMAIN: &str = "intranet.chat-segment.v1";

/// Most records one segment may carry, a decode bound rather than a target.
pub const MAX_RECORDS_PER_SEGMENT: usize = 4096;

/// A run of one author's records in one channel, published as one object.
///
/// # Why the segment carries no signature
///
/// Its authenticity comes from two places that already exist: the mutable
/// pointer naming its CID, signed by the owner, and every record's own
/// signature. A third signature over the same facts would create a state where
/// signers disagree with no rule saying which wins — the reasoning
/// `ModerationEntry` already gives for not carrying one inside a `LogEntry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// The channel these records belong to.
    pub channel: ChannelId,
    /// The author whose log this is.
    pub author: PerNetworkIdentityId,
    /// Position in that author's chain for this channel.
    pub sequence: u64,
    /// The previous segment, or `None` for the first.
    ///
    /// A hash chain backwards, so history is walkable and gap-detectable
    /// without consulting the pointer's version history.
    pub previous: Option<Cid>,
    /// The records, in the order the author appended them.
    pub records: Vec<Record>,
}

impl Segment {
    /// Starts an empty segment.
    pub const fn new(
        channel: ChannelId,
        author: PerNetworkIdentityId,
        sequence: u64,
        previous: Option<Cid>,
    ) -> Self {
        Self {
            channel,
            author,
            sequence,
            previous,
            records: Vec::new(),
        }
    }

    /// The segment's canonical bytes.
    ///
    /// Each record is embedded as its own complete canonical bytes, signature
    /// included, so a reader verifies from the segment alone and re-emits any
    /// record byte-identically.
    ///
    /// # Why the record list carries no count prefix
    ///
    /// Every other sequence in this project is count-prefixed, and this one
    /// deliberately is not. A count sits at the *head* of the encoding and
    /// changes on every append, so the first chunk gets a new CID every time a
    /// message is sent — one whole chunk re-fetched by every reader, per
    /// message, forever. That defeats the delta-fetch property the entire
    /// segment model exists to provide.
    ///
    /// Dropping it costs nothing in framing: every record is individually
    /// length-prefixed and the list runs to end of input, so the encoding stays
    /// injective and unambiguous. Everything before the list is fixed-width, so
    /// an append changes only the tail — which is the property this type is for.
    ///
    /// *Found by the P0 spike measuring bytes actually moved, not by reading the
    /// design. `design/08` §6 records it.*
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut e = Enc::domain(SEGMENT_DOMAIN);
        self.channel.encode(&mut e);
        self.author.encode(&mut e);
        e.u64(self.sequence);
        e.option(self.previous.as_ref(), |e, cid| {
            e.fixed(cid.hash().as_bytes());
        });
        for record in &self.records {
            e.bytes(&record.canonical_bytes());
        }
        e.finish()
    }

    /// Reads a segment.
    ///
    /// Records are decoded individually, so a malformed one fails on its own
    /// rather than taking the segment with it.
    pub fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        let mut dec = Dec::domain(bytes, SEGMENT_DOMAIN)?;
        let channel = ChannelId::from_bytes(dec.fixed::<32>()?);
        let author = PerNetworkIdentityId::from_verifying_key(
            intranet_crypto::VerifyingKey::from_bytes(dec.fixed::<32>()?)
                .map_err(|_| CoreError::BadSignature)?,
        );
        let sequence = dec.u64()?;
        let previous = dec.option(|d| {
            Ok::<_, CoreError>(Cid::from_hash(Hash::from_bytes(d.fixed::<32>()?)))
        })?;
        // Runs to end of input rather than to a count — see `canonical_bytes`.
        let mut records = Vec::new();
        while dec.remaining() > 0 {
            if records.len() == MAX_RECORDS_PER_SEGMENT {
                return Err(CoreError::TooLarge {
                    field: "records per segment",
                    actual: records.len() + 1,
                    limit: MAX_RECORDS_PER_SEGMENT,
                });
            }
            records.push(Record::decode(dec.bytes()?)?);
        }
        dec.finish()?;
        Ok(Self {
            channel,
            author,
            sequence,
            previous,
            records,
        })
    }

    /// Verifies every record's signature, and that each belongs here.
    ///
    /// A record naming a different channel or author than the segment claims is
    /// refused: without this a valid record could be lifted from one author's
    /// log into another's, where it would carry an authorship the pointer's
    /// owner never had.
    pub fn verify(&self) -> Result<(), CoreError> {
        for record in &self.records {
            if record.channel != self.channel || record.author != self.author {
                return Err(CoreError::BadSignature);
            }
            record.verify_signature()?;
        }
        Ok(())
    }

    /// Whether this segment's records are in a valid order.
    ///
    /// Two conditions, and the split between them is what makes multi-device
    /// authorship possible at all:
    ///
    /// 1. **Records ascend by `(hlc, id)`**, the same total order readers merge
    ///    by, so a segment is already in reading order and a reader never has to
    ///    sort within one.
    /// 2. **Readings strictly increase per device**, not per author. One device
    ///    always knows its own last reading and can advance past it; two devices
    ///    of one identity, writing concurrently, genuinely cannot without
    ///    coordinating — and requiring them to would make multi-device
    ///    authorship need a lock. Cross-device ties break by record id instead,
    ///    which every node computes identically.
    ///
    /// *The per-device split was found implementing pointer-collision recovery,
    /// where a merged segment necessarily interleaves two devices' records;
    /// `design/08` §4.1 records it.*
    pub fn ordering_is_valid(&self) -> bool {
        let ascending = self
            .records
            .windows(2)
            .all(|w| (w[0].hlc, w[0].id()) < (w[1].hlc, w[1].id()));

        let mut per_device: std::collections::BTreeMap<_, crate::Hlc> =
            std::collections::BTreeMap::new();
        let per_device_strict = self.records.iter().all(|record| {
            match per_device.insert(record.device, record.hlc) {
                Some(previous) => previous < record.hlc,
                None => true,
            }
        });

        ascending && per_device_strict
    }
}
