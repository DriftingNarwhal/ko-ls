//! A directory of networks — `design/09` §1.
//!
//! Creating a network moved out of `kols init` so that a window could do it
//! without a second copy of the genesis requirements, each of which is silent
//! when missed. These cover the moved path directly, because the interface that
//! calls it cannot be driven from a test.

mod common;

use kols_node::workspace::Workspace;

struct Dir(std::path::PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("kols-ws-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a workspace");
        Self(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn an_empty_workspace_holds_nothing_and_is_not_an_error() {
    // Where somebody starts. The window shows a picker rather than a failure.
    let dir = Dir::new("empty");
    assert!(Workspace::at(dir.0.clone()).list().is_empty());
}

#[test]
fn a_created_network_is_replayable_and_its_founder_is_a_member() {
    // The check `kols init` has always made and the reason creation is one path:
    // a genesis that replays but grants nothing looks like success until the
    // first post is refused by its own author's node.
    let dir = Dir::new("create");
    let workspace = Workspace::at(dir.0.clone());

    let store = workspace
        .create("the workshop", vec!["/ip4/198.51.100.7/tcp/4001".to_owned()])
        .expect("creates");
    let identity = store.identity().expect("an identity");
    let state = store.state().expect("replays");

    assert!(state.is_member(&identity.id()));
    assert_eq!(
        state.policy.bootstrap_relays,
        vec!["/ip4/198.51.100.7/tcp/4001".to_owned()]
    );
    // Everything a member needs, registered at genesis — the part that is silent
    // when missed.
    assert!(state.identity_holds(
        &identity.id(),
        &intranet_governance::Capability::extension("chat:post:*".to_owned())
    ));
}

#[test]
fn networks_are_listed_and_reachable_by_the_start_of_their_id() {
    let dir = Dir::new("list");
    let workspace = Workspace::at(dir.0.clone());
    workspace.create("first", Vec::new()).expect("creates");
    workspace.create("second", Vec::new()).expect("creates");

    let listed = workspace.list();
    assert_eq!(listed.len(), 2);
    // Stable order, so a list does not reshuffle between renders.
    assert_eq!(listed[0].label, "first");
    assert_eq!(listed[1].label, "second");

    let opened = workspace.open(&listed[0].id[..8]).expect("opens");
    assert_eq!(opened.network().as_bytes()[..], hex(&listed[0].id)[..]);
}

#[test]
fn an_ambiguous_or_unknown_prefix_is_refused_rather_than_guessed() {
    let dir = Dir::new("ambiguous");
    let workspace = Workspace::at(dir.0.clone());
    workspace.create("one", Vec::new()).expect("creates");
    workspace.create("two", Vec::new()).expect("creates");

    assert!(workspace.open("ffffffffffff").is_err(), "unknown prefix");
    // Every id starts with the empty string, so this is the ambiguous case
    // without needing two ids that happen to share a prefix.
    let complaint = match workspace.open("") {
        Err(why) => why,
        Ok(_) => panic!("an ambiguous prefix opened a network"),
    };
    assert!(complaint.contains("give more"), "{complaint}");
}

#[test]
fn a_home_that_is_itself_one_store_still_reads_as_one_network() {
    // What `kols --home` has always meant, and still does: a terminal is told
    // which network to work with, a window is given the directory holding many.
    // Both shapes have to be legible or the two front ends disagree about what a
    // path means.
    let dir = Dir::new("legacy");
    let inner = dir.0.join("single");
    let workspace = Workspace::at(dir.0.clone());
    workspace
        .create_at(inner.clone(), "alone", Vec::new())
        .expect("creates");

    let direct = Workspace::at(inner);
    let listed = direct.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].label, "alone");

    // And creating inside it is refused rather than nesting a network in a
    // network, which nothing would ever find.
    assert!(direct.create("nested", Vec::new()).is_err());
}

fn hex(text: &str) -> Vec<u8> {
    intranet_crypto::from_hex(text).expect("hex")
}

#[test]
fn a_created_network_lives_where_its_id_says_it_does() {
    // **The directory name and the network inside it must agree**, because
    // `path_for` is how a store is found by id — and finding it is what stops a
    // second join to a network you already hold from making a second identity in
    // it, which would look like two memberships and be two strangers.
    //
    // `create` used to mint an id to name the directory and then hand the path to
    // `create_at`, which minted another for the network. The name was therefore
    // about a network that did not exist. Nothing noticed until something looked
    // a store up by id.
    let dir = Dir::new("create-path");
    let workspace = Workspace::at(dir.0.clone());

    let store = workspace.create("alpha", Vec::new()).expect("creates");
    let network = *store.network();
    let root = store.root().to_path_buf();
    drop(store);

    assert_eq!(
        workspace.path_for(&network),
        root,
        "a created network must be found at the path its own id derives"
    );
}

/// The name a founder types has to travel, and for a long time it did not.
///
/// It was written to the local label and nowhere else, so the network had a name
/// on exactly one machine: its creator saw what they typed, and everybody they
/// invited saw an id in the picker and "unnamed network" over the channel list.
/// Spec 07 §1.7 has had the policy key throughout and the settings sheet has
/// written it throughout — nothing carried it at the one moment somebody is
/// unambiguously naming the thing.
#[test]
fn the_name_a_founder_types_is_the_networks_name_and_not_just_this_machines() {
    let dir = Dir::new("create-named");
    let workspace = Workspace::at(dir.0.clone());

    let store = workspace.create("the workshop", Vec::new()).expect("creates");
    let state = store.state().expect("replays");

    assert_eq!(
        kols_core::ChatPolicy::of(&state.policy).network_name(),
        Some("the workshop"),
        "the name belongs in replayed policy, where every joiner sees it"
    );
    assert_eq!(store.label().as_deref(), Some("the workshop"));
}

/// An unnamed network *has* no name, which is a different claim from being
/// called nothing — and the second is the one a joiner would replay forever.
#[test]
fn a_network_created_without_a_name_declares_none() {
    let dir = Dir::new("create-unnamed");
    let workspace = Workspace::at(dir.0.clone());

    let store = workspace.create("   ", Vec::new()).expect("creates");
    let state = store.state().expect("replays");

    assert_eq!(
        kols_core::ChatPolicy::of(&state.policy).network_name(),
        None,
        "blank is absent, not an empty name"
    );
}

#[test]
fn forgetting_removes_the_store_and_leaves_the_others() {
    let dir = Dir::new("forget");
    let workspace = Workspace::at(dir.0.clone());

    let alpha = workspace.create("alpha", Vec::new()).expect("creates");
    let kept = *alpha.network();
    drop(alpha);
    let beta = workspace.create("beta", Vec::new()).expect("creates");
    let going = *beta.network();
    let path = beta.root().to_path_buf();
    drop(beta);

    workspace.forget(&going).expect("forgets");

    assert!(!path.exists(), "the store should be gone from disk");
    let left: Vec<String> = workspace.list().into_iter().map(|k| k.label).collect();
    assert_eq!(left, vec!["alpha".to_owned()], "only the forgotten one goes");
    // And the one that stayed is still findable by its id, which is the property
    // the path fix above exists for.
    assert!(workspace.path_for(&kept).exists());
}

