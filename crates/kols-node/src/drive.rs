//! `kols-drive` — a whole installation, run headless, for tests that need two
//! machines.
//!
//! # What this is for
//!
//! `kols` is one store per `--home`, which is the right shape for testing a
//! network and the wrong one for a direct message: a conversation is a network
//! the *workspace* creates, borrows a relay for and wakes (D39, D41), and none
//! of that happens below the workspace. Until this existed, the only way to
//! watch two installations hold a conversation was two windows on two machines
//! and a person between them.
//!
//! This runs what the window runs — [`kols_node::nodes::Nodes`], fed by
//! [`kols_node::nodes::specs_for`], reconciled on the same tick and woken by the
//! same event — without a window. Like `kols`, it is a test harness and nothing
//! else (`design/00` D30): its output matters because scripts read it.
//!
//! # How it is driven
//!
//! `kols-drive --home W run` holds the installation and prints what happens, one
//! line each, prefixed with the wall clock in milliseconds. Every other
//! subcommand is a request to that process: it drops a file in `W/drive/inbox`,
//! waits for the answer beside it, and prints it. Files rather than a socket so
//! it builds everywhere `kols` does, with nothing but the standard library.
//!
//! Starting, accepting and sending each reconcile at once, as the window's
//! commands do; anything else waits for the tick.

#![deny(missing_docs)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime};

use clap::{Parser, Subcommand};
use intranet_crypto::to_hex;
use kols_api::{Command as ApiCommand, Event, Outcome};
use kols_node::executor::Executor;
use kols_node::nodes::{Nodes, TaggedReport, TaggedSink};
use kols_node::workspace::Workspace;

/// The window's supervisor tick (`kols-app` `SUPERVISOR_TICK`).
const WINDOW_TICK_SECS: u64 = 60;

/// How often the inbox is looked at.
const POLL: Duration = Duration::from_millis(50);

/// An installation, run headless.
#[derive(Parser)]
#[command(name = "kols-drive", version, about, long_about = None)]
struct Cli {
    /// The workspace: a directory of networks, not one store.
    #[arg(long)]
    home: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Hold the installation, run its nodes, and print what happens.
    Run {
        /// Seconds between supervisor passes. The window's is 60.
        #[arg(long, default_value_t = WINDOW_TICK_SECS)]
        tick_secs: u64,
    },
    /// Create a network with one relay. Prints its id.
    Create {
        /// What to call it.
        label: String,
        /// The relay, as a multiaddr ending in `/p2p/<peer-id>`.
        relay: String,
    },
    /// Print a network's recorded addresses, one per line.
    Addresses {
        /// The network, by the start of its id.
        network: String,
    },
    /// Mint a one-use invite. Prints it.
    Invite {
        /// The network, by the start of its id.
        network: String,
    },
    /// Redeem an invite. Prints the network id, then `admitted` or `waiting <identity>`.
    Join {
        /// The invite.
        invite: String,
    },
    /// Admit somebody waiting.
    Admit {
        /// The network, by the start of its id.
        network: String,
        /// Their identity, as hex.
        identity: String,
    },
    /// Print this installation's identity in a network, as hex.
    Identity {
        /// The network, by the start of its id.
        network: String,
    },
    /// Print `keyed` once this member holds the network's epoch key, else `unkeyed`.
    Keyed {
        /// The network, by the start of its id.
        network: String,
    },
    /// Offer a conversation to somebody in a network. Prints its id.
    Start {
        /// The shared network, by the start of its id.
        network: String,
        /// Their identity there, as hex.
        identity: String,
    },
    /// Print who has asked for a conversation in a network, one identity per line.
    Requests {
        /// The shared network, by the start of its id.
        network: String,
    },
    /// Accept a request. Prints the conversation's id.
    Accept {
        /// The shared network, by the start of its id.
        network: String,
        /// Who asked, as hex.
        from: String,
    },
    /// Say something in a conversation. Prints the record's id.
    Send {
        /// The conversation, by the start of its id.
        conversation: String,
        /// What to say.
        body: Vec<String>,
    },
    /// Print the running nodes and their tiers.
    Running,
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Run { tick_secs } => run(&cli.home, Duration::from_secs(tick_secs)),
        request => ask(&cli.home, &request_line(&request)),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("kols-drive: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// A request as one line of words, the inbox's format.
fn request_line(request: &Command) -> Vec<String> {
    let words: Vec<&str> = match request {
        Command::Run { .. } => unreachable!("run is not a request"),
        Command::Create { label, relay } => vec!["create", label, relay],
        Command::Addresses { network } => vec!["addresses", network],
        Command::Invite { network } => vec!["invite", network],
        Command::Join { invite } => vec!["join", invite],
        Command::Admit { network, identity } => vec!["admit", network, identity],
        Command::Identity { network } => vec!["identity", network],
        Command::Keyed { network } => vec!["keyed", network],
        Command::Start { network, identity } => vec!["start", network, identity],
        Command::Requests { network } => vec!["requests", network],
        Command::Accept { network, from } => vec!["accept", network, from],
        Command::Send { conversation, body } => {
            let mut words = vec!["send", conversation.as_str()];
            words.extend(body.iter().map(String::as_str));
            words
        }
        Command::Running => vec!["running"],
    };
    words.into_iter().map(str::to_owned).collect()
}

fn inbox(home: &Path) -> PathBuf {
    home.join("drive").join("inbox")
}

/// Hands a request to the running process and prints its answer.
fn ask(home: &Path, words: &[String]) -> Result<(), String> {
    let inbox = inbox(home);
    if !inbox.is_dir() {
        return Err(format!("nothing is running for {}", home.display()));
    }
    let name = format!("{}-{}", now_millis(), std::process::id());
    let partial = inbox.join(format!("{name}.part"));
    std::fs::write(&partial, words.join("\n")).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, inbox.join(format!("{name}.req"))).map_err(|e| e.to_string())?;

    let answer = inbox.join(format!("{name}.ans"));
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(&answer) {
            let _ = std::fs::remove_file(&answer);
            return match text.strip_prefix("ok\n") {
                Some(body) => {
                    print!("{body}");
                    Ok(())
                }
                None => Err(text.strip_prefix("err\n").unwrap_or(&text).to_owned()),
            };
        }
        std::thread::sleep(POLL);
    }
    Err("no answer within 120 s".into())
}

