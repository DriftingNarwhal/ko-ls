//! `kols` — a terminal client for ko-ls.
//!
//! # What this is for
//!
//! Every layer of this project was tested before anything composed them. That is
//! the right order and it leaves one thing unproven: whether the pieces fit.
//! They did not — wiring channel entries to a real governance log immediately
//! found that only a Founder could create a channel, because an entry declared a
//! capability name nobody could have registered. The encoding tests could not
//! have caught it, and neither could the wire tests.
//!
//! So this exists to be the seam. Create a network, define a channel, post to
//! it, read it back — through the same permission resolution, the same canonical
//! encoding and the same storage layer a desktop client would use, with nothing
//! stubbed on the path a message actually takes.
//!
//! # What it is not
//!
//! **A product surface** (`design/00` D30). Nobody is expected to use this
//! application from a command line — the window is the client — so nothing here
//! owes the window feature parity, discoverability or documentation aimed at
//! somebody who has not read this file. What it owes is the property above:
//! crossing the same boundary a window crosses, so that "works here, not in the
//! window" is a sentence that means something.
//!
//! # What it is now
//!
//! Argument parsing and rendering, over `kols_node::executor`. Every command a
//! user types becomes a `kols_api::Command`, crosses the same gate a webview's
//! would, and comes back as an `Outcome` this file prints. That division is the
//! point: the desktop client is expected to be a different front end over the
//! same submit path rather than a second copy of it.
//!
//! Three things stay outside the command vocabulary, deliberately. `init` and
//! `attach` create the state a command needs before any exists, and `whoami`
//! reads local state and asks the network nothing.
//!
//! # Where it takes a shortcut, it says so
//!
//! Nothing here ever retires a superseded epoch key
//! ([`kols_node::store::Store::channel_dek`]) — a retention decision rather than
//! an oversight, because dropping a key makes anything still wrapped under it
//! unreadable forever.

#![deny(missing_docs)]

use clap::{Parser, Subcommand};
use intranet_crypto::to_hex;
use kols_api::{Command as ApiCommand, Outcome};
use kols_node::executor::{ExecuteError, Executor};
use kols_node::store::Store;
use kols_node::{network, random_32, serve};
use kols_core::{ChannelChange, Hlc, Privacy};