/// The claim the confirmation dialog makes, and the one nothing was holding.
///
/// Forgetting is destructive in a way that has no undo and no recovery service,
/// and what makes it destructive is the seed: an identity is derived from it and
/// from the network id, and the id is public. So the seed is the whole of the
/// difference between coming back as yourself and coming back as a stranger.
///
/// This asserts the mechanism rather than observing a coincidence of randomness.
/// The second half is what makes the first half mean something: the same entropy
/// in the same network *does* reproduce the identity, so the identity is a pure
/// function of the two — which is precisely why deleting one of them is
/// irreversible rather than merely inconvenient.
#[test]
fn forgetting_destroys_the_seed_so_a_later_join_is_a_stranger() {
    use kols_node::store::Store;

    let dir = Dir::new("forget-identity");
    let workspace = Workspace::at(dir.0.clone());

    let store = workspace.create("the workshop", Vec::new()).expect("creates");
    let network = *store.network();
    let path = store.root().to_path_buf();
    let before = store.identity().expect("an identity").id();
    assert!(path.join("seed").is_file(), "the seed is what forget destroys");
    drop(store);

    workspace.forget(&network).expect("forgets");
    assert!(!path.exists(), "the store, and the seed inside it, are gone");

    // Joining the same network again, on this machine, with everything public
    // about it still known: the id names the network and nothing else is needed
    // to arrive at it.
    let again = Store::create(path.clone(), network, kols_node::random_32().expect("entropy"))
        .expect("a second store for the same network");
    assert_ne!(
        before,
        again.identity().expect("an identity").id(),
        "a join after forgetting must arrive as somebody the log has never seen"
    );
    drop(again);

    // And the converse, which is what makes the assertion above a property
    // rather than luck: the seed is the identity, so keeping it keeps it.
    let entropy = kols_node::random_32().expect("entropy");
    let one = Dir::new("forget-identity-a");
    let two = Dir::new("forget-identity-b");
    let a = Store::create(one.0.join("store"), network, entropy).expect("creates");
    let b = Store::create(two.0.join("store"), network, entropy).expect("creates");
    assert_eq!(
        a.identity().expect("an identity").id(),
        b.identity().expect("an identity").id(),
        "the same seed in the same network is the same member"
    );
}

/// What a process dying mid-write may not be allowed to do.
///
/// Closing the window ends the process without a shutdown protocol, and
/// `fs::write` truncates before it fills — so the question is what a reader
/// finds if those two steps are interrupted. For `entries/` the answer decides
/// whether the network opens again at all: `Store::log` refuses a file it cannot
/// decode, and refuses the whole log rather than the one entry, correctly.
///
/// This asserts the property that makes the interruption survivable rather than
/// trying to interrupt one: nothing is ever written in place, so the destination
/// holds either the old bytes or the new and never a mixture. The proxy is the
/// temporary — if a partial write can exist at all, it exists somewhere the
/// readers do not look.
#[test]
fn a_write_that_is_interrupted_cannot_leave_a_log_that_will_not_open() {
    use kols_node::store::Store;

    let dir = Dir::new("atomic");
    let workspace = Workspace::at(dir.0.clone());
    let store = workspace.create("the workshop", Vec::new()).expect("creates");
    let root = store.root().to_path_buf();

    // Everything a reader scans holds only what a reader expects. A temporary
    // beside an entry would be decoded as one; beside a chunk it would be served
    // as one; and `append_entry` numbers the next entry by counting the
    // directory, so it would take an index twice.
    store.set_label("renamed").expect("writes");
    for scanned in ["entries", "records", "chunks", "pointers", "segments"] {
        let Ok(entries) = std::fs::read_dir(root.join(scanned)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            assert!(
                !name.contains("tmp"),
                "{scanned}/{name} is scratch in a directory something reads"
            );
        }
    }

    // And the store still opens, which is the whole point of the exercise.
    assert_eq!(
        Store::open(root.clone()).expect("opens").label().as_deref(),
        Some("renamed")
    );

    // A temporary left by a process that died between the write and the rename
    // is swept when somebody next takes the claim — which is the one moment a
    // process knows no other node is writing here.
    std::fs::create_dir_all(root.join("tmp")).expect("scratch");
    std::fs::write(root.join("tmp").join("999.0.tmp"), b"half").expect("writes");
    let store = Store::open(root.clone()).expect("opens");
    let claim = store.hold_node().expect("claims");
    assert_eq!(
        std::fs::read_dir(root.join("tmp")).expect("scratch").count(),
        0,
        "taking the claim sweeps what an interrupted write left"
    );
    drop(claim);
}

#[test]
fn forgetting_a_network_nobody_holds_is_refused() {
    let dir = Dir::new("forget-missing");
    let workspace = Workspace::at(dir.0.clone());
    let absent = intranet_identity::NetworkId::from_bytes([3u8; 32]);
    assert!(workspace.forget(&absent).is_err());
}

/// O9 — a claim taken over while its holder slept.
///
/// # The failure this is about
///
/// Only one process may run a node for a network, because the MLS group is live
/// state: two would each advance it without seeing the other, and whichever
/// saved last would decide the network's key, with no symptom at the moment it
/// happens. A claim guards that, and it expires on wall-clock so a window
/// killed by the window manager does not leave the store locked forever.
///
/// Wall-clock cannot tell a dead process from a suspended one. A laptop asleep
/// past the staleness window looks exactly like a crash, so its claim is
/// legitimately taken over — and the sleeper used to wake up still believing it
/// held one. Two nodes, one network, and nothing to see.
#[test]
fn a_claim_taken_over_while_its_holder_slept_is_reported_lost() {
    use kols_node::store::{Beat, Store};

    let dir = Dir::new("claim-takeover");
    let root = dir.0.join("net");
    let network = intranet_identity::NetworkId::from_bytes([9u8; 32]);
    let store = Store::create(root.clone(), network, [4u8; 32]).expect("creates");

    let sleeper = store.hold_node().expect("claims");
    assert_eq!(sleeper.beat(), Beat::Held, "its own claim, before anything else");

    // The laptop sleeps. Reproduced by ageing the heartbeat rather than by
    // waiting out the window, because what is under test is the ownership
    // comparison and not the duration — the same reason the harness spec drives
    // bounded finality from a virtual clock.
    let beat = root.join("serving").join("heartbeat");
    std::fs::write(&beat, (0i64).to_string()).expect("ages the heartbeat");

    // Another process finds a stale claim and takes it, which is correct.
    let taker = Store::open(root.clone()).expect("opens");
    let taker = taker.hold_node().expect("takes over a stale claim");
    assert_eq!(taker.beat(), Beat::Held);

    // The sleeper wakes. It must find out rather than carry on.
    assert_eq!(
        sleeper.beat(),
        Beat::Lost,
        "a holder whose claim was taken over must be told, or two nodes advance \
         one MLS group and neither knows"
    );

    // And on its way out it must not take the successor's claim with it. Without
    // the ownership check in `Drop` this is where the fix would have introduced a
    // worse bug than the one it closed: a store that looks unclaimed while a node
    // is actively running against it.
    drop(sleeper);
    assert_eq!(
        taker.beat(),
        Beat::Held,
        "the loser's drop must leave the winner's claim alone"
    );
    assert!(beat.exists(), "the successor's heartbeat survives the loser's drop");
}

