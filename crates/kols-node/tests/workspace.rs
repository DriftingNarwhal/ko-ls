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
        !store.history_incomplete(&channel),
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

use intranet_governance::{EntryBody, GroupId, LogEntry, MembershipAction};
use kols_node::store::Store;

// ── a conversation borrows its rendezvous and never keeps it ────────────
//
// The rule these pin is a decision rather than a mechanism: a conversation
// between two people who cannot dial each other directly may use the **shared**
// network's relay, and only while both are still members of it. Designating it
// instead would write replayed state that outlives its reason, with nothing able
// to un-designate it — no member of a conversation can know the other two left
// some third network.

/// Admits `who` to `store`'s `everyone`, signed by `founder`.
fn admit_to(store: &Store, founder: &intranet_identity::PerNetworkIdentity, who: &intranet_identity::PerNetworkIdentityId) {
    let log = store.log().expect("log");
    let parent = log.canonical_chain().last().copied();
    store
        .append_entry(&LogEntry::create(
            founder,
            parent,
            intranet_crypto::Timestamp::from_millis(100),
            EntryBody::MembershipChange {
                group: GroupId::everyone(),
                identity: *who,
                action: MembershipAction::Add { via_invite: None },
            },
        ))
        .expect("admits");
}

/// Removes `who` from `store`'s `everyone`, signed by `founder`.
fn remove_from(store: &Store, founder: &intranet_identity::PerNetworkIdentity, who: &intranet_identity::PerNetworkIdentityId) {
    let log = store.log().expect("log");
    let parent = log.canonical_chain().last().copied();
    store
        .append_entry(&LogEntry::create(
            founder,
            parent,
            intranet_crypto::Timestamp::from_millis(200),
            EntryBody::MembershipChange {
                group: GroupId::everyone(),
                identity: *who,
                action: MembershipAction::Remove { cascade: None },
            },
        ))
        .expect("removes");
}

/// A shared network with a relay, a peer admitted to it, and a conversation
/// whose origin points back at that network and peer.
fn conversation_with_origin(dir: &Dir) -> (Workspace, Store, Store, intranet_identity::PerNetworkIdentity, intranet_identity::PerNetworkIdentityId) {
    let workspace = Workspace::at(dir.0.clone());
    let shared = workspace
        .create("the workshop", vec![format!("/dns4/relay.example/tcp/443/p2p/{RELAY}")])
        .expect("creates");
    let founder = shared.identity().expect("identity");
    let peer = intranet_identity::MasterSeed::from_entropy([77u8; 32])
        .identity_for(shared.network())
        .expect("identity")
        .id();
    admit_to(&shared, &founder, &peer);

    let conversation = workspace.create_conversation("alice").expect("creates");
    conversation.set_origin(shared.network(), &peer).expect("records origin");
    (workspace, shared, conversation, founder, peer)
}

#[test]
fn a_conversation_borrows_the_shared_networks_relay_while_both_are_members() {
    let dir = Dir::new("borrow-relay");
    let (workspace, _shared, conversation, _founder, _peer) = conversation_with_origin(&dir);

    let borrowed = workspace
        .borrowable_relay(&conversation)
        .expect("both are members, so there is somewhere to meet");
    assert_eq!(borrowed.len(), 1);
    assert!(borrowed[0].contains(RELAY), "the shared network's relay, not one of its own");
}

#[test]
fn the_relay_stops_being_borrowable_when_the_peer_leaves_the_shared_network() {
    // **The whole point of borrowing rather than designating.** A designated
    // relay would still be in the conversation's policy here, and nothing in the
    // conversation could know to remove it.
    let dir = Dir::new("borrow-revoked");
    let (workspace, shared, conversation, founder, peer) = conversation_with_origin(&dir);
    assert!(workspace.borrowable_relay(&conversation).is_some(), "borrowable first");

    remove_from(&shared, &founder, &peer);

    assert!(
        workspace.borrowable_relay(&conversation).is_none(),
        "once the shared membership ends, the rendezvous is returned with it"
    );
}

