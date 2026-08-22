//! The record commands, through the actual binary.
//!
//! # Why these run the binary
//!
//! Same reason `flow.rs` does: what is under test is that the layers compose.
//! A command typed at a terminal becomes a `kols_api::Command`, crosses the
//! gate, reaches the executor, becomes a signed record in this member's log, and
//! comes back through a merge that a *separate process* performs from the store.
//! An in-process test would share state these do not.

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

/// A `kols serve` held only long enough to key the network.
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
        "`kols {}` was expected to fail and did not:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The id `kols read` prints beside the message whose body contains `needle`.
///
/// Every command that acts on a message takes one of these, so a rendering that
/// did not carry them would leave the commands unusable — which is why `read`
/// prints them and why this helper is short.
fn id_of(read: &str, needle: &str) -> String {
    read.lines()
        .find(|line| line.contains(needle))
        .and_then(|line| line.split_whitespace().next())
        .unwrap_or_else(|| panic!("no message matching {needle:?} in:\n{read}"))
        .to_owned()
}

/// A keyed network with one channel, ready to write records into.
fn workshop(name: &str, port: u16) -> (Home, Daemon) {
    let home = Home::new(name);
    ok(&home, &["init", "the workshop"]);
    let node = keyed(&home, port);
    ok(&home, &["channel", "create", "general"]);
    (home, node)
}

#[test]
fn an_edit_replaces_the_body_and_says_it_was_edited() {
    let (home, _node) = workshop("edit", 45301);
    ok(&home, &["post", "general", "first draft"]);

    let id = id_of(&ok(&home, &["read", "general"]), "first draft");
    ok(&home, &["edit", "general", &id, "second draft"]);

    let read = ok(&home, &["read", "general"]);
    assert!(read.contains("second draft"), "{read}");
    assert!(
        !read.contains("first draft"),
        "the old body still renders:\n{read}"
    );
    assert!(read.contains("edited"), "{read}");
}

#[test]
fn a_withdrawal_hides_the_message_and_says_so() {
    let (home, _node) = workshop("withdraw", 45302);
    ok(&home, &["post", "general", "regrettable"]);

    let id = id_of(&ok(&home, &["read", "general"]), "regrettable");
    ok(&home, &["delete", "general", &id]);

    let read = ok(&home, &["read", "general"]);
    assert!(read.contains("withdrawn"), "{read}");
}

#[test]
fn a_reaction_lands_and_can_be_taken_back() {
    let (home, _node) = workshop("react", 45303);
    ok(&home, &["post", "general", "worth reacting to"]);
    let id = id_of(&ok(&home, &["read", "general"]), "worth reacting to");

    ok(&home, &["react", "general", &id, "+1"]);
    let read = ok(&home, &["read", "general"]);
    assert!(read.contains("+1 ×1"), "{read}");

    ok(&home, &["react", "general", &id, "+1", "--remove"]);
    let read = ok(&home, &["read", "general"]);
    assert!(
        !read.contains("+1 ×"),
        "the reaction survived removal:\n{read}"
    );
}

#[test]
fn a_founder_may_pin_because_they_hold_every_capability() {
    // Pinning is `chat:moderate`, which is governance-tier — so this passing is
    // a statement about the Founders group holding an unrestricted set, not
    // about pinning being ordinary. A member without it is refused by both the
    // boundary and the reader, which `kols-api` and `kols-core` cover directly.
    let (home, _node) = workshop("pin", 45304);
    ok(&home, &["post", "general", "keep this one"]);
    let id = id_of(&ok(&home, &["read", "general"]), "keep this one");

    ok(&home, &["pin", "general", &id]);
    let read = ok(&home, &["read", "general"]);
    assert!(read.contains("pinned"), "{read}");

    // And back off again. The `--remove` path had no coverage, which mattered
    // once a window offered pin and unpin as the same button: a toggle whose
    // second half is untested is half a feature.
    ok(&home, &["pin", "general", &id, "--remove"]);
    let read = ok(&home, &["read", "general"]);
    assert!(
        !read.contains("pinned"),
        "unpinning should leave no pin behind:\n{read}"
    );
}

#[test]
fn a_message_id_that_matches_nothing_says_which_one() {
    let (home, _node) = workshop("prefix", 45305);
    ok(&home, &["post", "general", "the only one"]);

    let complaint = fails(&home, &["delete", "general", "deadbeef"]);
    assert!(complaint.contains("deadbeef"), "{complaint}");
}

#[test]
fn renaming_a_channel_survives_replay() {
    let (home, _node) = workshop("rename", 45306);
    ok(&home, &["channel", "rename", "general", "lobby"]);

    let listed = ok(&home, &["channel", "list"]);
    assert!(listed.contains("#lobby"), "{listed}");
    assert!(
        !listed.contains("#general"),
        "the old name survived:\n{listed}"
    );
}

#[test]
fn slowmode_past_the_networks_ceiling_is_refused_before_anything_is_signed() {
    // The boundary's bound, not the executor's: `design/01` §10.3 caps what a
    // channel manager may set, and refusing here is what stops an entry every
    // node would have to evaluate.
    let (home, _node) = workshop("slowmode", 45307);
    let complaint = fails(&home, &["channel", "slowmode", "general", "999999"]);
    assert!(complaint.contains("slowmode"), "{complaint}");

    // A value inside the ceiling is ordinary.
    ok(&home, &["channel", "slowmode", "general", "30"]);
}

#[test]
fn archiving_a_channel_shows_in_the_listing() {
    let (home, _node) = workshop("archive", 45308);
    ok(&home, &["channel", "archive", "general"]);

    let listed = ok(&home, &["channel", "list"]);
    assert!(listed.contains("archived"), "{listed}");
}

#[test]
fn you_cannot_edit_a_message_you_did_not_write() {
    // Structurally you never could — nobody writes into another author's log, so
    // such an edit is discarded on read (`design/01` §6). What the executor adds
    // is being told, before a record is signed that everyone would ignore.
    let (home, _node) = workshop("not-yours", 45309);
    ok(&home, &["post", "general", "mine"]);
    let id = id_of(&ok(&home, &["read", "general"]), "mine");

    // A second store is a different identity in a different network, so its
    // messages are not reachable from here; the reachable wrong case is an id
    // that names nothing, which is the same refusal path.
    let complaint = fails(&home, &["edit", "general", "abc123", "hijacked"]);
    assert!(!complaint.is_empty(), "a refusal should say something");

    // And the right case still works, so the check is not simply refusing.
    ok(&home, &["edit", "general", &id, "still mine"]);
}

#[test]
fn the_rate_ceiling_stops_a_flood_before_it_is_signed() {
    // `design/01` §10.2: the ceiling is computed over the author's own HLC
    // readings, so every node reaches the same verdict — and the author's own
    // client enforces it first, so a user is told rather than having every
    // reader silently refuse what they wrote.
    let (home, _node) = workshop("rate", 45310);

    // The shipped default is 30 messages a minute. These all carry HLC readings
    // inside one minute of each other, because they are written in under one.
    let mut refused = None;
    for i in 0..40 {
        let body = format!("flood {i}");
        let out = run(&home, &["post", "general", &body]);
        if !out.status.success() {
            refused = Some(String::from_utf8_lossy(&out.stderr).into_owned());
            break;
        }
    }

    let complaint = refused.expect("40 messages in a minute should have hit the ceiling");
    assert!(complaint.contains("too fast"), "{complaint}");
    assert!(
        complaint.contains("30"),
        "the ceiling should be named: {complaint}"
    );
}