/// What this machine contributes to one network — Core §4.3, and O1's `SetContribution`.
///
/// # Why unset and a zeroed offer are asserted apart
///
/// They behave differently and only one of them is a decision. Nothing set means
/// the shipped defaults are in force and would follow revised ones; zeros are a
/// member having said *contribute nothing*. A reader that collapsed them would
/// restore a default over somebody who had deliberately opted out — the same
/// failure spec 07 §2.8's sentinel rule exists to prevent for retention.
#[test]
fn what_this_machine_contributes_survives_a_reopen_and_zero_is_not_unset() {
    use kols_node::store::{Contribution, Store};

    let dir = Dir::new("contribution");
    let root = dir.0.join("net");
    let network = intranet_identity::NetworkId::from_bytes([7u8; 32]);
    let store = Store::create(root.clone(), network, [5u8; 32]).expect("creates");

    assert_eq!(
        store.contribution(),
        None,
        "a fresh store has been asked nothing, which is not the same as being offered nothing"
    );

    let chosen = Contribution {
        storage_offered: 64 * 1024 * 1024,
        upload_offered: 250_000,
        download_offered: 4_000_000,
        relay_willing: true,
    };
    store.set_contribution(&chosen).expect("writes");
    assert_eq!(
        Store::open(root.clone()).expect("reopens").contribution(),
        Some(chosen),
        "the offer is durable — a setting that did not survive a restart would be a preference"
    );

    let nothing = Contribution {
        storage_offered: 0,
        upload_offered: 0,
        download_offered: 0,
        relay_willing: false,
    };
    store.set_contribution(&nothing).expect("writes zero");
    assert_eq!(
        Store::open(root.clone()).expect("reopens").contribution(),
        Some(nothing),
        "contributing nothing is a decision and must read back as one, never as unset"
    );

    // A file that does not parse whole reads as unset rather than as partly set,
    // for the same reason: a corrupt value must not be read as the strongest
    // opinion a member could have expressed.
    std::fs::write(root.join("contribution"), b"64\nnot a number\n").expect("corrupts it");
    assert_eq!(
        Store::open(root.clone()).expect("reopens").contribution(),
        None,
        "a corrupt offer falls back to the defaults rather than to a partial one"
    );

    // Reachability decides only whether relaying is worth *offering*. Absent
    // means not confirmed, which is weaker than not reachable.
    let store = Store::open(root).expect("reopens");
    assert_eq!(store.reachable(), None);
    store.set_reachable("/ip4/203.0.113.7/tcp/4001").expect("writes");
    assert_eq!(store.reachable().as_deref(), Some("/ip4/203.0.113.7/tcp/4001"));
}

/// Replica duty — Storage §3.3, and what makes a storage offer mean anything.
///
/// # What is under test
///
/// That this node holds objects *for the network* rather than only what it
/// fetched to read, that the ranking is the protocol's and not ours, and that
/// the three cases which decide whether an offer is honest all behave:
/// contributing nothing takes no duty, a network too small to meet its own
/// replication factor has everybody hold everything, and being ranked out
/// releases the duty without touching the bytes.
///
/// Built on a real genesis and real membership entries rather than a hand-made
/// state, because the layering is part of the claim: an advertisement is only
/// accepted from a current member, so placement depends on governance having
/// converged first (Core §4.5).
#[test]
fn a_node_takes_duty_for_what_placement_ranks_it_for() {
    use intranet_governance::{EntryBody, GroupId, LogEntry, MembershipAction};
    use intranet_ledger::{
        BandwidthCap, CapabilityAdvertisement, CapabilityLedger, ComputeClass, WeightField,
        placement,
    };

    let dir = Dir::new("duty");
    let workspace = Workspace::at(dir.0.clone());
    let store = workspace.create("the workshop", Vec::new()).expect("creates");
    let founder = store.identity().expect("identity");
    let me = founder.id();
    let network = *store.network();

    // Four more members, admitted for real so their advertisements are accepted.
    let others: Vec<_> = (20u8..24)
        .map(|n| {
            intranet_identity::MasterSeed::from_entropy([n; 32])
                .identity_for(&network)
                .expect("identity")
        })
        .collect();
    for member in &others {
        let log = store.log().expect("log");
        let parent = log.canonical_chain().last().copied();
        store
            .append_entry(&LogEntry::create(
                &founder,
                parent,
                intranet_crypto::Timestamp::from_millis(100),
                EntryBody::MembershipChange {
                    group: GroupId::everyone(),
                    identity: member.id(),
                    action: MembershipAction::Add { via_invite: None },
                },
            ))
            .expect("admits");
    }
    let state = store.state().expect("replays");

    // A chain of three: two sealed, one head. Only the sealed pair is duty —
    // a head's id moves under its author's next message.
    //
    // Stored as **real objects** rather than as bare ids, because duty now
    // records what an object weighs and an object whose manifest is not held has
    // no weight this node can state. That is not test scaffolding: a segment is
    // only ever absorbed after its manifest and chunks decoded, so a link
    // without a manifest is a state a running node never reaches.
    let dek = intranet_storage::Dek::generate().expect("a data key");
    let store_object = |plaintext: &[u8]| {
        let encoded = intranet_storage::encode(
            plaintext,
            &dek,
            intranet_storage::ChunkSpec::from_target(64 * 1024),
        );
        let cid = encoded.manifest_cid();
        store
            .put_chunk(&cid, &encoded.manifest.canonical_bytes())
            .expect("manifest");
        for (chunk, bytes) in &encoded.chunks {
            store.put_chunk(chunk, bytes).expect("chunk");
        }
        cid
    };
    let sealed_first = store_object(b"the first sealed segment");
    let sealed_second = store_object(b"the second sealed segment");
    let head = store_object(b"the open head segment");
    store.mark_segment_link(&sealed_first, 0, None).expect("link");
    store
        .mark_segment_link(&sealed_second, 1, Some(sealed_first))
        .expect("link");
    store
        .mark_segment_link(&head, 2, Some(sealed_second))
        .expect("link");

    // The timestamp is a parameter because the ledger ignores an advertisement
    // that is not strictly newer than the one it holds (§4.5: gossip reordering
    // is ordinary rather than a fault). Raising an offer therefore has to be a
    // later advertisement, which is also what a real node does.
    let advertise = |identity: &intranet_identity::PerNetworkIdentity, storage: u64, at: i64| {
        CapabilityAdvertisement::create(
            identity,
            storage,
            BandwidthCap {
                up_bytes_per_sec: 1_000_000,
                down_bytes_per_sec: 8_000_000,
                active_window: None,
            },
            false,
            false,
            ComputeClass::Modest,
            intranet_crypto::Timestamp::from_millis(at),
        )
    };
    let mut ledger = CapabilityLedger::new(network);
    // **A census nobody has answered**, which is what every assertion below is
    // about: placement alone. Repair reads the same store and acts only on a
    // fresh count, so an empty one leaves this measuring the ranking and nothing
    // else — and if that ever stops being true, these numbers move.
    let quiet = kols_node::replica::Census::default();
    // Room enough that nothing here is refused; the ceiling has its own test.
    let roomy = kols_node::replica::Budget {
        offered: u64::MAX,
        installation: u64::MAX,
        installation_used: 0,
    };

    // **Contributing nothing takes no duty**, and is not a degraded membership.
    // `placement::rank` excludes a zero-weight node entirely rather than ranking
    // it last, so this is the protocol's behaviour rather than a check here.
    ledger
        .insert(advertise(&founder, 0, 1), &state)
        .expect("advertises");
    let duty = kols_node::replica::evaluate(&store, &ledger, &me, 3, roomy, &quiet, 0).expect("evaluates");
    assert_eq!(duty.considered, 2, "only the sealed pair is considered");
    assert_eq!(duty.mine, 0, "a node offering nothing is never conscripted");
    assert!(!store.has_duty(&sealed_first));

    // **A network too small to meet its own factor has everybody hold
    // everything**, and it falls out of `select` returning fewer than asked for
    // rather than out of a rule kept in step with it (Storage §3.2).
    ledger
        .insert(advertise(&founder, 8 << 30, 2), &state)
        .expect("advertises");
    let duty = kols_node::replica::evaluate(&store, &ledger, &me, 3, roomy, &quiet, 0).expect("evaluates");
    assert_eq!(duty.mine, 2, "the only contributor holds everything");
    assert_eq!(
        duty.under_replicated, 2,
        "one holder against a factor of three is degraded, and must be visible"
    );
    assert!(store.has_duty(&sealed_first) && store.has_duty(&sealed_second));
    assert!(!store.has_duty(&head), "a head segment is never duty");

    // **What is given and what is used are two numbers.** Duty counts the sealed
    // pair and nothing else; the store holds the head as well, which is this
    // member's own and is not a contribution. Reporting one figure would either
    // overstate what somebody gives or understate what the application costs.
    let expected: u64 = [sealed_first, sealed_second]
        .iter()
        .map(|cid| store.object_bytes(cid).expect("held"))
        .sum();
    assert_eq!(duty.mine_bytes, expected, "the tier is summed from its own marks");
    assert_eq!(store.duty_bytes(), expected);
    assert!(
        store.stored_bytes() > expected,
        "the disk holds the head too, which is this member's own reading and not given \
         to anybody: {} against {expected}",
        store.stored_bytes()
    );

    // **Ranked against better-resourced members**, with a factor of one so that
    // exactly one of the five wins each object. Whichever way it falls, the
    // marks must agree with the ranking rather than with what was there before.
    for member in &others {
        ledger
            .insert(advertise(member, 64 << 30, 3), &state)
            .expect("advertises");
    }
    let duty = kols_node::replica::evaluate(&store, &ledger, &me, 1, roomy, &quiet, 0).expect("evaluates");
    assert_eq!(duty.under_replicated, 0, "five contributors meet a factor of one");
    let candidates: Vec<_> = ledger.entries().cloned().collect();
    let mut released = 0;
    for cid in [sealed_first, sealed_second] {
        let ranked = placement::select(
            cid.hash().as_bytes(),
            &candidates,
            WeightField::StorageOffered,
            1,
        );
        assert_eq!(
            store.has_duty(&cid),
            ranked.contains(&me),
            "the mark must follow the protocol's ranking, not this node's history"
        );
        if !ranked.contains(&me) {
            released += 1;
        }
    }
    assert!(
        released > 0,
        "four members offering eight times as much should win at least one of two objects; \
         if this ever fails, the ranking has stopped being weighted"
    );

    // And releasing duty leaves the record of the segment alone: the bytes are
    // held for whatever other reason wanted them, which is what stops a lowered
    // contribution taking away somebody's own reading.
    assert!(
        store.segment_link(&sealed_first).is_some(),
        "withdrawing a reason must not remove the object"
    );
}

