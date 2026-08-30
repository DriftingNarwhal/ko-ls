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
use kols_api::{Arrival, Event};
use kols_core::Record;
use kols_core::{AuthorLog, Authority, ChannelId, Placement, Segment, StateAuthority};
use kols_net::{fetch_segment, known_pointer, plan_fetch, publish_segment};
use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};
use std::collections::BTreeSet;
use std::time::Instant;

/// How much this node offers other members.
///
/// Modest on purpose. `design/02` §6.4 is explicit that a client which quietly
/// volunteers a laptop as infrastructure has misrepresented what its user
/// agreed to, so these are starting values a real client would put behind a
/// settings screen rather than defaults chosen to look generous.
const OFFERED_STORAGE_BYTES: u64 = 256 * 1024 * 1024;
const OFFERED_UPLOAD_BYTES_PER_SEC: u64 = 1_000_000;
const OFFERED_DOWNLOAD_BYTES_PER_SEC: u64 = 8_000_000;

/// When an open segment gets sealed and a fresh one starts.
///
/// `design/01` §3.1's size target. It is local publishing tuning and not a
/// validity rule — a reader accepts whatever boundaries an author chose — which
/// is exactly why `serve` lets it be overridden: a test needs boundaries it can
/// reach, and picking a smaller one produces history no less valid than this.
pub const SEAL_TARGET_BYTES: usize = 4 * 1024 * 1024;

/// The other seal threshold from `design/01` §3.1: how much time one segment may
/// span.
///
/// Measured across the segment's own records — newest minus oldest — rather than
/// against the clock. That distinction is what keeps the whole chain a pure
/// function of the record sequence: a rebuild months later re-derives the same
/// boundaries, where "older than a day *now*" would seal somewhere new every
/// time and publish a second, competing chain.
///
/// §3.1's reason for having it at all is fan-in: "a stale author's head segment
/// must not be a year of scrollback". Size alone does not cover that, because
/// the segment that needs splitting is the sparse one.
const SEAL_TARGET_SPAN_MILLIS: i64 = 24 * 60 * 60 * 1000;

/// How recent a record has to be for the live path to still carry it.
///
/// Publishing fails while nobody is subscribed to a topic, and a failed publish
/// is retried — which is right for a record written a moment before a peer
/// arrives, and wrong for one written last week. Without a bound the retry set
/// is *every record the node has ever written*, so an author's entire history
/// goes out over gossipsub the instant any peer subscribes. Spec 07 §6.1 says
/// nothing may **depend** on the live path; it does not license the live path to
/// stand in for the durable one, and a backlog delivered over a best-effort
/// broadcast is exactly that substitution.
///
/// A minute is far longer than the latency this path exists to save and far
/// shorter than any gap that counts as history, so nothing sits near the edge.
/// Like the seal thresholds it is local tuning — `serve` exposes it so a test can
/// reach the far side of the window without waiting out a minute of wall clock.
pub const LIVE_WINDOW_MILLIS: i64 = 60 * 1000;

/// Where a node's events go.
///
/// A terminal prints them and a window forwards them to a webview, and the loop
/// below knows about neither. This is the same division the command side
/// already draws: the executor returns `Outcome`s and prints nothing, because a
/// layer that decides how something looks is a layer no second interface can
/// reuse.
pub type Sink = std::sync::Arc<dyn Fn(&[Event]) + Send + Sync>;

/// A sink that prints, for `kols serve`.
pub fn printing() -> Sink {
    std::sync::Arc::new(render)
}

/// Where everything this loop emits goes.
///
/// Two channels rather than one because they carry different things: [`Sink`]
/// takes *events*, which `design/05` §3 defines as a boundary vocabulary, and
/// [`crate::Report`] takes the node's own lifecycle lines, which that section
/// deliberately excludes from it. A terminal wants both; a window wants the
/// first and has nowhere to put the second.
///
/// Grouped into a struct because passing them separately took [`serve`] to eight
/// arguments, one past what clippy tolerates — and the grouping is real rather
/// than arithmetic: this is one answer to "where does this node's output go".
/// The signature is at the threshold again, so a ninth thing to pass is the
/// prompt to give the six configuration parameters a struct of their own.
pub struct Output<'a> {
    /// Events, for whatever is rendering the network.
    pub events: &'a Sink,
    /// Lifecycle lines, for whoever has somewhere to print them.
    pub report: &'a crate::Report,
}

/// Runs the node until interrupted, on a runtime of its own.
///
/// What `kols serve` calls. A caller that already has a runtime — the desktop
/// shell, which must keep a window responsive while this runs — spawns
/// [`serve`] instead.
pub fn run(
    root: std::path::PathBuf,
    listen: &str,
    peers: &[String],
    seal_bytes: usize,
    live: bool,
    live_window: i64,
) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("could not start a runtime: {err}"))?;
    runtime.block_on(serve(
        root,
        listen,
        peers,
        seal_bytes,
        live,
        live_window,
        &Output {
            events: &printing(),
            report: &crate::printing_report(),
        },
    ))
}

/// How long an unkeyed node waits before saying so.
///
/// Well past a normal answer on any link, because this reports a stall rather
/// than a delay and crying early would train somebody to ignore it.
const UNKEYED_WARNING: std::time::Duration = std::time::Duration::from_secs(60);

/// How long to wait before asking for a key again.
///
/// Retrying is safe since Core §3.5.1 — a repeat request replaces this node's
/// leaf rather than adding a second one — and it is not free: every answer is a
/// real epoch rotation and a governance entry every member replays forever. So
/// this is deliberately slow relative to the two-second tick. A node stalled for
/// a minute asks twice, not thirty times.
const KEY_REQUEST_RETRY: std::time::Duration = std::time::Duration::from_secs(30);

