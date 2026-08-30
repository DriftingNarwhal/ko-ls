//! Three installs, and the question two of them cannot answer.
//!
//! # What this covers that `two_nodes` cannot
//!
//! With two nodes, every piece of content has exactly one other place to come
//! from — its author. So a network that only ever served content from whoever
//! wrote it would pass every test in that file, and would be a network that
//! stops working the moment somebody closes their laptop.
//!
//! Storage §4.2 says a node that fetched a chunk becomes a source for it. The
//! consequence is the property this file exists for: **a member can read what
//! another member wrote while that member is offline, because a third one kept
//! a copy.** That is the difference between a chat network and a chat network
//! that requires everybody to be online at once.
//!
//! Found in the field before it was found here: three people across three
//! networks, and the third saw nothing the second had written until the second
//! came back online — though the first had been connected to both the whole
//! time and held every byte.

mod common;
use common::{field, ok, patience, serve, serve_sealing, Home};
use std::time::Duration;

/// Held for the whole of each test in this file, so only one runs at a time.
///
/// # Why these cannot run beside each other
///
/// Each spawns three or four daemons that sign, verify and encrypt, where
/// `two_nodes` spawns two. Run in parallel this file alone puts a dozen up at
/// once, and they starve each other rather than the machine being slow: every
/// test here passes on its own and the full workspace run failed two of them,
/// which is the signature of contention and reads exactly like the feature
/// under test being broken.
///
/// Longer deadlines were tried first and are the wrong lever — they make the
/// suite slower and still flaky, because the contention scales with how many
/// daemons are up rather than with how long each is given. `patience` scales
/// with how many cores there are and cannot know how many daemons the suite
/// decided to start. This bounds the second number.
///
/// Poisoning is ignored deliberately: a panic in one of these is a test
/// failure that has already been reported, and turning it into a failure of
/// every later test would hide which one broke.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn exclusively() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(|held| held.into_inner())
}

/// Alice founds and stays up throughout. Bob writes and leaves. Carol arrives
/// afterwards and never meets Bob at all.
#[test]
fn a_member_reads_what_an_offline_member_wrote_because_a_third_kept_it() {
    let _one_at_a_time = exclusively();
    let alice = Home::new("relay-alice");
    let bob = Home::new("relay-bob");
    let carol = Home::new("relay-carol");

    let network = field(&ok(&alice, &["init", "the workshop"]), "network   ");
    let bob_id = field(&ok(&bob, &["attach", &network]), "kols admit ");
    let carol_id = field(&ok(&carol, &["attach", &network]), "kols admit ");
    ok(&alice, &["admit", &bob_id]);
    ok(&alice, &["admit", &carol_id]);

    let mut alice_node = serve(&alice, 45301, None);
    let listening = alice_node.wait_for("listening", patience(Duration::from_secs(20)));
    let alice_address = field(&listening, "listening ");
    ok(&alice, &["channel", "create", "general", "--topic", "shared"]);
    alice_node.wait_for("picked up", patience(Duration::from_secs(20)));

    // Bob joins, writes, and is picked up by Alice — who now holds his segment
    // as an ordinary fetched object, not as a special case.
    let mut bob_node = serve(&bob, 45302, Some(&alice_address));
    bob_node.wait_for("keyed into this network", patience(Duration::from_secs(90)));
    ok(&bob, &["post", "general", "bob wrote this before leaving"]);
    bob_node.wait_for("broadcast 1 record", patience(Duration::from_secs(20)));
    alice_node.wait_for("learned 1 record", patience(Duration::from_secs(90)));
    // Alice reads it the instant it arrives live, and cannot pass it on for
    // some seconds yet: displaying a record and holding the object it came out
    // of are separate, and only the second makes her a source. Killing Bob at
    // the "learned" line measures that gap rather than anything about the
    // network — the first version of this test did, and reported a race of its
    // own making as a defect.
    std::thread::sleep(patience(Duration::from_secs(20)));

    // Alice can read it, which is the part that already worked and is not the
    // property under test — holding the record is not the same as being able
    // to serve the object it came out of.
    let alices_view = ok(&alice, &["read", "general"]);
    assert!(
        alices_view.contains("bob wrote this before leaving"),
        "alice should hold bob's message before he leaves:\n{alices_view}"
    );

    // And leaves. From here nothing Bob holds is reachable; Alice is the only
    // node that has ever seen his segment.
    drop(bob_node);

    // Confirmed rather than assumed: Bob's message really did become a segment
    // in his log before he left. Publishing and broadcasting happen on the same
    // tick, so killing a daemon straight after a broadcast can outrun the
    // publish — and a test that did would be reporting its own race as a defect
    // in the network. This run is given no peer, so it proves the segment
    // exists without handing Alice a second chance to fetch it.
    let mut bob_alone = serve(&bob, 45304, None);
    bob_alone.wait_for(
        "published 1 segment(s) from this node",
        patience(Duration::from_secs(20)),
    );
    drop(bob_alone);

    // Carol is pointed at Alice and only Alice. She has never heard of Bob's
    // address and he is not listening on it anyway.
    let mut carol_node = serve(&carol, 45303, Some(&alice_address));
    carol_node.wait_for("keyed into this network", patience(Duration::from_secs(90)));
    carol_node.wait_for("learned 1 record", patience(Duration::from_secs(120)));

    let read = ok(&carol, &["read", "general"]);
    assert!(
        read.contains("bob wrote this before leaving"),
        "carol must read bob's message from alice while bob is offline; \
         a network where content dies with its author's uptime is not one:\n{read}"
    );

    let read = ok(&carol, &["read", "general"]);
    assert!(
        read.contains("bob wrote this before leaving"),
        "carol must read bob's message from alice while bob is offline; \
         a network where content dies with its author's uptime is not one:\n{read}"
    );
}

