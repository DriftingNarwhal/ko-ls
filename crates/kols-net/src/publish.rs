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
) -> Result<Segment, NetError> {
    let manifest_bytes = node
        .chunk_store()
        .get(&manifest_cid)
        .ok_or(NetError::ChunksUnavailable(vec![manifest_cid]))?;
    let manifest =
        intranet_storage::Manifest::from_bytes(manifest_bytes).map_err(NetError::Storage)?;

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

/// Everything a reader must fetch to read one author's log.
///
/// The manifest first, then the chunks it names — two rounds, because the
/// chunk list lives inside the manifest. A reader that already holds the
/// previous version's chunks fetches only what changed, which is the whole
/// point of the segment model.
pub fn wanted_chunks(node: &MemberNode, manifest_cid: Cid) -> Vec<Cid> {
    match node.chunk_store().get(&manifest_cid) {
        None => vec![manifest_cid],
        Some(bytes) => match intranet_storage::Manifest::from_bytes(bytes) {
            Err(_) => Vec::new(),
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
) {
    let wanted = wanted_chunks(node, manifest_cid);
    if !wanted.is_empty() {
        node.fetch_chunks(wanted, reader, concurrency);
    }
}
