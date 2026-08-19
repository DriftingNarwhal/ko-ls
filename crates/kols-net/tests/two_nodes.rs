//! P0 criteria 1–3 over the wire — `design/07` §3.
//!
//! `kols-core`'s tests prove the merge converges given a record set. These prove
//! the record set actually arrives: two live `MemberNode`s, real chunk transfer,
//! real DHT provider records, real pointer sync.

use intranet_crypto::{Hash, Timestamp};
use intranet_governance::{
    Capability, ContentType, EntryBody, GroupId, LogEntry, MembershipAction,
    NetworkPolicy,
};
use intranet_identity::{MasterSeed, NetworkId, PerNetworkIdentity};
use intranet_ledger::{BandwidthCap, CapabilityAdvertisement, ComputeClass};
use intranet_storage::{ChunkSpec, Dek};
use intranet_transport::{MemberNode, NodeEvent};
use kols_core::*;
use kols_net::*;
use libp2p::Multiaddr;
use libp2p::multiaddr::Protocol;
use std::time::Duration;

const NETWORK: NetworkId = NetworkId::from_bytes([42u8; 32]);

fn identity(n: u8) -> PerNetworkIdentity {
    MasterSeed::from_entropy([n; 32])
        .identity_for(&NETWORK)
        .unwrap()
}

fn channel() -> ChannelId {
    server_channel_id(&NETWORK, &[9u8; 32])
}

fn placement() -> Placement {
    Placement {
        channel: channel(),
        category: None,
    }
}

fn message(text: &str) -> RecordBody {
    RecordBody::Message {
        body: text.to_owned(),
        reply_to: None,
        attachments: Vec::new(),
    }
}

fn genesis(founder: &PerNetworkIdentity) -> LogEntry {
    let mut policy = NetworkPolicy::conservative_default();
    policy
        .content_type_allowlist
        .insert(ContentType::new(CHAT_LOG_CONTENT_TYPE));
    policy
        .extension_capabilities
        .extend(kols_core::capabilities::network_scoped());

    LogEntry::create(
        founder,
        None,
        Timestamp::from_millis(0),
        EntryBody::Genesis {
            network: NETWORK,
            policy,
            everyone_capabilities: [
                Capability::ReadContent,
                Capability::publish(CHAT_LOG_CONTENT_TYPE),
                Capability::extension("chat:post:*"),
            ]
            .into_iter()
            .collect(),
        },
    )
}

fn admit(founder: &PerNetworkIdentity, parent: Hash, joiner: &PerNetworkIdentity, at: i64) -> LogEntry {
    LogEntry::create(
        founder,
        Some(parent),
        Timestamp::from_millis(at),
        EntryBody::MembershipChange {
            group: GroupId::everyone(),
            identity: joiner.id(),
            action: MembershipAction::Add { via_invite: None },
        },
    )
}

async fn node(seed: u8) -> (MemberNode, Multiaddr) {
    let identity = identity(seed);
    let mut node = MemberNode::new(&identity).unwrap();
    node.listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap()).unwrap();
    // Provider records go unanswered in client mode, which is the default until
    // a node has a confirmed external address — on loopback that never happens,
    // so every lookup would return nobody.
    node.set_dht_server_mode(true);

    let address = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let NodeEvent::Listening(address) = node.next_event().await
                && address.iter().any(|p| matches!(p, Protocol::Tcp(_)))
            {
                return address;
            }
        }
    })
    .await
    .expect("listens");

    (node, address.with(Protocol::P2p(identity.peer_id())))
}

async fn drive(
    a: &mut MemberNode,
    b: &mut MemberNode,
    limit: Duration,
    done: impl Fn(&MemberNode, &MemberNode) -> bool,
) -> bool {
    tokio::time::timeout(limit, async {
        loop {
            if done(a, b) {
                return true;
            }
            tokio::select! {
                _ = a.next_event() => {}
                _ = b.next_event() => {}
            }
        }
    })
    .await
    .unwrap_or(false)
}

