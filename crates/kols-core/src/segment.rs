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
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut e = Enc::domain(SEGMENT_DOMAIN);
        self.channel.encode(&mut e);
        self.author.encode(&mut e);
        e.u64(self.sequence);
        e.option(self.previous.as_ref(), |e, cid| {
            e.fixed(cid.hash().as_bytes());
        });
        e.seq(self.records.iter(), |e, r| {
            e.bytes(&r.canonical_bytes());
        });
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
        let records = dec.seq(|d| Record::decode(d.bytes()?))?;
        if records.len() > MAX_RECORDS_PER_SEGMENT {
            return Err(CoreError::TooLarge {
                field: "records per segment",
                actual: records.len(),
                limit: MAX_RECORDS_PER_SEGMENT,
            });
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

    /// Whether an author's readings strictly increase through this segment.
    ///
    /// `design/08` §4.1 requires it, and it is what gives an author's own
    /// records a total order with no separate sequence field, plus duplicate-id
    /// prevention for free.
    pub fn hlcs_strictly_increase(&self) -> bool {
        self.records.windows(2).all(|w| w[0].hlc < w[1].hlc)
    }
}
