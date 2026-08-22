//! Redeeming an invite, across two processes — Core §5.6–5.7.
//!
//! # What this covers, and where it stops
//!
//! The invite's job ends at the first connection (§5.7): it carries addresses to
//! dial and enough to verify who issued it, and everything after is ordinary
//! post-connection sync that the existing two-node tests already exercise. So
//! this stops where the invite's responsibility does — a joiner who went from a
//! pasted string to a place in the network, and an admin who can see them.

use intranet_identity::{MasterSeed, NetworkId};
use intranet_transport::{NodeEvent, RelayNode};
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

/// A relay the test hosts itself, on a routable address.
///
/// Routable rather than loopback, and that is not incidental: a relay promotes
/// only non-loopback listen addresses to external ones, and libp2p builds the
/// address list it returns in a reservation from external addresses alone. A
/// loopback relay grants reservations carrying no address, so `kols serve` would
/// reserve nothing and `kols invite` would correctly refuse — the tests would
/// fail for a reason that has nothing to do with invites.
fn hosted_relay() -> String {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        runtime.block_on(async move {
            let identity = MasterSeed::from_entropy([77u8; 32])
                .identity_for(&NetworkId::from_bytes([42u8; 32]))
                .expect("derives");
            let mut relay = RelayNode::new(&identity).expect("a relay");
            relay
                .listen_on("/ip4/0.0.0.0/tcp/0".parse().expect("a multiaddr"))
                .expect("listens");

            let mut announced = false;
            loop {
                let event = relay.next_event().await;
                if let NodeEvent::Listening(address) = event
                    && !announced
                    && !address.to_string().contains("127.0.0.1")
                {
                    announced = true;
                    let full = format!("{address}/p2p/{}", identity.peer_id());
                    let _ = tx.send(full);
                }
            }
        });
    });
    rx.recv_timeout(patience(Duration::from_secs(10)))
        .expect("the relay reports a routable address")
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

    // A circuit, not merely an address: an invite that carried only this
    // machine's own addresses would reach nobody off it, which is what the
    // relay requirement exists to prevent.
    let deadline = Instant::now() + patience(Duration::from_secs(40));
    loop {
        let reserved = std::fs::read_to_string(home.path().join("addresses"))
            .map(|text| text.contains("p2p-circuit"))
            .unwrap_or(false);
        if reserved && home.path().join("rotation").exists() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the node never reserved a circuit"
        );
        std::thread::sleep(Duration::from_millis(200));
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
fn one_string_takes_a_stranger_from_nothing_to_a_place_in_the_network() {
    let relay = hosted_relay();
    let alice = Home::new("inv-alice");
    ok(&alice, &["init", "the workshop", "--relay", &relay]);
    let _node = serving(&alice, 45501);

    let minted = ok(&alice, &["invite", "--uses", "3", "--hours", "12"]);
    let uri = minted
        .lines()
        .next()
        .expect("an invite is printed")
        .to_owned();
    assert!(uri.starts_with("intranet-chat://join/"), "{uri}");

    // Bob holds one string and nothing else — no network id, no address, no
    // identity. That is the friction this exists to remove.
    let bob = Home::new("inv-bob");
    let joined = ok(&bob, &["join", &uri]);
    assert!(joined.contains("waiting to be admitted"), "{joined}");

    // The invite carried the relay circuit, so Bob keeps it rather than being
    // told an address by hand later — and it is the circuit that matters, since
    // that is the one address that works from another network.
    let peers = std::fs::read_to_string(bob.path().join("peers")).expect("peers were kept");
    assert!(
        peers.contains("p2p-circuit"),
        "no circuit in the invite:\n{peers}"
    );

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

    // **And then they are not waiting any more.**
    //
    // The node's waiting room is filled when a join is answered and emptied by
    // nothing that admission passes through — admitting writes a governance
    // entry, and no path from one reaches that room. So a founder kept being
    // shown somebody they had already let in, beside an `admit` button that had
    // already been pressed. The daemon recomputes the published list against
    // replayed membership, so this is the first tick after the entry lands.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut waiting = String::new();
    while Instant::now() < deadline {
        waiting = ok(&alice, &["waiting"]);
        if !waiting.contains(identity) {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(
        !waiting.contains(identity),
        "an admitted member is still shown at the door:\n{waiting}"
    );
}

#[test]
fn a_network_with_no_relay_cannot_invite_anybody() {
    // Core §5.5: two members behind NAT cannot reach each other directly, so an
    // invite from a network with no relay would only work for somebody already
    // able to dial this machine. Refused rather than minting a credential whose
    // failure lands on the joiner as an unexplained timeout.
    let home = Home::new("inv-norelay");
    ok(&home, &["init", "unreachable"]);

    let complaint = fails(&home, &["invite"]);
    assert!(complaint.contains("designates no relay"), "{complaint}");
    assert!(complaint.contains("relay set"), "{complaint}");
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
    let relay = hosted_relay();
    let home = Home::new("inv-cap");
    ok(&home, &["init", "capability", "--relay", &relay]);
    let _node = serving(&home, 45502);
    assert!(ok(&home, &["invite"]).starts_with("intranet-chat://join/"));
}

#[test]
fn the_window_takes_the_same_path_as_the_terminal_to_join() {
    // What `kols-app`'s join_network does, minus the Tauri wrapper: resolve the
    // store's place in a workspace from the invite's network, then redeem. The
    // button cannot be pressed from a test, so the path behind it is covered
    // here rather than not at all.
    let relay = hosted_relay();
    let alice = Home::new("win-alice");
    ok(&alice, &["init", "the workshop", "--relay", &relay]);
    let _node = serving(&alice, 45503);

    let minted = ok(&alice, &["invite"]);
    let uri = minted.lines().next().expect("an invite").to_owned();

    // A workspace, not a store: the window holds several networks and puts each
    // one where its id says, so redeeming the same invite twice lands in the
    // same place rather than making a second identity in that network.
    let workspace_dir = Home::new("win-bob");
    let workspace = kols_node::workspace::Workspace::at(workspace_dir.path().to_path_buf());
    let credential = kols_node::invite::from_uri(&uri).expect("decodes");
    let path = workspace.path_for(&credential.network);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let landed = runtime
        .block_on(kols_node::join::redeem(path.clone(), credential, 30, false))
        .expect("joins");

    // Explicit intake, so waiting is the expected landing and is a success.
    match landed {
        kols_node::join::Landed::Waiting { identity } => {
            assert_eq!(identity.len(), 64, "an identity to be admitted by");
            assert!(ok(&alice, &["waiting"]).contains(&identity));
        }
        kols_node::join::Landed::Admitted => panic!("this network screens its members"),
    }

    // And the workspace now holds it, so the window has something to open.
    let listed = workspace.list();
    assert_eq!(listed.len(), 1, "the joined network is in the workspace");
    assert_eq!(listed[0].path, path);
    assert!(!listed[0].keyed, "joined, and not keyed in until admitted");
}