/// A terminal client for ko-ls.
#[derive(Parser)]
#[command(name = "kols", version, about, long_about = None)]
struct Cli {
    /// Where state lives. Defaults to `$KOLS_HOME`, else `~/.kols`.
    #[arg(long, global = true)]
    home: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a network. You become its sole Founder.
    Init {
        /// What to call it locally.
        name: String,
        /// A relay this network uses as an entry point. Repeatable.
        ///
        /// Two members behind NAT cannot reach each other directly, so a network
        /// needs at least one (Core §5.5). `intranet-harness relay` runs one
        /// locally; DI-Relay deploys one. It can be replaced later with
        /// `kols relay set`.
        #[arg(long = "relay")]
        relays: Vec<String>,
    },
    /// Show this member's identity and what they may do.
    Whoami,
    /// Claim a display name in this network.
    ///
    /// Names are unique per network and bound permanently — including after you
    /// leave, so nobody can inherit yours and relabel what you wrote.
    Name {
        /// The name you want.
        name: Vec<String>,
    },
    /// Work with channels.
    #[command(subcommand)]
    Channel(ChannelCommand),
    /// Write a message to a channel.
    Post {
        /// The channel, by name or by the start of its id.
        channel: String,
        /// What to say.
        message: Vec<String>,
    },
    /// Revise one of your own messages.
    Edit {
        /// The channel, by name or by the start of its id.
        channel: String,
        /// The message, by the start of its id — `kols read` prints them.
        message: String,
        /// What it should say instead.
        body: Vec<String>,
    },
    /// Withdraw one of your own messages.
    Delete {
        /// The channel, by name or by the start of its id.
        channel: String,
        /// The message, by the start of its id.
        message: String,
    },
    /// React to a message, or take a reaction back.
    React {
        /// The channel, by name or by the start of its id.
        channel: String,
        /// The message, by the start of its id.
        message: String,
        /// The reaction itself.
        key: String,
        /// Remove this reaction rather than add it.
        #[arg(long)]
        remove: bool,
    },
    /// Pin a message, or unpin it. Needs chat:moderate.
    Pin {
        /// The channel, by name or by the start of its id.
        channel: String,
        /// The message, by the start of its id.
        message: String,
        /// Unpin rather than pin.
        #[arg(long)]
        remove: bool,
    },
    /// Render a channel.
    Read {
        /// The channel, by name or by the start of its id.
        channel: String,
    },
    /// Run this node so peers can reach it, and pull in what they hold.
    Serve {
        /// What to listen on. Empty means the dual-stack defaults.
        ///
        /// TCP and QUIC over IPv4 and IPv6, which Core §5.1 requires and §5.2
        /// depends on: a pair behind CGNAT usually cannot traverse IPv4, so
        /// IPv6 is the path the spec designates for them rather than a relay.
        #[arg(long, default_value = "")]
        listen: String,
        /// Peers to dial on startup, as multiaddrs including `/p2p/<peer-id>`.
        #[arg(long = "peer")]
        peers: Vec<String>,
        /// Seal a segment and start a new one once it reaches this many bytes.
        ///
        /// Local publishing tuning (`design/01` §3.1), not a validity rule:
        /// readers accept whatever boundaries an author chose.
        #[arg(long, default_value_t = serve::SEAL_TARGET_BYTES)]
        seal_bytes: usize,
        /// Turn off live gossip delivery, leaving only the durable path.
        ///
        /// Slower and completely correct (spec 07 §6.1), which also requires
        /// that conformance be testable this way.
        #[arg(long)]
        no_live: bool,
        /// How recent a record must be for the live path to still carry it.
        #[arg(long, default_value_t = serve::LIVE_WINDOW_MILLIS)]
        live_window_millis: i64,
    },
    /// Mint an invite somebody can redeem to join this network.
    Invite {
        /// How many people may join with it.
        #[arg(long, default_value_t = 1)]
        uses: u32,
        /// How long it stays valid, in hours.
        #[arg(long, default_value_t = 24)]
        hours: i64,
    },
    /// Redeem an invite and join the network it names.
    Join {
        /// The invite, as given to you.
        invite: String,
        /// How long to wait for an answer, in seconds.
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },
    /// Show who has redeemed an invite and is waiting to be admitted.
    Waiting,
    /// Work with this network's relays.
    #[command(subcommand)]
    Relay(RelayCommand),
    /// Admit an identity to this network.
    Admit {
        /// The joiner's identity in this network, as hex.
        identity: String,
    },
    /// Remove an identity from this network.
    Revoke {
        /// The member's identity in this network, as hex.
        identity: String,
    },
    /// Leave this network, telling it so — Core §2.5.1.
    ///
    /// Writes one membership removal per group you are in, signed by you and
    /// needing no capability. It does **not** delete anything: the store and the
    /// seed stay, and `forget` in the window is what removes those. The order
    /// matters in the other direction though — these entries are signed by the
    /// seed, so they cannot be written after it is gone.
    Leave,
    /// Prepare a store for a network created elsewhere, before syncing it.
    Attach {
        /// The network id, as hex.
        network: String,
        /// What to call it locally.
        #[arg(long, default_value = "attached")]
        name: String,
    },
}