fn now_millis() -> i64 {
    kols_node::chat::now_millis()
}

/// Prints one line of the run's log.
fn log(line: &str) {
    println!("{} {line}", now_millis());
}

/// What the running process holds.
struct Held {
    workspace: Workspace,
    nodes: Mutex<Nodes>,
    sink: TaggedSink,
    runtime: tokio::runtime::Runtime,
}

impl Held {
    /// One supervisor pass, as the window's `reconcile_nodes` makes it.
    fn reconcile(&self) {
        let specs = kols_node::nodes::specs_for(&self.workspace, SystemTime::now());
        if let Ok(mut nodes) = self.nodes.lock() {
            nodes.reconcile(
                &specs,
                None,
                &self.sink,
                kols_node::serve::SEAL_TARGET_BYTES,
                Instant::now(),
            );
        }
    }
}

fn run(home: &Path, tick: Duration) -> Result<(), String> {
    // To stderr, which `nat-dm.sh` sends to the same log; silent unless asked.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init();
    let workspace = Workspace::at(home.to_path_buf());
    std::fs::create_dir_all(inbox(home)).map_err(|e| e.to_string())?;
    workspace.ensure_unlocked()?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;

    // A knock that checks out wakes a conversation; the window reconciles on it
    // at once rather than waiting for its tick, and so does this. Sent to the
    // loop below rather than reconciled here: this runs inside a node's event
    // loop, and reconciling starts and stops nodes.
    let (woken, wakes) = mpsc::channel::<()>();
    let woken = Mutex::new(woken);
    let sink: TaggedSink = Arc::new(move |network, events| {
        for event in events {
            log(&format!("{} event {}", network.short(), describe(event)));
            if matches!(event, Event::ConversationWoken { .. })
                && let Ok(woken) = woken.lock()
            {
                let _ = woken.send(());
            }
        }
    });
    let report: TaggedReport = Arc::new(|network, line| {
        for part in line.lines() {
            log(&format!("{} node {part}", network.short()));
        }
    });

    let held = Held {
        nodes: Mutex::new(Nodes::new(runtime.handle().clone()).with_report(report)),
        workspace,
        sink,
        runtime,
    };
    log(&format!("run {}", home.display()));
    held.reconcile();
    let mut next_tick = Instant::now() + tick;

    loop {
        if wakes.recv_timeout(POLL).is_ok() {
            while wakes.try_recv().is_ok() {}
            log("supervisor woken");
            held.reconcile();
        }
        if Instant::now() >= next_tick {
            held.reconcile();
            next_tick = Instant::now() + tick;
        }
        for request in pending(&inbox(home)) {
            answer(&held, &request);
        }
    }
}

/// Requests waiting in the inbox, oldest first.
fn pending(inbox: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(inbox) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "req"))
        .collect();
    found.sort();
    found
}

fn answer(held: &Held, request: &Path) {
    let words: Vec<String> = std::fs::read_to_string(request)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    let _ = std::fs::remove_file(request);
    log(&format!("request {}", words.join(" ")));
    let reply = match handle(held, &words) {
        Ok(body) => format!("ok\n{body}"),
        Err(message) => {
            log(&format!("refused {message}"));
            format!("err\n{message}")
        }
    };
    let answer = request.with_extension("ans");
    let partial = request.with_extension("anspart");
    if std::fs::write(&partial, reply).is_ok() {
        let _ = std::fs::rename(&partial, answer);
    }
}

