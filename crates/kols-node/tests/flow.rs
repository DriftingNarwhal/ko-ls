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
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

mod common;
use common::patience;

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

/// A `kols serve` held only long enough to key the network.
///
/// Posting needs an epoch key and only the daemon mints one, because the MLS
/// group it comes from is live state no one-shot command can hold. That is the
/// real shape of the thing rather than an inconvenience to design around, so
/// these tests do what a user does: run the node once, then use it.
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
        .stderr(Stdio::null())
        .spawn()
        .expect("serve starts");

    // Wait for the key to land rather than for a log line: the store is the
    // contract between these processes, so that is the thing to observe.
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

#[test]
fn a_network_carries_a_conversation_from_creation_to_render() {
    let home = Home::new("flow");

    let created = ok(&home, &["init", "the workshop"]);
    assert!(created.contains("created the workshop"));
    let _node = keyed(&home, 45201);

    // The founder must actually be able to do things. A genesis that replays but
    // grants nothing is the failure mode worth catching here: it looks like
    // success until the first post.
    let who = ok(&home, &["whoami"]);
    assert!(who.contains("member true"), "{who}");
    assert!(who.contains("post             yes"), "{who}");
    assert!(who.contains("create channels  yes"), "{who}");

    ok(
        &home,
        &["channel", "create", "general", "--topic", "anything"],
    );
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
    let _node = keyed(&home, 45202);
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
        assert!(
            !out.status.success(),
            "`kols {}` should fail",
            args.join(" ")
        );
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
    let _node = keyed(&home, 45203);
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
    let _node = keyed(&home, 45204);
    ok(&home, &["channel", "create", "general"]);
    let out = run(&home, &["post", "general", "   "]);
    assert!(!out.status.success());
}

// ── what protects content at rest ──────────────────────────────────────

#[test]
fn the_key_that_protects_content_is_not_derivable_from_public_information() {
    // The property this replaced. An earlier version derived the DEK from the
    // network id — which travels in every invite, address and log entry — so
    // anyone who ever saw the network id could decrypt any segment they got hold
    // of. What protects content now is a random DEK wrapped under an epoch key
    // exported from a real MLS group, so two networks, and two members of one
    // network, share nothing an outsider can compute.
    let a = Home::new("keys-a");
    let b = Home::new("keys-b");
    ok(&a, &["init", "one"]);
    ok(&b, &["init", "two"]);
    let _a_node = keyed(&a, 45205);
    let _b_node = keyed(&b, 45206);
    ok(&a, &["channel", "create", "general"]);
    ok(&b, &["channel", "create", "general"]);
    ok(&a, &["post", "general", "in a"]);
    ok(&b, &["post", "general", "in b"]);

    let sealed = |home: &Home| {
        let dir = home.path().join("epochs");
        let entry = std::fs::read_dir(&dir)
            .expect("a stored epoch key")
            .next()
            .expect("at least one")
            .unwrap();
        std::fs::read(entry.path()).unwrap()
    };
    let epoch_a = sealed(&a);
    let epoch_b = sealed(&b);
    assert_ne!(epoch_a, epoch_b, "two networks must not share key material");

    let wrapping = |home: &Home| {
        let dir = home.path().join("deks");
        let entry = std::fs::read_dir(&dir)
            .expect("a wrapping directory")
            .next()
            .expect("one wrapping")
            .unwrap();
        std::fs::read(entry.path()).unwrap()
    };
    assert_ne!(
        wrapping(&a),
        wrapping(&b),
        "DEKs are random per object, so two must never coincide"
    );

    // And nothing on disk is the raw key: the epoch file is sealed, so it is
    // longer than the 32 bytes it protects.
    assert!(
        epoch_a.len() > 32,
        "the epoch key must be sealed at rest, not written bare"
    );
}