#[derive(Subcommand)]
enum RelayCommand {
    /// Show the relays this network designates.
    List,
    /// Replace them. Needs define-policy.
    Set {
        /// The multiaddrs, replacing the current set outright.
        relays: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ChannelCommand {
    /// Define a new channel.
    Create {
        /// Its name.
        name: String,
        /// Restrict it to a roster rather than the whole network.
        #[arg(long)]
        private: bool,
        /// A short description.
        #[arg(long, default_value = "")]
        topic: String,
    },
    /// List the channels replay currently knows about.
    List,
    /// Rename a channel. Needs chat:manage-channel.
    Rename {
        /// The channel, by name or by the start of its id.
        channel: String,
        /// Its new name.
        name: String,
    },
    /// Set a channel's topic. Needs chat:manage-channel.
    Topic {
        /// The channel, by name or by the start of its id.
        channel: String,
        /// The new topic.
        topic: Vec<String>,
    },
    /// Set a channel's slowmode, in seconds. Zero is off.
    Slowmode {
        /// The channel, by name or by the start of its id.
        channel: String,
        /// Seconds between one author's messages.
        seconds: u32,
    },
    /// Archive a channel: readable, not writable.
    Archive {
        /// The channel, by name or by the start of its id.
        channel: String,
    },
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let root = cli.home.unwrap_or_else(Store::default_root);

    let result = match cli.command {
        Command::Init { name, relays } => init(root, &name, relays),
        Command::Whoami => whoami(root),
        Command::Attach { network, name } => attach(root, &network, &name),
        Command::Serve {
            listen,
            peers,
            seal_bytes,
            no_live,
            live_window_millis,
        } => serve::run(
            root,
            &listen,
            &peers,
            seal_bytes,
            !no_live,
            live_window_millis,
        ),
        Command::Channel(ChannelCommand::List) => list_channels(root),
        Command::Join { invite, timeout } => kols_node::join::run(root, &invite, timeout),
        Command::Waiting => waiting(root),
        Command::Relay(RelayCommand::List) => list_relays(root),
        other => submit(root, other),
    };

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("kols: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Turns a typed subcommand into an API command, submits it, and renders it.
///
/// Everything that changes anything goes through here, which is what makes the
/// terminal an ordinary consumer of the boundary rather than a privileged one.
fn submit(root: std::path::PathBuf, command: Command) -> Result<(), String> {
    let executor = Executor::open(root).map_err(|e| e.to_string())?;

    let api = match command {
        Command::Post { channel, message } => {
            let channel = executor.resolve_channel(&channel).map_err(say)?;
            ApiCommand::SendMessage {
                channel,
                body: message.join(" "),
                reply_to: None,
                attachments: Vec::new(),
            }
        }
        Command::Edit {
            channel,
            message,
            body,
        } => {
            let channel = executor.resolve_channel(&channel).map_err(say)?;
            let target = executor.resolve_message(&channel, &message).map_err(say)?;
            ApiCommand::EditMessage {
                channel,
                target,
                body: body.join(" "),
            }
        }
        Command::Delete { channel, message } => {
            let channel = executor.resolve_channel(&channel).map_err(say)?;
            let target = executor.resolve_message(&channel, &message).map_err(say)?;
            ApiCommand::DeleteMessage { channel, target }
        }
        Command::React {
            channel,
            message,
            key,
            remove,
        } => {
            let channel = executor.resolve_channel(&channel).map_err(say)?;
            let target = executor.resolve_message(&channel, &message).map_err(say)?;
            ApiCommand::React {
                channel,
                target,
                key,
                remove,
            }
        }
        Command::Pin {
            channel,
            message,
            remove,
        } => {
            let channel = executor.resolve_channel(&channel).map_err(say)?;
            let target = executor.resolve_message(&channel, &message).map_err(say)?;
            ApiCommand::Pin {
                channel,
                target,
                remove,
            }
        }
        Command::Read { channel } => {
            let channel = executor.resolve_channel(&channel).map_err(say)?;
            ApiCommand::OpenChannel {
                channel,
                before: None,
                // A terminal has no scroll position to page from, the same
                // reason `kols serve` walks to the start of history. A UI bounds
                // this by pages (`design/01` §5).
                limit: usize::MAX,
            }
        }
        Command::Name { name } => ApiCommand::SetName {
            name: name.join(" "),
        },
        Command::Invite { uses, hours } => ApiCommand::CreateInvite {
            uses,
            valid_for_hours: hours,
        },
        Command::Relay(RelayCommand::Set { relays }) => ApiCommand::SetBootstrapRelays {
            relays: relays
                .iter()
                .map(|relay| kols_node::parse_relay(relay))
                .collect::<Result<Vec<_>, _>>()?,
        },
        Command::Admit { identity } => ApiCommand::AdmitMember {
            identity: kols_node::parse_identity(&identity)?,
        },
        Command::Revoke { identity } => ApiCommand::RevokeMember {
            identity: kols_node::parse_identity(&identity)?,
        },
        Command::Leave => ApiCommand::LeaveNetwork,
        Command::Channel(ChannelCommand::Create {
            name,
            private,
            topic,
        }) => ApiCommand::CreateChannel {
            name,
            category: None,
            privacy: if private {
                Privacy::Private
            } else {
                Privacy::Public
            },
            topic,
        },
        Command::Channel(channel_command) => {
            let (needle, change) = match channel_command {
                ChannelCommand::Rename { channel, name } => (channel, ChannelChange::Rename(name)),
                ChannelCommand::Topic { channel, topic } => {
                    (channel, ChannelChange::SetTopic(topic.join(" ")))
                }
                ChannelCommand::Slowmode { channel, seconds } => {
                    (channel, ChannelChange::SetSlowmode(seconds))
                }
                ChannelCommand::Archive { channel } => (channel, ChannelChange::Archive),
                ChannelCommand::Create { .. } | ChannelCommand::List => {
                    unreachable!("handled above")
                }
            };
            let channel = executor.resolve_channel(&needle).map_err(say)?;
            ApiCommand::UpdateChannel { channel, change }
        }
        Command::Init { .. }
        | Command::Whoami
        | Command::Attach { .. }
        | Command::Relay(RelayCommand::List)
        | Command::Waiting
        | Command::Join { .. }
        | Command::Serve { .. } => unreachable!("handled outside the boundary"),
    };

    let outcome = executor.submit(api).map_err(say)?;
    let names = executor
        .store()
        .state()
        .ok()
        .and_then(|state| executor.names(&state).ok())
        .unwrap_or_default();
    render(&outcome, &names);
    Ok(())
}

fn say(err: ExecuteError) -> String {
    err.to_string()
}

/// Prints what a command produced.
/// How an author is shown: their name where they have claimed one, and always
/// enough of their identity to tell two similar names apart.
///
/// Spec 07 §8 makes that an obligation rather than a courtesy. Uniqueness is
/// decided on a key that deliberately does not fold confusables, so `alice` and
/// a Cyrillic lookalike can both exist — and the id beside them is what a reader
/// checks when something feels wrong.
fn who(identity: &intranet_identity::PerNetworkIdentityId, names: &kols_core::Names) -> String {
    match names.of(identity) {
        Some(name) => format!("{name} ({})", &identity.short()[..8]),
        None => identity.short(),
    }
}

fn render(outcome: &Outcome, names: &kols_core::Names) {
    match outcome {
        Outcome::Opened {
            messages,
            rejected,
            authors,
            ..
        } => {
            if messages.is_empty() {
                println!("nothing here yet");
            }
            for message in messages {
                let mut flags = Vec::new();
                if message.edited {
                    flags.push("edited");
                }
                if message.withdrawn {
                    flags.push("withdrawn");
                }
                if message.redacted {
                    flags.push("redacted");
                }
                if message.pinned {
                    flags.push("pinned");
                }
                let suffix = if flags.is_empty() {
                    String::new()
                } else {
                    format!("  [{}]", flags.join(", "))
                };
                // The id is printed because every other command that acts on a
                // message needs one, and a user who cannot see it cannot act.
                println!(
                    "{}  [{}] {}  {}{}",
                    &to_hex(message.id.as_bytes())[..8],
                    stamp(message.hlc),
                    who(&message.author, names),
                    message.body,
                    suffix
                );
                for (key, who) in &message.reactions {
                    println!("          {key} ×{}", who.len());
                }
            }

            for (id, rejection) in rejected {
                eprintln!("refused {}: {rejection:?}", &to_hex(id.as_bytes())[..8]);
            }
            println!();
            println!(
                "{} message(s) from {authors} author(s). `kols serve` brings in what other \
                 members wrote.",
                messages.len()
            );
        }
        Outcome::Wrote {
            record,
            moved,
            total,
        } => {
            println!("wrote {}", &to_hex(record.as_bytes())[..8]);
            println!("  moved {moved} of {total} bytes");
        }
        Outcome::ChannelCreated {
            channel,
            name,
            privacy,
        } => {
            println!("created #{name}");
            println!("  id       {}", to_hex(channel.as_bytes()));
            println!(
                "  privacy  {}",
                if *privacy == Privacy::Private {
                    "private (roster keying is not implemented yet — see design/03 §3)"
                } else {
                    "public"
                }
            );
        }
        Outcome::NameClaimed { name } => {
            println!("you are {name} here");
            println!();
            println!("That name is yours in this network permanently — it stays bound to you");
            println!("even if you claim another, so nobody inherits it and your history with it.");
        }
        Outcome::InviteCreated {
            invite,
            expires_at_millis,
            uses,
        } => {
            println!("{}", kols_node::invite::to_uri_from_bytes(invite));
            println!();
            let hours = (expires_at_millis - kols_node::chat::now_millis()) / 3_600_000;
            println!("Good for {uses} join(s), for about {hours} more hour(s).");
            println!("Whoever has it runs `kols join <that>`.");
            println!();
            // Said rather than assumed: the addresses inside it are the ones
            // this node last reported, and an invite pointing at a node nobody
            // is running connects to nothing.
            println!("It carries this node's addresses, so `kols serve` has to be running");
            println!("for anybody to redeem it.");
        }
        Outcome::BootstrapRelaysSet { relays } => {
            if relays.is_empty() {
                println!("this network now designates no relays");
                println!();
                println!("Members who cannot already dial each other will not be able to");
                println!("reconnect. Core §5.5 covers why.");
            } else {
                println!("this network's relays:");
                for relay in relays {
                    println!("  {relay}");
                }
                println!();
                println!("Every member learns these by syncing, so a newly deployed relay");
                println!("reaches people who joined long ago.");
                println!();
                // The window restarts its own node here, being the same process.
                // A terminal cannot, so it says what it cannot do rather than
                // leaving a running daemon designating a relay it never dialled.
                println!("A node dials relays when it starts. Restart any `kols serve`");
                println!("running for this network to use these.");
            }
        }
        Outcome::ChannelUpdated { channel } => {
            println!("updated {}", &to_hex(channel.as_bytes())[..12]);
        }
        // Rendered but not reachable from any subcommand here. D30: the terminal
        // owes the window no parity, and folders are a window's affordance — what
        // it must keep is crossing the same boundary, which printing an outcome
        // it can receive is part of.
        Outcome::CategoryCreated { category, name } => {
            println!(
                "created category {name} {}",
                &to_hex(category.as_bytes())[..12]
            );
        }
        Outcome::CategoryUpdated { category } => {
            println!(
                "updated category {}",
                &to_hex(category.as_bytes())[..12]
            );
        }
        Outcome::Departed { groups } => {
            let named = groups
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            println!("left this network: {named}");
            // Said plainly because the difference is the whole of `design/02`
            // §6.5: writing the departure and having anybody hear it are two
            // events, and only the first has happened here.
            println!(
                "The entries are written locally. Other members learn of it when this \
                 node next reaches them, so keep `kols serve` running until it has."
            );
        }

        Outcome::MembershipChanged {
            identity,
            admitted,
            group,
        } if !group.is_everyone() => {
            // A role, not the network. Said differently on purpose: somebody
            // taken out of Moderators is still a member, and printing the
            // revocation wording would claim they had been removed from the
            // network — which is the one sentence here that would matter.
            if *admitted {
                println!("added {} to {group}", who(identity, names));
            } else {
                println!("removed {} from {group}", who(identity, names));
            }
        }
        Outcome::MembershipChanged {
            identity, admitted, ..
        } => {
            if *admitted {
                println!("admitted {}", who(identity, names));
                println!("They can read and post once they have synced this log.");
            } else {
                println!("removed {}", who(identity, names));
                println!("They are refused service by honest nodes from now on.");
                println!();
                println!(
                    "`kols serve` rotates the epoch to exclude them — until it runs, they can"
                );
                println!("still decrypt newly published content with the key they already hold.");
            }
        }
        Outcome::NetworkNamed { name } => {
            if name.is_empty() {
                println!("this network is now unnamed");
            } else {
                println!("this network is now called {name}");
            }
        }
        Outcome::ChatSettingSet { key, value } => {
            println!("{key} is now {value}");
        }
        Outcome::AdmissionModeSet { mode } => match mode {
            intranet_governance::AdmissionMode::AutoAdmit => {
                println!("a valid invite now admits somebody straight away");
                println!("Nobody reviews a joiner, and `everyone`'s capabilities are what");
                println!("they arrive holding.");
            }
            intranet_governance::AdmissionMode::ExplicitIntake => {
                println!("a valid invite now buys a connection and an identity, nothing more");
                println!("Joiners wait in the room until a member admits them: `kols waiting`.");
            }
        },
        Outcome::RoleCreated { group } => {
            println!("created the role {group}, holding nothing");
            println!("Grant it something before adding anybody, or it confers nothing.");
        }
        Outcome::PermissionSet {
            group,
            capability,
            granted,
        } => {
            if *granted {
                println!("{group} now holds {capability}");
            } else {
                println!("{group} no longer holds {capability}");
            }
        }
    }
}

fn stamp(hlc: Hlc) -> String {
    let secs = hlc.wall_millis / 1000;
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}")
}

/// Shows who redeemed an invite and is waiting for somebody to admit them.
///
/// Outside the command vocabulary because it is a local read of live node
/// state rather than an action — and because the waiting room lives in the
/// running daemon, which is why this reads what that daemon wrote down rather
/// than asking it.
fn waiting(root: std::path::PathBuf) -> Result<(), String> {
    let store = Store::open(root).map_err(|e| e.to_string())?;
    let state = store.state().map_err(|e| e.to_string())?;
    let identity = store.identity().map_err(|e| e.to_string())?;

    if !state.identity_holds(&identity.id(), &intranet_governance::Capability::ApproveNode) {
        return Err(
            "seeing who is waiting needs approve-node — the same capability admitting them does"
                .to_owned(),
        );
    }

    let waiting = store.waiting();
    if waiting.is_empty() {
        println!("nobody is waiting");
        println!();
        println!("A waiting room only fills while `kols serve` is running, since that is");
        println!("what answers an invite. `kols invite` mints one to hand out.");
        return Ok(());
    }

    println!("waiting to be admitted:");
    for who in &waiting {
        println!("  {who}");
        println!("    kols admit {who}");
    }
    Ok(())
}

/// Lists channels as replay understands them.
///
/// Outside the command vocabulary because there is no event surface yet for
/// channel state to arrive on (`design/05` §3). It reads replayed state and
/// signs nothing, so it is a local question like `whoami`.
fn list_channels(root: std::path::PathBuf) -> Result<(), String> {
    let store = Store::open(root).map_err(|e| e.to_string())?;
    let state = store.state().map_err(|e| e.to_string())?;
    let (channels, refused) = network::channels(&store, &state).map_err(|e| e.to_string())?;

    if channels.is_empty() {
        println!("no channels yet. `kols channel create <name>`");
    }
    for channel in channels.values() {
        let mut flags = Vec::new();
        if channel.privacy == Privacy::Private {
            flags.push("private");
        }
        if channel.archived {
            flags.push("archived");
        }
        let suffix = if flags.is_empty() {
            String::new()
        } else {
            format!("  [{}]", flags.join(", "))
        };
        println!(
            "#{:<20} {}{}",
            channel.name,
            &to_hex(channel.id.as_bytes())[..12],
            suffix
        );
        if !channel.topic.is_empty() {
            println!("  {}", channel.topic);
        }
    }

    // Surfaced rather than swallowed: a channel entry this build refuses is one
    // some other client may be showing, and silence would make the two look
    // like they agree.
    for refusal in refused {
        eprintln!("skipped an entry: {refusal}");
    }
    Ok(())
}

/// Shows the relays replay currently names, and what this node has cached.
///
/// Both, because they answer different questions: replayed state is what the
/// network says now, and the cache is what this node could actually dial before
/// it has synced anything.
fn list_relays(root: std::path::PathBuf) -> Result<(), String> {
    let store = Store::open(root).map_err(|e| e.to_string())?;
    let designated = store
        .state()
        .map(|state| state.policy.bootstrap_relays.clone())
        .unwrap_or_default();
    let cached = store.relays();

    if designated.is_empty() {
        println!("this network designates no relays");
        println!();
        println!("Two members behind NAT cannot reach each other without one. Run one with");
        println!("`intranet-harness relay`, or deploy DI-Relay, then `kols relay set <addr>`.");
    } else {
        println!("designated by this network:");
        for relay in &designated {
            println!("  {relay}");
        }
    }

    if !cached.is_empty() && cached != designated {
        println!();
        println!("cached locally, and dialable before this node has synced:");
        for relay in &cached {
            println!("  {relay}");
        }
    }
    Ok(())
}

fn init(root: std::path::PathBuf, name: &str, relays: Vec<String>) -> Result<(), String> {
    // Through the workspace, which is the one place a network comes into being.
    // It was here, which meant the desktop shell could not create one without a
    // second copy of the genesis requirements — each of which is silent when
    // missed.
    //
    // `--home` still names one network's store rather than a directory of them,
    // so this creates it in place: a terminal is told which network to work with,
    // and a window offers a choice.
    if root.join("seed").is_file() {
        return Err(format!(
            "a network already exists at {}. Refusing to overwrite it — the seed there \
             cannot be recovered if it is lost",
            root.display()
        ));
    }
    // Checked before the network exists rather than after: `--relay` goes
    // straight into the genesis policy, and a bad one there is replayed by
    // everybody who ever joins.
    let relays = relays
        .iter()
        .map(|relay| kols_node::parse_relay(relay))
        .collect::<Result<Vec<_>, _>>()?;
    let workspace = kols_node::workspace::Workspace::at(root.clone());
    let store = workspace
        .create_at(root, name, relays.clone())
        .map_err(|err| err.to_string())?;
    let founder = store.identity().map_err(|e| e.to_string())?;

    // The network's key group is deliberately *not* created here. An MLS group
    // is live cryptographic state that `GroupSession` keeps in an in-memory
    // provider, so it exists only for as long as the process that made it — and
    // keying a new member in requires that live group. A one-shot command cannot
    // hold one, so `kols serve` creates it and this leaves the network unkeyed
    // until it runs.

    println!("created {name}");
    println!("  network   {}", to_hex(store.network().as_bytes()));
    println!("  you       {}", founder.id().short());
    println!("  state     {}", store.root().display());
    println!();
    println!("The seed in {}/seed is the only copy.", store.root().display());
    println!("Losing it loses this identity, and there is no recovery service.");
    println!();
    if relays.is_empty() {
        println!("No relay designated. Two members behind NAT cannot reach each other");
        println!("without one, and `kols invite` will refuse until this network has one:");
        println!("  kols relay set /ip4/<host>/tcp/<port>/p2p/<peer-id>");
        println!();
    }
    println!("Next: `kols serve` keys this network, and must run before posting.");
    Ok(())
}

fn whoami(root: std::path::PathBuf) -> Result<(), String> {
    let store = Store::open(root).map_err(|e| e.to_string())?;
    let identity = store.identity().map_err(|e| e.to_string())?;
    let state = store.state().map_err(|e| e.to_string())?;

    println!("network  {}", store.label().unwrap_or_default());
    println!("  id     {}", to_hex(store.network().as_bytes()));
    println!("you      {}", identity.id().short());
    println!("  member {}", state.is_member(&identity.id()));

    // The name, and what to do about not having one. A member without a name is
    // an ordinary state rather than a broken one — spec 07 §3.9 makes claiming
    // it the member's own act, so nothing can do it on their behalf at join.
    let executor = Executor::open(store.root().to_path_buf()).map_err(|e| e.to_string())?;
    let names = executor.names(&state).map_err(|e| e.to_string())?;
    match names.of(&identity.id()) {
        Some(name) => println!("  name   {name}"),
        None => println!("  name   none yet — `kols name <name>` claims one"),
    }

    // Which epoch this node is on. A fingerprint rather than the key: epoch keys
    // implement no Debug precisely so a network's content confidentiality cannot
    // reach a log line by accident, and "which epoch am I on" is still a real
    // question when two nodes disagree about what they can read.
    match (store.epoch_key(), store.rotation_ref()) {
        (Ok(key), Ok(rotation)) => {
            println!("epoch    {}", &to_hex(key.fingerprint().as_bytes())[..16]);
            println!("  from   {}", &to_hex(rotation.as_bytes())[..16]);
        }
        _ => println!("epoch    none stored — this node cannot read or write content"),
    }

    // What this member may actually do, resolved rather than assumed — the same
    // question a reader asks before admitting one of their records.
    let verbs = [
        ("post", "chat:post:*"),
        ("read", "chat:read:*"),
        ("create channels", "chat:create-channel:*"),
        ("manage channels", "chat:manage-channel:*"),
        ("moderate", "chat:moderate:*"),
    ];
    println!("may:");
    for (label, capability) in verbs {
        let holds = state.identity_holds(
            &identity.id(),
            &intranet_governance::Capability::extension(capability.to_owned()),
        );
        println!("  {:<16} {}", label, if holds { "yes" } else { "no" });
    }
    Ok(())
}

/// Prepares a store for a network created elsewhere.
///
/// A joiner needs an identity in the network *before* anybody can admit them,
/// and that identity is derived from the network id (Core §1.2) — so this comes
/// first, prints who they will be, and leaves the log empty for a sync to fill.
fn attach(root: std::path::PathBuf, network_hex: &str, name: &str) -> Result<(), String> {
    let bytes = intranet_crypto::from_hex(network_hex.trim())
        .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
        .ok_or("a network id is 64 hex characters")?;
    let network = intranet_identity::NetworkId::from_bytes(bytes);
    let entropy = random_32()?;

    let store = Store::create(root, network, entropy).map_err(|e| e.to_string())?;
    let identity = store.identity().map_err(|e| e.to_string())?;
    store.set_label(name).map_err(|e| e.to_string())?;

    println!("attached to {}", to_hex(network.as_bytes()));
    println!("  you       {}", identity.id().short());
    println!();
    println!("Ask a member to run:");
    println!("  kols admit {}", to_hex(identity.id().verifying_key().as_bytes()));
    println!("then `kols serve --peer <their address>` to sync.");
    Ok(())
}


