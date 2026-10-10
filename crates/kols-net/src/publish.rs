//! Putting a segment on the wire, and getting one back.

use crate::NetError;
use intranet_governance::PointerId;
use intranet_identity::PerNetworkIdentity;
use intranet_storage::{Cid, Dek};
use intranet_transport::MemberNode;
use kols_core::{Published, Segment};

/// What a publish put on the wire.
#[derive(Debug, Clone)]
pub struct PublishOutcome {
    /// Every chunk of the segment, now held and announced by this node.
    pub chunks: Vec<Cid>,
    /// The chunks this publish introduced, which no peer holds yet.
    pub new_chunks: Vec<Cid>,
    /// The manifest peers resolve the pointer to.
    pub manifest_cid: Cid,
}

/// Stores, announces and points at a freshly published segment.
///
/// Announcement is per chunk rather than per object because that is what the
/// DHT indexes and what a fetching peer looks up. Re-announcing an unchanged
/// chunk is harmless — the record already exists — so this stays a simple loop
/// rather than a diff against what was announced last time.
///
/// **A chunk this node already held is still announced.** Kademlia provider
/// records expire, and dropping the re-announcement would make an old segment
/// quietly unfindable while its holder was still online and serving.
pub fn publish_segment(node: &mut MemberNode, published: &Published) -> PublishOutcome {
    let mut chunks = Vec::with_capacity(published.object.chunks.len());
    for (cid, bytes) in &published.object.chunks {
        let stored = node.store_chunk(bytes.clone());
        debug_assert_eq!(&stored, cid, "storing a chunk must not change its address");
        node.announce_chunk(stored);
        chunks.push(stored);
    }

    // The manifest is itself content the peer must fetch before it can know
    // which chunks to ask for, so it is stored and announced like any other.
    let manifest_bytes = published.object.manifest.canonical_bytes();
    let manifest_cid = node.store_chunk(manifest_bytes);
    node.announce_chunk(manifest_cid);

    node.accept_pointer(published.pointer.clone());

    PublishOutcome {
        chunks,
        new_chunks: published.new_chunks.clone(),
        manifest_cid,
    }
}

/// Reassembles a segment this node has already fetched the chunks for.
///
/// Split from fetching deliberately: fetching is asynchronous and event-driven,
/// while reassembly is a pure function of bytes already held. Keeping them apart
/// means the decode path is testable without a network and the caller decides
/// how long to wait for chunks.
pub fn fetch_segment(
    node: &MemberNode,
    manifest_cid: Cid,
    dek: &Dek,
    max_bytes: usize,
) -> Result<Segment, NetError> {
    let manifest_bytes = node
        .chunk_store()
        .get(&manifest_cid)
        .ok_or(NetError::ChunksUnavailable(vec![manifest_cid]))?;
    let manifest =
        intranet_storage::Manifest::from_bytes(manifest_bytes).map_err(NetError::Storage)?;
    within_ceiling(&manifest, max_bytes)?;

    let mut blobs = std::collections::BTreeMap::new();
    let mut missing = Vec::new();
    for cid in &manifest.chunks {
        match node.chunk_store().get(cid) {
            Some(bytes) => {
                blobs.insert(*cid, bytes.to_vec());
            }
            None => missing.push(*cid),
        }
    }
    if !missing.is_empty() {
        return Err(NetError::ChunksUnavailable(missing));
    }

    let plaintext = intranet_storage::decode(&manifest, &blobs, dek).map_err(NetError::Storage)?;
    Segment::decode(&plaintext).map_err(NetError::Malformed)
}

/// Refuses a segment whose manifest declares more than `max_bytes`.
///
/// Spec 07 §2.2 and §4.3: `chat:segment-max-bytes` is a validity rule readers
/// enforce, and the reason it exists is the fetch — so it is applied to the
/// manifest, which is one small object, before the chunks are asked for. It has
/// to come first for a second reason too, since `decode` reserves the declared
/// length up front.
///
/// **What it does not stop**, said here because the rule reads stronger than
/// this holds: a manifest that *understates* its length. `decode` refuses one,
/// since the bytes must match, but only once every chunk is held, so its chunks
/// are fetched first. Refusing a list longer than its byte count bounds how
/// many there are; how large each is, up to the transport's 16 MiB, only a
/// byte budget in the fetch could bound (`STATUS.md` O28).
///
/// **Flagged, twice.** Spec 07 does not say what the ceiling measures; this reads
/// it as the segment's canonical plaintext, which is what `plaintext_len` is and
/// what the writer and the executor measure too, so all three agree. And the
/// implausibility check assumes a chunk is never empty, which the storage spec
/// does not state; an honest chunker never emits one.
fn within_ceiling(manifest: &intranet_storage::Manifest, max_bytes: usize) -> Result<(), NetError> {
    if manifest.chunks.len() as u64 > manifest.plaintext_len {
        return Err(NetError::Implausible {
            chunks: manifest.chunks.len(),
            declared: manifest.plaintext_len,
        });
    }
    if manifest.plaintext_len > max_bytes as u64 {
        return Err(NetError::TooLarge {
            claimed: manifest.plaintext_len,
            limit: max_bytes,
        });
    }
    Ok(())
}

/// Everything a reader must fetch to read one author's log.
///
/// The manifest first, then the chunks it names — two rounds, because the
/// chunk list lives inside the manifest. A reader that already holds the
/// previous version's chunks fetches only what changed, which is the whole
/// point of the segment model.
///
/// A segment whose manifest declares more than `max_bytes` wants nothing more:
/// the manifest is the last thing fetched for it (spec 07 §2.2), and
/// [`fetch_segment`] refuses it with [`NetError::TooLarge`].
pub fn wanted_chunks(node: &MemberNode, manifest_cid: Cid, max_bytes: usize) -> Vec<Cid> {
    match node.chunk_store().get(&manifest_cid) {
        None => vec![manifest_cid],
        Some(bytes) => match intranet_storage::Manifest::from_bytes(bytes) {
            Err(_) => Vec::new(),
            Ok(manifest) if within_ceiling(&manifest, max_bytes).is_err() => Vec::new(),
            Ok(manifest) => manifest
                .chunks
                .into_iter()
                .filter(|cid| node.chunk_store().get(cid).is_none())
                .collect(),
        },
    }
}

/// The pointer a peer currently holds for an author log, if any.
pub fn known_pointer<'a>(
    node: &'a MemberNode,
    pointer_id: &PointerId,
) -> Option<&'a intranet_storage::MutablePointer> {
    node.pointer(pointer_id)
}

/// Convenience: the identity's own view of what it should fetch next.
pub fn plan_fetch(
    node: &mut MemberNode,
    reader: &PerNetworkIdentity,
    manifest_cid: Cid,
    concurrency: usize,
    max_bytes: usize,
) {
    let wanted = wanted_chunks(node, manifest_cid, max_bytes);
    if !wanted.is_empty() {
        node.fetch_chunks(wanted, reader, concurrency);
    }
}
