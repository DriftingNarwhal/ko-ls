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
//! **It runs a node for whichever network is open.** The same loop `kols serve`
//! runs — one implementation, two front ends, differing only in where its events
//! go: a terminal prints them, this forwards them to the webview.
//!
//! Only one process may run a node for a network, and the store enforces that:
//! the key group is live state, and two nodes would each advance it without
//! seeing the other. So the window refuses to open a network `kols serve` is
//! already serving, and says which.
//!
//! **It holds a workspace, not a store.** A person belongs to several networks
//! and a direct message is one too, so "which network" is a question the window
//! has to be able to ask — and answer with "none yet, make one". Each open
//! network is a separate node with a separate peer id, forced rather than chosen
//! (`design/09` §1), so this is a directory of networks and not a merged view of
//! them.
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
use kols_cli::workspace::Workspace;
use kols_core::ChannelId;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

/// What every command handler shares.
///
/// The open network is behind a lock because the interface can change it — the
/// window is one process holding a workspace, and which network it is showing is
/// state that outlives any single command.
struct App {
    workspace: Workspace,
    open: Mutex<Option<Executor>>,
    /// The node running for the open network.
    ///
    /// Dropping the handle aborts it, which is the whole shutdown protocol:
    /// there is no signal to forget to send, and switching networks drops the
    /// task for the one being left.
    node: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

impl App {
    /// Runs `f` against the open network, or says none is.
    fn with<T>(&self, f: impl FnOnce(&Executor) -> Result<T, String>) -> Result<T, String> {
        let open = self.open.lock().map_err(|_| "the workspace lock is poisoned")?;
        let executor = open
            .as_ref()
            .ok_or("no network is open — create or choose one first")?;
        f(executor)
    }

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
    app.with(me_of)
}

fn me_of(executor: &Executor) -> Result<dto::Me, String> {
    let store = executor.store();
    let identity = store.identity().map_err(|e| e.to_string())?;
    let state = store.state().map_err(|e| e.to_string())?;
    let holds = |name: &str| {
        state.identity_holds(
            &identity.id(),
            &intranet_governance::Capability::extension(name.to_owned()),
        )
    };

    let names = executor.names(&state).map_err(|e| e.to_string())?;

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
    app.with(|executor| {
        let store = executor.store();
        let state = store.state().map_err(|e| e.to_string())?;
        let (channels, _) = network::channels(store, &state).map_err(|e| e.to_string())?;
        Ok(channels.values().map(dto::Channel::of).collect())
    })
}

/// Opens a channel and renders it.
#[tauri::command]
fn open_channel(app: tauri::State<'_, App>, channel: String) -> Result<dto::Opened, String> {
    let channel = App::channel(&channel)?;
    app.with(|executor| open_one(executor, channel))
}

fn open_one(executor: &Executor, channel: ChannelId) -> Result<dto::Opened, String> {
    let outcome = executor
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

    let state = executor.store().state().map_err(|e| e.to_string())?;
    let names = executor.names(&state).map_err(|e| e.to_string())?;

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
    app.with(|executor| {
        executor
            .submit(Command::SendMessage {
                channel,
                body,
                reply_to: None,
                attachments: Vec::new(),
            })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

/// Claims a display name.
#[tauri::command]
fn set_name(app: tauri::State<'_, App>, name: String) -> Result<(), String> {
    app.with(|executor| {
        executor
            .submit(Command::SetName { name })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

/// Defines a channel.
#[tauri::command]
fn create_channel(app: tauri::State<'_, App>, name: String, topic: String) -> Result<(), String> {
    app.with(|executor| {
        executor
            .submit(Command::CreateChannel {
                name,
                category: None,
                privacy: kols_core::Privacy::Public,
                topic,
            })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

/// Every network this client holds a store for.
#[tauri::command]
fn networks(app: tauri::State<'_, App>) -> Result<Vec<dto::Network>, String> {
    let open = app
        .open
        .lock()
        .map_err(|_| "the workspace lock is poisoned")?
        .as_ref()
        .map(|executor| to_hex(executor.store().network().as_bytes()));

    Ok(app
        .workspace
        .list()
        .into_iter()
        .map(|known| dto::Network::of(&known, open.as_deref()))
        .collect())
}

/// Creates a network, with this member as its sole Founder.
///
/// Outside the command vocabulary for the same reason `kols init` is: it makes
/// the state every command needs before any exists, so there is nothing yet to
/// check a permission against.
#[tauri::command]
fn create_network(
    handle: tauri::AppHandle,
    app: tauri::State<'_, App>,
    name: String,
    relay: String,
) -> Result<dto::Network, String> {
    if name.trim().is_empty() {
        return Err("give the network a name".to_owned());
    }
    // A relay is optional here and required before inviting anybody (Core §5.5),
    // which is the honest ordering: you can make a network alone, and you cannot
    // hand somebody a way in until it has an entry point.
    let relays: Vec<String> = relay
        .split_whitespace()
        .filter(|address| !address.is_empty())
        .map(str::to_owned)
        .collect();

    let store = app.workspace.create(name.trim(), relays)?;
    let path = store.root().to_path_buf();
    drop(store);

    let executor = Executor::open(path.clone()).map_err(|err| err.to_string())?;
    let known = dto::Network {
        id: to_hex(executor.store().network().as_bytes()),
        label: name.trim().to_owned(),
        keyed: false,
        open: true,
    };
    *app.open.lock().map_err(|_| "the workspace lock is poisoned")? = Some(executor);
    start_node(&handle, app, path);
    Ok(known)
}

/// Opens one of this client's networks, and starts a node for it.
#[tauri::command]
fn open_network(
    handle: tauri::AppHandle,
    app: tauri::State<'_, App>,
    network: String,
) -> Result<(), String> {
    let store = app.workspace.open(&network)?;
    let root = store.root().to_path_buf();
    drop(store);
    let executor = Executor::open(root.clone()).map_err(|err| err.to_string())?;
    *app.open.lock().map_err(|_| "the workspace lock is poisoned")? = Some(executor);
    start_node(&handle, app, root);
    Ok(())
}

/// Runs a node for one network, forwarding what it learns to the interface.
///
/// Replaces whatever was running, because the window shows one network at a
/// time and the node for the one being left has nothing to do. `design/09` §2's
/// hot/warm/cold tiering is what turns this into several at once, and is not
/// this.
fn start_node(handle: &tauri::AppHandle, app: tauri::State<'_, App>, root: std::path::PathBuf) {
    let mut node = match app.node.lock() {
        Ok(node) => node,
        Err(_) => return,
    };
    if let Some(previous) = node.take() {
        previous.abort();
    }

    let emitter = handle.clone();
    let sink: kols_cli::serve::Sink = std::sync::Arc::new(move |events: &[kols_api::Event]| {
        for event in events {
            // Named for what happened rather than carrying the payload: the
            // interface re-reads the channel, because `design/05` §3's third
            // property is that a consumer merges rather than appends, and the
            // cheapest way to hold to that is to render from the projection
            // every time.
            let name = match event {
                kols_api::Event::Records { channel, .. } => {
                    let _ = emitter.emit("kols://records", to_hex(channel.as_bytes()));
                    continue;
                }
                kols_api::Event::Governance { .. } | kols_api::Event::Adopted { .. } => {
                    "kols://governance"
                }
                kols_api::Event::MemberKeyed { .. } | kols_api::Event::EpochRotated { .. } => {
                    "kols://keys"
                }
                kols_api::Event::JoinAnswered { .. } => "kols://joins",
                kols_api::Event::Degraded { reason } => {
                    let _ = emitter.emit("kols://degraded", reason.clone());
                    continue;
                }
            };
            let _ = emitter.emit(name, ());
        }
    });

    let failed = handle.clone();
    *node = Some(tauri::async_runtime::spawn(async move {
        let outcome = kols_cli::serve::serve(
            root,
            "/ip4/0.0.0.0/tcp/0",
            &[],
            kols_cli::serve::SEAL_TARGET_BYTES,
            true,
            kols_cli::serve::LIVE_WINDOW_MILLIS,
            &sink,
        )
        .await;
        if let Err(why) = outcome {
            // The one that matters here is another process already serving this
            // network, which is a thing to say rather than a window that quietly
            // never syncs.
            let _ = failed.emit("kols://degraded", why);
        }
    }));
}

fn main() {
    let workspace = Workspace::at(Workspace::default_root());

    // Whichever network is there, if exactly one is — the common case for
    // somebody who has made or joined a single network, and the case where being
    // asked to choose is noise. Anything else opens on the picker.
    let open = match workspace.list().as_slice() {
        [only] => Executor::open(only.path.clone()).ok(),
        _ => None,
    };

    let opened_at = open.as_ref().map(|executor| executor.store().root().to_path_buf());

    tauri::Builder::default()
        .setup(move |app| {
            // The node for a network opened at startup. In `setup` rather than
            // before the builder, because spawning needs a handle to emit
            // through, and there is nothing to emit to until there is an app.
            if let Some(root) = opened_at.clone() {
                let handle = app.handle().clone();
                start_node(&handle, handle.state::<App>(), root);
            }
            Ok(())
        })
        .manage(App {
            workspace,
            open: Mutex::new(open),
            node: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            me,
            channels,
            open_channel,
            send_message,
            create_channel,
            set_name,
            networks,
            create_network,
            open_network
        ])
        .run(tauri::generate_context!())
        .expect("the window opens");
}