/// The same journey with the live path carrying nothing.
///
/// Discriminates between two explanations of the test above: that a node cannot
/// serve another author's content at all, or that a node which received that
/// content *live* never obtained the object it came out of and so has nothing
/// to serve. With gossip off, Alice has no way to hold Bob's message except by
/// fetching his segment — so if she can pass it on here and not above, the live
/// path is what is short-circuiting her.
#[test]
fn the_same_relay_works_when_nothing_arrived_live() {
    let _one_at_a_time = exclusively();
    let alice = Home::new("nolive-alice");
    let bob = Home::new("nolive-bob");
    let carol = Home::new("nolive-carol");

    let network = field(&ok(&alice, &["init", "the workshop"]), "network   ");
    let bob_id = field(&ok(&bob, &["attach", &network]), "kols admit ");
    let carol_id = field(&ok(&carol, &["attach", &network]), "kols admit ");
    ok(&alice, &["admit", &bob_id]);
    ok(&alice, &["admit", &carol_id]);

    let mut alice_node = serve_sealing(&alice, 45311, None, None, false);
    let listening = alice_node.wait_for("listening", patience(Duration::from_secs(20)));
    let alice_address = field(&listening, "listening ");
    ok(&alice, &["channel", "create", "general", "--topic", "shared"]);
    alice_node.wait_for("picked up", patience(Duration::from_secs(20)));

    let mut bob_node = serve_sealing(&bob, 45312, Some(&alice_address), None, false);
    bob_node.wait_for("keyed into this network", patience(Duration::from_secs(90)));
    ok(&bob, &["post", "general", "bob wrote this before leaving"]);
    alice_node.wait_for("learned 1 record", patience(Duration::from_secs(90)));
    drop(bob_node);

    let mut carol_node = serve_sealing(&carol, 45313, Some(&alice_address), None, false);
    carol_node.wait_for("keyed into this network", patience(Duration::from_secs(90)));
    carol_node.wait_for("learned 1 record", patience(Duration::from_secs(120)));

    let read = ok(&carol, &["read", "general"]);
    assert!(
        read.contains("bob wrote this before leaving"),
        "carol must read bob's message from alice:\n{read}"
    );
}