/// The two ceilings, and the arithmetic that must not hand out room twice.
///
/// # Why saturating subtraction is the whole of this
///
/// A ceiling can be *lowered* below what is already held — a member changing
/// their mind is the ordinary case, not an exotic one — and that is exactly when
/// a plain subtraction underflows into an enormous allowance and a node starts
/// taking on work because it believes it has room for four exabytes. The test is
/// short because the failure is a single operator.
#[test]
fn a_budget_past_either_ceiling_offers_no_room_rather_than_underflowing() {
    use kols_node::replica::Budget;

    let gib = 1024 * 1024 * 1024;
    let budget = Budget {
        offered: 4 * gib,
        installation: 8 * gib,
        installation_used: 2 * gib,
    };
    assert_eq!(budget.remaining(gib), 3 * gib, "the offer binds while it is the smaller");

    // The installation ceiling binds first once the disk is nearly full, even
    // though this network has offered plenty. That is the point of having two:
    // a member reading heavily in one network leaves no room for duty anywhere,
    // and the contribution is what gives way rather than their own use.
    let pressed = Budget {
        installation_used: 7 * gib,
        ..budget
    };
    assert_eq!(pressed.remaining(0), gib, "the disk binds before the offer does");

    // Already past the offer, because it was lowered under what is held.
    assert_eq!(budget.remaining(9 * gib), 0, "no room, and no underflow");
    // Already past the installation ceiling, for the same reason.
    let over = Budget {
        installation_used: 99 * gib,
        ..budget
    };
    assert_eq!(over.remaining(0), 0, "no room, and no underflow");
}

/// A node at its offer declines new duty rather than exceeding what it promised.
#[test]
fn duty_stops_at_the_offer_and_says_it_refused() {
    use intranet_ledger::{BandwidthCap, CapabilityAdvertisement, CapabilityLedger, ComputeClass};
    use kols_node::replica::Budget;

    let dir = Dir::new("duty-ceiling");
    let workspace = Workspace::at(dir.0.clone());
    let store = workspace.create("the workshop", Vec::new()).expect("creates");
    let founder = store.identity().expect("identity");
    let me = founder.id();
    let network = *store.network();
    let state = store.state().expect("replays");

    let dek = intranet_storage::Dek::generate().expect("a data key");
    let store_object = |plaintext: &[u8]| {
        let encoded =
            intranet_storage::encode(plaintext, &dek, intranet_storage::ChunkSpec::from_target(64 * 1024));
        let cid = encoded.manifest_cid();
        store.put_chunk(&cid, &encoded.manifest.canonical_bytes()).expect("manifest");
        for (chunk, bytes) in &encoded.chunks {
            store.put_chunk(chunk, bytes).expect("chunk");
        }
        cid
    };
    let first = store_object(b"the first sealed segment");
    let second = store_object(b"the second sealed segment");
    let head = store_object(b"the open head");
    store.mark_segment_link(&first, 0, None).expect("link");
    store.mark_segment_link(&second, 1, Some(first)).expect("link");
    store.mark_segment_link(&head, 2, Some(second)).expect("link");

    let mut ledger = CapabilityLedger::new(network);
    ledger
        .insert(
            CapabilityAdvertisement::create(
                &founder,
                8 << 30,
                BandwidthCap {
                    up_bytes_per_sec: 1_000_000,
                    down_bytes_per_sec: 8_000_000,
                    active_window: None,
                },
                false,
                false,
                ComputeClass::Modest,
                intranet_crypto::Timestamp::from_millis(1),
            ),
            &state,
        )
        .expect("advertises");

    let quiet = kols_node::replica::Census::default();
    // Room for one of the two sealed objects and not the other.
    let one = store.object_bytes(&first).expect("held");
    let budget = Budget {
        offered: one,
        installation: u64::MAX,
        installation_used: 0,
    };
    let duty = kols_node::replica::evaluate(&store, &ledger, &me, 3, budget, &quiet, 0).expect("evaluates");
    assert_eq!(duty.mine, 1, "one object fits");
    assert_eq!(duty.refused, 1, "and declining the other is reported, not silent");
    assert!(
        duty.mine_bytes <= one,
        "a node must never hold more than it offered: {} against {one}",
        duty.mine_bytes
    );

    // The disk ceiling binds the same way even with a generous offer — a member
    // reading heavily leaves no room for duty, and the contribution gives way.
    let full = Budget {
        offered: u64::MAX,
        installation: 1,
        installation_used: 1,
    };
    let store2 = workspace.create("another", Vec::new()).expect("creates");
    drop(store2);
    let duty = kols_node::replica::evaluate(&store, &ledger, &me, 3, full, &quiet, 0).expect("evaluates");
    assert_eq!(duty.refused, 1, "no room on the disk means no new duty");
}