#[test]
fn secrets_are_written_unreadable_to_other_users() {
    // The seed and everything derived from it. A world-readable seed would make
    // the file permissions the only thing standing between another account on
    // this machine and every identity it holds.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let home = Home::new("perms");
        ok(&home, &["init", "private"]);
        let _node = keyed(&home, 45207);
        ok(&home, &["channel", "create", "general"]);
        ok(&home, &["post", "general", "secret"]);

        let mut checked = 0;
        let mut secrets = vec![home.path().join("seed")];
        for entry in std::fs::read_dir(home.path().join("epochs")).unwrap() {
            secrets.push(entry.unwrap().path());
        }
        for path in secrets {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o077,
                0,
                "{} is readable beyond its owner",
                path.display()
            );
            checked += 1;
        }
        for entry in std::fs::read_dir(home.path().join("deks")).unwrap() {
            let path = entry.unwrap().path();
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o077,
                0,
                "a DEK wrapping is readable beyond its owner"
            );
            checked += 1;
        }
        assert!(
            checked >= 3,
            "expected seed, epoch and at least one wrapping"
        );
    }
}

#[test]
fn a_store_whose_epoch_key_is_gone_refuses_rather_than_inventing_one() {
    // Fail closed. Silently minting a fresh key would produce a node that writes
    // content no other member can read, and reads nothing it wrote before —
    // divergence that looks like working software.
    let home = Home::new("no-epoch");
    ok(&home, &["init", "losing"]);
    let node = keyed(&home, 45208);
    ok(&home, &["channel", "create", "general"]);
    ok(&home, &["post", "general", "before"]);
    drop(node);
    std::fs::remove_dir_all(home.path().join("epochs")).unwrap();
    std::fs::remove_file(home.path().join("rotation")).unwrap();

    let out = run(&home, &["post", "general", "after"]);
    assert!(
        !out.status.success(),
        "posting without an epoch key must fail"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("epoch key"), "{stderr}");
}

#[test]
fn an_already_current_wrapping_is_not_rewritten_on_every_read() {
    // Half of what keeps "try every key" cheap. The other half — refreshing a
    // wrapping found under a superseded key — needs a real rotation, so it is
    // asserted in the revocation test where one actually happens.
    //
    // This half matters on its own: reading is the hottest path there is, and
    // rewriting the wrapping every time would be pointless disk traffic. Storage
    // §5.3 makes wrapping deterministic, so "unchanged" is a comparison rather
    // than a guess.
    let home = Home::new("rewrap");
    ok(&home, &["init", "refreshing"]);
    let _node = keyed(&home, 45209);
    ok(&home, &["channel", "create", "general"]);
    ok(&home, &["post", "general", "wrapped under the first epoch"]);

    let dir = home.path().join("deks");
    let path = std::fs::read_dir(&dir)
        .expect("a wrapping directory")
        .next()
        .expect("one wrapping")
        .unwrap()
        .path();
    let before = std::fs::read(&path).unwrap();

    ok(&home, &["read", "general"]);
    ok(&home, &["post", "general", "still the same epoch"]);

    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "an already-current wrapping must not be rewritten"
    );
}

#[test]
fn slowmode_stops_the_second_post_and_says_how_long() {
    // The writer half of `design/01` §10.3. The reader enforces this too, and
    // has its own tests in `kols-core`; this is the half that exists so a person
    // is *told* rather than having their record refused by everybody else
    // (§10.2). Both are needed and neither replaces the other.
    let home = Home::new("slowmode");
    ok(&home, &["init", "paced"]);
    let _node = keyed(&home, 45221);

    ok(&home, &["channel", "create", "general"]);
    ok(&home, &["post", "general", "first"]);

    // Slowmode is bounded by `chat:slowmode-max-seconds` and set by a
    // `chat:manage-channel` holder — the founder is one.
    ok(&home, &["channel", "slowmode", "general", "600"]);

    let refused = run(&home, &["post", "general", "second"]);
    assert!(
        !refused.status.success(),
        "a second post inside a ten-minute slowmode must be refused",
    );
    let said = String::from_utf8_lossy(&refused.stderr).to_string()
        + &String::from_utf8_lossy(&refused.stdout);
    assert!(said.contains("slowmode"), "{said}");
    assert!(
        said.contains("to go"),
        "a refusal should say how long, not just that it happened:\n{said}"
    );

    // And turning it off lets the same post through, so this is a live setting
    // rather than a channel that has been permanently soured.
    ok(&home, &["channel", "slowmode", "general", "0"]);
    ok(&home, &["post", "general", "second"]);
    let read = ok(&home, &["read", "general"]);
    assert!(read.contains("second"), "{read}");
}
