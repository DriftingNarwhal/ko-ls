//! The long-running node — `kols serve`.
//!
//! # Why a daemon at all
//!
//! Every other command is one-shot: it reads the store, does one thing, writes,
//! and exits. That works because the governance log and the records are on disk,
//! and a process can rebuild everything it needs from them.
//!
//! Two things cannot be rebuilt that way. A libp2p node has to *be somewhere* to
//! be reachable, and an MLS group holds live cryptographic state that
//! `GroupSession` keeps in an in-memory provider with no persistence — so
//! rotating an epoch key requires a process that has not exited since the group
//! was created. This is that process. Rotation is not implemented here yet, but
//! this is the only place it can be, which is why the group is loaded rather
//! than left for later.
//!
//! # The layering a sync actually depends on
//!
//! Governance, then ledger, then content — in that order, and not as a style
//! preference. A peer whose log has not caught up correctly refuses to serve
//! (Storage §5.4 gates on `read-content`), and a holder that has not advertised
//! capacity is dropped by source selection as not having volunteered, so the DHT
//! finding it is not enough. A fetch that mysteriously finds nothing is almost
//! always one of those two rather than a bug in the fetch.

use crate::network;
use crate::store::Store;
use intranet_crypto::Timestamp;
use intranet_ledger::{BandwidthCap, CapabilityAdvertisement, ComputeClass};
use intranet_storage::ChunkSpec;
use intranet_transport::{MemberNode, NodeEvent};
use kols_core::{AuthorLog, ChannelId, Segment};
use kols_net::{fetch_segment, known_pointer, plan_fetch, publish_segment};
use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};
use std::collections::BTreeSet;

/// How much this node offers other members.
///
/// Modest on purpose. `design/02` §6.4 is explicit that a client which quietly
/// volunteers a laptop as infrastructure has misrepresented what its user
/// agreed to, so these are starting values a real client would put behind a
/// settings screen rather than defaults chosen to look generous.
const OFFERED_STORAGE_BYTES: u64 = 256 * 1024 * 1024;
const OFFERED_UPLOAD_BYTES_PER_SEC: u64 = 1_000_000;
const OFFERED_DOWNLOAD_BYTES_PER_SEC: u64 = 8_000_000;

/// Runs the node until interrupted.
pub fn run(root: std::path::PathBuf, listen: &str, peers: &[String]) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("could not start a runtime: {err}"))?;
    runtime.block_on(serve(root, listen, peers))
}