/// Two connected members who can each publish chat logs, with ledger entries
/// in place so a fetch has a source it will actually rank.
async fn pair() -> (MemberNode, PerNetworkIdentity, MemberNode, PerNetworkIdentity) {
    let founder = identity(1);
    let peer = identity(2);
    let (mut a, _) = node(1).await;
    let (mut b, b_addr) = node(2).await;

    let root = a.append_entry(genesis(&founder)).unwrap();
    a.append_entry(admit(&founder, root, &peer, 5)).unwrap();

    a.dial_candidates([b_addr]).unwrap();
    assert!(
        drive(&mut a, &mut b, Duration::from_secs(20), |x, y| {
            x.governance_log().len() == 2 && y.governance_log().len() == 2
        })
        .await,
        "the log must reach both nodes before anything else can be checked"
    );

    // Ledger before fetch: a holder that never advertised capacity is dropped
    // by source selection as not having volunteered, so the DHT finding it is
    // not enough.
    for (node, who) in [(&mut a, &founder), (&mut b, &peer)] {
        node.advertise(CapabilityAdvertisement::create(
            who,
            1 << 30,
            BandwidthCap {
                up_bytes_per_sec: 1_000_000,
                down_bytes_per_sec: 8_000_000,
                active_window: None,
            },
            false,
            false,
            ComputeClass::Modest,
            Timestamp::from_millis(20),
        ))
        .unwrap();
    }
    let a_peer = a.peer_id();
    let b_peer = b.peer_id();
    a.sync_ledger_with(b_peer);
    b.sync_ledger_with(a_peer);
    assert!(
        drive(&mut a, &mut b, Duration::from_secs(20), |x, y| {
            x.capability_ledger().len() == 2 && y.capability_ledger().len() == 2
        })
        .await,
        "both nodes need each other's advertisement before a fetch can rank a source"
    );

    (a, founder, b, peer)
}

/// Runs one fetch round for whatever the reader is currently missing.
///
/// One round, not a loop: the manifest names the chunks, so a caller that wants
/// a whole object calls this twice — and a caller measuring what an append cost
/// needs to look *between* those two rounds, which a loop would hide.
async fn fetch_round(
    reader: &mut MemberNode,
    holder: &mut MemberNode,
    who: &PerNetworkIdentity,
    manifest: intranet_storage::Cid,
) -> bool {
    plan_fetch(reader, who, manifest, 4);
    if !reader.fetch_in_progress() {
        return true;
    }
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            tokio::select! {
                event = reader.next_event() => {
                    if matches!(event, NodeEvent::FetchComplete { .. }) {
                        return true;
                    }
                }
                _ = holder.next_event() => {}
            }
        }
    })
    .await
    .unwrap_or(false)
}

/// Fetches a whole object: the manifest, then the chunks it names.
async fn fetch(
    reader: &mut MemberNode,
    holder: &mut MemberNode,
    who: &PerNetworkIdentity,
    manifest: intranet_storage::Cid,
) -> bool {
    fetch_round(reader, holder, who, manifest).await
        && fetch_round(reader, holder, who, manifest).await
}

#[tokio::test]
async fn a_channel_crosses_the_wire_and_renders_identically() {
    let (mut a, founder, mut b, peer) = pair().await;
    let state = a.governance_log().replay_canonical().unwrap();

    let mut log = AuthorLog::open(
        &founder,
        channel(),
        Dek::from_bytes([5u8; 32]),
        ChunkSpec::from_target(16 * 1024),
    );

    // Enough messages to span several chunks, so this exercises multi-chunk
    // assembly rather than a single blob.
    let mut published = None;
    for i in 0..120u32 {
        published = Some(
            log.append(
                &founder,
                Record::create(
                    &founder,
                    channel(),
                    Hlc::new(1_700_000_000_000, i),
                    message(&format!("message {i} — a sentence of ordinary chat text")),
                ),
                &state,
            )
            .unwrap(),
        );
    }
    let published = published.unwrap();
    let outcome = publish_segment(&mut a, &published);
    assert!(outcome.chunks.len() > 1, "the segment must span several chunks");

    // The reader learns the pointer through ordinary pointer sync, not by being
    // handed it — the DEK is the only thing passed out of band here, standing in
    // for the epoch-key delivery a real join performs.
    let a_peer = a.peer_id();
    b.sync_pointers_with(a_peer);
    assert!(
        drive(&mut a, &mut b, Duration::from_secs(20), |_, y| {
            known_pointer(y, log.pointer_id()).is_some()
        })
        .await,
        "the pointer must reach the reader"
    );

    let pointer = known_pointer(&b, log.pointer_id()).unwrap().clone();
    pointer.verify().unwrap();
    assert!(fetch(&mut b, &mut a, &peer, pointer.current_cid).await, "fetch timed out");

    let segment = fetch_segment(&b, pointer.current_cid, &Dek::from_bytes([5u8; 32]))
        .expect("segment reassembles");
    segment.verify().expect("every record verifies after the round trip");
    assert!(segment.ordering_is_valid());

    // The point of the exercise: both nodes render the same history, one from
    // its own writes and one from bytes off the wire.
    let authority = StateAuthority { state: &state };
    let mut writer_view = ChannelView::new(placement());
    writer_view.admit(log.segment().records.clone(), &authority);
    let mut reader_view = ChannelView::new(placement());
    reader_view.admit(segment.records.clone(), &authority);

    assert_eq!(reader_view.render().len(), 120);
    assert_eq!(reader_view.render(), writer_view.render());
}

