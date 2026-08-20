//! `kols-desktop` — a window over the ko-ls API boundary.
//!
//! # What this process is
//!
//! A Tauri v2 shell holding one [`Executor`], and a webview holding no keys, no
//! sockets and no files (`design/05` §1). Everything the interface can do is a
//! command that crosses `kols-api` and comes back as an outcome; everything it
//! learns is an event. This file is the crossing and nothing else — it builds
//! commands from plain arguments, submits them, and converts what comes back
//! into the view shapes in [`dto`].
//!
//! # What it deliberately does not do
//!
//! **It does not run a node.** `kols serve` does that, and until this shell
//! grows its own the window shows what this node's store already holds: its own
//! messages, and anything a daemon fetched. Posting works, because writing a
//! record is a local act; hearing from anybody else needs a node, and pretending
//! otherwise would be a window that looks connected and is not.
//!
//! **It does not choose a network.** One store is one network, so this opens the
//! one at `$KOLS_HOME`. `design/09` §1's switcher needs a store of stores, which
//! does not exist yet.
//!
//! # The CSP is load-bearing
//!
//! `tauri.conf.json` permits no remote origins at all. That is what makes the
//! user themes of `design/09` §6 safe rather than merely unlikely to leak: CSS
//! can exfiltrate exactly one way, by causing a network request, and `url()`,
//! `@import` and `@font-face src` are the complete set. Under a CSP with no
//! remote origins, arbitrary user CSS *cannot* phone home.

#![deny(missing_docs)]

mod dto;

use intranet_crypto::to_hex;
use kols_api::{Command, Outcome};
use kols_cli::executor::Executor;
use kols_cli::network;
use kols_core::ChannelId;

/// What every command handler shares: one open store, and the executor over it.
struct App {
    executor: Executor,
}

impl App {
    /// Parses a channel id the interface handed back.
    ///
    /// The interface only ever returns an id this process gave it, so a
    /// malformed one is a bug rather than an attack — but it is refused the same
    /// way regardless, because the alternative is deciding which it was.
    fn channel(hex: &str) -> Result<ChannelId, String> {
        intranet_crypto::from_hex(hex.trim())
            .and_then(|bytes| <[u8; 32]>::try_from(bytes.as_slice()).ok())
            .map(ChannelId::from_bytes)
            .ok_or_else(|| "that is not a channel id".to_owned())
    }
}

/// Who this member is here, and what they may do.
#[tauri::command]
fn me(app: tauri::State<'_, App>) -> Result<dto::Me, String> {
    let store = app.executor.store();
    let identity = store.identity().map_err(|e| e.to_string())?;
    let state = store.state().map_err(|e| e.to_string())?;
    let holds = |name: &str| {
        state.identity_holds(
            &identity.id(),
            &intranet_governance::Capability::extension(name.to_owned()),
        )
    };

    let names = app.executor.names(&state).map_err(|e| e.to_string())?;

    Ok(dto::Me {
        identity: identity.id().short(),
        name: names.of(&identity.id()).map(str::to_owned),
        network: to_hex(store.network().as_bytes()),
        label: store.label().unwrap_or_default(),
        has_key: store.epoch_key().is_ok(),
        may_post: holds("chat:post:*"),
        may_create_channel: holds("chat:create-channel:*"),
        may_invite: state.identity_holds(&identity.id(), &intranet_governance::Capability::ApproveNode),
    })
}

/// The channels replay currently knows about.
#[tauri::command]
fn channels(app: tauri::State<'_, App>) -> Result<Vec<dto::Channel>, String> {
    let store = app.executor.store();
    let state = store.state().map_err(|e| e.to_string())?;
    let (channels, _) = network::channels(store, &state).map_err(|e| e.to_string())?;
    Ok(channels.values().map(dto::Channel::of).collect())
}

/// Opens a channel and renders it.
#[tauri::command]
fn open_channel(app: tauri::State<'_, App>, channel: String) -> Result<dto::Opened, String> {
    let channel = App::channel(&channel)?;
    let outcome = app
        .executor
        .submit(Command::OpenChannel {
            channel,
            before: None,
            // No scroll position yet, so this asks for everything the store
            // holds. `design/01` §5 bounds it by pages once there is one.
            limit: usize::MAX,
        })
        .map_err(|err| err.to_string())?;

    let Outcome::Opened {
        messages,
        rejected,
        authors,
        ..
    } = outcome
    else {
        return Err("opening a channel produced something else".to_owned());
    };

    let state = app.executor.store().state().map_err(|e| e.to_string())?;
    let names = app.executor.names(&state).map_err(|e| e.to_string())?;

    Ok(dto::Opened {
        channel: to_hex(channel.as_bytes()),
        messages: messages
            .iter()
            .map(|message| dto::Message::of(message, &names))
            .collect(),
        authors,
        refused: rejected
            .iter()
            .map(|(id, why)| format!("{}: {why:?}", &to_hex(id.as_bytes())[..8]))
            .collect(),
    })
}

/// Writes a message.
#[tauri::command]
fn send_message(app: tauri::State<'_, App>, channel: String, body: String) -> Result<(), String> {
    let channel = App::channel(&channel)?;
    app.executor
        .submit(Command::SendMessage {
            channel,
            body,
            reply_to: None,
            attachments: Vec::new(),
        })
        .map(|_| ())
        .map_err(|err| err.to_string())
}

/// Claims a display name.
#[tauri::command]
fn set_name(app: tauri::State<'_, App>, name: String) -> Result<(), String> {
    app.executor
        .submit(Command::SetName { name })
        .map(|_| ())
        .map_err(|err| err.to_string())
}

/// Defines a channel.
#[tauri::command]
fn create_channel(app: tauri::State<'_, App>, name: String, topic: String) -> Result<(), String> {
    app.executor
        .submit(Command::CreateChannel {
            name,
            category: None,
            privacy: kols_core::Privacy::Public,
            topic,
        })
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn main() {
    let root = kols_cli::store::Store::default_root();
    let executor = match Executor::open(root.clone()) {
        Ok(executor) => executor,
        Err(err) => {
            // Said plainly rather than shown as an empty window: there is no
            // network here yet, and the fix is a command this shell does not
            // have. `init` and `attach` create the state every command needs
            // before any exists, which is why they are not commands.
            eprintln!("kols-desktop: no network at {}: {err}", root.display());
            eprintln!("Create one with `kols init <name>`, or join one with `kols attach <id>`.");
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        .manage(App { executor })
        .invoke_handler(tauri::generate_handler![
            me,
            channels,
            open_channel,
            send_message,
            create_channel,
            set_name
        ])
        .run(tauri::generate_context!())
        .expect("the window opens");
}