/// The census: asking who else holds something, and waiting for the answer.
///
/// # The three states this has to keep apart
///
/// *Never asked*, *asked and waiting*, and *told that nobody holds it* look the
/// same to a caller that only stores a number. Only the third is a reason to
/// act, and reading either of the first two as it is how a last copy gets
/// dropped because the DHT had not answered yet.
#[test]
fn a_census_tells_never_asked_from_told_nobody_has_it() {
    use kols_node::replica::{ANSWER_FRESH_MILLIS, ASK_TIMEOUT_MILLIS, Census};
    use intranet_storage::Cid;

    let object = Cid::of(b"an object");
    let me = intranet_identity::MasterSeed::from_entropy([1u8; 32])
        .identity_for(&intranet_identity::NetworkId::from_bytes([2u8; 32]))
        .expect("identity")
        .id();
    let other = intranet_identity::MasterSeed::from_entropy([9u8; 32])
        .identity_for(&intranet_identity::NetworkId::from_bytes([2u8; 32]))
        .expect("identity")
        .id();

    let mut census = Census::default();
    assert!(census.should_ask(&object, 0), "nothing known, so worth asking");
    assert_eq!(
        census.others_holding(&object, 0),
        None,
        "never asked is not zero, and a caller must not be able to read it as zero"
    );

    // Asked and waiting: not worth asking again, and still no answer.
    census.asked(object, 0);
    assert!(!census.should_ask(&object, 0));
    assert_eq!(census.others_holding(&object, 0), None);

    // An unanswered question times out rather than being waited on forever — a
    // query that resolves to nothing is indistinguishable from one in flight.
    assert!(
        census.should_ask(&object, ASK_TIMEOUT_MILLIS),
        "a question nobody answered must be asked again"
    );

    // **This node is excluded from its own count**, because the question is
    // *would this survive without me* and including it answers a different one.
    census.heard(object, &[me, other], &me, 1_000);
    assert_eq!(
        census.others_holding(&object, 1_000),
        Some(1),
        "two providers, one of them this node, is one other holder"
    );

    // Told that nobody else holds it: a real answer, and the opposite of not
    // having asked, though both would be `None` to a lazier reader.
    census.heard(object, &[me], &me, 2_000);
    assert_eq!(census.others_holding(&object, 2_000), Some(0));

    // An answer goes stale rather than standing forever: provider records
    // outlive the node that stopped holding the bytes, so an old count is a
    // claim about a network that has moved on.
    assert_eq!(
        census.others_holding(&object, 2_000 + ANSWER_FRESH_MILLIS),
        None,
        "a stale answer reads as unknown, never as its last value"
    );
    assert!(census.should_ask(&object, 2_000 + ANSWER_FRESH_MILLIS));

    census.forget(&object);
    assert_eq!(census.others_holding(&object, 2_000), None);
    assert!(census.should_ask(&object, 2_000));
}

/// Shedding cached copies to stay under the ceiling.
///
/// # The two things this must never do
///
/// Drop a **duty** object, which is a promise to the network given up under a
/// different rule with a round trip in it; and drop a member's **records**,
/// which are what rendering reads and are their own history rather than
/// something held for anybody else. Both are asserted, because either would
/// turn a disk ceiling into something that takes away what a member came for.
#[test]
fn shedding_gives_back_cached_copies_and_never_duty_or_records() {
    let dir = Dir::new("shed");
    let workspace = Workspace::at(dir.0.clone());
    let store = workspace.create("the workshop", Vec::new()).expect("creates");
    let channel = kols_core::ChannelId::from_bytes([5u8; 32]);

    let dek = intranet_storage::Dek::generate().expect("a data key");
    let store_object = |plaintext: &[u8]| {
        let encoded = intranet_storage::encode(
            plaintext,
            &dek,
            intranet_storage::ChunkSpec::from_target(64 * 1024),
        );
        let cid = encoded.manifest_cid();
        store.put_chunk(&cid, &encoded.manifest.canonical_bytes()).expect("manifest");
        for (chunk, bytes) in &encoded.chunks {
            store.put_chunk(chunk, bytes).expect("chunk");
        }
        cid
    };

    // Two sealed segments and a head. One sealed one is duty.
    let older = store_object(&[b'a'; 4096]);
    let newer = store_object(&[b'b'; 4096]);
    let head = store_object(&[b'c'; 4096]);
    store.mark_segment_link(&older, 0, None).expect("link");
    store.mark_segment_link(&newer, 1, Some(older)).expect("link");
    store.mark_segment_link(&head, 2, Some(newer)).expect("link");
    store.mark_chain_whole(&newer).expect("mark");
    store.take_duty(&newer, store.object_bytes(&newer).expect("held")).expect("duty");

    // A record, standing in for the member's own history.
    let founder = store.identity().expect("identity");
    let record = kols_core::Record::create(
        &founder,
        channel,
        kols_core::Hlc::new(10, 0),
        kols_core::RecordBody::Message {
            body: "mine to keep".to_owned(),
            reply_to: None,
            attachments: Vec::new(),
        },
    );
    store.put_record(&channel, &record).expect("record");

    let before = store.stored_bytes();
    let shed = kols_node::replica::shed_cache(&store, 1, 0).expect("sheds");

    assert_eq!(shed.objects, 1, "only the cached sealed segment is sheddable");
    assert!(shed.bytes > 0);
    assert!(
        store.object_bytes(&older).is_none(),
        "the oldest cached copy is the one given back"
    );
    assert!(
        store.object_bytes(&newer).is_some(),
        "duty is a promise to the network and is not shed under disk pressure"
    );
    assert!(
        store.object_bytes(&head).is_some(),
        "a head is republished on every append and shares chunks with its successors; \
         dropping one takes the current version's bytes with it"
    );
    assert_eq!(
        store.records(&channel).expect("records").len(),
        1,
        "records are what a member reads and are never what a storage ceiling takes"
    );
    assert!(store.stored_bytes() < before, "the disk actually got smaller");

    // **The link and the whole-chain mark both survive, and that is correct.**
    // Those describe the *chain*, which a walk hops along using links and never
    // needs the bytes for. Clearing them would send the walk back down a chain it
    // already knows, re-deriving hops it holds, to no purpose. What is gone is
    // the servable copy, which is a different question from whether this node
    // knows the shape of the history.
    assert!(store.segment_link(&older).is_some());
    assert!(store.chain_whole(&newer));
    assert!(
        !store.history_incomplete(),
        "shedding a servable copy must be invisible to the member: the links still \
         describe a whole chain and the records still render, so nothing tells them \
         their history shrank — because it did not"
    );

    // And the ids are handed back, so the caller can stop advertising them. A
    // node that drops bytes and keeps claiming them sends every peer that
    // believes it on a fetch that fails, which counts against the serving node.
    assert!(
        !shed.dropped.is_empty(),
        "shedding must say what went, or the announcements outlive the bytes"
    );
}

