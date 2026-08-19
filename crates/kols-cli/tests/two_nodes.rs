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
