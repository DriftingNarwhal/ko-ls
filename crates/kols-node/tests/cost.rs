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
    let root = store.root().to_path_buf();
    drop(store);
    // Keyed, because writing a record requires an epoch key — a member who holds
    // none cannot key what they are about to write, and the executor refuses
    // rather than storing something that can never be published.
    common::brief_run(root.clone());
    let executor = Executor::open(root).expect("opens");

    // A channel to post into, so the measurement can tell an ordinary command
    // from one that writes governance. They are different questions now that the
    // log is cached: a send invalidates nothing and should not care how much
    // structure the network has, while a governance write pays for the rebuild
    // its own entry causes.
    executor
        .submit(Command::SetChatSetting {
            setting: kols_core::ChatSetting::MessageRate,
            value: 0,
        })
        .expect("no rate ceiling");
    let talk = match executor
        .submit(Command::CreateChannel {
            name: "talk".to_owned(),
            category: None,
            privacy: kols_core::Privacy::Public,
            topic: String::new(),
        })
        .expect("creates a channel")
    {
        kols_api::Outcome::ChannelCreated { channel, .. } => channel,
        other => panic!("unexpected outcome: {other:?}"),
    };

    println!("\n  channels    state()    channels()    governance    a send    settled");
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
        let state = executor.store().state().expect("replays");
        let replay = started.elapsed();

        let started = Instant::now();
        let _ = kols_node::network::channels(executor.store(), &state).expect("reads channels");
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

        // The one a member actually waits for, and the one the cache is for: it
        // writes a record and no governance entry, so nothing it does should
        // depend on how much structure the log carries.
        let started = Instant::now();
        executor
            .submit(Command::SendMessage {
                channel: talk,
                body: "ordinary".to_owned(),
                reply_to: None,
                attachments: Vec::new(),
            })
            .expect("posts");
        let send = started.elapsed();

        // And a second one, which is the shape a member actually produces:
        // messages follow messages, not governance writes. The first send above
        // lands straight after ten channel creations and pays for the rebuild
        // they caused, which is a real cost and a rare one.
        let started = Instant::now();
        executor
            .submit(Command::SendMessage {
                channel: talk,
                body: "ordinary".to_owned(),
                reply_to: None,
                attachments: Vec::new(),
            })
            .expect("posts");
        let again = started.elapsed();

        println!(
            "  {:>8}    {:>5.0}ms    {:>7.0}ms    {:>8.0}ms    {:>6.1}ms    {:>6.1}ms",
            (round + 1) * 10,
            millis(replay),
            millis(listing),
            millis(command),
            millis(send),
            millis(again)
        );
    }
}

#[test]
#[ignore = "a measurement, not an assertion — see the module comment"]
fn what_opening_a_channel_costs_as_it_fills() {
    // The read path's twin of the send measurement, and what O4 exists for.
    //
    // **This used to read every record in the channel** from its own file,
    // decode it, merge the whole set and run the reader-side rate pass over it —
    // so rendering a screenful cost the size of the conversation, about 0.037 ms
    // a record, and roughly four seconds at a hundred thousand. On a control the
    // window re-runs every two seconds, which is what made it the half that
    // bites first.
    //
    // It now asks the index which records the range holds and reads those. Both
    // columns are measured, because the answer that matters is not how fast a
    // page is but whether it stops depending on the channel.
    let dir = Dir::new("open");
    let workspace = Workspace::at(dir.0.clone());
    let store = workspace.create("cost", Vec::new()).expect("creates");
    let root = store.root().to_path_buf();
    drop(store);
    common::brief_run(root.clone());
    let executor = Executor::open(root).expect("opens");

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

    println!("\n  records    settled page    first after a batch    whole channel");
    let batch = 500;
    for round in 0..rounds("KOLS_COST_BATCHES", 8) {
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

        // **The first open after a batch pays for folding the batch**, which is
        // an honest cost and not the page's: five hundred records arrived at
        // once. In use a tick sees one or two, so this column is a worst case
        // that nothing real produces.
        let started = Instant::now();
        executor
            .submit(Command::OpenChannel {
                channel,
                window: kols_core::Window::opening(50),
            })
            .expect("opens");
        let cold = started.elapsed();

        // The settled page, which is what the window's two-second tick actually
        // costs once the index is up to date. This is the number O4 is about.
        let started = Instant::now();
        executor
            .submit(Command::OpenChannel {
                channel,
                window: kols_core::Window::opening(50),
            })
            .expect("opens");
        let page = started.elapsed();

        // The same call asking for everything — what the terminal does, and what
        // the window did until this change. Measured beside the page so the two
        // curves can be compared rather than described.
        //
        // **Asked under the same limits**, which sounds like a detail and is
        // not: a verdict is only true of the limits that produced it, so a
        // measurement that varied them would re-fold the channel on every call
        // and report the cost of the fold as the cost of the read. That is
        // exactly what an earlier version of this did.
        let started = Instant::now();
        executor
            .submit(Command::OpenChannel {
                channel,
                window: kols_core::Window::opening(usize::MAX),
            })
            .expect("opens");
        let whole = started.elapsed();

        println!(
            "  {:>7}    {:>10.1}ms    {:>17.1}ms    {:>10.1}ms",
            (round + 1) * batch,
            millis(page),
            millis(cold),
            millis(whole)
        );
    }
}