#[test]
fn the_relay_stops_being_borrowable_when_this_member_is_revoked() {
    // **Both sides, not just the peer.** Borrowing on one's own membership alone
    // would let a departed member keep using their old network's infrastructure
    // to reach somebody still in it.
    //
    // Constructing this needs somebody with the authority to remove *me*, so the
    // peer is promoted to `Founders` first and does it.
    //
    // Two earlier attempts were wrong in the same way and are worth recording.
    // Both removed me from `everyone` — and a network's creator is never in
    // `everyone`: Core §2.3 puts them in `Founders` alone, and `everyone` is
    // where *admitted* members land (§2.4). So the entry removed a non-member,
    // governance refused the whole log, `state()` errored, and `borrowable_relay`
    // returned `None` through the `?` without the membership check running at
    // all. It survived the probe that deletes that check, which is how it was
    // caught. The founder is removed from the group they are actually in.
    let dir = Dir::new("borrow-mine");
    let (workspace, shared, conversation, founder, peer) = conversation_with_origin(&dir);
    let me = shared.identity().expect("identity").id();

    // Promote the peer so there is somebody who may remove me.
    let log = shared.log().expect("log");
    let parent = log.canonical_chain().last().copied();
    shared
        .append_entry(&LogEntry::create(
            &founder,
            parent,
            intranet_crypto::Timestamp::from_millis(150),
            EntryBody::MembershipChange {
                group: GroupId::founders(),
                identity: peer,
                action: MembershipAction::Add { via_invite: None },
            },
        ))
        .expect("promotes");

    let peer_identity = intranet_identity::MasterSeed::from_entropy([77u8; 32])
        .identity_for(shared.network())
        .expect("identity");
    let log = shared.log().expect("log");
    let parent = log.canonical_chain().last().copied();
    shared
        .append_entry(&LogEntry::create(
            &peer_identity,
            parent,
            intranet_crypto::Timestamp::from_millis(200),
            EntryBody::MembershipChange {
                group: GroupId::founders(),
                identity: me,
                action: MembershipAction::Remove { cascade: None },
            },
        ))
        .expect("revokes me");

    // The log still replays — which is what makes this a test of the membership
    // check rather than of a broken log.
    let state = shared.state().expect("replays");
    assert!(!state.is_member(&me), "the revocation took effect");
    assert!(state.is_member(&peer), "and the peer is still there, so only my side changed");

    assert!(
        workspace.borrowable_relay(&conversation).is_none(),
        "a member who left borrows nothing, even from a network still holding the other party"
    );
}

#[test]
fn a_conversation_with_no_origin_borrows_nothing() {
    // Fail-closed: a conversation whose origin was never recorded reads exactly
    // like one whose shared network is gone, and both borrow nothing.
    let dir = Dir::new("borrow-no-origin");
    let workspace = Workspace::at(dir.0.clone());
    workspace
        .create("the workshop", vec![format!("/dns4/relay.example/tcp/443/p2p/{RELAY}")])
        .expect("creates");
    let orphan = workspace.create_conversation("nobody").expect("creates");

    assert!(workspace.borrowable_relay(&orphan).is_none());
}

#[test]
fn nothing_about_borrowing_is_written_into_the_conversation() {
    // The conversation's own designated relays stay empty throughout, which is
    // what makes this borrowing rather than designating — there is no state to
    // outlive the membership that justified it, and so nothing to un-designate.
    let dir = Dir::new("borrow-unwritten");
    let (workspace, _shared, conversation, _founder, _peer) = conversation_with_origin(&dir);

    assert!(workspace.borrowable_relay(&conversation).is_some());
    assert!(
        conversation.relays().is_empty(),
        "asking for a rendezvous must not record one"
    );
}

#[test]
fn an_origin_survives_being_written_and_read_back() {
    let dir = Dir::new("origin-roundtrip");
    let (_workspace, shared, conversation, _founder, peer) = conversation_with_origin(&dir);

    let (network, who) = conversation.origin().expect("recorded");
    assert_eq!(network, *shared.network());
    assert_eq!(who, peer);

    // And a server network has none, so the question is answerable for every
    // store rather than only for conversations.
    assert!(shared.origin().is_none());
}

// ── running a node per network — `design/05` §4, `design/09` §2 ─────────
//
// The unit tests beside `nodes::tier_for` pin the policy. These pin what it is
// for and what the client could not do until now: **several networks live at the
// same time**. A member in a dozen networks receives in all of them, and a
// conversation — a network neither party is usually looking at — is reachable
// without anybody opening it.

