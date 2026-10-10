//! What a running conversation costs while nobody is saying anything.
//!
//! `design/09` §2 keeps every conversation warm, and `05` §4 says idle ones
//! "should be suspended and woken on demand" with a threshold that wants
//! measuring. Whether a conversation nobody has written in for a week should be
//! set aside depends on this number, and nobody had taken it.
//!
//! Each run builds `N` two-member conversations the way the app does — Alice
//! creates each one, Bob redeems a bearer invite for it — and then measures
//! one process holding them through the same supervisor the window uses:
//!
//! - **alone**: Alice's `N` nodes with Bob stopped — somebody whose contacts
//!   are all offline.
//! - **paired**: Alice's and Bob's `N` nodes each, connected to one another.
//!   Both sides live in this process, so a per-conversation figure divides
//!   the difference over `2N`: what *one* side of one conversation costs.
//!
//! What it does **not** cover, said here so the numbers are not read as more:
//! the peers reach each other over this machine's own addresses, so there is no
//! relay reservation and no NAT, and a conversation that borrows a relay
//! (D39) pays for a circuit on top. Memory is the process's resident set,
//! which an allocator does not always give back; read growth, not exact
//! deltas. And none of the window's own cost is here.
//!
//! **Ignored by default** — it takes minutes and prints numbers to read rather
//! than a pass to trust. Run it deliberately, in release for the reason
//! `cost.rs` gives:
//!
//!     KOLS_PASSWORD=x cargo test --release -p kols-node --test idle_cost \
//!         -- --ignored --nocapture
//!
//! `KOLS_IDLE_COUNTS` (default `10,25,50`) sets the sizes and `KOLS_IDLE_SECS`
//! (default 60) how long each idle window is watched.