async fn serve(root: std::path::PathBuf, listen: &str, peers: &[String]) -> Result<(), String> {
    let store = Store::open(root).map_err(|e| e.to_string())?;
    let identity = store.identity().map_err(|e| e.to_string())?;

    let mut node = MemberNode::new(&identity).map_err(|err| format!("could not start: {err}"))?;
    let address: Multiaddr = listen
        .parse()
        .map_err(|err| format!("{listen:?} is not a multiaddr: {err}"))?;
    node.listen_on(address)
        .map_err(|err| format!("could not listen: {err}"))?;

    // Kademlia stays in client mode until a node has a confirmed external
    // address, and on loopback that never happens — so every provider lookup
    // would return nobody, and a fetch would fail in a way that looks like the
    // content is missing rather than like the DHT is not answering.
    node.set_dht_server_mode(true);

    // The stored log into the node, ancestors first — `insert` refuses an entry
    // whose parent it has not seen.
    let log = store.log().map_err(|e| e.to_string())?;
    for hash in log.canonical_chain() {
        if let Some(entry) = log.get(&hash) {
            node.append_entry(entry.clone())
                .map_err(|err| format!("the stored log will not load: {err}"))?;
        }
    }

    println!("serving as {}", identity.id().short());
    println!("  peer id   {}", node.peer_id());

    // Both of these are best-effort at startup and repeated after every
    // governance sync, because a node that has just attached to a network is
    // not a member yet: it holds an identity and nothing else until somebody
    // admits it, and an advertisement from a non-member is correctly refused.
    // Treating that as fatal would make the one command that *fixes* it —
    // syncing — impossible to run.
    // The network's key group. Whoever created the network makes it here on the
    // first run, because an MLS group is live state with no persistence — see
    // this module's header. A node that already holds a key does not remake one:
    // a second group would produce a different epoch key and orphan every DEK
    // wrapped under the first.
    let mut keyed = store.epoch_key().is_ok();

    // A group saved by a previous run comes back first. Core §3.3.1: holding the
    // epoch key is not the same as holding the group — a node with only the key
    // reads fine and can never rotate, welcome or revoke, so a founder that
    // restarted could never key anybody in again. Before this existed, every
    // restart was that.
    let mut holds_group = false;
    if let Some(saved) = store.group_state().map_err(|e| e.to_string())? {
        let rotation = store.rotation_ref().map_err(|e| e.to_string())?;
        match node.restore_epoch_group(&saved, rotation) {
            Ok(_) => {
                holds_group = true;
                println!("  group     restored from the last run");
            }
            // Not fatal: this node can still read with the keys it holds, and
            // saying so beats refusing to start over state it can re-fetch.
            Err(err) => println!("  group     could not be restored ({err})"),
        }
    }

    if !holds_group && !keyed && store.head().map_err(|e| e.to_string())?.is_some() {
        let state = store.state().map_err(|e| e.to_string())?;
        if founder_of(&state, &identity.id()) {
            node.create_epoch_group(&identity)
                .map_err(|err| format!("could not key this network: {err}"))?;
            if node.epoch_keyring().current().is_some() {
                persist_keyring(&store, &node)?;
                persist_group(&store, &node)?;
                holds_group = true;
                keyed = true;
                println!("  keyed     this network for the first time");
            }
        }
    }

    match (keyed, holds_group) {
        (true, true) => println!("  epoch     held, and this node can key others in"),
        (true, false) => println!("  epoch     held, but no group — this node cannot key others in"),
        _ => println!("  epoch     none — this node can fetch content and open none of it"),
    }

    match ready(&store, &mut node, &identity) {
        Ok(published) => println!("  published {published} segment(s) from this node"),
        Err(_) => println!("  not a member of this network yet — syncing will settle it"),
    }

    for peer in peers {
        let address: Multiaddr = peer
            .parse()
            .map_err(|err| format!("{peer:?} is not a multiaddr: {err}"))?;
        node.dial_candidates([address])
            .map_err(|err| format!("could not dial {peer}: {err}"))?;
        println!("  dialing   {peer}");
    }

    let mut connected: BTreeSet<PeerId> = BTreeSet::new();
    let mut announced = BTreeSet::new();
    let mut fetched = BTreeSet::new();
    let mut listening = Vec::new();

    // One-shot commands write to the same store this node reads, so a `post`
    // in another terminal is invisible until something re-reads it. Polling is
    // the honest mechanism here: watching the filesystem would be less portable
    // and no more correct, and the interval only bounds how stale this node's
    // published view is, never whether it converges.
    let mut refresh = tokio::time::interval(std::time::Duration::from_secs(2));
    refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        let event = tokio::select! {
            event = node.next_event() => event,
            _ = refresh.tick() => {
                adopt_local_changes(&store, &mut node, &identity)?;
                if holds_group {
                    let excluded = exclude_removed_members(&store, &mut node, &identity)?;
                    if excluded > 0 {
                        println!("rotated the epoch to exclude {excluded} removed member(s)");
                    }
                }

                // Re-asked rather than assumed settled. Everything here is
                // pull-based — the governance log, the ledger and pointers alike
                // — so a peer that changed anything after the last exchange is
                // invisible until somebody asks again.
                //
                // **The ledger is not optional here, and leaving it out cost a
                // day.** Source selection drops a holder that has not advertised
                // capacity, as not having volunteered. A joiner advertises only
                // once it is admitted, which is *after* the ledger sync that ran
                // when it connected — so without re-asking, a peer stays
                // permanently unrankable and every fetch from it fails with the
                // chunk simply never arriving. The crate's own guidance says a
                // fetch that mysteriously finds nothing is usually this, and it
                // was.
                for peer in connected.iter().copied() {
                    node.sync_with(peer);
                    node.sync_ledger_with(peer);
                    node.sync_pointers_with(peer);
                }
                request_foreign_segments(&store, &mut node, &identity, &mut fetched)?;
                let learned = absorb_segments(&store, &mut node, &identity)?;
                if learned > 0 {
                    println!("learned {learned} record(s)");
                }
                continue;
            }
        };
        match event {
            NodeEvent::Listening(address) => {
                // Printed with the peer id appended, because that is the form
                // another node can actually dial — an address without it names a
                // machine rather than a member.
                let full = address.clone().with(Protocol::P2p(identity.peer_id()));
                if listening.iter().all(|seen| seen != &full) {
                    println!("  listening {full}");
                    listening.push(full);
                }
            }

            NodeEvent::Connected { peer, .. } => {
                println!("connected to {peer}");
                connected.insert(peer);
                start_sync(&mut node, peer);
            }

            NodeEvent::Disconnected { peer } => {
                connected.remove(&peer);
            }

            // Governance first. Everything else is gated on it, so a peer's
            // content is not worth asking for until this settles.
            NodeEvent::Synced { accepted, peer, .. } if accepted > 0 => {
                let learned = persist_governance(&store, &node)?;
                if learned > 0 {
                    println!("learned {learned} governance entr(ies)");
                    // The log that just arrived may be the one that admits this
                    // node. Advertising and publishing are retried here rather
                    // than only at startup for exactly that reason, and the
                    // ledger is re-synced because a peer that refused this
                    // node's advertisement a moment ago will accept it now.
                    if ready(&store, &mut node, &identity).is_ok() {
                        node.sync_ledger_with(peer);
                        node.sync_pointers_with(peer);
                        // A member with no key can fetch every byte of this
                        // network's content and open none of it, so asking for
                        // one is the first thing worth doing after admission.
                        if !keyed && let Some(from) = peer_identity(peer, &store)? {
                            match node.request_epoch_key(from, &identity) {
                                Ok(_) => println!("asked {} to key us in", from.short()),
                                Err(err) => println!("could not ask for a key: {err}"),
                            }
                        }
                    }
                }
            }

            // A ledger that just accepted somebody may have made a source
            // rankable that was not before, so this is where a stalled fetch
            // gets its second chance.
            NodeEvent::LedgerSynced { accepted, .. } if accepted > 0 => {
                request_foreign_segments(&store, &mut node, &identity, &mut fetched)?;
            }

            NodeEvent::PointersReceived { .. } => {
                request_foreign_segments(&store, &mut node, &identity, &mut fetched)?;
            }

            NodeEvent::FetchComplete { .. } => {
                let learned = absorb_segments(&store, &mut node, &identity)?;
                if learned > 0 {
                    println!("learned {learned} record(s)");
                }
                request_foreign_segments(&store, &mut node, &identity, &mut fetched)?;
            }

            // Somebody asked to be keyed in. Every gate — the request signature,
            // the connection binding, `read-content`, the key package — was
            // already applied before this arrives; what is left needs an
            // identity to sign with and a clock to sign at.
            NodeEvent::EpochKeyRequested { requester, request, .. } => {
                // Answering appends a rotation, parented on this node's head —
                // so the store's head has to *be* this node's head first, and
                // nothing else may append in between. Adopt under the lock, then
                // rotate, then write back.
                let lock = store.lock().map_err(|e| e.to_string())?;
                adopt_local_changes(&store, &mut node, &identity)?;
                let answered = node.answer_epoch_key(
                    request,
                    &identity,
                    Timestamp::from_millis(crate::chat::now_millis()),
                );
                match answered {
                    Ok(_) => {
                        println!("keyed in {}", requester.short());
                        // Adding a member rotates the epoch, so this node now
                        // holds a key it did not before. Both the rotation entry
                        // and the new key belong in the store — and the *old*
                        // key stays, because everything published under it is
                        // still wrapped under it.
                        persist_governance(&store, &node)?;
                        persist_keyring(&store, &node)?;
                        // The add advanced the group, so what was saved is now
                        // a state behind. Saving here rather than on exit means
                        // a crash costs nothing that was already agreed.
                        persist_group(&store, &node)?;
                    }
                    Err(err) => println!("could not key in {}: {err}", requester.short()),
                }
                drop(lock);
            }

            NodeEvent::EpochKeyDelivered {
                rotation_ref,
                historical_keys,
                ..
            } => {
                let _ = rotation_ref;
                if node.epoch_keyring().current().is_some() {
                    // Every key, not just the newest. Content written before
                    // this node joined is wrapped under an earlier rotation, and
                    // keeping only the current key would leave it permanently
                    // unreadable while every byte of it fetched successfully.
                    persist_keyring(&store, &node)?;
                    persist_group(&store, &node)?;
                    keyed = true;
                    println!(
                        "keyed into this network ({historical_keys} historical key(s) came with it)"
                    );
                    request_foreign_segments(&store, &mut node, &identity, &mut fetched)?;
                }
            }

            NodeEvent::EpochKeyUnavailable { reason, .. } => {
                println!("not keyed in: {reason}");
            }

            // A chunk this node fetched makes it a source for that chunk
            // (Storage §4.2), so it is announced rather than held quietly.
            NodeEvent::ChunkReceived { cid, .. } if announced.insert(cid) => {
                node.announce_chunk(cid);
            }

            _ => {}
        }
    }
}

