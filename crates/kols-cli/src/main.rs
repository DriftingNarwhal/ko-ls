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
//! # What it deliberately is not
//!
//! Not the product. `design/05` describes a Tauri client with a capability-shaped
//! API boundary, and this is not a step toward it — it is a way to exercise the
//! layers underneath it from a terminal. Where it takes a shortcut, the shortcut
//! is named where it is taken rather than hidden behind a plausible surface: see
//! [`store::Store::channel_dek`], which is the significant one.

#![deny(missing_docs)]

mod chat;
mod network;
mod serve;
mod store;

use clap::{Parser, Subcommand};
use intranet_crypto::to_hex;
use store::Store;

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
    },
    /// Show this member's identity and what they may do.
    Whoami,
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
    /// Render a channel.
    Read {
        /// The channel, by name or by the start of its id.
        channel: String,
    },
    /// Run this node so peers can reach it, and pull in what they hold.
    Serve {
        /// What to listen on.
        #[arg(long, default_value = "/ip4/0.0.0.0/tcp/0")]
        listen: String,
        /// Peers to dial on startup, as multiaddrs including `/p2p/<peer-id>`.
        #[arg(long = "peer")]
        peers: Vec<String>,
    },
    /// Admit an identity to this network.
    Admit {
        /// The joiner's identity in this network, as hex.
        identity: String,
    },
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
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let root = cli.home.unwrap_or_else(Store::default_root);

    let result = match cli.command {
        Command::Init { name } => init(root, &name),
        Command::Whoami => whoami(root),
        Command::Channel(ChannelCommand::Create {
            name,
            private,
            topic,
        }) => chat::create_channel(root, &name, private, &topic),
        Command::Channel(ChannelCommand::List) => chat::list_channels(root),
        Command::Post { channel, message } => chat::post(root, &channel, &message.join(" ")),
        Command::Read { channel } => chat::read(root, &channel),
        Command::Serve { listen, peers } => serve::run(root, &listen, &peers),
        Command::Admit { identity } => admit(root, &identity),
        Command::Attach { network, name } => attach(root, &network, &name),
    };

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("kols: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn init(root: std::path::PathBuf, name: &str) -> Result<(), String> {
    // Both are random and independent: the network id names the network, the
    // entropy derives this member's identity in it. Deriving one from the other
    // would make a member's identity a function of public information.
    let network_bytes = random_32()?;
    let entropy = random_32()?;
    let network = intranet_identity::NetworkId::from_bytes(network_bytes);

    let store = Store::create(root, network, entropy).map_err(|e| e.to_string())?;
    let founder = store.identity().map_err(|e| e.to_string())?;
    let genesis = network::genesis(&founder, network);
    store.append_entry(&genesis).map_err(|e| e.to_string())?;
    store.set_label(name).map_err(|e| e.to_string())?;

    // The network's key group is deliberately *not* created here. An MLS group
    // is live cryptographic state that `GroupSession` keeps in an in-memory
    // provider, so it exists only for as long as the process that made it — and
    // keying a new member in requires that live group. A one-shot command cannot
    // hold one, so `kols serve` creates it and this leaves the network unkeyed
    // until it runs.

    // Replay immediately rather than trusting the entry we just wrote. A genesis
    // this node cannot replay is a network nobody can join, and finding that out
    // now costs one line.
    let state = store.state().map_err(|e| e.to_string())?;
    if !state.is_member(&founder.id()) {
        return Err("genesis replayed but did not make its founder a member".to_owned());
    }

    println!("created {name}");
    println!("  network   {}", to_hex(network.as_bytes()));
    println!("  you       {}", founder.id().short());
    println!("  state     {}", store.root().display());
    println!();
    println!("The seed in {}/seed is the only copy.", store.root().display());
    println!("Losing it loses this identity, and there is no recovery service.");
    println!();
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

/// Admits an identity to this network.
///
/// The explicit-intake half of `design/02` §6.2: a joiner reaching this network
/// holds connectivity and an identity, and nothing else, until somebody with the
/// authority puts them in a group. Adding them to `everyone` is what grants the
/// capabilities genesis handed that group.
fn admit(root: std::path::PathBuf, identity_hex: &str) -> Result<(), String> {
    let store = Store::open(root).map_err(|e| e.to_string())?;
    let admitter = store.identity().map_err(|e| e.to_string())?;
    let joiner = parse_identity(identity_hex)?;

    let head = store
        .head()
        .map_err(|e| e.to_string())?
        .ok_or("this network has no genesis to build on")?;
    let entry = intranet_governance::LogEntry::create(
        &admitter,
        Some(head),
        intranet_crypto::Timestamp::from_millis(chat::now_millis()),
        intranet_governance::EntryBody::MembershipChange {
            group: intranet_governance::GroupId::everyone(),
            identity: joiner,
            action: intranet_governance::MembershipAction::Add { via_invite: None },
        },
    );
    store.append_entry(&entry).map_err(|e| e.to_string())?;

    // Replay rather than trust: admission is gated on `approve-node`, and an
    // entry the log accepts structurally is still refused by replay if the
    // admitter did not hold it. Reporting success without checking would tell
    // somebody they had let a person in when they had not.
    let state = store.state().map_err(|err| {
        format!("{err}\n\nAdmission needs approve-node, which the founder holds by default.")
    })?;
    if !state.is_member(&joiner) {
        return Err("the entry was written but replay did not admit them".to_owned());
    }

    println!("admitted {}", joiner.short());
    println!("They can read and post once they have synced this log.");
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

fn parse_identity(hex: &str) -> Result<intranet_identity::PerNetworkIdentityId, String> {
    let bytes = intranet_crypto::from_hex(hex.trim())
        .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
        .ok_or("an identity is 64 hex characters")?;
    let key = intranet_crypto::VerifyingKey::from_bytes(bytes)
        .map_err(|_| "those bytes are not a valid identity key".to_owned())?;
    Ok(intranet_identity::PerNetworkIdentityId::from_verifying_key(key))
}

/// 32 bytes from the OS.
fn random_32() -> Result<[u8; 32], String> {
    let mut bytes = [0u8; 32];
    intranet_crypto::random_bytes(&mut bytes)
        .map_err(|err| format!("could not read entropy: {err}"))?;
    Ok(bytes)
}