use kols_node::nodes::{Nodes, Spec, TaggedSink};
use kols_node::store::{self, Store};
use kols_node::workspace::Workspace;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("kols-idle-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a workspace");
        Self(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn setting(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

/// What this process is using, read from `/proc`. Linux only, which is where
/// the suite runs.
#[derive(Clone, Copy)]
struct Sample {
    at: Instant,
    rss_kib: u64,
    threads: u64,
    sockets: u64,
    cpu_ticks: u64,
    loopback_bytes: u64,
}

fn status_field(status: &str, key: &str) -> u64 {
    status
        .lines()
        .find_map(|line| line.strip_prefix(key))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn sample() -> Sample {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let mut sockets = 0;
    if let Ok(entries) = std::fs::read_dir("/proc/self/fd") {
        for entry in entries.flatten() {
            if std::fs::read_link(entry.path())
                .map(|target| target.to_string_lossy().starts_with("socket:"))
                .unwrap_or(false)
            {
                sockets += 1;
            }
        }
    }
    // utime and stime are the 14th and 15th fields; the command name before
    // them is parenthesised and may hold spaces, so count from its close.
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let after = stat.rsplit_once(')').map(|(_, rest)| rest).unwrap_or("");
    let fields: Vec<&str> = after.split_whitespace().collect();
    let tick = |i: usize| {
        fields
            .get(i)
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    };
    // Both peers are in this process and dial each other on this machine, so
    // their traffic is all loopback. Received bytes, since every byte sent
    // here is also received here.
    let loopback_bytes = std::fs::read_to_string("/proc/net/dev")
        .unwrap_or_default()
        .lines()
        .find_map(|line| line.trim().strip_prefix("lo:").map(str::to_owned))
        .and_then(|rest| rest.split_whitespace().next().and_then(|v| v.parse().ok()))
        .unwrap_or(0);
    Sample {
        at: Instant::now(),
        rss_kib: status_field(&status, "VmRSS:"),
        threads: status_field(&status, "Threads:"),
        sockets,
        cpu_ticks: tick(11) + tick(12),
        loopback_bytes,
    }
}

/// What one idle window cost, across however many conversation-sides were
/// running in it.
struct Watched {
    start: Sample,
    end: Sample,
}

impl Watched {
    fn seconds(&self) -> f64 {
        (self.end.at - self.start.at).as_secs_f64()
    }

    /// Percent of one core, averaged over the window. `/proc` counts in
    /// `USER_HZ`, which is 100 on every Linux this runs on.
    fn cpu_percent(&self) -> f64 {
        (self.end.cpu_ticks - self.start.cpu_ticks) as f64 / self.seconds()
    }

    fn loopback_per_second(&self) -> f64 {
        self.end
            .loopback_bytes
            .saturating_sub(self.start.loopback_bytes) as f64
            / self.seconds()
    }
}

async fn watch(seconds: u64) -> Watched {
    let start = sample();
    tokio::time::sleep(Duration::from_secs(seconds)).await;
    Watched {
        start,
        end: sample(),
    }
}

fn specs_for(stores: &[(intranet_identity::NetworkId, PathBuf)]) -> Vec<Spec> {
    stores
        .iter()
        .map(|(network, root)| Spec {
            network: *network,
            root: root.clone(),
            set_aside: false,
        })
        .collect()
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("after 1970")
        .as_millis() as i64
}

/// Waits until every store has written the addresses its node listens on,
/// which is what an invite for it needs.
async fn listening(stores: &[Store]) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while stores.iter().any(|store| store.addresses().is_empty()) {
        assert!(
            Instant::now() < deadline,
            "a conversation node never listened"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn report(
    label: &str,
    count: usize,
    sides: usize,
    base: &Sample,
    watched: &Watched,
    linked: usize,
) {
    let end = &watched.end;
    let per = |now: u64, then: u64| (now as f64 - then as f64) / sides.max(1) as f64;
    println!(
        "{label:>7} N={count:<4} linked {linked:>4}/{count:<4} | rss {:>7.1} MiB ({:>6.0} KiB/side) \
         | threads {:>4} ({:>4.1}/side) | sockets {:>5} ({:>4.1}/side) \
         | cpu {:>5.1}% of a core ({:>5.2}%/side) | loopback {:>9.0} B/s ({:>7.0} B/s/side)",
        end.rss_kib as f64 / 1024.0,
        per(end.rss_kib, base.rss_kib),
        end.threads,
        per(end.threads, base.threads),
        end.sockets,
        per(end.sockets, base.sockets),
        watched.cpu_percent(),
        watched.cpu_percent() / sides.max(1) as f64,
        watched.loopback_per_second(),
        watched.loopback_per_second() / sides.max(1) as f64,
    );
}

async fn measure(count: usize, idle: u64, base: &Sample) {
    let alice_dir = Dir::new(&format!("alice-{count}"));
    let bob_dir = Dir::new(&format!("bob-{count}"));
    let alice = Workspace::at(alice_dir.0.clone());
    let bob = Workspace::at(bob_dir.0.clone());
    let sink: TaggedSink = std::sync::Arc::new(|_, _| {});
    let seal = kols_node::serve::SEAL_TARGET_BYTES;

    let theirs: Vec<Store> = (0..count)
        .map(|i| {
            alice
                .create_conversation(&format!("c{i}"))
                .expect("creates")
        })
        .collect();
    let alice_roots: Vec<_> = theirs
        .iter()
        .map(|s| (*s.network(), s.root().to_path_buf()))
        .collect();

    let mut alice_nodes = Nodes::here();
    alice_nodes.reconcile(&specs_for(&alice_roots), None, &sink, seal, Instant::now());
    listening(&theirs).await;

    // Bob joins each the way an accepted request does: a bearer invite naming
    // the conversation's addresses, redeemed into his own workspace.
    let mut joins = tokio::task::JoinSet::new();
    for store in &theirs {
        let issuer = store.identity().expect("identity");
        let now = now_millis();
        let invite = intranet_invite::Invite::issue(
            &issuer,
            store.addresses(),
            intranet_invite::InviteSubject::Bearer,
            intranet_crypto::Timestamp::from_millis(now),
            intranet_crypto::Timestamp::from_millis(now + 600_000),
            1,
        );
        let network = *store.network();
        let root = bob.path_for(&network);
        joins.spawn(async move {
            let quiet = kols_node::quiet();
            kols_node::join::redeem(
                root,
                invite,
                30,
                &quiet,
                kols_core::NetworkProfile::Conversation,
            )
            .await
        });
    }
    // **What a redeem lands as is reported, not assumed.** A conversation keeps
    // auto-admit off, so a redeem is answered *waiting*, and the issuer's node
    // then admits the redeemer as a `Founder` (`serve::admit_redeemers`).
    // Who became a member is counted below rather than taken on trust, since
    // an idle cost of two nodes that cannot read each other is not the cost
    // being asked about.
    let mut outcomes = std::collections::BTreeMap::<&str, usize>::new();
    while let Some(result) = joins.join_next().await {
        let landed = result.expect("a join task");
        let kind = match landed {
            Ok(kols_node::join::Landed::Admitted) => "admitted",
            Ok(kols_node::join::Landed::Waiting { .. }) => "waiting",
            Ok(kols_node::join::Landed::Unanswered { .. }) => "unanswered",
            Err(_) => "failed",
        };
        *outcomes.entry(kind).or_default() += 1;
    }
    drop(theirs);
    let bob_roots: Vec<_> = alice_roots
        .iter()
        .map(|(network, _)| (*network, bob.path_for(network)))
        .filter(|(_, root)| root.exists())
        .collect();
    println!("        N={count:<4} redeems landed as {outcomes:?}");

    // Alone: Bob is not running, so Alice's nodes have nobody to reach.
    tokio::time::sleep(Duration::from_secs(10)).await;
    let alone = watch(idle).await;
    report("alone", count, count, base, &alone, 0);

    let members = |roots: &[(intranet_identity::NetworkId, PathBuf)]| {
        roots
            .iter()
            .filter(|(network, root)| {
                let Ok(theirs) = Store::open(root.clone()) else {
                    return false;
                };
                let Ok(state) = theirs.state() else {
                    return false;
                };
                // Bob's identity in this network, from his own store.
                Store::open(bob.path_for(network))
                    .ok()
                    .and_then(|his| his.identity().ok())
                    .is_some_and(|his| state.is_member(&his.id()))
            })
            .count()
    };

    // Both sides up and given time to find each other.
    let mut bob_nodes = Nodes::here();
    bob_nodes.reconcile(&specs_for(&bob_roots), None, &sink, seal, Instant::now());
    tokio::time::sleep(Duration::from_secs(20)).await;
    println!(
        "        N={count:<4} members: {}/{count}",
        members(&alice_roots)
    );
    let paired = watch(idle).await;
    let linked = alice_roots
        .iter()
        .filter(|(_, root)| !store::connected_in(root).is_empty())
        .count();
    report("paired", count, 2 * count, base, &paired, linked);

    for task in alice_nodes
        .stop_all()
        .into_iter()
        .chain(bob_nodes.stop_all())
    {
        let _ = task.await;
    }
}

#[test]
#[ignore = "a measurement, not an assertion — see the module comment"]
fn what_an_idle_conversation_costs_to_keep_running() {
    let counts: Vec<usize> = setting("KOLS_IDLE_COUNTS", "10,25,50")
        .split(',')
        .filter_map(|n| n.trim().parse().ok())
        .collect();
    let idle: u64 = setting("KOLS_IDLE_SECS", "60").parse().expect("seconds");

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(async {
            let base = sample();
            println!(
                "baseline: rss {:.1} MiB, threads {}, sockets {}; idle window {idle} s",
                base.rss_kib as f64 / 1024.0,
                base.threads,
                base.sockets
            );
            for count in counts {
                measure(count, idle, &base).await;
                // Let the stopped nodes' sockets close before the next size's
                // baseline is compared against.
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
}