/// Advertises this node's contribution and publishes its logs.
///
/// Fails while this node is not yet a member, which is a state to sync out of
/// rather than an error to stop on.
fn ready(
    store: &Store,
    node: &mut MemberNode,
    identity: &intranet_identity::PerNetworkIdentity,
) -> Result<usize, String> {
    node.advertise(CapabilityAdvertisement::create(
        identity,
        OFFERED_STORAGE_BYTES,
        BandwidthCap {
            up_bytes_per_sec: OFFERED_UPLOAD_BYTES_PER_SEC,
            down_bytes_per_sec: OFFERED_DOWNLOAD_BYTES_PER_SEC,
            active_window: None,
        },
        false,
        false,
        ComputeClass::Modest,
        Timestamp::from_millis(crate::chat::now_millis()),
    ))
    .map_err(|err| format!("could not advertise: {err}"))?;
    publish_own_logs(store, node)
}

/// Publishes every author log this node owns into the node's chunk store.
fn publish_own_logs(store: &Store, node: &mut MemberNode) -> Result<usize, String> {
    let identity = store.identity().map_err(|e| e.to_string())?;
    let Some(state) = replayable(store) else {
        return Ok(0);
    };
    let mut published = 0;

    for channel in store.channels_with_records().map_err(|e| e.to_string())? {
        let own = store
            .own_records(&channel, &identity.id())
            .map_err(|e| e.to_string())?;
        if own.is_empty() {
            continue;
        }
        let pointer = kols_core::author_log_pointer(&channel, &identity.id());
        let dek = store.channel_dek(&pointer).map_err(|e| e.to_string())?;
        let mut log = AuthorLog::open(
            &identity,
            channel,
            dek.clone(),
            ChunkSpec::from_target(64 * 1024),
        );
        // The last append's outcome is the whole current segment: appending
        // republishes the same object, so what it returns is the object as it
        // now stands rather than a delta to be assembled.
        let mut latest = None;
        for record in own {
            latest = Some(
                log.append(&identity, record, &state)
                    .map_err(|err| format!("a stored record no longer appends: {err}"))?,
            );
        }
        if let Some(segment) = latest {
            let outcome = publish_segment(node, &segment);
            let _ = outcome;

            // The wrapping is what lets anybody else read this. A pointer
            // carries a commitment to its DEK, never the DEK itself, so a peer
            // that fetches every chunk still needs a wrapping under an epoch key
            // it holds — and pointer sync carries wrappings alongside pointers
            // precisely because the two are useless apart (Storage §5.3).
            if let (Ok(epoch), Ok(rotation)) = (store.epoch_key(), store.rotation_ref()) {
                node.accept_wrapping(intranet_storage::DekWrapping::create(
                    &identity,
                    pointer,
                    &dek,
                    &epoch,
                    rotation,
                ));
            }
            published += 1;
        }
    }
    Ok(published)
}