/// Duty eviction: evidence before dropping, and a window before losing.
///
/// # The three answers this has to keep apart
///
/// *Others hold it* is a reason to give a replica back. *Nobody holds it* is a
/// reason to hold it past the ceiling and say so. *Nobody has answered* is
/// neither, and treating it as the second is how a last copy gets dropped
/// because the network was slow to reply.
#[test]
fn duty_is_given_up_only_with_evidence_and_never_silently() {
    use kols_node::replica::{Census, GRACE_MILLIS, shed_duty};

    let dir = Dir::new("evict");
    let workspace = Workspace::at(dir.0.clone());
    let store = workspace.create("the workshop", Vec::new()).expect("creates");
    let me = store.identity().expect("identity").id();
    let network = *store.network();
    let peer = |n: u8| {
        intranet_identity::MasterSeed::from_entropy([n; 32])
            .identity_for(&network)
            .expect("identity")
            .id()
    };

    let dek = intranet_storage::Dek::generate().expect("a data key");
    let store_object = |plaintext: &[u8]| {
        let encoded = intranet_storage::encode(
            plaintext,
            &dek,
            intranet_storage::ChunkSpec::from_target(64 * 1024),
        );
        let cid = encoded.manifest_cid();
        store.put_chunk(&cid, &encoded.manifest.canonical_bytes()).expect("manifest");
        for (chunk, bytes) in &encoded.chunks {
            store.put_chunk(chunk, bytes).expect("chunk");
        }
        store.take_duty(&cid, store.object_bytes(&cid).expect("held")).expect("duty");
        cid
    };
    let held = store_object(&[b'a'; 4096]);
    let lonely = store_object(&[b'b'; 4096]);
    let unknown = store_object(&[b'c'; 4096]);
    store.mark_segment_link(&held, 0, None).expect("link");
    store.mark_segment_link(&lonely, 1, Some(held)).expect("link");
    store.mark_segment_link(&unknown, 2, Some(lonely)).expect("link");

    let mut census = Census::default();
    census.heard(held, &[me, peer(20), peer(21)], &me, 0);
    census.heard(lonely, &[me], &me, 0);
    // `unknown` is deliberately never heard about.

    let evicted = shed_duty(&store, &census, u64::MAX, 4, 0).expect("evicts");

    assert!(
        store.object_bytes(&held).is_none(),
        "two other holders is evidence, and giving the replica back is the point"
    );
    assert!(
        store.object_bytes(&lonely).is_some(),
        "the last known copy is held past the ceiling rather than destroyed"
    );
    assert_eq!(evicted.at_risk, vec![lonely], "and is reported as at risk");
    assert!(
        store.object_bytes(&unknown).is_some(),
        "nobody answered, which is not the same as nobody holding it"
    );
    assert_eq!(
        evicted.awaiting,
        vec![unknown],
        "an unanswered question must be reported as one so it gets asked, not acted on"
    );
    assert!(evicted.lost.is_empty(), "nothing is lost inside the window");

    // A network too small to produce evidence cannot be asked for it — and must
    // not therefore give everything up. With one other contributor the bar is
    // one, and with none nothing is evictable at all.
    let mut alone = Census::default();
    alone.heard(unknown, &[me, peer(20)], &me, 0);
    let evicted = shed_duty(&store, &alone, u64::MAX, 0, 0).expect("evicts");
    assert!(
        store.object_bytes(&unknown).is_some(),
        "with nobody else contributing there is no evidence to be had, and nothing may go"
    );
    assert!(evicted.objects == 0);

    // **A week-old count is not evidence, and the window does not override
    // that.** The answer recorded at zero has long since gone stale by the time
    // the window runs out, so the object reads as unknown and is held — a last
    // copy must not be destroyed on the strength of what the network said seven
    // days ago.
    let later = GRACE_MILLIS + 1;
    let evicted = shed_duty(&store, &census, u64::MAX, 4, later).expect("evicts");
    assert!(
        store.object_bytes(&lonely).is_some(),
        "an expired window plus a stale answer is still not a reason to destroy anything"
    );
    assert!(evicted.lost.is_empty());
    assert!(evicted.awaiting.contains(&lonely), "it is asked about again instead");

    // Asked again, and the answer is the same: still nobody. *Now* the window
    // has run on a current answer, and the copy goes — written down, because a
    // member who was told and did nothing chose, and one who was never told had
    // it chosen for them.
    census.heard(lonely, &[me], &me, later);
    let evicted = shed_duty(&store, &census, u64::MAX, 4, later).expect("evicts");
    assert_eq!(evicted.lost, vec![lonely], "the window runs out and the copy goes");
    assert!(store.object_bytes(&lonely).is_none());
    let dropped = store.dropped();
    assert_eq!(dropped.len(), 1, "what was given up is recorded, not merely done");
    assert_eq!(dropped[0].0, later);
}