fn handle(held: &Held, words: &[String]) -> Result<String, String> {
    let word = |n: usize| words.get(n).map(String::as_str).ok_or("too few words");
    let ws = &held.workspace;
    match word(0)? {
        "create" => {
            let relay = kols_node::parse_relay(word(2)?)?;
            let store = ws.create(word(1)?, vec![relay])?;
            held.reconcile();
            Ok(format!("{}\n", to_hex(store.network().as_bytes())))
        }
        "addresses" => Ok(lines(ws.open(word(1)?)?.addresses())),
        "invite" => {
            let store = ws.open(word(1)?)?;
            let outcome = Executor::open(store.root().to_path_buf())
                .map_err(|e| e.to_string())?
                .submit(ApiCommand::CreateInvite {
                    uses: 1,
                    valid_for_hours: 24,
                })
                .map_err(|e| e.to_string())?;
            match outcome {
                Outcome::InviteCreated { invite, .. } => Ok(format!(
                    "{}\n",
                    kols_node::invite::to_uri_from_bytes(&invite)
                )),
                other => Err(format!("unexpected {other:?}")),
            }
        }
        "join" => {
            let invite = kols_node::invite::from_uri(word(1)?)?;
            let network = invite.network;
            let landed = held.runtime.block_on(kols_node::join::redeem(
                ws.path_for(&network),
                invite,
                30,
                &kols_node::quiet(),
                kols_core::NetworkProfile::Server,
            ))?;
            held.reconcile();
            let how = match landed {
                kols_node::join::Landed::Admitted => "admitted".to_owned(),
                kols_node::join::Landed::Waiting { identity } => format!("waiting {identity}"),
                other => format!("{other:?}"),
            };
            Ok(format!("{}\n{how}\n", to_hex(network.as_bytes())))
        }
        "admit" => {
            let store = ws.open(word(1)?)?;
            let identity = kols_node::parse_identity(word(2)?)?;
            Executor::open(store.root().to_path_buf())
                .map_err(|e| e.to_string())?
                .submit(ApiCommand::AdmitMember {
                    identity,
                    via_invite: None,
                })
                .map_err(|e| e.to_string())?;
            Ok(String::new())
        }
        "identity" => {
            let store = ws.open(word(1)?)?;
            let id = store.identity().map_err(|e| e.to_string())?.id();
            Ok(format!("{}\n", to_hex(id.verifying_key().as_bytes())))
        }
        "keyed" => {
            let store = ws.open(word(1)?)?;
            Ok(if store.epoch_key().is_ok() {
                "keyed\n".into()
            } else {
                "unkeyed\n".into()
            })
        }
        "start" => {
            let shared = ws.open(word(1)?)?;
            let with = kols_node::parse_identity(word(2)?)?;
            let started = kols_node::dm::start(ws, &shared, &with, None)?;
            // So its node starts now, as the window's `start_conversation` does.
            held.reconcile();
            Ok(format!("{}\n", to_hex(started.conversation.as_bytes())))
        }
        "requests" => {
            let shared = ws.open(word(1)?)?;
            Ok(lines(
                shared
                    .requests()
                    .into_iter()
                    .map(|(from, _)| to_hex(from.verifying_key().as_bytes())),
            ))
        }
        "accept" => {
            let shared = *ws.open(word(1)?)?.network();
            let from = kols_node::parse_identity(word(2)?)?;
            let accepted = held
                .runtime
                .block_on(kols_node::dm::accept(ws, &shared, &from));
            // Before the result is looked at, as the window does: `accept` can
            // fail after joining, and that conversation still needs its node.
            held.reconcile();
            Ok(format!("{}\n", to_hex(accepted?.as_bytes())))
        }
        "send" => {
            let store = ws.open(word(1)?)?;
            let conversation = *store.network();
            let body = words[2..].join(" ");
            let outcome = Executor::open(store.root().to_path_buf())
                .map_err(|e| e.to_string())?
                .submit(ApiCommand::SendMessage {
                    channel: kols_core::conversation_channel_id(&conversation),
                    body,
                    reply_to: None,
                    attachments: Vec::new(),
                })
                .map_err(|e| e.to_string())?;
            let Outcome::Wrote { record } = outcome else {
                return Err(format!("unexpected {outcome:?}"));
            };
            log(&format!("{} sent {}", conversation.short(), record.short()));
            held.reconcile();
            Ok(format!("{}\n", record.short()))
        }
        "running" => {
            let nodes = held.nodes.lock().map_err(|_| "poisoned")?;
            Ok(lines(nodes.running().into_iter().map(|(network, tier)| {
                format!("{} {}", network.short(), tier.label())
            })))
        }
        other => Err(format!("unknown request {other}")),
    }
}

fn lines(items: impl IntoIterator<Item = String>) -> String {
    items.into_iter().map(|item| format!("{item}\n")).collect()
}

/// One event, as one line. A record says how late it is by its own clock, which
/// on one host is the same clock its sender stamped it with.
fn describe(event: &Event) -> String {
    match event {
        Event::Records {
            records, arrival, ..
        } => {
            let now = now_millis();
            records
                .iter()
                .map(|record| {
                    format!(
                        "record {} {:?} lag {}ms",
                        record.id().short(),
                        arrival,
                        now - record.hlc.wall_millis
                    )
                })
                .collect::<Vec<_>>()
                .join(" ; ")
        }
        other => {
            let text = format!("{other:?}");
            text.chars().take(240).collect()
        }
    }
}