/// Writes any governance entries the node learned into the store.
fn persist_governance(store: &Store, node: &MemberNode) -> Result<usize, String> {
    let held = store.log().map_err(|e| e.to_string())?;
    let known: BTreeSet<_> = held.canonical_chain().into_iter().collect();
    let mut learned = 0;
    for hash in node.governance_log().canonical_chain() {
        if known.contains(&hash) {
            continue;
        }
        if let Some(entry) = node.governance_log().get(&hash) {
            store.append_entry(entry).map_err(|e| e.to_string())?;
            learned += 1;
        }
    }
    Ok(learned)
}

/// Asks for any author log this node knows a pointer for and has not fetched.
fn request_foreign_segments(
    store: &Store,
    node: &mut MemberNode,
    identity: &intranet_identity::PerNetworkIdentity,
    fetched: &mut BTreeSet<intranet_storage::Cid>,
) -> Result<(), String> {
    let Some(state) = replayable(store) else {
        return Ok(());
    };
    let (channels, _) = network::channels(store, &state).map_err(|e| e.to_string())?;

    // Every member's log in every channel, computed rather than looked up. This
    // is what derived pointer ids buy: any member can work out where any other
    // member's messages in any channel would live, from public information alone
    // and with no directory to be stale (`design/01` §3.2).
    let mut wanted = Vec::new();
    for channel in channels.keys() {
        for member in members(&state) {
            if member == identity.id() {
                continue;
            }
            let pointer_id = kols_core::author_log_pointer(channel, &member);
            if let Some(pointer) = known_pointer(node, &pointer_id) {
                wanted.push(pointer.current_cid);
            }
        }
    }

    // Asked for repeatedly on purpose. A fetch is **two rounds** — the manifest,
    // then the chunks it names — because the chunk list lives inside the
    // manifest. Remembering "this object was already requested" and skipping it
    // would stop after the first round and leave every segment permanently
    // half-fetched, which is exactly the bug this replaced. `wanted_chunks`
    // returns only what is genuinely missing, so re-asking is self-terminating
    // and re-asking for a complete object is free.
    for cid in wanted {
        if node.fetch_in_progress() {
            break;
        }
        if fetched.insert(cid) || !kols_net::wanted_chunks(node, cid).is_empty() {
            plan_fetch(node, identity, cid, 4);
        }
    }
    Ok(())
}

