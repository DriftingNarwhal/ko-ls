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

    let before = field(&ok(&alice, &["whoami"]), "epoch    ");

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
    ok(&alice, &["post", "general", "after the revocation"]);
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