#[test]
fn a_supervisor_runs_every_joined_network_at_once() {
    use kols_node::nodes::{Nodes, Spec, Tier};

    let dir = Dir::new("supervisor-many");
    let workspace = Workspace::at(dir.0.clone());
    let a = workspace.create("the workshop", Vec::new()).expect("creates");
    let b = workspace.create("the other one", Vec::new()).expect("creates");
    let c = workspace.create_conversation("sam").expect("creates");

    let (a_id, b_id, c_id) = (*a.network(), *b.network(), *c.network());
    let specs = vec![
        Spec { network: a_id, root: a.root().to_path_buf(), set_aside: false },
        Spec { network: b_id, root: b.root().to_path_buf(), set_aside: false },
        Spec { network: c_id, root: c.root().to_path_buf(), set_aside: false },
    ];

    // Events are tagged, because `Event` says what happened and not where —
    // unambiguous while one node ran, and not now.
    let sink: kols_node::nodes::TaggedSink = std::sync::Arc::new(|_, _| {});

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(async {
            let now = std::time::Instant::now();
            let mut nodes = Nodes::here();
            nodes.reconcile(&specs, Some(&a_id), &sink, kols_node::serve::SEAL_TARGET_BYTES, now);

            assert_eq!(nodes.tier(&a_id), Tier::Hot, "the one in view");
            assert_eq!(nodes.tier(&b_id), Tier::Warm, "a server nobody is looking at still runs");
            assert_eq!(nodes.tier(&c_id), Tier::Warm, "and so does a conversation");
            assert_eq!(nodes.running().len(), 3, "three networks, three nodes");

            // Long enough for all three to have claimed their stores. If they
            // contended for one claim, some would have exited.
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            assert_eq!(
                nodes.running().len(),
                3,
                "the claim is per store, so running many trips nothing"
            );

            // Looking elsewhere moves the labels and stops nothing, which is the
            // correction: a network does not go dark because you looked away.
            nodes.reconcile(&specs, Some(&b_id), &sink, kols_node::serve::SEAL_TARGET_BYTES, now);
            assert_eq!(nodes.tier(&a_id), Tier::Warm);
            assert_eq!(nodes.tier(&b_id), Tier::Hot);
            assert_eq!(nodes.running().len(), 3, "still all three");

            for task in nodes.stop_all() {
                let _ = task.await;
            }
            assert!(nodes.running().is_empty());
        });
}

#[test]
fn a_network_the_member_set_aside_is_polled_rather_than_abandoned() {
    use kols_node::nodes::{Nodes, POLL_INTERVAL, POLL_SETTLE, Spec, Tier};

    // **Cold is a schedule, not an absence**, and the difference reaches other
    // people: a network with no node running serves nothing, so a machine that
    // had taken replica duty there quietly stops holding up its end for
    // everybody else in it.
    let dir = Dir::new("supervisor-cold");
    let workspace = Workspace::at(dir.0.clone());
    let quiet = workspace.create("the quiet one", Vec::new()).expect("creates");
    let quiet_id = *quiet.network();
    let specs = vec![Spec {
        network: quiet_id,
        root: quiet.root().to_path_buf(),
        set_aside: true,
    }];
    let sink: kols_node::nodes::TaggedSink = std::sync::Arc::new(|_, _| {});

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(async {
            let start = std::time::Instant::now();
            let mut nodes = Nodes::here();

            // First reconcile wakes it, because it has never been polled.
            nodes.reconcile(&specs, None, &sink, kols_node::serve::SEAL_TARGET_BYTES, start);
            assert_eq!(nodes.running().len(), 1, "a cold network is woken to catch up");
            assert_eq!(nodes.tier(&quiet_id), Tier::Cold, "woken, and still cold");

            // Mid-poll it is left alone rather than restarted on every tick.
            nodes.reconcile(&specs, None, &sink, kols_node::serve::SEAL_TARGET_BYTES, start);
            assert_eq!(nodes.running().len(), 1);

            // Once it has had time to catch up, it goes back to sleep.
            let settled = start + POLL_SETTLE + std::time::Duration::from_secs(1);
            nodes.reconcile(&specs, None, &sink, kols_node::serve::SEAL_TARGET_BYTES, settled);
            assert!(nodes.running().is_empty(), "the poll ends rather than becoming a warm node");

            // And it is not woken again until its poll comes round.
            let soon = settled + std::time::Duration::from_secs(1);
            nodes.reconcile(&specs, None, &sink, kols_node::serve::SEAL_TARGET_BYTES, soon);
            assert!(nodes.running().is_empty(), "not every tick");

            let due = start + POLL_INTERVAL + std::time::Duration::from_secs(1);
            nodes.reconcile(&specs, None, &sink, kols_node::serve::SEAL_TARGET_BYTES, due);
            assert_eq!(nodes.running().len(), 1, "woken again when the poll comes round");

            for task in nodes.stop_all() {
                let _ = task.await;
            }
        });
}

#[test]
fn reconciling_twice_changes_nothing_and_a_tier_change_is_not_a_restart() {
    use kols_node::nodes::{Nodes, Spec, Tier};

    let dir = Dir::new("supervisor-idempotent");
    let workspace = Workspace::at(dir.0.clone());
    let a = workspace.create("the workshop", Vec::new()).expect("creates");
    let b = workspace.create_conversation("sam").expect("creates");
    let (a_id, b_id) = (*a.network(), *b.network());
    let specs = vec![
        Spec { network: a_id, root: a.root().to_path_buf(), set_aside: false },
        Spec { network: b_id, root: b.root().to_path_buf(), set_aside: false },
    ];
    let sink: kols_node::nodes::TaggedSink = std::sync::Arc::new(|_, _| {});

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(async {
            let now = std::time::Instant::now();
            let mut nodes = Nodes::here();
            nodes.reconcile(&specs, Some(&a_id), &sink, kols_node::serve::SEAL_TARGET_BYTES, now);
            let first = nodes.running();

            // Idempotent: a caller may run this on every tick without tracking
            // what it did last time, which keeps the policy in one place.
            nodes.reconcile(&specs, Some(&a_id), &sink, kols_node::serve::SEAL_TARGET_BYTES, now);
            assert_eq!(nodes.running(), first, "nothing restarted");

            // Moving the view raises one and lowers the other without restarting
            // either — hot and warm are both continuously running, and dropping a
            // node's connections to change a label would be a cost for nothing.
            nodes.reconcile(&specs, Some(&b_id), &sink, kols_node::serve::SEAL_TARGET_BYTES, now);
            assert_eq!(nodes.tier(&b_id), Tier::Hot);
            assert_eq!(nodes.tier(&a_id), Tier::Warm);
            assert_eq!(nodes.running().len(), 2, "both still up; only the labels moved");
            for task in nodes.stop_all() {
                let _ = task.await;
            }
        });
}