/// The same journey, with the keeper restarted in between.
///
/// A node announces a chunk when it *receives* one (Storage §4.2), and the set
/// of what it has announced lives in memory. So a node that holds another
/// member's content from a previous run holds it silently: it can read it, and
/// nobody can discover it there. Every close and reopen makes a node's whole
/// stored contribution invisible until each object is fetched again.
///
/// This is the field report the file header describes, in its most likely
/// shape — the people involved had been closing and reopening clients all
/// session.
#[test]
fn a_restarted_keeper_still_serves_what_it_kept() {
    let _one_at_a_time = exclusively();
    let alice = Home::new("restart-alice");
    let bob = Home::new("restart-bob");
    let carol = Home::new("restart-carol");

    let network = field(&ok(&alice, &["init", "the workshop"]), "network   ");
    let bob_id = field(&ok(&bob, &["attach", &network]), "kols admit ");
    let carol_id = field(&ok(&carol, &["attach", &network]), "kols admit ");
    ok(&alice, &["admit", &bob_id]);
    ok(&alice, &["admit", &carol_id]);

    let mut alice_node = serve(&alice, 45331, None);
    let listening = alice_node.wait_for("listening", patience(Duration::from_secs(20)));
    let alice_address = field(&listening, "listening ");
    ok(&alice, &["channel", "create", "general", "--topic", "shared"]);
    alice_node.wait_for("picked up", patience(Duration::from_secs(20)));

    let mut bob_node = serve(&bob, 45332, Some(&alice_address));
    bob_node.wait_for("keyed into this network", patience(Duration::from_secs(90)));
    ok(&bob, &["post", "general", "bob wrote this before leaving"]);
    alice_node.wait_for("learned 1 record", patience(Duration::from_secs(90)));
    // Alice reads it the instant it arrives live, and cannot pass it on for
    // some seconds yet: displaying a record and holding the object it came out
    // of are separate, and only the second makes her a source. Killing Bob at
    // the "learned" line measures that gap rather than anything about the
    // network — the first version of this test did, and reported a race of its
    // own making as a defect.
    std::thread::sleep(patience(Duration::from_secs(20)));
    drop(bob_node);

    // Alice really did keep it, and can still read it after the restart. This
    // is the half that works, and the half a user sees — which is why the
    // other half went unnoticed.
    drop(alice_node);
    let mut alice_again = serve(&alice, 45333, None);
    let relisten = alice_again.wait_for("listening", patience(Duration::from_secs(20)));
    let alice_address = field(&relisten, "listening ");
    // Said, so that a pass means the restart put back what it kept rather than
    // that Carol found the content somewhere else. There is nowhere else here —
    // Bob is gone and Carol has nothing — but a test that would still pass if
    // the mechanism were removed is not testing the mechanism.
    assert!(
        relisten.contains("restored") && relisten.contains("kept for other members"),
        "alice should say what she put back on restarting:\n{relisten}"
    );
    // Both halves, named. A pointer whose chunks were lost names a segment that
    // cannot be assembled, and chunks whose pointer was lost are bytes nobody
    // can ask for by name — so "some of it came back" is not the property.
    assert!(
        !relisten.contains("and 0 pointer(s)") && !relisten.contains("restored  0 chunk(s)"),
        "both chunks and pointers must come back, not one of them:\n{relisten}"
    );
    let alices_view = ok(&alice, &["read", "general"]);
    assert!(
        alices_view.contains("bob wrote this before leaving"),
        "alice should still hold bob's message after restarting:\n{alices_view}"
    );

    let mut carol_node = serve(&carol, 45334, Some(&alice_address));
    carol_node.wait_for("keyed into this network", patience(Duration::from_secs(90)));
    carol_node.wait_for("learned 1 record", patience(Duration::from_secs(120)));

    let read = ok(&carol, &["read", "general"]);
    assert!(
        read.contains("bob wrote this before leaving"),
        "carol must read bob's message from a restarted alice; what a node \
         kept is not kept if restarting makes it undiscoverable:\n{read}"
    );
}