/// Decodes whatever complete segments the node now holds, and stores the records.
fn absorb_segments(
    store: &Store,
    node: &mut MemberNode,
    identity: &intranet_identity::PerNetworkIdentity,
) -> Result<usize, String> {
    let Some(state) = replayable(store) else {
        return Ok(0);
    };
    let (channels, _) = network::channels(store, &state).map_err(|e| e.to_string())?;
    let mut learned = 0;

    // Loaded once. This reads and opens every stored epoch key, so calling it
    // per (channel, member) meant a long-lived network paid a full directory
    // scan a thousand times per tick — far more than the unwrap it was there to
    // serve. Lazily, because a node with nothing to absorb should not pay for it
    // at all.
    let mut keys: Option<Vec<_>> = None;

    for channel in channels.keys() {
        for member in members(&state) {
            if member == identity.id() {
                continue;
            }
            let pointer_id = kols_core::author_log_pointer(channel, &member);
            let Some(pointer) = known_pointer(node, &pointer_id) else {
                continue;
            };
            let cid = pointer.current_cid;
            let commitment = pointer.dek_commitment;

            // The cached DEK first, checked against the pointer's own commitment
            // so a stale one — the author sealed that object and started another
            // — is discarded rather than used to fail at decryption. In the
            // steady state this is one unwrap under the current key and no scan.
            let dek = match store
                .known_dek(&pointer_id, Some(&commitment))
                .map_err(|e| e.to_string())?
            {
                Some(dek) => dek,
                None => {
                    // Never `channel_dek` here: that mints a key for an object
                    // this node owns, which for somebody else's log would open
                    // nothing. A foreign DEK can only come from their wrapping,
                    // validated against the owner's commitment rather than
                    // against who signed it — which is what makes "any current
                    // member may re-wrap" safe.
                    let held = match &keys {
                        Some(keys) => keys,
                        None => keys.insert(store.epoch_keys().map_err(|e| e.to_string())?),
                    };
                    let Some(dek) =
                        node.wrappings_for(&pointer_id).into_iter().find_map(|wrapping| {
                            held.iter()
                                .find_map(|(_, key)| wrapping.unwrap(key, &commitment).ok())
                        })
                    else {
                        continue;
                    };
                    // Remembered under the current epoch, so this scan happens
                    // once per object rather than on every tick forever.
                    store
                        .remember_dek(&pointer_id, &dek)
                        .map_err(|e| e.to_string())?;
                    dek
                }
            };
            // A segment we cannot assemble yet is not an error: the fetch runs
            // in rounds, and the manifest arrives before the chunks it names.
            let Ok(segment) = fetch_segment(node, cid, &dek) else {
                continue;
            };
            learned += store_segment(store, channel, &segment)?;
        }
    }
    Ok(learned)
}