#[test]
fn setting_a_network_aside_survives_a_reopen() {
    let dir = Dir::new("set-aside-persists");
    let workspace = Workspace::at(dir.0.clone());
    let network = workspace.create("the quiet one", Vec::new()).expect("creates");
    assert!(!network.is_set_aside(), "warm is the default");

    network.set_aside(true).expect("sets aside");
    let reopened = workspace.open(&network.network().short()).expect("opens");
    assert!(reopened.is_set_aside(), "a preference nobody has to set twice");

    reopened.set_aside(false).expect("brings it back");
    assert!(!reopened.is_set_aside());
}


#[test]
fn reconciling_from_a_synchronous_caller_does_not_panic() {
    // **The crash `v0.13.0` shipped.** `open_network` is a synchronous Tauri
    // command, so no runtime is entered on the thread it runs on — and
    // `tokio::spawn` panics outside a runtime context rather than returning an
    // error. Selecting a network from the picker took the window down every
    // time, and nothing in this suite saw it because every other test here
    // reconciles inside `block_on`.
    use kols_node::nodes::{Nodes, Spec};

    let dir = Dir::new("reconcile-sync");
    let workspace = Workspace::at(dir.0.clone());
    let network = workspace.create("the workshop", Vec::new()).expect("creates");
    let specs = vec![Spec {
        network: *network.network(),
        root: network.root().to_path_buf(),
        set_aside: false,
    }];
    let sink: kols_node::nodes::TaggedSink = std::sync::Arc::new(|_, _| {});

    // Deliberately *not* inside a runtime: this is the caller the shell has.
    // The runtime is supplied rather than found, which is the fix — a handle
    // works from any thread, and `tokio::spawn`'s ambient lookup does not.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let mut nodes = Nodes::new(runtime.handle().clone());
    nodes.reconcile(
        &specs,
        None,
        &sink,
        kols_node::serve::SEAL_TARGET_BYTES,
        std::time::Instant::now(),
    );
    let _ = nodes.stop_all();
}

// --- Starting a conversation (spec 07 §6.2, `design/05` §3.1) ---------------

#[test]
fn starting_a_conversation_makes_a_network_records_its_origin_and_offers_it() {
    let dir = Dir::new("dm-start");
    let workspace = Workspace::at(dir.0.clone());
    let shared = workspace
        .create("the workshop", vec![format!("/dns4/relay.example/tcp/443/p2p/{RELAY}")])
        .expect("creates");
    let founder = shared.identity().expect("identity");
    let peer = intranet_identity::MasterSeed::from_entropy([91u8; 32])
        .identity_for(shared.network())
        .expect("identity")
        .id();
    admit_to(&shared, &founder, &peer);

    let started = kols_node::dm::start(&workspace, &shared, &peer, None).expect("starts");

    // A separate network, which is the whole of `design/03` §4's decision.
    assert_ne!(
        &started.conversation,
        shared.network(),
        "a conversation is its own network, never a channel in the shared one"
    );

    // **Not deliverable yet, and that is the ordinary state rather than a
    // failure.** An invite must carry an address and only a running node knows
    // one (`02` §6.1); no node has run for this network yet.
    assert!(
        !started.deliverable,
        "nothing can be sent until the conversation's own node has an address"
    );

    let conversation = workspace
        .list()
        .into_iter()
        .filter_map(|known| Store::open(known.path).ok())
        .find(|store| *store.network() == started.conversation)
        .expect("the conversation is on the disk");

    // D39: the origin is what makes the rendezvous borrowable, recomputed each
    // time rather than designated once.
    let (origin_network, origin_peer) = conversation.origin().expect("origin recorded");
    assert_eq!(&origin_network, shared.network());
    assert_eq!(origin_peer, peer);
    assert!(
        workspace.borrowable_relay(&conversation).is_some(),
        "both are still members, so the shared network's relay is borrowable"
    );

    // The want the daemon will honour, in the shared network's store — which is
    // the one that knows both the person and the route (spec 07 §6.2).
    let offered = shared.offered_conversations();
    assert_eq!(offered, vec![(peer, started.conversation)]);
}

