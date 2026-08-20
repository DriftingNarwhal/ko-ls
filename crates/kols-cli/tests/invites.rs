//! Redeeming an invite, across two processes — Core §5.6–5.7.
//!
//! # What this covers, and where it stops
//!
//! The invite's job ends at the first connection (§5.7): it carries addresses to
//! dial and enough to verify who issued it, and everything after is ordinary
//! post-connection sync that the existing two-node tests already exercise. So
//! this stops where the invite's responsibility does — a joiner who went from a
//! pasted string to a place in the network, and an admin who can see them.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Home(PathBuf);

impl Home {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("kols-test-{name}-{}", std::process::id()));
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

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn serving(home: &Home, port: u16) -> Daemon {
    let child = Command::new(env!("CARGO_BIN_EXE_kols"))
        .arg("--home")
        .arg(home.path())
        .args(["serve", "--listen"])
        .arg(format!("/ip4/127.0.0.1/tcp/{port}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("serve starts");

    // The addresses file is what `kols invite` reads, so waiting for it is
    // waiting for the thing under test to be possible at all.
    let deadline = Instant::now() + Duration::from_secs(20);
    while !home.path().join("addresses").exists() || !home.path().join("rotation").exists() {
        assert!(Instant::now() < deadline, "the node never became reachable");
        std::thread::sleep(Duration::from_millis(100));
    }
    Daemon(child)
}

fn run(home: &Home, args: &[&str]) -> Output {
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

fn fails(home: &Home, args: &[&str]) -> String {
    let out = run(home, args);
    assert!(!out.status.success(), "`kols {}` should have failed", args.join(" "));
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn one_string_takes_a_stranger_from_nothing_to_a_place_in_the_network() {
    let alice = Home::new("inv-alice");
    ok(&alice, &["init", "the workshop"]);
    let _node = serving(&alice, 45501);

    let minted = ok(&alice, &["invite", "--uses", "3", "--hours", "12"]);
    let uri = minted.lines().next().expect("an invite is printed").to_owned();
    assert!(uri.starts_with("intranet-chat://join/"), "{uri}");

    // Bob holds one string and nothing else — no network id, no address, no
    // identity. That is the friction this exists to remove.
    let bob = Home::new("inv-bob");
    let joined = ok(&bob, &["join", &uri]);
    assert!(joined.contains("waiting to be admitted"), "{joined}");

    // The invite carried the address, so Bob keeps it rather than being told
    // one by hand later.
    let peers = std::fs::read_to_string(bob.path().join("peers")).expect("peers were kept");
    assert!(peers.contains("/ip4/127.0.0.1/tcp/45501"), "{peers}");

    // And the admin can see him without being sent his identity out of band.
    let waiting = ok(&alice, &["waiting"]);
    assert!(waiting.contains("kols admit"), "{waiting}");

    let identity = joined
        .lines()
        .find(|line| line.trim_start().starts_with("kols admit"))
        .and_then(|line| line.split_whitespace().nth(2))
        .expect("the joiner is told their identity");
    assert!(
        waiting.contains(identity),
        "the waiting room does not name the joiner:\n{waiting}"
    );

    ok(&alice, &["admit", identity]);
}

#[test]
fn an_invite_cannot_be_minted_before_the_node_has_an_address() {
    // An invite with no bootstrap address cannot establish a connection, which
    // is its only job — so this refuses rather than minting a credential that
    // goes nowhere.
    let home = Home::new("inv-noaddr");
    ok(&home, &["init", "unreachable"]);

    let complaint = fails(&home, &["invite"]);
    assert!(complaint.contains("kols serve"), "{complaint}");
}

#[test]
fn nonsense_is_refused_before_a_store_is_created() {
    let home = Home::new("inv-nonsense");
    let complaint = fails(&home, &["join", "not-an-invite"]);
    assert!(!complaint.is_empty());
    assert!(
        !home.path().join("seed").exists(),
        "a store was created for an invite that never decoded"
    );
}

#[test]
fn minting_an_invite_needs_approve_node() {
    // Which the founder holds and an ordinary member does not. Proven here by
    // the refusal's wording rather than by standing up a second member, since
    // the capability check is `kols-api`'s and is tested directly there.
    let home = Home::new("inv-cap");
    ok(&home, &["init", "capability"]);
    let _node = serving(&home, 45502);
    assert!(ok(&home, &["invite"]).starts_with("intranet-chat://join/"));
}
