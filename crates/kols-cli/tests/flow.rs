//! The whole flow, through the actual binary.
//!
//! # Why the binary rather than the functions
//!
//! Every other test in this workspace calls a library. This one runs `kols`,
//! because what it is checking is that the layers *compose* — genesis writes a
//! policy that permission resolution can read, a channel entry survives replay,
//! a record admits against the state that replay produced, and all of it
//! survives being written to disk and read back by a separate process. A test
//! that called the functions in one process would share state the real thing
//! does not, and would prove less than it appeared to.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A scratch home, removed when the test ends.
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

#[test]
fn a_network_carries_a_conversation_from_creation_to_render() {
    let home = Home::new("flow");

    let created = ok(&home, &["init", "the workshop"]);
    assert!(created.contains("created the workshop"));

    // The founder must actually be able to do things. A genesis that replays but
    // grants nothing is the failure mode worth catching here: it looks like
    // success until the first post.
    let who = ok(&home, &["whoami"]);
    assert!(who.contains("member true"), "{who}");
    assert!(who.contains("post             yes"), "{who}");
    assert!(who.contains("create channels  yes"), "{who}");

    ok(&home, &["channel", "create", "general", "--topic", "anything"]);
    let listed = ok(&home, &["channel", "list"]);
    assert!(listed.contains("#general"), "{listed}");
    assert!(listed.contains("anything"), "{listed}");

    ok(&home, &["post", "general", "first"]);
    ok(&home, &["post", "general", "second"]);

    let read = ok(&home, &["read", "general"]);
    assert!(read.contains("first"), "{read}");
    assert!(read.contains("second"), "{read}");
    assert!(
        read.find("first") < read.find("second"),
        "messages must render in the order they were written:\n{read}"
    );
}

#[test]
fn state_survives_between_invocations() {
    // Each command is a separate process, so anything held only in memory is
    // gone by the next one. This is the property that makes the store the source
    // of truth rather than a cache.
    let home = Home::new("persist");
    ok(&home, &["init", "persistent"]);
    ok(&home, &["channel", "create", "notes"]);
    ok(&home, &["post", "notes", "written earlier"]);

    let read = ok(&home, &["read", "notes"]);
    assert!(read.contains("written earlier"), "{read}");

    // And the log still replays cleanly from disk after all of it.
    let listed = ok(&home, &["channel", "list"]);
    assert!(listed.contains("#notes"), "{listed}");
}

#[test]
fn a_second_init_refuses_rather_than_overwriting_the_seed() {
    // The most destructive thing this program could do. The seed cannot be
    // rebuilt from the network, unlike everything else in the store.
    let home = Home::new("clobber");
    ok(&home, &["init", "first"]);
    let seed = std::fs::read(home.path().join("seed")).expect("a seed was written");

    let second = run(&home, &["init", "second"]);
    assert!(!second.status.success(), "a second init must fail");
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("already exists"),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(
        std::fs::read(home.path().join("seed")).unwrap(),
        seed,
        "and must not have touched the seed"
    );
}

#[test]
fn commands_refuse_helpfully_before_a_network_exists() {
    let home = Home::new("empty");
    for args in [
        vec!["whoami"],
        vec!["channel", "list"],
        vec!["post", "general", "hello"],
        vec!["read", "general"],
    ] {
        let out = run(&home, &args);
        assert!(!out.status.success(), "`kols {}` should fail", args.join(" "));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("kols init"),
            "the error should say how to fix it, got: {stderr}"
        );
    }
}

#[test]
fn posting_to_a_channel_that_does_not_exist_says_so() {
    let home = Home::new("nochannel");
    ok(&home, &["init", "sparse"]);
    let out = run(&home, &["post", "nowhere", "hello"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no channel matching"), "{stderr}");
}

#[test]
fn a_channel_is_addressable_by_name_or_by_the_start_of_its_id() {
    let home = Home::new("addressing");
    ok(&home, &["init", "addressing"]);
    let created = ok(&home, &["channel", "create", "general"]);
    let id = created
        .lines()
        .find_map(|line| line.trim().strip_prefix("id       "))
        .expect("create prints the id")
        .to_owned();

    ok(&home, &["post", "general", "by name"]);
    ok(&home, &["post", &id[..12], "by id"]);
    // The leading `#` people type out of habit is accepted rather than being a
    // mistake to correct.
    ok(&home, &["post", "#general", "by hash-name"]);

    let read = ok(&home, &["read", "general"]);
    for expected in ["by name", "by id", "by hash-name"] {
        assert!(read.contains(expected), "{expected} missing from:\n{read}");
    }
}

#[test]
fn an_empty_message_is_refused() {
    let home = Home::new("empty-message");
    ok(&home, &["init", "quiet"]);
    ok(&home, &["channel", "create", "general"]);
    let out = run(&home, &["post", "general", "   "]);
    assert!(!out.status.success());
}