fn store_segment(store: &Store, channel: &ChannelId, segment: &Segment) -> Result<usize, String> {
    let mut learned = 0;
    for record in &segment.records {
        // A record must belong to the segment carrying it (spec 07 §3.5).
        // Without this a validly-signed record could be lifted from one author's
        // log into another's, carrying an authorship its signature never
        // claimed — and the signature stays genuine throughout, so checking
        // signatures alone does not catch it.
        if &record.channel != channel || record.author != segment.author {
            continue;
        }
        if record.verify_signature().is_err() {
            continue;
        }
        if store
            .put_record(channel, record)
            .map_err(|e| e.to_string())?
        {
            learned += 1;
        }
    }
    Ok(learned)
}

/// This node's replayed state, or `None` while it has no log to replay.
///
/// A node that has attached to a network but not yet synced holds an identity
/// and nothing else. Every periodic task has to tolerate that rather than treat
/// it as a failure, because syncing — the thing that fixes it — is what the
/// periodic tasks are running to achieve.
fn replayable(store: &Store) -> Option<intranet_governance::GovernanceState> {
    store.state().ok()
}

/// Loads anything the store gained since this node last looked.
///
/// Governance entries first, then republishing, because a channel this node did
/// not know about is one it cannot have published records for.
fn adopt_local_changes(
    store: &Store,
    node: &mut MemberNode,
    identity: &intranet_identity::PerNetworkIdentity,
) -> Result<Option<()>, String> {
    let held: BTreeSet<_> = node.governance_log().canonical_chain().into_iter().collect();
    let stored = store.log().map_err(|e| e.to_string())?;
    let mut adopted = 0;
    for hash in stored.canonical_chain() {
        if held.contains(&hash) {
            continue;
        }
        if let Some(entry) = stored.get(&hash) {
            node.append_entry(entry.clone())
                .map_err(|err| format!("a locally written entry will not load: {err}"))?;
            adopted += 1;
        }
    }
    if adopted > 0 {
        println!("picked up {adopted} locally written governance entr(ies)");
    }

    // Republished unconditionally rather than only on change: appending to an
    // author log republishes the same object, and a chunk that has not changed
    // re-derives to the same CID, so this costs a re-announcement rather than a
    // re-upload. Kademlia provider records expire, so the re-announcement is
    // work worth doing anyway.
    if ready(store, node, identity).is_ok() {
        Ok(Some(()))
    } else {
        Ok(None)
    }
}

