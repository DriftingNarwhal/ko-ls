//! What the same work done repeatedly actually costs — O5.
//!
//! `design/05` §5 asks for this explicitly and `WORKING.md` makes it the step
//! before O4: a number on how long a send takes in a channel with a long author
//! log is what says whether this is a real cost or a tidy one, and it is also
//! the thing that will show whether the projection delivered.
//!
//! **Ignored by default.** It is a measurement rather than an assertion — it
//! takes minutes and its output is numbers to read, not a pass to trust. Run it
//! deliberately:
//!
//!     cargo test --release -p kols-node --test cost -- --ignored --nocapture \
//!         --test-threads=1
//!
//! **`--release` is not optional and `--test-threads=1` is not tidiness.** A
//! debug build runs this path about forty times slower, because almost all of it
//! is signing, hashing and encrypting — measuring there answers a question about
//! `rustc -O0`. And the two measurements interleave their output otherwise,
//! which makes two clean curves look like one noisy one.
//!
//! The sizes default to what finishes in about a minute. `KOLS_COST_BATCHES`
//! and `KOLS_COST_ROUNDS` take it further when the shape of the curve matters
//! more than the wait — which is the point at which this stops being cheap, and
//! is why the default is not the interesting number.

mod common;

use kols_api::Command;
use kols_node::executor::Executor;
use kols_node::workspace::Workspace;
use std::time::{Duration, Instant};

struct Dir(std::path::PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("kols-cost-{name}-{}", std::process::id()));
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

fn millis(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn rounds(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[test]
#[ignore = "a measurement, not an assertion — see the module comment"]
fn what_a_send_costs_as_an_author_log_grows() {
    let dir = Dir::new("send");
    let workspace = Workspace::at(dir.0.clone());
    let store = workspace.create("cost", Vec::new()).expect("creates");
    let root = store.root().to_path_buf();
    drop(store);
    // Nothing can be posted without an epoch key, and only a running node
    // creates the founder's group (Core §3.3) — so the network is keyed once,
    // in process, before anything is timed.
    common::brief_run(root.clone());
    let executor = Executor::open(root).expect("opens");

    // The rate ceiling is enforced before anything is signed (`01` §10.2), so a
    // tight posting loop is refused long before it is slow. Raised here because
    // the thing under measurement is the append, not the ceiling.
    executor
        .submit(Command::SetChatSetting {
            setting: kols_core::ChatSetting::MessageRate,
            value: 0,
        })
        .expect("no rate ceiling");

    let channel = match executor
        .submit(Command::CreateChannel {
            name: "general".to_owned(),
            category: None,
            privacy: kols_core::Privacy::Public,
            topic: String::new(),
        })
        .expect("creates a channel")
    {
        kols_api::Outcome::ChannelCreated { channel, .. } => channel,
        other => panic!("unexpected outcome: {other:?}"),
    };

    println!("\n  records    per send      batch");
    let batch = 25;
    let batches = rounds("KOLS_COST_BATCHES", 8);
    for round in 0..batches {
        let started = Instant::now();
        for n in 0..batch {
            executor
                .submit(Command::SendMessage {
                    channel,
                    body: format!("message {}", round * batch + n),
                    reply_to: None,
                    attachments: Vec::new(),
                })
                .expect("posts");
        }
        let took = started.elapsed();
        println!(
            "  {:>7}    {:>7.1}ms    {:>7.0}ms",
            (round + 1) * batch,
            millis(took) / batch as f64,
            millis(took)
        );
    }
}

#[test]
#[ignore = "a measurement, not an assertion — see the module comment"]
fn what_replay_costs_as_structure_grows() {
    // The other half of O5: replay walks the log once per question, so reading
    // channels and reading categories are two walks over the same entries. Only
    // *structure* enters the governance log — no message ever does (`00` §4) —
    // so this grows with channels and roles rather than with traffic.
    let dir = Dir::new("replay");
    let workspace = Workspace::at(dir.0.clone());
    let store = workspace.create("cost", Vec::new()).expect("creates");
    let executor = Executor::open(store.root().to_path_buf()).expect("opens");

    println!("\n  channels    state()    channels()    one command");
    for round in 0..rounds("KOLS_COST_ROUNDS", 5) {
        for n in 0..10 {
            executor
                .submit(Command::CreateChannel {
                    name: format!("channel-{}-{}", round, n),
                    category: None,
                    privacy: kols_core::Privacy::Public,
                    topic: String::new(),
                })
                .expect("creates a channel");
        }

        let started = Instant::now();
        let state = store.state().expect("replays");
        let replay = started.elapsed();

        let started = Instant::now();
        let _ = kols_node::network::channels(&store, &state).expect("reads channels");
        let listing = started.elapsed();

        // What a member actually waits for: one ordinary command, which replays
        // the log and walks it again before it authorizes anything.
        let started = Instant::now();
        executor
            .submit(Command::SetName {
                name: format!("name-{round}"),
            })
            .expect("sets a name");
        let command = started.elapsed();

        println!(
            "  {:>8}    {:>5.0}ms    {:>7.0}ms    {:>8.0}ms",
            (round + 1) * 10,
            millis(replay),
            millis(listing),
            millis(command)
        );
    }
}