#[tokio::test]
async fn a_reader_refetches_only_the_tail_after_an_append() {
    let (mut a, founder, mut b, peer) = pair().await;
    let state = a.governance_log().replay_canonical().unwrap();

    let mut log = AuthorLog::open(
        &founder,
        channel(),
        Dek::from_bytes([5u8; 32]),
        ChunkSpec::from_target(16 * 1024),
    );
    let mut first = None;
    for i in 0..120u32 {
        first = Some(
            log.append(
                &founder,
                Record::create(
                    &founder,
                    channel(),
                    Hlc::new(1_700_000_000_000, i),
                    message(&format!("message {i} — a sentence of ordinary chat text")),
                ),
                &state,
            )
            .unwrap(),
        );
    }
    let first = first.unwrap();
    let first_outcome = publish_segment(&mut a, &first);

    let a_peer = a.peer_id();
    b.sync_pointers_with(a_peer);
    assert!(
        drive(&mut a, &mut b, Duration::from_secs(20), |_, y| {
            known_pointer(y, log.pointer_id()).is_some()
        })
        .await
    );
    assert!(fetch(&mut b, &mut a, &peer, first_outcome.manifest_cid).await);

    // One more message, republished.
    let second = log
        .append(
            &founder,
            Record::create(
                &founder,
                channel(),
                Hlc::new(1_700_000_000_001, 0),
                message("one more"),
            ),
            &state,
        )
        .unwrap();
    let second_outcome = publish_segment(&mut a, &second);

    // What the reader must now fetch, holding the previous version: the new
    // manifest plus whatever chunks actually changed — not the segment.
    let outstanding = wanted_chunks(&b, second_outcome.manifest_cid);
    assert_eq!(
        outstanding,
        vec![second_outcome.manifest_cid],
        "the reader should be missing only the manifest at this point"
    );

    // One round brings the manifest. Measure *here*, before the chunk round:
    // this is what an append actually costs a reader who holds the previous
    // version, and a fetch loop would have hidden it by completing both rounds.
    assert!(fetch_round(&mut b, &mut a, &peer, second_outcome.manifest_cid).await);
    let after_manifest = wanted_chunks(&b, second_outcome.manifest_cid);
    println!(
        "reader refetched {} chunk(s) of {} after an append",
        after_manifest.len(),
        second_outcome.chunks.len()
    );
    assert!(
        !after_manifest.is_empty(),
        "the tail chunk changed, so the reader must want at least one chunk — \
         measuring after both rounds would have reported zero and proved nothing"
    );
    assert!(
        after_manifest.len() <= 2,
        "reader wanted {} chunks, expected at most 2",
        after_manifest.len()
    );

    assert!(fetch_round(&mut b, &mut a, &peer, second_outcome.manifest_cid).await);

    let segment = fetch_segment(&b, second_outcome.manifest_cid, &Dek::from_bytes([5u8; 32]))
        .expect("reassembles from mostly-cached chunks");
    assert_eq!(segment.records.len(), 121);
}