/// Rotates the epoch away from anybody the log has removed.
///
/// The second half of a revocation. `kols revoke` writes the membership removal
/// and stops there, because rotating needs the live MLS group that only this
/// process holds — and Core §3.3 requires that order anyway: a rotation minted
/// while somebody is still a current member produces a key they remain entitled
/// to, and §3.1 is explicit that a key cannot be un-known afterwards.
///
/// Every identity the log has ever named is offered to `revoke_epoch_member`,
/// which returns `None` for anybody with no leaf in the tree. That is cheaper
/// than it looks and more robust than tracking who this node keyed in: a member
/// removed while this node was offline gets excluded on the next run, which is
/// exactly the convergent cascade `design/03` §5 describes — a removed member
/// loses access as each node with the group processes the removal, not instantly.
fn exclude_removed_members(
    store: &Store,
    node: &mut MemberNode,
    identity: &intranet_identity::PerNetworkIdentity,
) -> Result<usize, String> {
    // Same discipline as answering a key request: this appends a rotation
    // parented on the node's head, so the store's head has to be the node's head
    // and nothing else may append in between.
    let _lock = store.lock().map_err(|e| e.to_string())?;
    adopt_local_changes(store, node, identity)?;

    let Some(state) = replayable(store) else {
        return Ok(0);
    };
    let log = store.log().map_err(|e| e.to_string())?;

    let mut named = BTreeSet::new();
    for hash in log.canonical_chain() {
        if let Some(entry) = log.get(&hash)
            && let intranet_governance::EntryBody::MembershipChange { identity: who, .. } =
                &entry.body
        {
            named.insert(*who);
        }
    }

    let mut excluded = 0;
    for who in named {
        if state.is_member(&who) {
            continue;
        }
        match node.revoke_epoch_member(
            &who,
            identity,
            Timestamp::from_millis(crate::chat::now_millis()),
        ) {
            // No leaf in the tree: either they were never keyed in, or this node
            // already excluded them. Both are the steady state, not a problem.
            Ok(None) => {}
            Ok(Some(_)) => {
                excluded += 1;
                // The rotation is a governance entry and it advanced the group,
                // so all three have to be written before anything else can go
                // wrong. Saving here rather than on exit means a crash cannot
                // lose a rotation other members have already been told about.
                persist_governance(store, node)?;
                persist_keyring(store, node)?;
                persist_group(store, node)?;
            }
            Err(err) => println!("could not exclude {}: {err}", who.short()),
        }
    }
    Ok(excluded)
}

/// Saves the node's MLS group, if it has one.
fn persist_group(store: &Store, node: &MemberNode) -> Result<(), String> {
    match node.save_epoch_group() {
        Ok(Some(state)) => store.set_group_state(&state).map_err(|e| e.to_string()),
        Ok(None) => Ok(()),
        Err(err) => Err(format!("could not save the group: {err}")),
    }
}

/// Writes every epoch key the node holds into the store.
fn persist_keyring(store: &Store, node: &MemberNode) -> Result<(), String> {
    let keyring = node.epoch_keyring();
    let keys: Vec<_> = keyring
        .records()
        .map(|record| (record.rotation_ref, record.key.clone()))
        .collect();
    let current = keyring
        .current()
        .map(|(rotation, _)| *rotation)
        .ok_or("this node holds no current epoch key")?;
    store
        .set_epoch_keys(&keys, current)
        .map_err(|e| e.to_string())
}

/// Whether this identity founded the network, and so should mint its first key.
fn founder_of(
    state: &intranet_governance::GovernanceState,
    identity: &intranet_identity::PerNetworkIdentityId,
) -> bool {
    state
        .groups
        .get(&intranet_governance::GroupId::founders())
        .is_some_and(|group| group.members.contains_key(identity))
}

/// The member behind a peer id, if this node's replayed state knows one.
///
/// A peer id is derived from an identity's key, so this is a lookup rather than
/// a claim to be trusted: nothing here believes what a peer says about itself.
fn peer_identity(
    peer: PeerId,
    store: &Store,
) -> Result<Option<intranet_identity::PerNetworkIdentityId>, String> {
    let Some(state) = replayable(store) else {
        return Ok(None);
    };
    Ok(members(&state).into_iter().find(|id| id.peer_id() == peer))
}

/// Every identity in any group, which is every member whose log might exist.
///
/// The union rather than `everyone`'s roster alone: a founder is placed in
/// `Founders` and deliberately not in `everyone` (Core §2.3), so reading one
/// group would miss exactly the member who created the network.
fn members(
    state: &intranet_governance::GovernanceState,
) -> BTreeSet<intranet_identity::PerNetworkIdentityId> {
    state
        .groups
        .values()
        .flat_map(|group| group.members.keys().copied())
        .collect()
}

fn start_sync(node: &mut MemberNode, peer: PeerId) {
    node.sync_with(peer);
    node.sync_ledger_with(peer);
    node.sync_pointers_with(peer);
}
