//! Two `kols` installs reaching each other, through the actual binaries.
//!
//! # What this covers that nothing else does
//!
//! `kols-net`'s tests move chunks between two `MemberNode`s in one process with
//! a hardcoded key on both sides. This is the same journey with nothing shared:
//! two stores, two seeds, two processes, and a joiner who starts knowing only a
//! network id. Everything between — admission, epoch-key delivery, pointer sync,
//! the two-round fetch, unwrapping a DEK under the right rotation — has to work
//! for a single message to arrive.
//!
//! Four bugs found by running exactly this, each invisible to every other test:
//! a joiner could not advertise before syncing and so could never sync; a fetch
//! was requested once when it needs two rounds, so every segment stayed
//! half-fetched; only the newest epoch key was kept, so content written before
//! the joiner arrived fetched perfectly and decrypted never; and the capability
//! ledger was never re-exchanged, so a joiner who advertised after being
//! admitted stayed unrankable as a source forever.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Home(PathBuf);

impl Home {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("kols-2n-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A `kols serve` that is killed when the test ends, however it ends.
struct Daemon {
    child: Child,
    log: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.log);
    }
}

impl Daemon {
    fn output(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Waits for `needle` to appear, or gives up and shows what did appear.
    fn wait_for(&self, needle: &str, within: Duration) -> String {
        let deadline = Instant::now() + within;
        loop {
            let seen = self.output();
            if seen.contains(needle) {
                return seen;
            }
            assert!(
                Instant::now() < deadline,
                "waited {within:?} for {needle:?}, saw:\n{seen}"
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

fn run(home: &Home, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_kols"))
        .arg("--home")
        .arg(home.path())
        .args(args)
        .output()
        .expect("the binary runs")
}

fn ok(home: &Home, args: &[&str]) -> String {
    let out = run(home, args);
    assert!(
        out.status.success(),
        "`kols {}` failed:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn serve(home: &Home, port: u16, peer: Option<&str>) -> Daemon {
    serve_tuned(home, port, peer, None, true, None)
}

fn serve_sealing(
    home: &Home,
    port: u16,
    peer: Option<&str>,
    seal_bytes: Option<usize>,
    live: bool,
) -> Daemon {
    serve_tuned(home, port, peer, seal_bytes, live, None)
}

/// `serve`, with an optional segment-seal threshold.
///
/// Sealing at `design/01` §3.1's real 4 MiB target would need a test to write
/// four megabytes of chat to produce a single boundary. The threshold is local
/// publishing tuning rather than a validity rule — a reader accepts whatever
/// boundaries an author chose — so a small one here produces history that is
/// ordinary in every respect except how quickly it reaches the second segment.
fn serve_tuned(
    home: &Home,
    port: u16,
    peer: Option<&str>,
    seal_bytes: Option<usize>,
    live: bool,
    live_window_millis: Option<i64>,
) -> Daemon {
    let log = std::env::temp_dir().join(format!("kols-2n-{port}-{}.log", std::process::id()));
    let file = std::fs::File::create(&log).expect("a log file");
    let mut command = Command::new(env!("CARGO_BIN_EXE_kols"));
    command
        .arg("--home")
        .arg(home.path())
        .args(["serve", "--listen"])
        .arg(format!("/ip4/127.0.0.1/tcp/{port}"));
    if let Some(peer) = peer {
        command.args(["--peer", peer]);
    }
    if let Some(bytes) = seal_bytes {
        command.args(["--seal-bytes", &bytes.to_string()]);
    }
    if !live {
        command.arg("--no-live");
    }
    if let Some(window) = live_window_millis {
        command.args(["--live-window-millis", &window.to_string()]);
    }
    let child = command
        .stdout(Stdio::from(file))
        .stderr(Stdio::null())
        .spawn()
        .expect("serve starts");
    Daemon { child, log }
}

fn field(output: &str, prefix: &str) -> String {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix(prefix))
        .unwrap_or_else(|| panic!("no {prefix:?} in:\n{output}"))
        .trim()
        .to_owned()
}

#[test]
fn a_joiner_is_admitted_keyed_and_reads_what_was_written_before_they_arrived() {
    let alice = Home::new("alice");
    let bob = Home::new("bob");

    let created = ok(&alice, &["init", "the workshop"]);
    let network = field(&created, "network   ");

    // Bob starts knowing one thing: the network id. His identity in it is
    // derived from that plus his own seed (Core §1.2), so it exists before
    // anybody has heard of him — which is what lets him be admitted by name.
    let attached = ok(&bob, &["attach", &network]);
    let bob_identity = field(&attached, "kols admit ");
    ok(&alice, &["admit", &bob_identity]);

    // Alice's daemon keys the network on first run, because an MLS group is live
    // state no one-shot command can hold.
    let alice_node = serve(&alice, 45101, None);
    let listening = alice_node.wait_for("listening", Duration::from_secs(20));
    let address = field(&listening, "listening ");
    assert!(
        listening.contains("keyed     this network"),
        "the founder's first serve should key the network:\n{listening}"
    );

    // Written before Bob has ever connected, and under the epoch that exists
    // now — which is not the epoch that will exist once he is keyed in.
    ok(&alice, &["channel", "create", "general", "--topic", "shared"]);
    ok(&alice, &["post", "general", "written before bob arrived"]);
    alice_node.wait_for("picked up", Duration::from_secs(20));

    let bob_node = serve(&bob, 45102, Some(&address));
    let keyed = bob_node.wait_for("keyed into this network", Duration::from_secs(45));
    assert!(
        keyed.contains("learned 3 governance entr"),
        "bob should learn genesis, the channel and his own admission:\n{keyed}"
    );
    bob_node.wait_for("learned 1 record", Duration::from_secs(45));

    // The whole point: content that predates the joiner, readable by them.
    let read = ok(&bob, &["read", "general"]);
    assert!(
        read.contains("written before bob arrived"),
        "bob should read what alice wrote before he joined:\n{read}"
    );

    let listed = ok(&bob, &["channel", "list"]);
    assert!(listed.contains("#general"), "{listed}");
}

#[test]
fn a_reply_travels_back_and_both_sides_agree_on_the_order() {
    let alice = Home::new("alice-reply");
    let bob = Home::new("bob-reply");

    let created = ok(&alice, &["init", "duplex"]);
    let network = field(&created, "network   ");
    let attached = ok(&bob, &["attach", &network]);
    ok(&alice, &["admit", &field(&attached, "kols admit ")]);

    let alice_node = serve(&alice, 45103, None);
    let address = field(
        &alice_node.wait_for("listening", Duration::from_secs(20)),
        "listening ",
    );
    ok(&alice, &["channel", "create", "general"]);
    ok(&alice, &["post", "general", "first from alice"]);
    alice_node.wait_for("picked up", Duration::from_secs(20));

    let bob_node = serve(&bob, 45104, Some(&address));
    bob_node.wait_for("learned 1 record", Duration::from_secs(45));

    // Bob replies with **both daemons still up and nothing restarted**, which is
    // the property under test rather than an incidental detail. Everything here
    // is pull-based — the governance log, the capability ledger and pointers
    // alike — so each side has to keep asking; nothing is pushed.
    //
    // This failed for a day, and the reason is worth keeping: source selection
    // drops a holder that has not advertised capacity, and a joiner advertises
    // only once admitted, which is *after* the ledger exchange that ran when it
    // connected. Without re-asking for the ledger, Bob stayed permanently
    // unrankable and every fetch from him failed with the chunk simply never
    // arriving — the pointer and its wrapping having arrived perfectly.
    ok(&bob, &["post", "general", "then from bob"]);
    alice_node.wait_for("learned 1 record", Duration::from_secs(60));

    let read = ok(&alice, &["read", "general"]);
    assert!(read.contains("first from alice"), "{read}");
    assert!(read.contains("then from bob"), "{read}");
    assert!(
        read.find("first from alice") < read.find("then from bob"),
        "both authors' records merge in HLC order, not arrival order:\n{read}"
    );
    assert!(
        read.contains("from 2 author(s)"),
        "the view should span both logs:\n{read}"
    );
}

#[test]
fn a_founder_can_still_key_somebody_in_after_restarting() {
    // Core §3.3.1. An MLS group is live state, and openmls keeps it in memory by
    // default — so before it was persisted, a founder who restarted kept their
    // epoch keys, could still read, and could never welcome anybody again. The
    // network was one process exit away from admitting nobody, forever, with no
    // symptom until the next person tried to join.
    let alice = Home::new("restart-alice");
    let bob = Home::new("restart-bob");

    let created = ok(&alice, &["init", "durable"]);
    let network = field(&created, "network   ");

    // First run: creates the group and the network's first epoch key.
    let first = serve(&alice, 45111, None);
    let opened = first.wait_for("keyed     this network", Duration::from_secs(20));
    assert!(
        opened.contains("epoch     held, and this node can key others in"),
        "{opened}"
    );
    ok(&alice, &["channel", "create", "general"]);
    ok(&alice, &["post", "general", "written before the restart"]);
    first.wait_for("picked up", Duration::from_secs(20));
    drop(first);

    // Second run: a different process, which must come back holding the group
    // rather than only the key it can read with.
    let second = serve(&alice, 45112, None);
    let restarted = second.wait_for("listening", Duration::from_secs(20));
    assert!(
        restarted.contains("group     restored from the last run"),
        "the group must survive a restart:\n{restarted}"
    );
    assert!(
        restarted.contains("epoch     held, and this node can key others in"),
        "and the restarted node must still be able to key somebody in:\n{restarted}"
    );
    let address = field(&restarted, "listening ");

    // The proof: somebody who has never been seen before is admitted and keyed
    // in by the *restarted* founder, and reads what predates them.
    let attached = ok(&bob, &["attach", &network]);
    ok(&alice, &["admit", &field(&attached, "kols admit ")]);

    let bob_node = serve(&bob, 45113, Some(&address));
    bob_node.wait_for("keyed into this network", Duration::from_secs(45));
    bob_node.wait_for("learned 1 record", Duration::from_secs(45));

    let read = ok(&bob, &["read", "general"]);
    assert!(
        read.contains("written before the restart"),
        "bob should read what was written before the founder restarted:\n{read}"
    );
}

#[test]
fn a_revocation_rotates_the_epoch_and_leaves_the_network_working() {
    // Revocation is split across two processes on purpose. `kols revoke` writes
    // the membership removal; the daemon rotates the epoch to exclude them,
    // because rotating needs the live MLS group only it holds — and Core §3.3
    // requires that order anyway, since a rotation minted while somebody is
    // still a member produces a key they remain entitled to, and §3.1 says a
    // key cannot be un-known afterwards.
    //
    // Most of what this asserts is that nothing *else* broke, because the first
    // working revocation broke two things at once: the rotation landed as a
    // sibling of a concurrently-written channel definition, and fork-choice
    // voided the channel; and the author's own DEK stopped unwrapping, its
    // wrapping being under the epoch that had just been superseded.
    let alice = Home::new("revoke-alice");
    let bob = Home::new("revoke-bob");

    let created = ok(&alice, &["init", "revocable"]);
    let network = field(&created, "network   ");
    let attached = ok(&bob, &["attach", &network]);
    let bob_identity = field(&attached, "kols admit ");
    ok(&alice, &["admit", &bob_identity]);

    let alice_node = serve(&alice, 45121, None);
    let address = field(
        &alice_node.wait_for("listening", Duration::from_secs(20)),
        "listening ",
    );
    ok(&alice, &["channel", "create", "general"]);
    ok(&alice, &["post", "general", "before the revocation"]);
    alice_node.wait_for("picked up", Duration::from_secs(20));

    let bob_node = serve(&bob, 45122, Some(&address));
    bob_node.wait_for("keyed into this network", Duration::from_secs(45));
    bob_node.wait_for("learned 1 record", Duration::from_secs(45));
    drop(bob_node);

    //
    // Every wrapping is compared, not one of them. A channel holds several DEKs
    // now — one per segment plus the head index (`design/01` §3.1.0) — and
    // reading whichever the directory happened to list first asserted on an
    // arbitrary one of them.
    let wrappings = |home: &Home| {
        let mut all: Vec<Vec<u8>> = std::fs::read_dir(home.path().join("deks"))
            .expect("a wrapping directory")
            .map(|entry| std::fs::read(entry.unwrap().path()).unwrap())
            .collect();
        all.sort();
        assert!(!all.is_empty(), "alice must hold at least one wrapping");
        all
    };

    let before = field(&ok(&alice, &["whoami"]), "epoch    ");

    // Captured *before* the rotation, which is what makes this deterministic.
    // A read refreshes a wrapping to the current epoch on the way through, so
    // sampling after the rotation races the daemon's next tick: if it has
    // already refreshed, nothing changes afterwards and the assertion is simply
    // wrong rather than failing honestly.
    let stale = wrappings(&alice);

    let removed = ok(&alice, &["revoke", &bob_identity]);
    assert!(removed.contains("removed"), "{removed}");
    // The command says plainly it has done half the job: until the daemon
    // rotates, the removed member still decrypts newly published content.
    assert!(removed.contains("rotates the epoch"), "{removed}");

    alice_node.wait_for("rotated the epoch to exclude", Duration::from_secs(40));
    let after = field(&ok(&alice, &["whoami"]), "epoch    ");
    assert_ne!(
        before, after,
        "a removal must advance the epoch, or the removed member reads on"
    );

    // The channel survives. A rotation appended as a sibling of the channel
    // definition forks the log, and fork-choice voids a branch — correct
    // protocol behaviour which, the first time it happened here, deleted the
    // channel without a word.
    let listed = ok(&alice, &["channel", "list"]);
    assert!(listed.contains("#general"), "the channel must survive:\n{listed}");

    // And the network still works: posting needs the author's own DEK, whose
    // wrapping is now under a superseded epoch key.
    //
    // The wrapping must also be *refreshed* to the current epoch on the way
    // through. Without that, every future read falls back through every key the
    // network has ever rotated through — measured at ~0.72ms per thousand keys,
    // paid per unwrap, on a list that grows with every membership change.
    ok(&alice, &["post", "general", "after the revocation"]);
    alice_node.wait_for("picked up", Duration::from_secs(30));

    assert_ne!(
        wrappings(&alice),
        stale,
        "a wrapping opened under a superseded key must be re-wrapped under the current one"
    );
    let read = ok(&alice, &["read", "general"]);
    assert!(read.contains("before the revocation"), "{read}");
    assert!(read.contains("after the revocation"), "{read}");
}

#[test]
fn revoking_a_non_member_is_refused() {
    // Removing somebody who is not there would append a governance entry that
    // changes nothing and rotate the epoch for no reason, which costs every
    // member a re-wrap.
    let alice = Home::new("revoke-refuse");
    let stranger = Home::new("revoke-stranger");

    let created = ok(&alice, &["init", "careful"]);
    let network = field(&created, "network   ");
    let stranger_identity = field(&ok(&stranger, &["attach", &network]), "kols admit ");

    let out = run(&alice, &["revoke", &stranger_identity]);
    assert!(!out.status.success(), "a non-member cannot be removed");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not a member"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_node_offline_across_a_rotation_catches_up_and_can_still_read() {
    // Every rotation is a governance entry carrying an MLS commit (Core §3.3),
    // and applying those in order is how a member derives the keys it missed.
    // So absence costs nothing — *provided the node catches up*. Before it did,
    // a node that was offline across a rotation held only the keys it produced
    // or was handed directly.
    //
    // The gap was invisible in every earlier test, and the reason is worth
    // knowing: an object keeps its DEK for life, so the absent node could still
    // read appends to logs it already knew. It took a *new* object — one whose
    // wrapping is under an epoch it never derived — to show anything wrong, and
    // then it presented as content that fetched perfectly and would not open.
    let alice = Home::new("catchup-alice");
    let bob = Home::new("catchup-bob");
    let carol = Home::new("catchup-carol");

    let created = ok(&alice, &["init", "catching up"]);
    let network = field(&created, "network   ");

    let bob_identity = field(&ok(&bob, &["attach", &network]), "kols admit ");
    ok(&alice, &["admit", &bob_identity]);

    let alice_node = serve(&alice, 45131, None);
    let address = field(
        &alice_node.wait_for("listening", Duration::from_secs(20)),
        "listening ",
    );
    ok(&alice, &["channel", "create", "general"]);

    // Bob joins and is keyed, then goes away.
    let bob_node = serve(&bob, 45132, Some(&address));
    bob_node.wait_for("keyed into this network", Duration::from_secs(45));
    drop(bob_node);

    // While Bob is offline, Carol is admitted and keyed — which rotates the
    // epoch — and then writes a log Bob has never seen.
    let carol_identity = field(&ok(&carol, &["attach", &network]), "kols admit ");
    ok(&alice, &["admit", &carol_identity]);
    let carol_node = serve(&carol, 45133, Some(&address));
    carol_node.wait_for("keyed into this network", Duration::from_secs(45));
    ok(&carol, &["post", "general", "written while bob was away"]);
    // Waited for on *Alice's* side, because Carol's daemon prints nothing when
    // it publishes — "picked up" is about governance entries, and a post is a
    // record. Alice learning it is the signal that Carol's segment is actually
    // out there to be fetched.
    alice_node.wait_for("learned 1 record", Duration::from_secs(60));

    // Bob returns. Carol's log is a new object, wrapped under the epoch that
    // admitting her produced — one Bob was not present for and must derive from
    // the commit in the log.
    let bob_again = serve(&bob, 45134, Some(&address));
    bob_again.wait_for("caught up on", Duration::from_secs(45));
    bob_again.wait_for("learned 1 record", Duration::from_secs(60));

    let read = ok(&bob, &["read", "general"]);
    assert!(
        read.contains("written while bob was away"),
        "bob must derive the epoch he missed and read what was written then:\n{read}"
    );
}

#[test]
fn a_record_goes_out_live_and_arrives_exactly_once() {
    // Spec 07 §6.1's two halves, and precisely not more than it promises.
    //
    // An earlier version of this test waited for the record to reach Bob *live*
    // and failed, which was the test being wrong rather than the path: on
    // loopback the durable path is milliseconds too, so whichever arrives first
    // is a race, and §6.1 guarantees only that the record arrives. Demanding
    // live win it would have been asserting something the design explicitly
    // declines to offer.
    //
    // So what is asserted is what is actually promised: the record genuinely
    // goes out over the live path, it arrives, and — because both paths carry
    // the identical canonical bytes — it lands exactly once however many ways it
    // came.
    let alice = Home::new("live-alice");
    let bob = Home::new("live-bob");

    let created = ok(&alice, &["init", "lively"]);
    let network = field(&created, "network   ");
    let attached = ok(&bob, &["attach", &network]);
    ok(&alice, &["admit", &field(&attached, "kols admit ")]);

    let alice_node = serve(&alice, 45141, None);
    let address = field(
        &alice_node.wait_for("listening", Duration::from_secs(20)),
        "listening ",
    );
    ok(&alice, &["channel", "create", "general"]);

    let bob_node = serve(&bob, 45142, Some(&address));
    bob_node.wait_for("keyed into this network", Duration::from_secs(45));

    ok(&alice, &["post", "general", "sent while you were watching"]);

    // A publish only succeeds once somebody is subscribed to the topic, so this
    // line is the live path demonstrably working rather than merely attempted.
    alice_node.wait_for("broadcast 1 record(s) live", Duration::from_secs(45));

    // Bob ends up with it, whichever path won.
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let read = ok(&bob, &["read", "general"]);
        if read.contains("sent while you were watching") {
            break;
        }
        assert!(Instant::now() < deadline, "the record must arrive:\n{read}");
        std::thread::sleep(Duration::from_millis(300));
    }

    // Exactly once. Records are content-addressed, so a copy from each path is
    // one record rather than two — §6.1's idempotence requirement, and the
    // reason gossipsub is configured to identify messages by content hash
    // instead of by sender and sequence.
    std::thread::sleep(Duration::from_secs(6));
    let again = ok(&bob, &["read", "general"]);
    assert_eq!(
        again.matches("sent while you were watching").count(),
        1,
        "arriving by both paths must yield one record:\n{again}"
    );
}

#[test]
fn history_still_converges_with_the_live_path_carrying_nothing() {
    // §6.1 requires conformance be testable with gossip disabled: "a client with
    // gossip disabled is slower and completely correct". `--no-live` is that
    // switch, and it turns off both halves — a node that published but never
    // subscribed would be neither on nor off. Both sides run without it here, so
    // no payload can reach Bob live and everything he ends up with came through
    // the durable path alone.
    //
    // This used to rely on never overlapping the two daemons instead. That was
    // weaker than it looked: an author retries a record that failed to publish,
    // so a backlog goes out the moment a peer subscribes, and the arrangement
    // held only while there was a single record for the durable path to win the
    // race on.
    let alice = Home::new("nolive-alice");
    let bob = Home::new("nolive-bob");

    let created = ok(&alice, &["init", "quiet"]);
    let network = field(&created, "network   ");
    let attached = ok(&bob, &["attach", &network]);
    ok(&alice, &["admit", &field(&attached, "kols admit ")]);

    let alice_node = serve_sealing(&alice, 45143, None, None, false);
    let address = field(
        &alice_node.wait_for("listening", Duration::from_secs(20)),
        "listening ",
    );
    ok(&alice, &["channel", "create", "general"]);
    ok(&alice, &["post", "general", "written with nobody listening"]);
    alice_node.wait_for("picked up", Duration::from_secs(20));

    let bob_node = serve_sealing(&bob, 45144, Some(&address), None, false);
    bob_node.wait_for("keyed into this network", Duration::from_secs(45));
    bob_node.wait_for("learned 1 record", Duration::from_secs(45));

    let output = bob_node.output();
    assert!(
        !output.contains("learned 1 record(s) live"),
        "nothing could have arrived live here, so the durable path is what is under test:\n{output}"
    );

    let read = ok(&bob, &["read", "general"]);
    assert!(
        read.contains("written with nobody listening"),
        "the durable path alone must carry it:\n{read}"
    );
}

#[test]
fn a_joiner_walks_back_through_sealed_segments_to_read_the_start() {
    let alice = Home::new("alice-backfill");
    let bob = Home::new("bob-backfill");

    let created = ok(&alice, &["init", "long history"]);
    let network = field(&created, "network   ");
    let attached = ok(&bob, &["attach", &network]);
    ok(&alice, &["admit", &field(&attached, "kols admit ")]);

    // A threshold small enough that ordinary chat crosses it repeatedly, so this
    // writes a chain rather than the single ever-growing segment the daemon
    // produced before sealing existed.
    let alice_node = serve_sealing(&alice, 45109, None, Some(1024), true);
    let address = field(
        &alice_node.wait_for("listening", Duration::from_secs(20)),
        "listening ",
    );
    ok(&alice, &["channel", "create", "general"]);

    // Written with Bob's daemon down, so none of it can reach him live and the
    // durable path is the only way any of it arrives.
    const MESSAGES: usize = 30;
    for n in 0..MESSAGES {
        ok(&alice, &["post", "general", &format!("message {n}")]);
    }
    alice_node.wait_for("picked up", Duration::from_secs(30));

    // Bob runs with the live path off (spec 07 §6.1), so every record he ends
    // up with came through the durable path. Without this Alice re-broadcasts
    // her whole backlog the moment he subscribes and he learns all thirty
    // records live, which reads like a pass and tests nothing.
    let bob_node = serve_sealing(&bob, 45110, Some(&address), None, false);

    // The property. Without the walk Bob absorbs the segment the pointer names
    // and stops, which is the tail of the conversation — he would still read
    // *something*, which is exactly why this asserts on the earliest message
    // rather than on any message at all.
    bob_node.wait_for("backfilled", Duration::from_secs(90));

    // Polled rather than read once. A hop the local store cannot already answer
    // costs a tick, so a chain several seals long converges over several ticks —
    // reading at the first "backfilled" catches the walk partway and reports a
    // missing history that is merely a slow one.
    let deadline = Instant::now() + Duration::from_secs(120);
    let read = loop {
        let read = ok(&bob, &["read", "general"]);
        let missing: Vec<_> = (0..MESSAGES)
            .filter(|n| !read.contains(&format!("message {n}")))
            .collect();
        if missing.is_empty() {
            break read;
        }
        assert!(
            Instant::now() < deadline,
            "bob never reached {missing:?}:\n{read}\n\nhis daemon said:\n{}",
            bob_node.output()
        );
        std::thread::sleep(Duration::from_millis(500));
    };

    // Named explicitly because it is the one the head segment cannot carry: it
    // is several seals back, so it can only have come from a chain walk.
    assert!(read.contains("message 0"), "{read}");
}

#[test]
fn history_is_not_re_broadcast_live_to_a_peer_that_arrives_later() {
    // The live path is a latency optimisation over records being written now
    // (spec 07 §6.1). A failed publish is retried, because a record written a
    // moment before a peer subscribed should still go out — but unbounded, that
    // retry set is *everything the node ever wrote*, so an author's whole
    // history goes over gossipsub the instant anybody subscribes. §6.1 says
    // nothing may depend on the live path; a backlog delivered over it is the
    // opposite failure, the durable path being the one nothing depends on.
    let alice = Home::new("stale-alice");
    let bob = Home::new("stale-bob");

    let created = ok(&alice, &["init", "yesterday"]);
    let network = field(&created, "network   ");
    let attached = ok(&bob, &["attach", &network]);
    ok(&alice, &["admit", &field(&attached, "kols admit ")]);

    // A window a test can get to the far side of. The bound is local tuning, so
    // a small one exercises the same rule the default does.
    let alice_node = serve_tuned(&alice, 45151, None, None, true, Some(1_000));
    let address = field(
        &alice_node.wait_for("listening", Duration::from_secs(20)),
        "listening ",
    );
    ok(&alice, &["channel", "create", "general"]);
    ok(&alice, &["post", "general", "written well before bob showed up"]);
    alice_node.wait_for("picked up", Duration::from_secs(20));

    // Past the window, so this record is now history by the node's own reckoning
    // — and history is the durable path's job.
    std::thread::sleep(Duration::from_secs(3));

    let bob_node = serve(&bob, 45152, Some(&address));
    bob_node.wait_for("learned 1 record", Duration::from_secs(60));

    let output = bob_node.output();
    assert!(
        !output.contains("learned 1 record(s) live"),
        "a record older than the live window must not be broadcast to a peer \
         that subscribes afterwards:\n{output}"
    );

    let read = ok(&bob, &["read", "general"]);
    assert!(read.contains("written well before bob showed up"), "{read}");
}
