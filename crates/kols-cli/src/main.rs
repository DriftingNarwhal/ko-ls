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

    // A real MLS group of one, whose exported secret is this network's first
    // epoch key (Core §3.3). Every DEK is wrapped under it, so what protects
    // content is key material only members hold rather than anything derivable
    // from the network's public identifiers.
    let group = intranet_epoch::GroupSession::create(&intranet_epoch::identity_label(&founder.id()))
        .map_err(|err| format!("could not create the network's key group: {err}"))?;
    let epoch = group
        .epoch_key()
        .map_err(|err| format!("could not derive the first epoch key: {err}"))?;
    // Genesis is the rotation this first epoch belongs to. A rotation reference
    // is an entry hash rather than an epoch ordinal, because two branches can
    // each produce "the next epoch" with the same number (Storage §5.3).
    let rotation = store
        .head()
        .map_err(|e| e.to_string())?
        .ok_or("genesis was written but the log has no head")?;
    store
        .set_epoch_key(&epoch, rotation)
        .map_err(|e| e.to_string())?;

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

/// 32 bytes from the OS.
fn random_32() -> Result<[u8; 32], String> {
    let mut bytes = [0u8; 32];
    intranet_crypto::random_bytes(&mut bytes)
        .map_err(|err| format!("could not read entropy: {err}"))?;
    Ok(bytes)
}