/// The node loop.
///
/// Runs until the future is dropped, which is how a caller stops it: there is no
/// shutdown signal to forget to send, and a shell switching networks drops the
/// task for the one it is leaving.
pub async fn serve(
    root: std::path::PathBuf,
    listen: &str,
    peers: &[String],
    seal_bytes: usize,
    live: bool,
    live_window: i64,
    out: &Output<'_>,
) -> Result<(), String> {
    let Output { events: sink, report } = out;
    let store = Store::open(root).map_err(|e| e.to_string())?;
    // Claimed before anything else, and held for the whole loop: only one
    // process may run a node for a network, because the key group is live state
    // and two would each advance it without seeing the other.
    let _claim = store.hold_node().map_err(|e| e.to_string())?;
    let identity = store.identity().map_err(|e| e.to_string())?;

    let mut node = MemberNode::new(&identity).map_err(|err| format!("could not start: {err}"))?;
    // **Dual-stack unless told otherwise, and this is load-bearing.**
    //
    // This bound `/ip4/0.0.0.0/tcp/0` and nothing else: no IPv6, no QUIC. Core
    // §5.1 requires both families and both transports, and §5.2 says why in the
    // case that actually bites — a pair behind CGNAT typically cannot traverse
    // IPv4 at all, so **IPv6 is the path the spec designates for them**, not a
    // relay circuit. Offering only IPv4 removes tier 1's better half and leaves
    // such a pair on tier 3, which §5.2 calls a correctness guarantee and not a
    // usable path, and which §5.3's ceilings are designed to make untenable.
    //
    // `--listen` still overrides, for a test that wants one specific socket.
    if listen.trim().is_empty() {
        node.listen_default()
            .map_err(|err| err.to_string())?;
    } else {
        let address: Multiaddr = listen
            .parse()
            .map_err(|err| format!("{listen:?} is not a multiaddr: {err}"))?;
        node.listen_on(address)
            .map_err(|err| err.to_string())?;
    }

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

    crate::say!(report, "serving as {}", identity.id().short());
    crate::say!(report, "  peer id   {}", node.peer_id());

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
    // When this node started waiting for a key, so a stall can say so.
    let mut unkeyed_since: Option<Instant> = None;
    let mut said_unkeyed = false;
    // When this node last asked to be keyed in, so the ask can be repeated on a
    // schedule rather than being a single chance.
    let mut last_asked: Option<Instant> = None;

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
                crate::say!(report, "  group     restored from the last run");
            }
            // Not fatal: this node can still read with the keys it holds, and
            // saying so beats refusing to start over state it can re-fetch.
            Err(err) => crate::say!(report, "  group     could not be restored ({err})"),
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
                crate::say!(report, "  keyed     this network for the first time");
            }
        }
    }

    match (keyed, holds_group) {
        (true, true) => crate::say!(report, "  epoch     held, and this node can key others in"),
        (true, false) => crate::say!(report, "  epoch     held, but no group — this node cannot key others in"),
        _ => crate::say!(report, "  epoch     none — this node can fetch content and open none of it"),
    }

    // What this node kept for other members, put back before anything asks for
    // it. `ready` below re-derives and re-announces this node's *own* segments
    // from its records, which is why an author's content always came back; this
    // is the other half, and without it a restart retired everything a node was
    // holding on everybody else's behalf (`Store::put_chunk`).
    match restore_contribution(&store, &mut node) {
        Ok((0, 0)) => {}
        Ok((chunks, pointers)) => crate::say!(
            report,
            "  restored  {chunks} chunk(s) and {pointers} pointer(s) kept for other members"
        ),
        // Not fatal. A node that cannot re-read what it kept is a node that
        // serves less than it could, which is worth saying and is not worth
        // refusing to start over.
        Err(err) => crate::say!(report, "  restored  nothing kept for other members ({err})"),
    }

    match ready(&store, &mut node, &identity, seal_bytes) {
        Ok(published) => crate::say!(report, "  published {published} segment(s) from this node"),
        Err(_) => crate::say!(report, "  not a member of this network yet — syncing will settle it"),
    }

    // A circuit on one of the network's relays, before anything else needs an
    // address to hand out. Core §5.5: two members behind NAT cannot reach each
    // other directly, so this is what makes this node reachable at all — and
    // what `kols invite` will carry.
    //
    // The ordering matters and `reserve_via_relay` documents why: a wildcard
    // bind registers its listeners asynchronously, so reserving in the same
    // breath as binding finds nothing to reuse and produces an observed address
    // pointing at a port with no listener. Nothing else surfaces that. So the
    // listen above has already happened, and this waits for the grant before
    // treating the circuit as usable.
    let designated = {
        let replayed = store
            .state()
            .map(|state| state.policy.bootstrap_relays.clone())
            .unwrap_or_default();
        if replayed.is_empty() {
            // Nothing replayed yet — a node that has never synced still has to
            // reach a relay to sync at all, which is what the cache is for.
            store.relays()
        } else {
            // Refreshed, so a relay deployed since this node last ran is
            // dialable next time before it has synced.
            let _ = store.set_relays(&replayed);
            replayed
        }
    };

    let mut failures: Vec<String> = Vec::new();
    let reserved = reserve_any_reporting(&mut node, &designated, sink, &mut failures).await;
    if designated.is_empty() {
        crate::say!(report, "  relay     none designated — this node is reachable only on its own addresses");
    }
    // Emitted in every case, including the good one. A terminal has had this all
    // along only to the report; a window had only the failures, through `Degraded`.
    sink(&[Event::Relay {
        reserved,
        designated: designated.len(),
        failures,
    }]);

    // What `--peer` named, plus what an invite left in the store. A joiner
    // should not have to be told an address by hand when the invite already
    // carried one.
    let mut dialling: Vec<String> = peers.to_vec();
    for remembered in store.peers() {
        if !dialling.contains(&remembered) {
            dialling.push(remembered);
        }
    }

    for peer in &dialling {
        let address: Multiaddr = peer
            .parse()
            .map_err(|err| format!("{peer:?} is not a multiaddr: {err}"))?;
        node.dial_candidates([address])
            .map_err(|err| format!("could not dial {peer}: {err}"))?;
        crate::say!(report, "  dialing   {peer}");
    }

    let mut connected: BTreeSet<PeerId> = BTreeSet::new();
    // Records already broadcast, so republishing does not rebroadcast history
    // every couple of seconds.
    // Voided entries already reported, so a heal is announced once rather than
    // on every sync that follows it. In memory rather than in the store: a
    // restart re-reporting is *correct*, because nothing here knows whether the
    // member ever acted on it.
    let mut reorged: BTreeSet<intranet_crypto::Hash> = BTreeSet::new();
    let mut broadcast: BTreeSet<kols_core::MessageId> = BTreeSet::new();
    let mut announced = BTreeSet::new();
    let mut fetched = BTreeSet::new();
    // Older segments discovered by walking a `previous` chain and not held yet.
    // Kept beside `fetched` rather than inside it because these are *wanted*
    // rather than *asked for*: `request_foreign_segments` reads this to widen
    // what it asks for, and `absorb_chain` clears an entry once it lands.
    let mut backfill: BTreeSet<intranet_storage::Cid> = BTreeSet::new();
    let mut listening = Vec::new();
    // Asked once. The routing table decides which of this machine's LAN
    // addresses an invite is allowed to name, and it does not change often
    // enough to re-ask on every listen event.
    let preferred = crate::preferred_source_addresses();

    // One-shot commands write to the same store this node reads, so a `post`
    // in another terminal is invisible until something re-reads it. Polling is
    // the honest mechanism here: watching the filesystem would be less portable
    // and no more correct, and the interval only bounds how stale this node's
    // published view is, never whether it converges.
    let mut refresh = tokio::time::interval(std::time::Duration::from_secs(2));
    refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // A reservation is not held once, it is *kept*.
    //
    // libp2p renews one on its own for as long as the relay connection lasts —
    // an hour-long lease, renewed well before it lapses — so this is not about
    // expiry. It is about the connection ending: a relay holds reservation state
    // in memory and is stateless across restarts (Core §5.5), so every redeploy
    // drops every reservation it held. Nothing then asked again, and a node that
    // had been reachable stayed running and quietly stopped being so. The
    // symptom is somebody else's invite timing out.
    let mut relay_watch = tokio::time::interval(RELAY_RECHECK);
    relay_watch.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    relay_watch.reset();

    // Nothing re-dialled a peer that went away.
    //
    // The addresses were dialled once, at startup, and a peer lost after that
    // stayed lost — so a relay going down and coming back left two nodes that
    // could see it, had re-reserved on it, and never spoke to each other again.
    // A reconnect is a sync (`sync_with` runs on every `Connected`), so getting
    // the connection back is the whole of the recovery.
    let mut redial = tokio::time::interval(REDIAL_INTERVAL);
    redial.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    redial.reset();

    // A routing table goes stale: peers leave, buckets thin out, and a table
    // that is never refreshed slowly stops being able to route. Kademlia's own
    // guidance is to re-bootstrap periodically rather than once at startup.
    let mut dht = tokio::time::interval(DHT_BOOTSTRAP_INTERVAL);
    dht.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    dht.reset();

    loop {
        let event = tokio::select! {
            event = node.next_event() => event,
            // Only while nothing is connected. A node with a live peer has no
            // reason to dial, and this way an unreachable peer is retried
            // without a node that is fine doing anything at all.
            _ = redial.tick(), if connected.is_empty() && !dialling.is_empty() => {
                for peer in &dialling {
                    if let Ok(address) = peer.parse::<Multiaddr>() {
                        let _ = node.dial_candidates([address]);
                    }
                }
                continue;
            }
            _ = dht.tick(), if !connected.is_empty() => {
                node.bootstrap_dht();
                continue;
            }
            _ = relay_watch.tick(), if !designated.is_empty() => {
                if !node.has_circuit() {
                    let regained = reserve_any(&mut node, &designated, sink).await;
                    sink(&[Event::Relay {
                        reserved: regained,
                        designated: designated.len(),
                        // The reasons were reported when the reservation was
                        // first settled. Repeating them every recheck would bury
                        // whatever is happening now.
                        failures: Vec::new(),
                    }]);
                }
                continue;
            }
            _ = refresh.tick() => {
                adopt_local_changes(&store, &mut node, &identity, seal_bytes, sink)?;
                // Recomputed on the tick rather than only when somebody knocks.
                // Admission is a governance entry, and no path from one reaches
                // the waiting room — so this is where an admitted member stops
                // being shown at the door, whoever admitted them and whichever
                // process wrote the entry.
                record_waiting(&store, &node, &identity, sink);
                // Both halves of the live path, together. Spec 07 §6.1 requires
                // conformance be testable with gossip disabled — "a client with
                // gossip disabled is slower and completely correct" — and a node
                // that published but never subscribed, or the reverse, would be
                // neither on nor off.
                if live {
                    subscribe_channels(&store, &mut node)?;
                    publish_unsent_live(
                        &store,
                        &mut node,
                        &identity,
                        &mut broadcast,
                        live_window,
                    )?;
                }
                if holds_group {
                    catch_up_epochs(&store, &mut node)?;
                    let excluded = exclude_removed_members(&store, &mut node, &identity, seal_bytes, sink)?;
                    if excluded > 0 {
                        sink(&[Event::EpochRotated { excluded }]);
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
                // Said every tick, so the claim outlives this process by at
                // most its staleness window even when the window manager ends
                // it without running a destructor.
                _claim.beat();

                // Ask for a key if that has not happened yet.
                //
                // **The ask used to live only inside `Synced { accepted > 0 }`,
                // nested under `learned > 0`** — so it fired only on the sync
                // that accepted new governance entries, and only if this node
                // happened to be `ready` at that instant. Miss it and there is
                // no second chance: every later tick syncs, accepts nothing new,
                // and skips the whole block. A joiner then sits forever having
                // learned its own admission and never asked to be keyed in.
                //
                // Reproduced by starving the machine, which is what made the
                // race lose: `ready` fails while the ledger advertisement has
                // not landed, and on a quiet loopback the entries were all
                // accepted in one go, so the one opportunity was the one that
                // failed.
                //
                // **Asked on a schedule, not once.** A founder has ordinary
                // reasons not to answer at the instant a request arrives —
                // answering appends a rotation and so takes the store's append
                // lock every one-shot command also takes — and a lost race
                // reports a degradation on *its* terminal and tells the asker
                // nothing. A single ask therefore stranded a member permanently.
                // Repeating it is safe since Core §3.5.1: a repeat request
                // replaces this node's leaf rather than minting a second one,
                // which is what used to fork the log against the entry that
                // admitted them.
                if !keyed
                    && last_asked.is_none_or(|at: Instant| at.elapsed() >= KEY_REQUEST_RETRY)
                    && ready(&store, &mut node, &identity, seal_bytes).is_ok()
                {
                    for peer in connected.iter().copied() {
                        if let Some(from) = peer_identity(peer, &store)?
                            && node.request_epoch_key(from, &identity).is_ok()
                        {
                            crate::say!(report, "asked {} to key us in", from.short());
                            unkeyed_since.get_or_insert_with(Instant::now);
                            last_asked = Some(Instant::now());
                            break;
                        }
                    }
                }

                // A stall here is silent otherwise, and it is the one place in
                // this loop that cannot recover on its own.
                //
                // **The request is sent once**, on the `Synced` event that first
                // learned this node had been admitted, and that event need never
                // recur. A founder has ordinary reasons not to answer at that
                // instant — answering appends a rotation and so takes the store's
                // append lock, which every one-shot command also takes — and a
                // lost race reports a degradation to *its* terminal and tells the
                // asker nothing.
                //
                // This node does keep asking now (Core §3.5.1 made that safe),
                // so a stall this long is no longer "nothing will retry" — it
                // means the answers are not arriving or not being given, and the
                // reason is on the other side's terminal rather than this one's.
                if !keyed
                    && !said_unkeyed
                    && unkeyed_since.is_some_and(|at: Instant| at.elapsed() >= UNKEYED_WARNING)
                {
                    said_unkeyed = true;
                    sink(&[Event::Degraded {
                        reason: format!(
                            "still waiting for a key {}s after first asking, and still asking \
                             every {}s. The answer is what is missing, so check the other side's \
                             log for a refusal or for a node that holds no group",
                            UNKEYED_WARNING.as_secs(),
                            KEY_REQUEST_RETRY.as_secs()
                        ),
                    }]);
                }

                for peer in connected.iter().copied() {
                    node.sync_with(peer);
                    node.sync_ledger_with(peer);
                    node.sync_pointers_with(peer);
                }
                request_foreign_segments(&store, &mut node, &identity, &mut fetched, &backfill)?;
                sink(&absorb_segments(
                    &store,
                    &mut node,
                    &identity,
                    &mut backfill,
                )?);
                continue;
            }
        };
        match event {
            // A circuit address going away means the reservation ended, and the
            // watchdog above is what asks for another. Said out loud because
            // the node keeps running and keeps its direct listeners: from every
            // other angle nothing happened.
            NodeEvent::ListenAddrGone(address) => {
                if crate::is_circuit_address(&address) {
                    crate::say!(report, "  relay     the circuit on {address} ended — asking again");
                    sink(&[Event::Relay {
                        reserved: None,
                        designated: designated.len(),
                        failures: Vec::new(),
                    }]);
                }
                continue;
            }
            NodeEvent::Listening(address) => {
                // Printed with the peer id appended, because that is the form
                // another node can actually dial — an address without it names a
                // machine rather than a member.
                // Appended only when it is missing. A direct listen address
                // names a machine until the peer id makes it name a member — but
                // a circuit address already ends in one, and appending a second
                // produces `/p2p-circuit/p2p/<id>/p2p/<id>`, which is not what
                // anybody dials.
                let full = if address
                    .iter()
                    .any(|part| matches!(part, Protocol::P2p(_)))
                {
                    address.clone()
                } else {
                    address.clone().with(Protocol::P2p(identity.peer_id()))
                };
                if listening.iter().all(|seen| seen != &full) {
                    crate::say!(report, "  listening {full}");
                    listening.push(full);
                    // Written down so a one-shot `kols invite` can carry them.
                    // Only a running node knows what it is reachable on, and an
                    // invite with no bootstrap address cannot connect anybody —
                    // but most of what it listens on cannot answer a stranger,
                    // so this is a selection rather than the whole list. See
                    // `addresses_for_an_invite` for which ones and why.
                    let addresses = crate::addresses_for_an_invite(&listening, &preferred);
                    if let Err(err) = store.set_addresses(&addresses) {
                        sink(&[Event::Degraded {
                            reason: format!("could not record this node's addresses: {err}"),
                        }]);
                    }
                }
            }

            // Whether tier 2 actually happened, said out loud.
            //
            // It was neither printed nor reported, so "is this still going
            // through the relay?" had no answer short of taking the relay away
            // — which is how somebody found out that losing a circuit dropped a
            // peer they could still reach directly.
            NodeEvent::HolePunchSucceeded { peer } => {
                crate::say!(report, "  direct    hole-punched to {peer} — the relay is out of the path");
                sink(&[Event::Degraded {
                    reason: format!("connected directly to {peer}; the relay introduced you and is no longer carrying anything"),
                }]);
                continue;
            }
            NodeEvent::HolePunchFailed { peer } => {
                // **Core §5.2 says these two peers do not connect**, and the
                // transport enforces it: the circuit existed to carry this
                // negotiation, so it is closed and the peer disconnected rather
                // than left to become the path. Nothing is owed here beyond
                // saying so, and saying so matters — a pair that silently keeps
                // talking over a relay is the exact failure the rule prevents,
                // and it looks like success from both ends.
                //
                // This is not a partition from the network. Everything here is
                // pull-based and content-addressed, so the two converge through
                // any member both can reach; what they have lost is each other.
                crate::say!(report, 
                    "  direct    could not hole-punch to {peer} — the circuit is closed \
                     (Core §5.2). You still converge through any member you both reach"
                );
                sink(&[Event::Degraded {
                    reason: format!(
                        "no direct connection to {peer}. Core §5.2: a pair that cannot \
                         hole-punch reaches each other over IPv6 or not at all, and a relay \
                         carries the negotiation and nothing else — so the circuit has been \
                         closed. You are not cut off from the network: anything either of you \
                         publishes still reaches the other through any member you both reach"
                    ),
                }]);
                continue;
            }

            NodeEvent::Connected { peer, .. } => {
                crate::say!(report, "connected to {peer}");
                connected.insert(peer);
                record_connected(&store, &connected, sink);
                // A new peer is a new place to walk from. The routing table is
                // filled from peers this node connects to and nothing else, so
                // until it is walked it is one hop deep — and a provider query
                // over a one-hop table can only ask people already on the other
                // end of a socket. That is what made this look like it needed
                // every member connected to every other one.
                node.bootstrap_dht();
                start_sync(&mut node, peer);
            }

            NodeEvent::Disconnected { peer } => {
                connected.remove(&peer);
                record_connected(&store, &connected, sink);
            }

            // Governance first. Everything else is gated on it, so a peer's
            // content is not worth asking for until this settles.
            NodeEvent::Synced { accepted, peer, .. } if accepted > 0 => {
                let learned = persist_governance(&store, &node)?;
                if learned > 0 {
                    sink(&[Event::Governance { learned }]);
                    // A heal is exactly when a losing branch appears, so this is
                    // where the voided-actions report is worth asking for.
                    if let Some(reorg) = voided_report(&store, &identity.id(), &mut reorged) {
                        sink(&[reorg]);
                    }
                    // The entries that just arrived may include rotations this
                    // node was not present for.
                    catch_up_epochs(&store, &mut node)?;
                    // The log that just arrived may be the one that admits this
                    // node. Advertising and publishing are retried here rather
                    // than only at startup for exactly that reason, and the
                    // ledger is re-synced because a peer that refused this
                    // node's advertisement a moment ago will accept it now.
                    if ready(&store, &mut node, &identity, seal_bytes).is_ok() {
                        node.sync_ledger_with(peer);
                        node.sync_pointers_with(peer);
                        // A member with no key can fetch every byte of this
                        // network's content and open none of it, so asking for
                        // one is the first thing worth doing after admission.
                        if !keyed
                            && last_asked
                                .is_none_or(|at: Instant| at.elapsed() >= KEY_REQUEST_RETRY)
                            && let Some(from) = peer_identity(peer, &store)?
                        {
                            match node.request_epoch_key(from, &identity) {
                                Ok(_) => {
                                    crate::say!(report, "asked {} to key us in", from.short());
                                    unkeyed_since.get_or_insert_with(Instant::now);
                                    last_asked = Some(Instant::now());
                                }
                                Err(err) => sink(&[Event::Degraded {
                                    reason: format!("could not ask for a key: {err}"),
                                }]),
                            }
                        }
                    }
                }
            }

            // A ledger that just accepted somebody may have made a source
            // rankable that was not before, so this is where a stalled fetch
            // gets its second chance.
            NodeEvent::LedgerSynced { accepted, .. } if accepted > 0 => {
                request_foreign_segments(&store, &mut node, &identity, &mut fetched, &backfill)?;
            }

            NodeEvent::PointersReceived { .. } => {
                keep_pointers(&store, &node, sink);
                request_foreign_segments(&store, &mut node, &identity, &mut fetched, &backfill)?;
            }

            NodeEvent::FetchComplete { .. } => {
                sink(&absorb_segments(
                    &store,
                    &mut node,
                    &identity,
                    &mut backfill,
                )?);
                request_foreign_segments(&store, &mut node, &identity, &mut fetched, &backfill)?;
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
                adopt_local_changes(&store, &mut node, &identity, seal_bytes, sink)?;
                let answered = node.answer_epoch_key(
                    request,
                    &identity,
                    Timestamp::from_millis(crate::chat::now_millis()),
                );
                match answered {
                    Ok(_) => {
                        sink(&[Event::MemberKeyed {
                            identity: requester,
                        }]);
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
                    Err(err) => sink(&[Event::Degraded {
                        reason: format!("could not key in {}: {err}", requester.short()),
                    }]),
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
                    crate::say!(report, 
                        "keyed into this network ({historical_keys} historical key(s) came with it)"
                    );
                    request_foreign_segments(&store, &mut node, &identity, &mut fetched, &backfill)?;
                }
            }

            // Somebody presented an invite. Surfaced rather than answered
            // automatically by the transport because validating one needs a
            // clock and admitting needs a signature, neither of which that
            // layer holds.
            NodeEvent::JoinRequested {
                joiner, request, ..
            } => {
                // Held across the answer because under auto-admit this appends
                // a membership entry, and an append racing the daemon's own
                // would fork the log.
                let lock = store.lock().map_err(|e| e.to_string())?;
                let answered =
                    node.answer_join(request, &identity, Timestamp::from_millis(crate::chat::now_millis()));
                drop(lock);

                match answered {
                    Ok(response) => {
                        // The membership entry, if the network auto-admits, is
                        // in the node's log and not yet in the store.
                        persist_governance(&store, &node)?;
                        record_waiting(&store, &node, &identity, sink);
                        sink(&[Event::JoinAnswered {
                            joiner,
                            accepted: !matches!(
                                response,
                                intranet_invite::JoinResponse::Refused { .. }
                            ),
                        }]);
                    }
                    Err(err) => sink(&[Event::Degraded {
                        reason: format!("could not answer a join from {}: {err}", joiner.short()),
                    }]),
                }
            }

            NodeEvent::EpochKeyUnavailable { reason, .. } => {
                sink(&[Event::Degraded {
                    reason: format!("not keyed in: {reason}"),
                }]);
            }

            // A live payload — spec 07 §6.1. Nothing depends on this arriving:
            // the same record reaches every member through the durable path, so
            // anything refused here costs latency and nothing else.
            // Reported in the same words the durable path uses, with the path
            // named. Two vocabularies for one event would make "did this
            // record arrive" depend on which way it happened to come — and a
            // record that arrived live is *already stored*, so the durable
            // absorb that follows correctly reports nothing.
            NodeEvent::LiveReceived { payload, .. } => match admit_live(&store, &payload) {
                Ok(Some(record)) => sink(&[Event::Records {
                    channel: record.channel,
                    records: vec![record],
                    arrival: Arrival::Live,
                }]),
                Ok(None) => {}
                // Surfaced rather than swallowed: a refusal is either a peer
                // sending what it should not, or this node missing a key it
                // should have. Both are worth seeing; neither is worth stopping.
                Err(why) => sink(&[Event::Degraded {
                    reason: format!("refused a live payload: {why}"),
                }]),
            },

            // A chunk this node fetched makes it a source for that chunk
            // (Storage §4.2), so it is announced rather than held quietly — and
            // written down, because being a source has to outlive the process
            // that became one.
            NodeEvent::ChunkReceived { cid, .. } if announced.insert(cid) => {
                node.announce_chunk(cid);
                if let Some(bytes) = node.chunk_store().get(&cid)
                    && let Err(err) = store.put_chunk(&cid, bytes)
                {
                    sink(&[Event::Degraded {
                        reason: format!("could not keep a chunk across restarts: {err}"),
                    }]);
                }
            }

            _ => {}
        }
    }
}

/// Puts back what a previous run kept for other members.
///
/// Returns how many chunks and pointers came back, so a restart can say what it
/// is carrying rather than leaving it to be inferred.
///
/// The chunks are announced as they land, because a chunk this node holds and
/// has not announced is one nobody can discover here — which is the whole of
/// what a restart used to lose. Announcing is best-effort by design: Kademlia
/// refuses to start providing before there is a peer to publish to, and
/// `ChunkReceived` re-announces anything fetched later anyway.
fn restore_contribution(store: &Store, node: &mut MemberNode) -> Result<(usize, usize), String> {
    let mut chunks = 0;
    for bytes in store.chunks().map_err(|e| e.to_string())? {
        let cid = intranet_storage::Cid::of(&bytes);
        // `insert` re-derives the CID and refuses a mismatch, so a file that was
        // corrupted on disk is dropped here rather than served on.
        if node.chunk_store_mut().insert(cid, bytes).is_ok() {
            node.announce_chunk(cid);
            chunks += 1;
        }
    }

    let mut pointers = 0;
    for record in store.pointers().map_err(|e| e.to_string())? {
        // The pointer first: a wrapping names the pointer it opens, and one
        // accepted for a pointer this node does not hold opens nothing.
        node.accept_pointer(record.pointer);
        for wrapping in record.wrappings {
            node.accept_wrapping(wrapping);
        }
        pointers += 1;
    }
    Ok((chunks, pointers))
}

/// Writes down the pointers this node holds, with the wrappings that open them.
///
/// Called when a sync brings some in rather than on a timer: a pointer that has
/// not moved re-encodes to the same bytes, so this is a rewrite of what changed
/// and a no-op for the rest.
///
/// This node's own pointers are written too. They are re-derived at startup by
/// `publish_own_logs` and so do not need to be, but excluding them would mean
/// asking on every pointer which kind it is, to save a few files.
fn keep_pointers(store: &Store, node: &MemberNode, sink: &Sink) {
    let held: Vec<_> = node
        .pointers()
        .map(|pointer| {
            (
                pointer.clone(),
                node.wrappings_for(&pointer.pointer_id)
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    for (pointer, wrappings) in held {
        if let Err(err) = store.put_pointer(&pointer, wrappings) {
            sink(&[Event::Degraded {
                reason: format!("could not keep a pointer across restarts: {err}"),
            }]);
            return;
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
    seal_bytes: usize,
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
    publish_own_logs(store, node, seal_bytes)
}

/// Publishes every author log this node owns into the node's chunk store.
///
/// One pass rebuilds a channel's whole chain from the stored records, sealing at
/// the thresholds as it goes, and publishes each segment under its own pointer
/// and its own key. Retention is applied per segment here rather than per log —
/// which is the point of the per-segment keys, and is what `design/01` §8 means
/// by dropping *old* history rather than a whole conversation.
fn publish_own_logs(
    store: &Store,
    node: &mut MemberNode,
    seal_bytes: usize,
) -> Result<usize, String> {
    let identity = store.identity().map_err(|e| e.to_string())?;
    let Some(state) = replayable(store) else {
        return Ok(0);
    };
    let mut published = 0;

    let retention = kols_core::ChatPolicy::of(&state.policy).retain_messages();
    let now = crate::chat::now_millis();
    let spec = ChunkSpec::from_target(64 * 1024);

    for channel in store.channels_with_records().map_err(|e| e.to_string())? {
        let own = store
            .own_records(&channel, &identity.id())
            .map_err(|e| e.to_string())?;
        if own.is_empty() {
            continue;
        }

        let author = identity.id();
        let mut sequence = 0u64;
        let mut pointer = kols_core::author_segment_pointer(&channel, &author, sequence);
        let mut dek = store.channel_dek(&pointer).map_err(|e| e.to_string())?;
        let mut log = AuthorLog::open(&identity, channel, dek.clone(), spec);

        // The open segment's newest record and the last thing publishing it
        // produced. Held together because retention is judged on the newest
        // record a segment carries, and that is only known once the segment is
        // complete.
        let mut newest = 0i64;
        let mut latest: Option<kols_core::Published> = None;

        for record in own {
            // **Checked before the append, not after.** Sealing after the record
            // that crosses the threshold leaves the pass ending on an empty new
            // segment — nothing to publish, and a head index naming a segment
            // that does not exist. Sealing lazily, when the next record needs
            // somewhere to go, means the head always has content.
            if let Some(complete) = latest.take_if(|_| should_seal(&log, seal_bytes)) {
                if publish_retained(
                    store, node, &identity, &pointer, &dek, &complete, newest, &retention, now,
                )? {
                    published += 1;
                }
                sequence += 1;
                pointer = kols_core::author_segment_pointer(&channel, &author, sequence);
                dek = store.channel_dek(&pointer).map_err(|e| e.to_string())?;
                log.seal(complete.object.manifest_cid(), dek.clone());
            }
            newest = record.hlc.wall_millis;
            latest = Some(
                log.append(&identity, record, &state)
                    .map_err(|err| format!("a stored record no longer appends: {err}"))?,
            );
        }

        let Some(head) = latest else {
            continue;
        };
        if publish_retained(
            store, node, &identity, &pointer, &dek, &head, newest, &retention, now,
        )? {
            published += 1;
        }

        // The index last, so it never names a segment this pass has not put in
        // the chunk store. A reader that found it pointing at a missing segment
        // would have no way to tell that from history it is not allowed to read.
        let index_pointer = kols_core::author_log_pointer(&channel, &author);
        let index_dek = store
            .channel_dek(&index_pointer)
            .map_err(|e| e.to_string())?;
        let index =
            kols_core::publish_head_index(&identity, channel, sequence, &index_dek, spec, &state)
                .map_err(|err| format!("could not publish a head index: {err}"))?;
        let _ = publish_segment(node, &index);
        wrap_for(store, node, &identity, &index_pointer, &index_dek);
    }
    Ok(published)
}

/// Whether the open segment has reached a threshold and should be sealed.
///
/// `design/01` §3.1's two, and both are **local publishing tuning** rather than
/// validity rules — a reader accepts whatever boundaries an author chose.
///
/// Age is measured across the segment's own records, newest minus oldest, and
/// not against the clock. That is what keeps the chain a pure function of the
/// record sequence: a rebuild months later re-derives the same boundaries and
/// republishes the identical chain, where "older than a day *now*" would seal
/// somewhere new on every restart. It is also why sealing needs no persisted
/// state of its own — the record sequence is the state.
fn should_seal(log: &AuthorLog, seal_bytes: usize) -> bool {
    let segment = log.segment();
    if segment.records.is_empty() {
        return false;
    }
    if segment.canonical_bytes().len() >= seal_bytes {
        return true;
    }
    match (segment.records.first(), segment.records.last()) {
        (Some(oldest), Some(newest)) => {
            newest.hlc.wall_millis - oldest.hlc.wall_millis >= SEAL_TARGET_SPAN_MILLIS
        }
        _ => false,
    }
}

/// Publishes one segment, unless retention says this node should stop carrying it.
///
/// Retention is the *absence* of maintenance rather than a deletion (`design/01`
/// §8): a segment past the window stops being republished and stops being
/// re-wrapped, and content with no live wrapping goes dark on its own (Storage
/// §5.2). No new mechanism — only the decision to stop.
///
/// Judged on the **newest** record the segment carries, not the oldest, for the
/// same reason the per-log version was: a segment is retired when everything in
/// it has aged out, not when it started long ago.
///
/// Returns whether anything was published, so a caller can report how much of a
/// log it is still carrying rather than how much exists.
#[allow(clippy::too_many_arguments)]
fn publish_retained(
    store: &Store,
    node: &mut MemberNode,
    identity: &intranet_identity::PerNetworkIdentity,
    pointer: &intranet_governance::PointerId,
    dek: &intranet_storage::Dek,
    segment: &kols_core::Published,
    newest: i64,
    retention: &kols_core::Retention,
    now: i64,
) -> Result<bool, String> {
    if retires(retention, newest, now) {
        return Ok(false);
    }
    let _ = publish_segment(node, segment);
    wrap_for(store, node, identity, pointer, dek);
    Ok(true)
}

/// Whether a segment whose newest record is at `newest` has aged out.
///
/// Judged on the **newest** record it carries rather than the oldest. A segment
/// is retired once everything in it has aged out, not because it started long
/// ago — the alternative retires the beginning of a conversation somebody is
/// still having.
fn retires(retention: &kols_core::Retention, newest: i64, now: i64) -> bool {
    let age_days = u32::try_from((now - newest).max(0) / 86_400_000).unwrap_or(u32::MAX);
    !retention.covers(age_days)
}

/// Wraps a DEK under the current epoch so anybody else can open the object.
///
/// A pointer carries a commitment to its DEK, never the DEK itself, so a peer
/// that fetches every chunk still needs a wrapping under an epoch key it holds —
/// and pointer sync carries wrappings alongside pointers precisely because the
/// two are useless apart (Storage §5.3).
fn wrap_for(
    store: &Store,
    node: &mut MemberNode,
    identity: &intranet_identity::PerNetworkIdentity,
    pointer: &intranet_governance::PointerId,
    dek: &intranet_storage::Dek,
) {
    if let (Ok(epoch), Ok(rotation)) = (store.epoch_key(), store.rotation_ref()) {
        node.accept_wrapping(intranet_storage::DekWrapping::create(
            identity, *pointer, dek, &epoch, rotation,
        ));
    }
}

/// Writes any governance entries the node learned into the store.
/// The voided-actions report, if reconciliation undid anything not yet reported.
///
/// **Core §2.7.1 point 5 makes producing this mandatory and acting on it a
/// client's job**, and until now nothing here asked. The consequence is not
/// abstract: an entry that removed somebody — a revocation, a moderation, an
/// epoch rotation — is treated as never having happened once its branch loses,
/// so the member it removed is current again and holds the key, for as long as
/// it takes a person to notice. This is what assigns the noticing.
///
/// `None` when nothing was voided, or when everything voided has already been
/// reported once. Reporting the same heal on every subsequent sync would train
/// somebody to ignore it, which is worse than not reporting it.
fn voided_report(
    store: &Store,
    me: &intranet_identity::PerNetworkIdentityId,
    reported: &mut BTreeSet<intranet_crypto::Hash>,
) -> Option<Event> {
    let mut log = store.log().ok()?;
    let reconciliation = log.reconcile(Timestamp::from_millis(crate::chat::now_millis()));

    report_of(&reconciliation.voided, me, reported)
}

/// Turns a voided list into the event, or nothing if it is all old news.
///
/// Split out from [`voided_report`] so the rules that matter here — whose
/// actions these were, and not saying the same thing twice — can be tested
/// without a partitioned store to produce them.
fn report_of(
    voided: &[intranet_governance::VoidedEntry],
    me: &intranet_identity::PerNetworkIdentityId,
    reported: &mut BTreeSet<intranet_crypto::Hash>,
) -> Option<Event> {
    let fresh: Vec<_> = voided
        .iter()
        .filter(|entry| !reported.contains(&entry.hash))
        .collect();
    if fresh.is_empty() {
        return None;
    }
    for entry in &fresh {
        reported.insert(entry.hash);
    }

    let mine: Vec<kols_api::VoidedAction> = fresh
        .iter()
        .filter(|entry| &entry.author == me)
        .map(|entry| kols_api::VoidedAction {
            kind: entry.kind.to_owned(),
            security_relevant: entry.security_relevant,
        })
        .collect();
    let others = fresh.len() - mine.len();

    // Reported even when none of it is this member's. Somebody else's voided
    // revocation restores a member here too, and a node that stayed quiet about
    // it would be keeping the more useful half to itself.
    Some(Event::GovernanceReorg { mine, others })
}

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
    backfill: &BTreeSet<intranet_storage::Cid>,
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

    // Older segments a chain walk reached and could not read. They go in the
    // same queue as the heads: a sealed segment is an ordinary object, fetched
    // the ordinary way, and nothing about backfill needs a second path.
    wanted.extend(backfill.iter().copied());

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
///
/// `backfill` collects the older segments this pass wanted and could not read
/// yet; `request_foreign_segments` asks for them on the next tick.
fn absorb_segments(
    store: &Store,
    node: &mut MemberNode,
    identity: &intranet_identity::PerNetworkIdentity,
    backfill: &mut BTreeSet<intranet_storage::Cid>,
) -> Result<Vec<Event>, String> {
    let Some(state) = replayable(store) else {
        return Ok(Vec::new());
    };
    let (channels, _) = network::channels(store, &state).map_err(|e| e.to_string())?;
    let mut events = Vec::new();

    // Loaded once. This reads and opens every stored epoch key, so calling it
    // per (channel, member) meant a long-lived network paid a full directory
    // scan a thousand times per tick — far more than the unwrap it was there to
    // serve. Lazily, because a node with nothing to absorb should not pay for it
    // at all.
    let mut keys: Option<Vec<_>> = None;

    for channel in channels.keys() {
        let mut took = Absorbed::default();
        for member in members(&state) {
            if member == identity.id() {
                continue;
            }
            // The index first, and only then the segment it names. This is the
            // indirection per-segment keys cost: `author_log_pointer` is what a
            // reader can derive from public information alone, and it now names
            // which segment is the head rather than the head itself.
            let index_id = kols_core::author_log_pointer(channel, &member);
            let Some((index_cid, index_dek)) = resolve(store, node, &mut keys, &index_id)? else {
                continue;
            };
            let Ok(index) = fetch_segment(node, index_cid, &index_dek) else {
                backfill.insert(index_cid);
                continue;
            };

            let head_id = kols_core::author_segment_pointer(channel, &member, index.sequence);
            let Some((cid, dek)) = resolve(store, node, &mut keys, &head_id)? else {
                continue;
            };
            // A segment we cannot assemble yet is not an error: the fetch runs
            // in rounds, and the manifest arrives before the chunks it names.
            let Ok(segment) = fetch_segment(node, cid, &dek) else {
                // Queued the same way a backfill hop is. Only the index CID is
                // derivable from public information; everything past it — the
                // head included — is learned by reading, so this is how a head
                // segment gets asked for at all.
                backfill.insert(cid);
                continue;
            };
            let one =
                absorb_chain(store, node, channel, &member, (cid, segment), &mut keys, backfill)?;
            took.absorb(one);
        }
        events.extend(took.into_events(*channel));
    }
    Ok(events)
}

/// Finds the CID and DEK of an object this node does not own.
///
/// Returns `None` while either half is missing, which is an ordinary state
/// rather than a failure: a pointer arrives before its wrapping sometimes, a
/// wrapping under an epoch this node has not caught up to is unusable until it
/// does, and a segment past its author's retention window will never have a live
/// wrapping again. All three look the same from here, and should — a reader has
/// no business distinguishing history it may not read from history that has not
/// arrived yet.
fn resolve(
    store: &Store,
    node: &MemberNode,
    keys: &mut Option<Vec<(intranet_crypto::Hash, intranet_storage::EpochKey)>>,
    pointer_id: &intranet_governance::PointerId,
) -> Result<Option<(intranet_storage::Cid, intranet_storage::Dek)>, String> {
    let Some(pointer) = known_pointer(node, pointer_id) else {
        return Ok(None);
    };
    let commitment = pointer.dek_commitment;

    // The cached DEK first, checked against the pointer's own commitment so a
    // stale one is discarded rather than used to fail at decryption. In the
    // steady state this is one unwrap under the current key and no scan.
    if let Some(dek) = store
        .known_dek(pointer_id, Some(&commitment))
        .map_err(|e| e.to_string())?
    {
        return Ok(Some((pointer.current_cid, dek)));
    }

    // Never `channel_dek` here: that mints a key for an object this node owns,
    // which for somebody else's log would open nothing. A foreign DEK can only
    // come from their wrapping, validated against the owner's commitment rather
    // than against who signed it — which is what makes "any current member may
    // re-wrap" safe.
    let held = match keys {
        Some(keys) => keys,
        None => keys.insert(store.epoch_keys().map_err(|e| e.to_string())?),
    };
    let Some(dek) = node.wrappings_for(pointer_id).into_iter().find_map(|wrapping| {
        held.iter()
            .find_map(|(_, key)| wrapping.unwrap(key, &commitment).ok())
    }) else {
        return Ok(None);
    };
    // Remembered under the current epoch, so this scan happens once per object
    // rather than on every tick forever.
    store
        .remember_dek(pointer_id, &dek)
        .map_err(|e| e.to_string())?;
    Ok(Some((pointer.current_cid, dek)))
}

/// Writes down who is waiting, so a one-shot command can see it.
///
/// The waiting room is live state in the running node (Core §2.4) — a valid
/// identity holding no capabilities and no key until somebody admits it. Nothing
/// outside this process can ask about it, so the daemon writes what it sees and
/// `kols waiting` reads that. Best-effort: failing to write it should not stop a
/// node from having let somebody in.
fn record_waiting(
    store: &Store,
    node: &MemberNode,
    identity: &intranet_identity::PerNetworkIdentity,
    sink: &Sink,
) {
    let Some(occupants) = node.waiting_room_for(&identity.id()) else {
        return;
    };

    // **Nothing at the door and nothing written down: stop here.**
    //
    // This runs on every tick, and the replay below reads the whole governance
    // log off disk and re-applies it — `Store::state` is not a lookup. Paying
    // that every two seconds for the lifetime of a node, to answer a question
    // whose answer is almost always "nobody", is the kind of cost that does not
    // show up until a log is long.
    //
    // Both halves are needed: the first skips the usual case, and the second is
    // what still lets a room that has just emptied write that fact down once.
    // `waiting()` is a small file; `state()` is the log.
    if occupants.is_empty() && store.waiting().is_empty() {
        return;
    }

    // **Anybody already admitted is no longer waiting.**
    //
    // **Anybody already admitted is no longer waiting.** The node's room is
    // populated when a join is answered and emptied by nothing that admission
    // goes through — admitting writes a governance entry, and no path from one
    // reaches this room — so a founder who admitted somebody kept being shown
    // them at the door, with an `admit` button that had already been pressed.
    // **The union of what this node knows now and what was written down, not a
    // replacement.** The waiting room is in-memory state on the node — a fresh
    // `WaitingRoom` at construction — so a restart empties it, and writing the
    // empty room over the file *erased the person standing at the door*. From
    // both ends it then looked like the join had never happened: their client
    // had been told once that it was waiting and does not ask again, and the
    // admin's door was blank with nothing to admit.
    //
    // Closing the window and switching networks both restart the node, which
    // makes this the ordinary case rather than a rare one. Somebody who knocked
    // is still knocking after the application is reopened, and this is the file
    // that has to remember it, since nothing else does.
    //
    // Growth is bounded by the filter below rather than by forgetting: an
    // admitted member leaves on the next tick, and somebody who knocked and was
    // never admitted is genuinely still waiting, which is what the entry says.
    let members = store.state().ok();
    let mut identities: std::collections::BTreeSet<String> =
        store.waiting().into_iter().collect();
    identities.extend(
        occupants
            .iter()
            .map(|entry| intranet_crypto::to_hex(entry.identity.verifying_key().as_bytes())),
    );

    // Filtered against replayed state rather than by remembering who was
    // admitted, because replay is the authority on membership — so this stays
    // correct for somebody admitted by another member, and across a restart.
    let identities: Vec<String> = identities
        .into_iter()
        .filter(|hex| {
            crate::parse_identity(hex).is_ok_and(|id| {
                members.as_ref().is_none_or(|state| !state.is_member(&id))
            })
        })
        .collect();
    if let Err(err) = store.set_waiting(&identities) {
        sink(&[Event::Degraded {
            reason: format!("could not record the waiting room: {err}"),
        }]);
    }
}

/// Writes down who this node is connected to, so anything else can read it.
///
/// Written on change rather than on a tick, because it *is* the change — unlike
/// the waiting room, nothing here has to be recomputed against replayed state,
/// so there is no reason to pay for it on a schedule.
///
/// A peer whose identity cannot be resolved is left out rather than shown as a
/// peer id: this list exists to answer "which members am I talking to", and a
/// libp2p peer id is not a member (Core §1.2 — the mapping is what makes an
/// identity per-network in the first place).
fn record_connected(store: &Store, connected: &BTreeSet<PeerId>, sink: &Sink) {
    // Replayed once for the whole set, not once per peer. `peer_identity` reads
    // and replays the entire governance log, so mapping five peers through it
    // would be five replays for one answer — the repeated-replay cost `design/05`
    // §5 carries, and not worth introducing again on a path that fires on every
    // connection.
    let Some(state) = replayable(store) else {
        return;
    };
    let roster = members(&state);
    let identities: Vec<String> = connected
        .iter()
        .filter_map(|peer| roster.iter().find(|id| id.peer_id() == *peer))
        .map(|id| intranet_crypto::to_hex(id.verifying_key().as_bytes()))
        .collect();
    if let Err(err) = store.set_connected(&identities) {
        sink(&[Event::Degraded {
            reason: format!("could not record who this node is connected to: {err}"),
        }]);
    }
}

/// Prints what the node learned, and stays quiet when it learned nothing.
///
/// The one place this daemon turns an [`Event`] into words, which is the point:
/// a terminal renders these, and a webview would render the same values
/// differently. The wording for records is load-bearing — every path reports an
/// arrival in the same words with only the path named, so that "did this record
/// arrive" does not depend on which way it came.
fn render(events: &[Event]) {
    for event in events {
        match event {
            Event::Records {
                records, arrival, ..
            } => match arrival {
                Arrival::Head => println!("learned {} record(s)", records.len()),
                Arrival::Live => println!("learned {} record(s) live", records.len()),
                Arrival::Backfill { segments } => println!(
                    "backfilled {} record(s) from {segments} older segment(s)",
                    records.len()
                ),
            },
            Event::Governance { learned } => println!("learned {learned} governance entr(ies)"),
            Event::GovernanceReorg { mine, others } => {
                // Loud on purpose. A voided revocation restores a member nobody
                // meant to restore, and the whole reason this event exists is
                // that it used to happen with nobody assigned to notice.
                let risky = mine.iter().filter(|a| a.security_relevant).count();
                println!(
                    "a fork healed: {} of your action(s) were voided ({risky} security-relevant), \
                     and {others} of other members'",
                    mine.len()
                );
                for action in mine.iter().filter(|a| a.security_relevant) {
                    println!(
                        "  {} was undone — resubmit it, or whatever it removed is back",
                        action.kind
                    );
                }
            }
            Event::Adopted { entries } => {
                println!("picked up {entries} locally written governance entr(ies)");
            }
            Event::EpochRotated { excluded } => {
                println!("rotated the epoch to exclude {excluded} removed member(s)");
            }
            Event::MemberKeyed { identity } => println!("keyed in {}", identity.short()),
            Event::JoinAnswered { joiner, accepted } => {
                if *accepted {
                    println!("let {} in — `kols waiting` shows who is waiting", joiner.short());
                } else {
                    println!("refused a join from {}", joiner.short());
                }
            }
            Event::Degraded { reason } => println!("{reason}"),
            // Nothing: the terminal already printed the relay's standing where
            // it was settled, in the startup block with the rest of the header.
            // This event exists for consumers that had no equivalent.
            Event::Relay { .. } => {}
        }
    }
}

/// How often to check that a relay circuit is still held.
///
/// Not a renewal interval — libp2p renews an hour-long reservation on its own.
/// This is how long a node can be silently unreachable after a relay restarts,
/// which is the case that actually happens.
const RELAY_RECHECK: std::time::Duration = std::time::Duration::from_secs(20);

/// How often to re-dial known peers while none is connected.
///
/// Long enough not to hammer a peer that is genuinely away, short enough that a
/// relay coming back is measured in seconds rather than in restarts.
const REDIAL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

/// How often to walk the DHT again.
///
/// Not a cost worth minimising — a bootstrap is a handful of queries — and a
/// routing table that is never refreshed thins out as peers leave until it can
/// no longer route, which reads as content having no providers.
const DHT_BOOTSTRAP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);

/// Reserves a circuit on the first designated relay that grants one.
///
/// Shared by startup and by the watchdog, so a reservation regained after a
/// relay restart is obtained exactly the way the first one was.
async fn reserve_any(
    node: &mut MemberNode,
    designated: &[String],
    sink: &Sink,
) -> Option<String> {
    let mut discarded = Vec::new();
    reserve_any_reporting(node, designated, sink, &mut discarded).await
}

/// As [`reserve_any`], collecting the reason each relay failed.
async fn reserve_any_reporting(
    node: &mut MemberNode,
    designated: &[String],
    sink: &Sink,
    failures: &mut Vec<String>,
) -> Option<String> {
    for relay in designated {
        let address: Multiaddr = match relay.parse() {
            Ok(address) => address,
            Err(err) => {
                let reason = format!("this network names an unusable relay {relay:?}: {err}");
                failures.push(reason.clone());
                sink(&[Event::Degraded { reason }]);
                continue;
            }
        };
        match node.reserve_via_relay(address).await {
            Ok(()) => {
                if node.await_reservation().await {
                    println!("  relay     reserved a circuit on {relay}");
                    return Some(relay.clone());
                }
                // Deliberately does not say the relay answered.
                // `reserve_via_relay` returning `Ok` means the reservation was
                // *started* — a circuit listener registered — not that anything
                // replied. Claiming otherwise sent a real deployment looking at
                // a correctly configured relay for an evening, because the
                // message asserted more than the code knew.
                let reason = format!(
                    "no circuit from {relay} within the reservation window. Either \
                     nothing reached it — check the relay's own log for a connection — \
                     or it replied announcing no address of its own"
                );
                failures.push(reason.clone());
                sink(&[Event::Degraded { reason }]);
            }
            Err(err) => {
                let reason = format!(
                    "could not reach the relay {relay} at all: {err}. Nothing answered \
                     there, so this is the address, the port or the network — not what \
                     the relay announces"
                );
                failures.push(reason.clone());
                sink(&[Event::Degraded { reason }]);
            }
        }
    }
    None
}

/// What one absorb pass took in.
///
/// `backfilled` is counted apart from `learned` because the two mean different
/// things to somebody watching a node: records off the head segment are the
/// conversation arriving, records off an older one are history being recovered.
/// A node that has caught up reports the first and never the second.
#[derive(Default)]
struct Absorbed {
    learned: Vec<Record>,
    backfilled: Vec<Record>,
    segments: usize,
}

impl Absorbed {
    fn absorb(&mut self, other: Self) {
        self.learned.extend(other.learned);
        self.backfilled.extend(other.backfilled);
        self.segments += other.segments;
    }

    /// One channel's absorb, as the events a consumer sees.
    ///
    /// Two events rather than one, because the head and the chain behind it mean
    /// different things to whoever is watching: the first is the conversation
    /// arriving, the second is history being recovered.
    fn into_events(self, channel: ChannelId) -> Vec<Event> {
        let mut events = Vec::new();
        if !self.learned.is_empty() {
            events.push(Event::Records {
                channel,
                records: self.learned,
                arrival: Arrival::Head,
            });
        }
        if !self.backfilled.is_empty() {
            events.push(Event::Records {
                channel,
                records: self.backfilled,
                arrival: Arrival::Backfill {
                    segments: self.segments,
                },
            });
        }
        events
    }
}

/// Stores a segment, then walks its `previous` chain backwards.
///
/// This is history backfill (`design/01` §5). An author seals a segment once it
/// crosses a size threshold and starts a fresh one, so a reader that only ever
/// read the segment the head index names would see the tail of a conversation
/// and nothing before it. The seal leaves a hash chain (`design/01` §3.1: "so
/// history is walkable and gap-detectable without consulting the pointer's
/// version history"), and this walks it.
///
/// The walk runs as far as the local chunk store can carry it and stops at the
/// first hop it does not hold, queueing that one instead of blocking on it. So a
/// node absorbs a chain it already has in a single pass, and pays a round per
/// hop only for the parts it still has to fetch — which is what keeps this off
/// the critical path of a tick that has live records to deliver.
///
/// **Each hop needs its own key**, since each segment lives under its own
/// pointer (`author_segment_pointer`) — the arrangement that lets an author
/// forget old history without forgetting all of it. A hop whose wrapping this
/// node cannot open is where the walk ends, and that is exactly what reading
/// past a retention boundary looks like from the outside: indistinguishable from
/// history that has not arrived, and rightly so.
fn absorb_chain(
    store: &Store,
    node: &MemberNode,
    channel: &ChannelId,
    author: &intranet_identity::PerNetworkIdentityId,
    head: (intranet_storage::Cid, Segment),
    keys: &mut Option<Vec<(intranet_crypto::Hash, intranet_storage::EpochKey)>>,
    backfill: &mut BTreeSet<intranet_storage::Cid>,
) -> Result<Absorbed, String> {
    let mut took = Absorbed::default();
    let mut walked = Vec::new();
    let mut whole = false;
    let mut current = Some(head);
    let mut head_segment = true;

    while let Some((cid, segment)) = current.take() {
        let stored = store_segment(store, channel, &segment)?;
        if head_segment {
            took.learned.extend(stored);
            head_segment = false;
        } else {
            took.backfilled.extend(stored);
            took.segments += 1;
        }
        walked.push(cid);
        backfill.remove(&cid);
        store
            .mark_segment_link(&cid, segment.sequence, segment.previous)
            .map_err(|e| e.to_string())?;

        let (Some(previous), Some(earlier)) = (segment.previous, segment.sequence.checked_sub(1))
        else {
            // The start of this author's history in this channel.
            whole = true;
            continue;
        };
        // Everything behind a marked segment is already held, by the invariant
        // the marking below maintains.
        if store.chain_whole(&previous) {
            whole = true;
            continue;
        }
        // A hop already read: taken from the stored link rather than fetched
        // again. This is the path a re-walk takes, and it is why a chain that
        // ends at a segment this node may never open — the steady state once
        // retention is active — costs file reads per tick instead of decrypting
        // and re-verifying every signature behind it.
        if let Some((sequence, before)) = store.segment_link(&previous) {
            current = Some((previous, Segment::new(*channel, *author, sequence, before)));
            continue;
        }
        let previous_id = kols_core::author_segment_pointer(channel, author, earlier);
        let Some((_, dek)) = resolve(store, node, keys, &previous_id)? else {
            continue;
        };
        match fetch_segment(node, previous, &dek) {
            // Already in the chunk store, so the walk continues for free.
            Ok(older) => current = Some((previous, older)),
            // Not held yet. Queued rather than waited on, and the walk stops
            // here: this is the deepest point reached, so nothing below it
            // exists to keep walking towards.
            Err(_) => {
                backfill.insert(previous);
            }
        }
    }

    // **Marked only once the chain behind it is whole, and that is the whole
    // subtlety here.** A mark that meant merely "this segment is stored" read
    // correctly and behaved wrongly: the walk stops at the first marked segment,
    // so marking one whose own ancestors were still missing walled off
    // everything behind it permanently. A reader would take exactly one hop of
    // history and then quietly stop, for good — which is what this did before,
    // and it looked like success because a hop's worth of backfill was reported.
    //
    // Deferring costs a re-walk of the held part of the chain each tick until it
    // completes. That re-walk stores nothing (records are content-addressed and
    // already present) and ends the moment the last hop lands.
    if whole {
        for cid in walked {
            store.mark_chain_whole(&cid).map_err(|e| e.to_string())?;
        }
    }
    Ok(took)
}

fn store_segment(
    store: &Store,
    channel: &ChannelId,
    segment: &Segment,
) -> Result<Vec<Record>, String> {
    let mut learned = Vec::new();
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
            learned.push(record.clone());
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
    seal_bytes: usize,
    sink: &Sink,
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
        sink(&[Event::Adopted { entries: adopted }]);
    }

    // Republished unconditionally rather than only on change: appending to an
    // author log republishes the same object, and a chunk that has not changed
    // re-derives to the same CID, so this costs a re-announcement rather than a
    // re-upload. Kademlia provider records expire, so the re-announcement is
    // work worth doing anyway.
    if ready(store, node, identity, seal_bytes).is_ok() {
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
    seal_bytes: usize,
    sink: &Sink,
) -> Result<usize, String> {
    // **Checked before locking, and that order matters.** This runs on every
    // tick, while one-shot commands need the same lock to append at all — so
    // taking it unconditionally meant a `kols admit` could wait behind thirty
    // acquisitions a minute and time out, which is exactly what happened. The
    // lock guards *appends*; reading needs no permission.
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

    // Nobody to exclude is the overwhelmingly common case, and it costs nothing
    // beyond the read above.
    let departed: Vec<_> = named
        .into_iter()
        .filter(|who| !state.is_member(who))
        .filter(|who| node.is_keyed_member(who))
        .collect();
    if departed.is_empty() {
        return Ok(0);
    }

    // Only now, and for the same reason as answering a key request: this appends
    // a rotation parented on the node's head, so the store's head has to *be*
    // the node's head and nothing else may append in between.
    let _lock = store.lock().map_err(|e| e.to_string())?;
    adopt_local_changes(store, node, identity, seal_bytes, sink)?;

    let mut excluded = 0;
    for who in departed {
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

/// Subscribes to the live topic of every channel this node knows.
///
/// On demand in the sense §6.1 means it — a topic per channel this member
/// actually has, rather than one per channel in existence — though a real client
/// would narrow further to channels currently open or flagged, since each topic
/// carries its own mesh maintenance. Subscribing twice is a no-op, so this is
/// safe to call on every tick as the channel set changes.
fn subscribe_channels(store: &Store, node: &mut MemberNode) -> Result<(), String> {
    let Some(state) = replayable(store) else {
        return Ok(());
    };
    let (channels, _) = network::channels(store, &state).map_err(|e| e.to_string())?;
    for channel in channels.keys() {
        node.subscribe_live(&kols_core::gossip_topic(channel))
            .map_err(|err| format!("could not subscribe: {err}"))?;
    }
    Ok(())
}

/// Broadcasts this node's own records that have not gone out live yet.
///
/// **Failures are ignored on purpose.** The commonest one is having no peer
/// subscribed to the topic, which is the ordinary state of a quiet channel, and
/// §6.1 is explicit that nothing may depend on this path — the record reaches
/// everybody through the durable one regardless. Treating it as an error would
/// report a problem that does not exist.
///
/// Records are remembered as sent so that republishing a segment does not
/// rebroadcast its whole history every couple of seconds.
fn publish_unsent_live(
    store: &Store,
    node: &mut MemberNode,
    identity: &intranet_identity::PerNetworkIdentity,
    sent: &mut BTreeSet<kols_core::MessageId>,
    window_millis: i64,
) -> Result<(), String> {
    let mut broadcast = 0usize;
    let Some(state) = replayable(store) else {
        return Ok(());
    };
    let (Ok(epoch), Ok(rotation)) = (store.epoch_key(), store.rotation_ref()) else {
        return Ok(());
    };
    let (channels, _) = network::channels(store, &state).map_err(|e| e.to_string())?;
    let now = crate::chat::now_millis();

    for channel in channels.keys() {
        let topic = kols_core::gossip_topic(channel);
        for record in store
            .own_records(channel, &identity.id())
            .map_err(|e| e.to_string())?
        {
            if sent.contains(&record.id()) {
                continue;
            }
            // Retired rather than skipped: a record too old to broadcast is one
            // this node should stop reconsidering, so it goes into the same set
            // as one that went out. Otherwise every tick rescans the whole of
            // history to decide against it again.
            if now - record.hlc.wall_millis > window_millis {
                sent.insert(record.id());
                continue;
            }
            let payload = kols_core::LivePayload::seal(&record, &epoch, rotation);
            // Marked as sent only when it actually went out. Publishing fails
            // while no peer is subscribed to the topic, which is the ordinary
            // state of a channel nobody is watching *and* the state a moment
            // before somebody starts — so recording the attempt rather than the
            // send would mean a message posted just before a peer subscribed
            // never went live at all, which is the one case the path exists for.
            if node.publish_live(&topic, payload.encode()).is_ok() {
                sent.insert(record.id());
                broadcast += 1;
            }
        }
    }

    // Reported because a publish only succeeds once somebody is actually
    // subscribed, so this is the moment the live path became useful rather than
    // the moment a record was written. Without it there is no way to tell a
    // channel nobody is watching from one where delivery is broken.
    if broadcast > 0 {
        println!("broadcast {broadcast} record(s) live");
    }
    Ok(())
}

/// Opens a live payload and stores the record it carries, if it is admissible.
///
/// Returns whether anything new was learned. Everything here is a refusal the
/// transport could not make for us: it carries opaque bytes and validates
/// nothing (Core §5.1), so signature, membership and the author's right to post
/// are all checked here — the same three-part discipline an append-set entry
/// gets, and for the same reason.
///
/// A payload this node cannot open is not necessarily hostile: a member mid-
/// rotation may hold a key this one does not yet. It is refused either way,
/// because guessing is not available, and the record will arrive on the durable
/// path regardless.
fn admit_live(store: &Store, payload: &[u8]) -> Result<Option<Record>, String> {
    let live = kols_core::LivePayload::decode(payload)
        .map_err(|err| format!("not a live payload: {err}"))?;

    // Every key, because a sender mid-rotation may have sealed under an epoch
    // this node no longer treats as current.
    let keys = store.epoch_keys().map_err(|e| e.to_string())?;
    let record = keys
        .iter()
        .find_map(|(_, key)| live.open(key).ok())
        .ok_or("no held epoch key opens it")?;

    let Some(state) = replayable(store) else {
        return Ok(None);
    };
    let (channels, _) = network::channels(store, &state).map_err(|e| e.to_string())?;
    let Some(channel) = channels.get(&record.channel) else {
        return Err("a channel this node does not know".to_owned());
    };

    // The same gate a stored record passes. A live payload arrives with no
    // pointer vouching for it, so skipping this would mean admitting whatever a
    // peer chose to broadcast.
    let placement = Placement {
        channel: channel.id,
        category: channel.category,
    };
    let authority = StateAuthority { state: &state };
    if !authority.may_post(&record.author, &placement) {
        return Err(format!("{} may not post there", record.author.short()));
    }

    // `Some` only when the record was new here. A record that arrived live and
    // again inside a segment is one record, and saying so twice would make a
    // duplicate look like a second message.
    let stored = store
        .put_record(&record.channel, &record)
        .map_err(|e| e.to_string())?;
    Ok(stored.then_some(record))
}

/// Derives any epoch keys this node was absent for.
///
/// # Why a node cannot simply be given these
///
/// Every rotation is a governance entry carrying an MLS commit (Core §3.3), and
/// applying those commits in order is how a member derives the keys it missed.
/// So absence costs a member nothing *provided it catches up*: the log never
/// shrinks, so the commits are always there to replay.
///
/// Without this, a node that was offline across a rotation held only the keys it
/// produced or was handed directly. It could still read anything wrapped under
/// an epoch it already had — including appends to objects it already knew, since
/// an object keeps its DEK for life — so the gap stayed invisible until it met a
/// *new* object wrapped under an epoch it never derived, and then presented as
/// content that fetched perfectly and would not open.
///
/// Applying is idempotent by way of the keyring: a rotation already held is
/// skipped, and a commit that will not apply is skipped rather than fatal, since
/// it may belong to a branch this node cannot reach — which reconciliation
/// resolves by re-welcome rather than by replay.
fn catch_up_epochs(store: &Store, node: &mut MemberNode) -> Result<(), String> {
    let applied = node.apply_pending_rotations();
    if applied.is_empty() {
        return Ok(());
    }
    println!("caught up on {} epoch rotation(s)", applied.len());
    // Both, because applying a commit advances the group as well as the keyring,
    // and a restart that recovered one without the other would be worse than
    // recovering neither.
    persist_keyring(store, node)?;
    persist_group(store, node)?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::retires;
    use kols_core::Retention;

    const DAY: i64 = 86_400_000;

    #[test]
    fn nothing_ages_out_of_the_default_window() {
        // `design/01` §8's default. Text is cheap enough that keeping it is the
        // honest default, so a network that has chosen nothing forgets nothing.
        assert!(!retires(&Retention::Forever, 0, 10_000 * DAY));
    }

    #[test]
    fn a_segment_is_retired_once_its_newest_record_has_aged_out() {
        let now = 100 * DAY;
        assert!(!retires(&Retention::Days(30), now - 29 * DAY, now));
        assert!(!retires(&Retention::Days(30), now - 30 * DAY, now));
        assert!(retires(&Retention::Days(30), now - 31 * DAY, now));
    }

    #[test]
    fn a_segment_still_being_written_to_is_not_retired_for_starting_long_ago() {
        // The rule that keeps retention from eating the start of an active
        // conversation: what matters is the newest record, so a segment whose
        // first message is ancient stays while its last one is recent.
        let now = 1_000 * DAY;
        assert!(!retires(&Retention::Days(7), now - DAY, now));
    }

    #[test]
    fn a_clock_that_runs_backwards_does_not_retire_anything() {
        // A record dated in the future gives a negative age. Saturating at zero
        // keeps it maintained rather than wrapping into a huge age and retiring
        // content the moment somebody's clock is fast.
        let now = 10 * DAY;
        assert!(!retires(&Retention::Days(1), now + 5 * DAY, now));
    }
}

#[cfg(test)]
mod voided_tests {
    use super::report_of;
    use intranet_crypto::{Hash, Timestamp};
    use intranet_governance::VoidedEntry;
    use intranet_identity::{MasterSeed, NetworkId, PerNetworkIdentityId};
    use std::collections::BTreeSet;

    fn who(seed: u8) -> PerNetworkIdentityId {
        MasterSeed::from_entropy([seed; 32])
            .identity_for(&NetworkId::from_bytes([4u8; 32]))
            .expect("derives")
            .id()
    }

    fn voided(hash: u8, author: PerNetworkIdentityId, security_relevant: bool) -> VoidedEntry {
        VoidedEntry {
            hash: Hash::from_bytes([hash; 32]),
            author,
            timestamp: Timestamp::from_millis(0),
            kind: "MembershipChange",
            security_relevant,
        }
    }

    #[test]
    fn nothing_voided_says_nothing() {
        let mut seen = BTreeSet::new();
        assert!(report_of(&[], &who(1), &mut seen).is_none());
    }

    #[test]
    fn my_own_actions_are_separated_from_everybody_elses() {
        // Core §2.7.1 point 5 makes watching for *your own* the client's job,
        // because those are the ones this member can resubmit. The rest is
        // counted rather than dropped: it is the difference between "your action
        // lost" and "a partition healed and took a lot with it".
        let me = who(1);
        let mut seen = BTreeSet::new();
        let report = report_of(
            &[
                voided(1, me, true),
                voided(2, who(2), true),
                voided(3, who(3), false),
            ],
            &me,
            &mut seen,
        );

        let Some(kols_api::Event::GovernanceReorg { mine, others }) = report else {
            panic!("expected a report");
        };
        assert_eq!(mine.len(), 1);
        assert!(mine[0].security_relevant);
        assert_eq!(others, 2);
    }

    #[test]
    fn a_heal_is_announced_once_rather_than_on_every_sync_after_it() {
        // Repeating it would train somebody to ignore it, and this is the one
        // notice they get that a removed member is current again.
        let me = who(1);
        let mut seen = BTreeSet::new();
        let entries = [voided(1, me, true)];

        assert!(report_of(&entries, &me, &mut seen).is_some());
        assert!(report_of(&entries, &me, &mut seen).is_none());
    }

    #[test]
    fn a_second_fork_is_reported_even_after_a_first_one_was() {
        // The dedupe is per entry, not a latch. A node that healed once and then
        // went quiet about every heal after it would be worse than one that
        // never reported at all, because the silence would look like safety.
        let me = who(1);
        let mut seen = BTreeSet::new();
        assert!(report_of(&[voided(1, me, true)], &me, &mut seen).is_some());
        assert!(report_of(&[voided(2, me, true)], &me, &mut seen).is_some());
    }

    #[test]
    fn a_report_with_none_of_mine_is_still_a_report() {
        // Somebody else's voided revocation restores that member here too.
        let mut seen = BTreeSet::new();
        let report = report_of(&[voided(9, who(7), true)], &who(1), &mut seen);
        let Some(kols_api::Event::GovernanceReorg { mine, others }) = report else {
            panic!("expected a report");
        };
        assert!(mine.is_empty());
        assert_eq!(others, 1);
    }
}