#[test]
fn a_second_offer_to_one_person_replaces_the_first_rather_than_queueing() {
    // A request is a standing ask rather than a message, so two of them from one
    // person mean the same thing — and re-asking is how a member retries
    // somebody who was unreachable (`03` §4.3 makes retrying the sender's job).
    let dir = Dir::new("dm-reoffer");
    let workspace = Workspace::at(dir.0.clone());
    let shared = workspace.create("the workshop", Vec::new()).expect("creates");
    let founder = shared.identity().expect("identity");
    let peer = intranet_identity::MasterSeed::from_entropy([92u8; 32])
        .identity_for(shared.network())
        .expect("identity")
        .id();
    admit_to(&shared, &founder, &peer);

    let first = kols_node::dm::start(&workspace, &shared, &peer, None).expect("starts");
    let second = kols_node::dm::start(&workspace, &shared, &peer, None).expect("starts again");

    assert_ne!(
        first.conversation, second.conversation,
        "a second conversation is a second network — `03` §4.4 already has that shape, and \
         refusing it here would be a product rule nothing asked for"
    );
    let offered = shared.offered_conversations();
    assert_eq!(
        offered,
        vec![(peer, second.conversation)],
        "one offer per person, the newer one — not a queue"
    );
}

#[test]
fn a_conversation_is_refused_with_a_stranger_and_with_yourself() {
    let dir = Dir::new("dm-refusals");
    let workspace = Workspace::at(dir.0.clone());
    let shared = workspace.create("the workshop", Vec::new()).expect("creates");
    let mine = shared.identity().expect("identity").id();

    // A stranger: the recipient is obliged to refuse a sender who is not a
    // current member (spec 07 §6.2), so this end refuses first rather than
    // minting a network nothing will ever use.
    let stranger = intranet_identity::MasterSeed::from_entropy([93u8; 32])
        .identity_for(shared.network())
        .expect("identity")
        .id();
    let refused = kols_node::dm::start(&workspace, &shared, &stranger, None)
        .expect_err("not a member of this network");
    assert!(refused.contains("not a member"), "{refused}");

    let myself =
        kols_node::dm::start(&workspace, &shared, &mine, None).expect_err("not with yourself");
    assert!(myself.contains("somebody else"), "{myself}");

    // Neither refusal left a network behind.
    assert_eq!(
        workspace.list().len(),
        1,
        "a refused start creates nothing — only the shared network is on the disk"
    );
}

#[test]
fn a_store_that_is_its_own_home_belongs_to_no_workspace() {
    // **The hazard this is guarding, stated because the loose version looks
    // right.** A `--home` pointing straight at one store is a shape the terminal
    // has always supported, and answering "the workspace is its parent" for one
    // would root a workspace in whatever directory it happens to sit in — `/tmp`
    // during a test run, a home directory in the field — and sweep every
    // unrelated store beside it into this installation's networks.
    let dir = Dir::new("containing-single");
    let lone = Workspace::at(dir.0.clone());
    let store = lone.create("on its own", Vec::new()).expect("creates");

    // The store this workspace laid out *is* found from its own path, because
    // `path_for` would have put it exactly there.
    let found = Workspace::containing(&store).expect("laid out by a workspace");
    assert_eq!(found.root(), lone.root());

    // A store opened as its own home is not: the directory above it is not a
    // workspace, whatever else is in it.
    let alone = Store::open(store.root().to_path_buf()).expect("opens");
    let as_home = Workspace::at(store.root().to_path_buf());
    assert_eq!(as_home.list().len(), 1, "a workspace of one, which is the supported shape");
    assert!(
        Workspace::containing(&alone).is_some_and(|w| w.root() == lone.root()),
        "found by layout rather than by nesting, so the answer does not change \
         with how the store was opened"
    );

    // And the decisive case: a store sitting somewhere `path_for` would never
    // have put it belongs to no workspace at all.
    let stray = dir.0.join("not-a-network-id-directory");
    std::fs::create_dir_all(&stray).expect("makes it");
    for name in ["seed", "network"] {
        let from = store.root().join(name);
        if from.exists() {
            std::fs::copy(&from, stray.join(name)).expect("copies");
        }
    }
    if let Ok(moved) = Store::open(stray) {
        assert!(
            Workspace::containing(&moved).is_none(),
            "a store `path_for` would not have placed here has no workspace, so nothing \
             goes looking through its neighbours"
        );
    }
}

// --- Delivering and receiving a conversation request -----------------------
//
// # Where the wire is tested, and why it is not here
//
// Core §5.1's carrier is tested upstream over two live nodes
// (`intranet-transport/tests/direct_delivery.rs`), including the acknowledgement
// this work added. What is left for this side is what the carrier deliberately
// cannot do: build the payload, and make the two checks spec 07 §6.2 says no
// platform can make for it. Those are properties of two stores and a payload, so
// they are tested against two stores and a payload — the same reasoning
// `design/05` §8 gives for testing provider discovery where the topology is
// rather than where the containers are.
//
// **Still owed: the live two-daemon path.** The terminal's `--home` names one
// store rather than a workspace, so a `two_nodes`-style test cannot reach this
// flow yet, and that is recorded rather than worked around.

