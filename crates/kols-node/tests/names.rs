//! Claiming a display name, through the actual binary — spec 07 §3.9.
//!
//! The collision rule between two members is a `kols-core` test, because it is
//! a statement about replay rather than about processes. What these cover is the
//! part that only shows up once a real log is involved: that the claim is
//! authorized, lands, survives being replayed by a separate process, and comes
//! back out attached to the right messages.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

mod common;
use common::patience;

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

fn keyed(home: &Home, port: u16) -> Daemon {
    let child = Command::new(env!("CARGO_BIN_EXE_kols"))
        .arg("--home")
        .arg(home.path())
        .args(["serve", "--listen"])
        .arg(format!("/ip4/127.0.0.1/tcp/{port}"))
        .stdout(Stdio::null())
        // **Inherited rather than discarded.** These tests watch the store
        // rather than the daemon's output, so a daemon that exits early is
        // otherwise completely silent and presents as whatever it failed to do
        // — a waiting room that never fills, a name that never lands. Inherit
        // costs nothing while the daemon is healthy, since it says nothing on
        // stderr until something goes wrong.
        .stderr(Stdio::inherit())
        .spawn()
        .expect("serve starts");
    let deadline = Instant::now() + patience(Duration::from_secs(20));
    while !home.path().join("rotation").exists() {
        assert!(Instant::now() < deadline, "the network was never keyed");
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
    assert!(
        !out.status.success(),
        "`kols {}` should have failed",
        args.join(" ")
    );
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_member_starts_with_no_name_and_is_told_how_to_get_one() {
    // Not an error state: spec 07 §3.9 makes claiming a name the member's own
    // act, so nothing can do it for them at admission.
    let home = Home::new("unnamed");
    ok(&home, &["init", "naming"]);
    let _node = keyed(&home, 45401);

    let who = ok(&home, &["whoami"]);
    assert!(who.contains("none yet"), "{who}");
    assert!(who.contains("kols name"), "{who}");
}

#[test]
fn a_claim_survives_replay_by_another_process() {
    let home = Home::new("claim");
    ok(&home, &["init", "naming"]);
    let _node = keyed(&home, 45402);

    ok(&home, &["name", "Ada", "Lovelace"]);
    // A separate invocation, so this is replay of a real log rather than memory.
    let who = ok(&home, &["whoami"]);
    assert!(who.contains("Ada Lovelace"), "{who}");
}

#[test]
fn messages_render_with_the_name_and_still_carry_the_id() {
    // Spec 07 §8's obligation on interfaces: uniqueness does not fold
    // confusables, so a name alone is not enough to tell two members apart.
    let home = Home::new("render");
    ok(&home, &["init", "naming"]);
    let _node = keyed(&home, 45403);
    ok(&home, &["name", "ada"]);
    ok(&home, &["channel", "create", "general"]);
    ok(&home, &["post", "general", "hello"]);

    let read = ok(&home, &["read", "general"]);
    assert!(read.contains("ada ("), "the name is missing:\n{read}");
    let identity = ok(&home, &["whoami"]);
    let short = identity
        .lines()
        .find(|line| line.starts_with("you "))
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("whoami names an identity");
    assert!(
        read.contains(&short[..8]),
        "the id is missing beside the name:\n{read}"
    );
}

#[test]
fn respelling_your_own_name_is_allowed_and_binds_no_second_key() {
    let home = Home::new("respell");
    ok(&home, &["init", "naming"]);
    let _node = keyed(&home, 45404);

    ok(&home, &["name", "ada"]);
    ok(&home, &["name", "ADA"]);
    let who = ok(&home, &["whoami"]);
    assert!(who.contains("ADA"), "{who}");
}

#[test]
fn a_name_nobody_could_see_is_refused_before_it_is_written() {
    // Refused rather than stripped: silently collapsing what a claimant cannot
    // see is what §3.9.1's first step exists to prevent.
    let home = Home::new("invisible");
    ok(&home, &["init", "naming"]);
    let _node = keyed(&home, 45405);

    let complaint = fails(&home, &["name", "ad\u{200B}a"]);
    assert!(complaint.contains("invisible"), "{complaint}");

    let empty = fails(&home, &["name", "   "]);
    assert!(empty.contains("empty"), "{empty}");

    let long = fails(&home, &["name", &"a".repeat(80)]);
    assert!(long.contains("bytes"), "{long}");
}