/// Repair — Storage §3.4's other half, and what makes a generous node a backstop.
///
/// # What was missing, and why it is not the same as placement
///
/// Placement decides what a node holds when everything is well. It says nothing
/// about content whose holders have gone, so a member offering a great deal of
/// disk used to catch falling content only where the ranking had already put
/// them — durability by coincidence, one layer up from the coincidence duty was
/// built to remove.
///
/// §3.4 asks for the shortfall to be re-placed onto *the next nodes in the same
/// deterministic ranking*, so this is the same HRW list read further down rather
/// than a second policy. That is the property under test here: which nodes step
/// in is determined by the ranking and the size of the hole, not by who noticed.
///
/// # The three things that have to hold
///
/// - A standby the shortfall reaches **takes** the object on.
/// - A standby further down than the shortfall reaches **does not**, or every
///   node in the network piles onto one missing copy.
/// - An answer that the hole has closed **gives it back**, and no answer at all
///   does not — because *nobody told me* and *nobody holds it* are opposite
///   states, and only one of them is a reason to act.
#[test]
fn a_node_takes_on_under_replicated_content_it_is_the_next_ranked_for() {
    use intranet_governance::{EntryBody, GroupId, LogEntry, MembershipAction};
    use intranet_ledger::{
        BandwidthCap, CapabilityAdvertisement, CapabilityLedger, ComputeClass, WeightField,
        placement,
    };
    use kols_node::replica::{Budget, Census};

    let dir = Dir::new("repair");
    let workspace = Workspace::at(dir.0.clone());
    let store = workspace.create("the workshop", Vec::new()).expect("creates");
    let founder = store.identity().expect("identity");
    let me = founder.id();
    let network = *store.network();

    // Nine other members, so that this node is somewhere in the middle of the
    // ranking for most objects rather than at the top of it.
    let others: Vec<_> = (2u8..=10)
        .map(|seed| {
            intranet_identity::MasterSeed::from_entropy([seed; 32])
                .identity_for(&network)
                .expect("identity")
        })
        .collect();
    for member in &others {
        let log = store.log().expect("log");
        let parent = log.canonical_chain().last().copied();
        store
            .append_entry(&LogEntry::create(
                &founder,
                parent,
                intranet_crypto::Timestamp::from_millis(100),
                EntryBody::MembershipChange {
                    group: GroupId::everyone(),
                    identity: member.id(),
                    action: MembershipAction::Add { via_invite: None },
                },
            ))
            .expect("admits");
    }
    let state = store.state().expect("replays");

    let dek = intranet_storage::Dek::generate().expect("a data key");
    let store_object = |plaintext: &[u8]| {
        let encoded = intranet_storage::encode(
            plaintext,
            &dek,
            intranet_storage::ChunkSpec::from_target(64 * 1024),
        );
        let cid = encoded.manifest_cid();
        store
            .put_chunk(&cid, &encoded.manifest.canonical_bytes())
            .expect("manifest");
        for (chunk, bytes) in &encoded.chunks {
            store.put_chunk(chunk, bytes).expect("chunk");
        }
        cid
    };
    // Six sealed segments and a head, chained, so there is a spread of rankings
    // to choose a case from.
    let mut chain = Vec::new();
    let mut previous = None;
    for index in 0..7u64 {
        let cid = store_object(format!("segment number {index}").as_bytes());
        store
            .mark_segment_link(&cid, index, previous)
            .expect("link");
        previous = Some(cid);
        chain.push(cid);
    }
    let sealed: Vec<_> = chain[..6].to_vec();

    let mut ledger = CapabilityLedger::new(network);
    let mut advertise = |identity: &intranet_identity::PerNetworkIdentity| {
        ledger
            .insert(
                CapabilityAdvertisement::create(
                    identity,
                    8 << 30,
                    BandwidthCap {
                        up_bytes_per_sec: 1_000_000,
                        down_bytes_per_sec: 8_000_000,
                        active_window: None,
                    },
                    false,
                    false,
                    ComputeClass::Modest,
                    intranet_crypto::Timestamp::from_millis(1),
                ),
                &state,
            )
            .expect("advertises");
    };
    advertise(&founder);
    for member in &others {
        advertise(member);
    }

    // **The case is derived from the ranking rather than assumed.** Everybody
    // here offers the same, so where this node lands for a given object is the
    // hash's business; the test picks an object where it lands deep enough to be
    // a standby with room to test the depth rule below it.
    let candidates: Vec<_> = ledger.entries().cloned().collect();
    let position = |cid: &intranet_storage::Cid| {
        placement::rank(cid.hash().as_bytes(), &candidates, WeightField::StorageOffered)
            .iter()
            .position(|scored| scored.node == me)
            .expect("every member offers storage, so every member is ranked")
    };
    let (target, index) = sealed
        .iter()
        .map(|cid| (*cid, position(cid)))
        // Deep enough that the factor below leaves room for a two-copy hole:
        // standby number one exists only where at least three nodes rank ahead.
        .filter(|(_, index)| *index >= 3)
        .min_by_key(|(_, index)| *index)
        .expect("with ten members and six objects, one of them ranks this node fourth or worse");

    let roomy = Budget {
        offered: u64::MAX,
        installation: u64::MAX,
        installation_used: 0,
    };
    let now = 1_000_000;
    // A factor one short of this node's position makes it standby number one:
    // the *second* node repair would reach, which is what the depth rule is for.
    let factor = index - 1;

    // **A hole one copy deep does not reach the second standby.** Without this
    // rule every node that noticed would adopt, and a network would answer one
    // missing copy with as many new copies as it has members.
    let mut census = Census::default();
    let holders: Vec<_> = others.iter().take(factor - 1).map(|m| m.id()).collect();
    census.heard(target, &holders, &me, now);
    let duty =
        kols_node::replica::evaluate(&store, &ledger, &me, factor, roomy, &census, now).expect("evaluates");
    assert_eq!(duty.adopted, 0, "one missing copy wakes one standby, not every standby");
    assert!(!store.has_duty(&target));

    // Two copies short, and the second standby is exactly who §3.4 re-places on.
    let holders: Vec<_> = others.iter().take(factor - 2).map(|m| m.id()).collect();
    census.heard(target, &holders, &me, now);
    let duty =
        kols_node::replica::evaluate(&store, &ledger, &me, factor, roomy, &census, now).expect("evaluates");
    assert_eq!(duty.adopted, 1, "the shortfall reached this node's place in the ranking");
    assert_eq!(
        duty.adopted_bytes,
        store.object_bytes(&target).expect("held"),
        "what was taken on is weighed, because it is spent out of the member's offer"
    );
    assert!(store.has_duty(&target) && store.has_repair(&target));

    // **No answer is not an answer**, and a repair survives one. This is the
    // state a real node is in most of the time: the count went stale, nothing
    // came back yet, and giving the object up here would undo the repair on
    // exactly the evidence that is missing.
    let much_later = now + 10 * kols_node::replica::ANSWER_FRESH_MILLIS;
    let duty = kols_node::replica::evaluate(&store, &ledger, &me, factor, roomy, &census, much_later)
        .expect("evaluates");
    assert_eq!(duty.returned, 0, "a stale count is not evidence the hole closed");
    assert!(store.has_duty(&target), "and the object stays held");

    // Told the network has its target without this node, the repair ends. The
    // bytes stay — releasing duty has never removed anything — so what changes
    // is only whether this node is promising them.
    let holders: Vec<_> = others.iter().take(factor).map(|m| m.id()).collect();
    census.heard(target, &holders, &me, much_later);
    let duty = kols_node::replica::evaluate(&store, &ledger, &me, factor, roomy, &census, much_later)
        .expect("evaluates");
    assert_eq!(duty.returned, 1, "evidence the hole closed is the one way a repair ends");
    assert!(!store.has_duty(&target) && !store.has_repair(&target));
    assert!(
        store.object_bytes(&target).is_some(),
        "giving back a promise must not destroy the copy"
    );

    // **A repair becomes ordinary duty when placement catches up.** Read at the
    // factor that ranks this node inside the replica set, the same object is
    // held under the ranking rather than under the shortfall — and the mark that
    // exempts it from the ordinary release rule has to go with it, or it would
    // be held under a reason that no longer applies.
    let holders: Vec<_> = others.iter().take(factor - 2).map(|m| m.id()).collect();
    census.heard(target, &holders, &me, much_later);
    kols_node::replica::evaluate(&store, &ledger, &me, factor, roomy, &census, much_later)
        .expect("evaluates");
    assert!(store.has_repair(&target), "adopted again");
    kols_node::replica::evaluate(&store, &ledger, &me, index + 1, roomy, &census, much_later)
        .expect("evaluates");
    assert!(
        store.has_duty(&target) && !store.has_repair(&target),
        "ranked for it now, so it is held for the ordinary reason and ends for the ordinary one"
    );
}

// ── a relay shared between two of one member's networks — O11, D29 ──────
//
// Real peer ids, borrowed from `invite_length.rs`, because the comparison reads
// the peer id out of the address: a placeholder that failed to parse would make
// every assertion below pass without exercising anything.
const RELAY: &str = "12D3KooWAT1R2JjcZbnVUKLX8Xo1Qg5APTWMkpHarHY4Uo1YpGzT";
const ELSEWHERE: &str = "12D3KooWMHZbUfFYuqe6NxXBFSg3aLzfTSa1B5QKGNKwMrWz5FaD";