/// Attaches a workspace to an existing network, in the layout `path_for` uses.
///
/// The terminal's `attach` writes a store straight at `--home`, which is its
/// single-store shape; a workspace needs the store where `path_for` would put
/// it, or nothing above will find it.
fn attach_in(
    workspace: &Workspace,
    network: &intranet_identity::NetworkId,
    label: &str,
) -> Store {
    let store = Store::create(
        workspace.path_for(network),
        *network,
        kols_node::random_32().expect("entropy"),
    )
    .expect("attaches");
    store.set_label(label).expect("labels");
    store
}

/// Alice and Bob, both members of one network, each with their own workspace.
fn two_sides(name: &str) -> (Dir, Dir, Workspace, Store, Workspace, Store) {
    let alice_dir = Dir::new(&format!("{name}-alice"));
    let bob_dir = Dir::new(&format!("{name}-bob"));
    let alice_ws = Workspace::at(alice_dir.0.clone());
    let bob_ws = Workspace::at(bob_dir.0.clone());

    let alice = alice_ws.create("the workshop", Vec::new()).expect("creates");
    let founder = alice.identity().expect("identity");

    // Bob's identity in that network exists before anybody has heard of him,
    // because it is derived from the network id and his own seed (Core §1.2) —
    // which is what lets him be admitted by name.
    let bob = attach_in(&bob_ws, alice.network(), "the workshop");
    let bob_id = bob.identity().expect("identity").id();
    admit_to(&alice, &founder, &bob_id);

    // Bob replays the same log, so his store agrees about who is a member.
    for hash in alice.log().expect("log").canonical_chain() {
        if let Some(entry) = alice.log().expect("log").get(&hash) {
            bob.append_entry(entry).expect("adopts");
        }
    }
    (alice_dir, bob_dir, alice_ws, alice, bob_ws, bob)
}

#[test]
fn a_request_built_on_one_side_verifies_on_the_other() {
    let (_a, _b, alice_ws, alice, _bob_ws, bob) = two_sides("dm-round-trip");
    let bob_id = bob.identity().expect("identity").id();

    let started = kols_node::dm::start(&alice_ws, &alice, &bob_id, None).expect("starts");

    // Nothing to deliver yet: the conversation's node has recorded no address,
    // so there is nothing for an invite to carry. This is the ordinary state
    // rather than a failure, and it is why the offer is a want.
    assert!(
        kols_node::dm::deliverable(&alice_ws, &alice, 1_000).expect("builds").is_empty(),
        "an offer with no address is not deliverable"
    );

    // What `kols serve` does for the conversation once it runs.
    let conversation = alice_ws.store_for(&started.conversation).expect("on the disk");
    conversation
        .set_addresses(&["/ip4/127.0.0.1/tcp/4001".to_owned()])
        .expect("records an address");

    let ready = kols_node::dm::deliverable(&alice_ws, &alice, 1_000).expect("builds");
    assert_eq!(ready.len(), 1, "now there is something to send");
    assert_eq!(ready[0].to, bob_id);

    // The other side, doing what the daemon does on `DirectReceived`.
    let alice_id = alice.identity().expect("identity").id();
    match kols_node::dm::receive(&bob, &alice_id, &ready[0].payload) {
        kols_node::dm::Arrived::Request { from } => assert_eq!(from, alice_id),
        other => panic!("a well-formed request from a member should be kept: {other:?}"),
    }

    // Kept, because the answer is a person's and a person is not there at the
    // instant it arrives (spec 07 §6.2).
    let waiting = bob.requests();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].0, alice_id);

    // And the invite inside it names a *different* network, which is the whole
    // of `design/03` §4's decision.
    let request = kols_core::DmInvite::decode(&waiting[0].1).expect("decodes");
    assert_eq!(&request.invite().network, &started.conversation);
    assert_ne!(&request.invite().network, alice.network());
}

#[test]
fn a_proof_about_somebody_else_is_refused_however_genuine_its_signatures() {
    // **The forgery that matters** (Core §1.2, spec 07 §6.2). A proof carrying
    // two real signatures over a true statement about a *different* pair
    // verifies perfectly and says nothing about who is asking — so verifying the
    // signatures is necessary and nowhere near sufficient.
    let (_a, _b, alice_ws, alice, _bob_ws, bob) = two_sides("dm-wrong-pair");
    let bob_id = bob.identity().expect("identity").id();

    // Carol builds an entirely honest request of her own.
    let carol_dir = Dir::new("dm-wrong-pair-carol");
    let carol_ws = Workspace::at(carol_dir.0.clone());
    let carol_shared = attach_in(&carol_ws, alice.network(), "the workshop");
    for hash in alice.log().expect("log").canonical_chain() {
        if let Some(entry) = alice.log().expect("log").get(&hash) {
            let _ = carol_shared.append_entry(entry);
        }
    }
    let carol_id = carol_shared.identity().expect("identity").id();
    // Admitted too, or Bob refuses her for being a stranger and the control
    // below would pass for the wrong reason — which is the failure this test is
    // about, arriving from the other direction.
    admit_to(&alice, &alice.identity().expect("identity"), &carol_id);
    for hash in alice.log().expect("log").canonical_chain() {
        if let Some(entry) = alice.log().expect("log").get(&hash) {
            let _ = bob.append_entry(entry);
            let _ = carol_shared.append_entry(entry);
        }
    }
    kols_node::dm::start(&carol_ws, &carol_shared, &bob_id, None).expect("starts");
    let conversation = carol_ws
        .store_for(&carol_shared.offered_conversations()[0].1)
        .expect("on the disk");
    conversation
        .set_addresses(&["/ip4/127.0.0.1/tcp/4002".to_owned()])
        .expect("records an address");
    let carols = kols_node::dm::deliverable(&carol_ws, &carol_shared, 1_000).expect("builds");

    // Alice replays Carol's request as her own. Every signature in it is
    // genuine; the pair it names is not the pair Bob is talking to.
    let alice_id = alice.identity().expect("identity").id();
    let _ = alice_ws;
    match kols_node::dm::receive(&bob, &alice_id, &carols[0].payload) {
        kols_node::dm::Arrived::Refused { from, why } => {
            assert_eq!(from, alice_id);
            assert!(why.contains("identity link"), "{why}");
        }
        other => panic!("a proof about another pair must not pass: {other:?}"),
    }
    assert!(
        bob.requests().is_empty(),
        "nothing unverified reaches the disk, because the disk is what an interface renders"
    );

    // Carol's own request, from Carol, is fine — so the refusal above is about
    // the pair and not about the payload being malformed.
    assert!(matches!(
        kols_node::dm::receive(&bob, &carol_id, &carols[0].payload),
        kols_node::dm::Arrived::Request { .. }
    ));
}

#[test]
fn a_request_from_somebody_who_is_not_a_member_is_refused() {
    // Core §5.1 says plainly that the carrier cannot check this — it holds no
    // governance state for the purpose — and spec 07 §6.2 is the section that
    // owes it. Answered by replay, so a sender revoked since composing the
    // request is refused now rather than as of then.
    let (_a, _b, alice_ws, alice, _bob_ws, bob) = two_sides("dm-non-member");
    let bob_id = bob.identity().expect("identity").id();

    kols_node::dm::start(&alice_ws, &alice, &bob_id, None).expect("starts");
    let conversation = alice_ws
        .store_for(&alice.offered_conversations()[0].1)
        .expect("on the disk");
    conversation
        .set_addresses(&["/ip4/127.0.0.1/tcp/4003".to_owned()])
        .expect("records an address");
    let ready = kols_node::dm::deliverable(&alice_ws, &alice, 1_000).expect("builds");

    // A store that never learned Alice was admitted — which is what a node that
    // has not synced, or one that has seen her removed, looks like.
    let stranger_dir = Dir::new("dm-non-member-stranger");
    let stranger_ws = Workspace::at(stranger_dir.0.clone());
    let unsynced = attach_in(&stranger_ws, alice.network(), "the workshop");

    let alice_id = alice.identity().expect("identity").id();
    match kols_node::dm::receive(&unsynced, &alice_id, &ready[0].payload) {
        kols_node::dm::Arrived::Refused { why, .. } => {
            assert!(why.contains("not a current member") || why.contains("replay"), "{why}");
        }
        other => panic!("a sender this node cannot see as a member must be refused: {other:?}"),
    }
}