#[test]
fn a_relay_another_network_uses_is_reported_however_it_is_addressed() {
    // The property the whole check turns on. One relay answers at several
    // addresses — a DNS name and a bare IPv4, TCP beside QUIC — so comparing the
    // strings would report *no overlap* in exactly the case D29 is about, and
    // would do it silently. The two addresses below name one relay and share not
    // one character outside the peer id.
    let dir = Dir::new("shared-relay");
    let workspace = Workspace::at(dir.0.clone());

    workspace
        .create("the workshop", vec![format!("/dns4/relay.example/tcp/443/p2p/{RELAY}")])
        .expect("creates");
    let second = workspace.create("the other one", Vec::new()).expect("creates");

    let shared = workspace.shared_relays(
        Some(second.network()),
        &[format!("/ip4/198.51.100.7/udp/4001/quic-v1/p2p/{RELAY}")],
    );
    assert_eq!(shared.len(), 1, "one relay, named differently, is still one relay");
    assert_eq!(shared[0].label, "the workshop", "the warning has to name which network");
    assert!(
        shared[0].relay.contains("198.51.100.7"),
        "echoed back as the member typed it, so they can tell which entry is meant"
    );

    // And a relay nobody else uses is not reported, or the notice would be
    // noise that teaches people to click through it.
    assert!(
        workspace
            .shared_relays(
                Some(second.network()),
                &[format!("/dns4/relay.example/tcp/443/p2p/{ELSEWHERE}")]
            )
            .is_empty()
    );
}

#[test]
fn a_networks_own_relay_is_not_reported_against_itself() {
    // The obvious way to get this wrong, and it would fire on the commonest act
    // there is: re-designating the relay a network already had.
    let dir = Dir::new("own-relay");
    let workspace = Workspace::at(dir.0.clone());
    let address = format!("/dns4/relay.example/tcp/443/p2p/{RELAY}");

    let store = workspace.create("the workshop", vec![address.clone()]).expect("creates");

    assert!(
        workspace
            .shared_relays(Some(store.network()), std::slice::from_ref(&address))
            .is_empty(),
        "a network does not share a relay with itself"
    );
}

#[test]
fn a_network_that_does_not_exist_yet_is_compared_against_every_other() {
    // Creating a network with a relay is a designation like any other, and it is
    // the *first* one most people make — so the check has to answer before there
    // is an id to exclude. `None` is that network, and excluding the open one
    // instead would hide an overlap with the network the member is looking at.
    let dir = Dir::new("new-network");
    let workspace = Workspace::at(dir.0.clone());
    let address = format!("/dns4/relay.example/tcp/443/p2p/{RELAY}");

    workspace.create("the workshop", vec![address.clone()]).expect("creates");

    let shared = workspace.shared_relays(None, std::slice::from_ref(&address));
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].label, "the workshop");
}

// ── conversation-profile networks — O2, spec 07 §1.2, Core §5.1.1 ───────

#[test]
fn a_conversation_declares_its_profile_and_a_server_deliberately_does_not() {
    // The asymmetry is the point. Absent means `server`, so a server that wrote
    // today's default would be frozen at it for nothing — while a conversation
    // cannot rely on absence for the opposite reason: readers have to *refuse*
    // channel entries in one, and absent would permit them.
    let dir = Dir::new("profiles");
    let workspace = Workspace::at(dir.0.clone());

    let server = workspace.create("the workshop", Vec::new()).expect("creates");
    let server_state = server.state().expect("replays");
    assert!(
        !server_state
            .policy
            .app_policy
            .contains_key(kols_core::keys::PROFILE),
        "a server writes no profile, so a revised default still reaches it"
    );
    assert_eq!(
        kols_core::ChatPolicy::of(&server_state.policy).profile(),
        kols_core::NetworkProfile::Server
    );

    let conversation = workspace.create_conversation("sam").expect("creates");
    let conversation_state = conversation.state().expect("replays");
    assert_eq!(
        kols_core::ChatPolicy::of(&conversation_state.policy).profile(),
        kols_core::NetworkProfile::Conversation,
        "a conversation declares itself, because the rule keys off the declaration"
    );
    // D29: a conversation designates no relay of its own. It reaches its peer
    // over the connection the shared network already has (E13), and naming one
    // here is exactly the sharing D29 refuses.
    assert!(conversation_state.policy.bootstrap_relays.is_empty());
}

#[test]
fn the_behaviour_set_a_node_runs_follows_what_the_network_is() {
    // Core §5.1.1 fixes this at construction, so it is read from the network's
    // own policy rather than chosen per run. For a conversation it is a privacy
    // requirement rather than a saving: a DM node holding a routing table joins
    // whatever table it meets, and where a hole punch fails that is the shared
    // network's relay's — D29 reached with nobody designating anything.
    let dir = Dir::new("discovery");
    let workspace = Workspace::at(dir.0.clone());

    let server = workspace.create("the workshop", Vec::new()).expect("creates");
    let conversation = workspace.create_conversation("sam").expect("creates");

    assert_eq!(
        kols_node::discovery_for(&server.state().expect("replays").policy),
        intranet_transport::Discovery::Full
    );
    assert_eq!(
        kols_node::discovery_for(&conversation.state().expect("replays").policy),
        intranet_transport::Discovery::Off
    );
}

#[test]
fn a_conversation_refuses_channel_structure_before_anything_is_signed() {
    // The gate already refuses this — `kols-api`'s `Refusal::NotAServer`, at all
    // four channel and category sites — and had all along. What did not exist
    // was a conversation to refuse it in, so the rule was unreachable rather
    // than unbuilt, and this is the first thing to exercise it against a network
    // this client actually created rather than a hand-built policy.
    //
    // **Kept because writing it found the executor guard I had just added to be
    // a second copy of it**, at the wrong layer: the check belongs where
    // authorization happens, and the probe that was meant to prove my guard
    // worked passed with it removed.
    let dir = Dir::new("no-channels");
    let workspace = Workspace::at(dir.0.clone());

    let make = |store: &kols_node::store::Store| {
        kols_node::executor::Executor::open(store.root().to_path_buf())
            .expect("opens")
            .submit(kols_api::Command::CreateChannel {
                name: "general".to_owned(),
                category: None,
                privacy: kols_core::Privacy::Public,
                topic: String::new(),
            })
            .map(|_| ())
            .map_err(|err| err.to_string())
    };

    // The control matters: the founder holds every capability in both networks,
    // so without it a refusal could be about permission rather than the profile.
    let server = workspace.create("the workshop", Vec::new()).expect("creates");
    assert!(make(&server).is_ok(), "a server takes channels");

    let conversation = workspace.create_conversation("sam").expect("creates");
    let refusal = make(&conversation).expect_err("a conversation has one implied channel");
    assert!(refusal.contains("conversation"), "{refusal}");
}

#[test]
fn a_node_for_a_conversation_is_built_without_discovery_and_says_so() {
    // The one thing the unit test above cannot reach: that `serve` actually
    // asks. A conversation running *with* discovery behaves identically in every
    // visible way — it listens, dials, relays, hole-punches, gossips and serves
    // exactly the same — and differs only by having joined a routing table it
    // should not be in. An invisible failure is a test's job.
    let dir = Dir::new("serve-discovery");
    let workspace = Workspace::at(dir.0.clone());

    let conversation = workspace.create_conversation("sam").expect("creates");
    let said = common::brief_run(conversation.root().to_path_buf());
    assert!(
        said.iter().any(|line| line.contains("discovery off")),
        "a conversation has nobody to find: {said:?}"
    );

    // And the control, because a line that appeared for everything would say
    // nothing about the choice.
    let server = workspace.create("the workshop", Vec::new()).expect("creates");
    let said = common::brief_run(server.root().to_path_buf());
    assert!(
        said.iter().any(|line| line.contains("peer id")),
        "the node started at all: {said:?}"
    );
    assert!(
        !said.iter().any(|line| line.contains("discovery off")),
        "a server needs discovery: {said:?}"
    );
}