#[test]
fn a_payload_that_is_not_a_conversation_request_is_refused_rather_than_guessed_at() {
    let (_a, _b, _alice_ws, alice, _bob_ws, bob) = two_sides("dm-garbage");
    let alice_id = alice.identity().expect("identity").id();
    match kols_node::dm::receive(&bob, &alice_id, b"not a request at all") {
        kols_node::dm::Arrived::Refused { why, .. } => assert!(why.contains("decode"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn being_told_that_closing_is_not_quitting_is_remembered_across_launches() {
    // `design/09` §1.11. Once means once per installation: a notice on every
    // close is one nobody reads, and one every morning is the same notice.
    let dir = Dir::new("told-about-tray");
    let workspace = Workspace::at(dir.0.clone());
    assert!(!workspace.told_closing_is_not_quitting(), "nothing said yet");
    workspace.remember_closing_is_not_quitting().expect("remembers");
    assert!(workspace.told_closing_is_not_quitting());
    // A second workspace over the same root is a second launch.
    assert!(Workspace::at(dir.0.clone()).told_closing_is_not_quitting(), "and it survives");
}

// --- One application per installation (`design/09` §1.1) -------------------

#[test]
fn a_second_launch_asks_the_first_to_show_itself_rather_than_starting() {
    use kols_node::workspace::Launch;

    let dir = Dir::new("one-application");
    let workspace = Workspace::at(dir.0.clone());

    let Launch::First(first) = workspace.hold_application() else {
        panic!("the first launch is the application");
    };
    assert!(first.beat(), "and it holds its own claim");
    assert!(!first.asked_to_show(), "nobody has asked it to do anything yet");

    // **The holder has to actually answer**, which is the protocol rather than
    // a detail of the test: a launch concludes somebody is there by watching
    // its ask be taken, never by reading a heartbeat. So this stands in for the
    // beat that would consume it in a running application.
    let answering = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let stop = answering.clone();
    let watched = dir.0.join("running").join("raise");
    let holder = std::thread::spawn(move || {
        while stop.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = std::fs::remove_file(&watched);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    });

    let second = Workspace::at(first.root().to_path_buf());
    assert!(
        matches!(second.hold_application(), Launch::Second),
        "a second launch does not become a second application"
    );

    answering.store(false, std::sync::atomic::Ordering::Relaxed);
    holder.join().expect("the stand-in stops");
}

#[test]
fn a_heartbeat_nobody_answers_is_not_a_running_application() {
    use kols_node::workspace::Launch;

    // **The case that made the protocol an exchange.** A process killed leaves
    // its last heartbeat behind, fresh for the rest of the staleness window —
    // so a launch inside those seconds would exit as a second instance with
    // nobody to raise, which presents as an application that will not start.
    // The member's remedy is to try again, and trying again is the gesture that
    // keeps failing.
    //
    // Found by killing one and relaunching rather than by review.
    let dir = Dir::new("heartbeat-without-a-holder");
    let workspace = Workspace::at(dir.0.clone());
    std::fs::create_dir_all(dir.0.join("running")).expect("the claim directory");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock")
        .as_millis() as i64;
    // As fresh as a heartbeat gets, and with nothing behind it.
    std::fs::write(dir.0.join("running").join("heartbeat"), now.to_string()).expect("a beat");
    std::fs::write(dir.0.join("running").join("owner"), "12345").expect("a token");

    assert!(
        matches!(workspace.hold_application(), Launch::First(_)),
        "a heartbeat nobody stands behind does not make this a second launch"
    );
    assert!(
        !dir.0.join("running").join("raise").exists(),
        "and the unanswered ask is cleared rather than left for the next launch to find"
    );
}

#[test]
fn a_claim_left_by_a_crash_is_taken_over_rather_than_waited_on_forever() {
    use kols_node::workspace::Launch;

    // A crash leaves the claim behind: `Drop` does not run, and the heartbeat
    // stops. The next launch has to become the application rather than
    // reporting that one is already running — which would make a crash
    // permanent.
    let dir = Dir::new("stale-application");
    let workspace = Workspace::at(dir.0.clone());
    std::fs::create_dir_all(dir.0.join("running")).expect("the claim directory");
    // Older than the staleness window, which is what a stopped heartbeat looks
    // like however the process stopped.
    let long_ago = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock")
        .as_millis() as i64
        - 60_000;
    std::fs::write(dir.0.join("running").join("heartbeat"), long_ago.to_string())
        .expect("a stale beat");
    std::fs::write(dir.0.join("running").join("owner"), "12345").expect("somebody else's token");

    assert!(
        matches!(workspace.hold_application(), Launch::First(_)),
        "a stale claim is taken over"
    );
}

#[test]
fn the_holder_stops_answering_once_another_process_has_taken_the_claim() {
    use kols_node::workspace::Launch;

    // The suspend case: this process slept past the staleness window, another
    // launch legitimately became the application, and this one wakes up. It
    // must stop answering for the installation — raising *its* window would
    // put the wrong one in front of somebody — but it deliberately does not
    // stop, because the correctness claim is the node's and closing a member's
    // windows under them is a larger harm than two window sets.
    let dir = Dir::new("application-taken-over");
    let workspace = Workspace::at(dir.0.clone());
    let Launch::First(first) = workspace.hold_application() else {
        panic!("the first launch is the application");
    };

    // Somebody else's token, as a takeover would leave it.
    std::fs::write(dir.0.join("running").join("owner"), "999").expect("a new owner");
    assert!(!first.beat(), "the claim is no longer this process's");

    // And on the way out it leaves the successor's claim alone, which is the
    // failure the node claim's `Drop` check exists to prevent.
    drop(first);
    assert!(
        dir.0.join("running").join("owner").is_file(),
        "the successor's claim survives the loser's drop"
    );
}
