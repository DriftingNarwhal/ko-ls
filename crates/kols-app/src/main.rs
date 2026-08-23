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
use kols_node::executor::Executor;
use kols_node::network;
use kols_node::workspace::Workspace;
use kols_core::ChannelId;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

/// What the node last said about its relay.
///
/// A struct rather than a tuple because it grew a third field and the third one
/// is the interesting one: without the reasons, "no circuit" is a symptom with
/// two opposite causes.
struct RelayStanding {
    /// The relay a circuit was reserved on, when one was.
    reserved: Option<String>,
    /// Why each designated relay did not work, in the order tried.
    failures: Vec<String>,
}

/// What every command handler shares.
///
/// The open network is behind a lock because the interface can change it — the
/// window is one process holding a workspace, and which network it is showing is
/// state that outlives any single command.
struct App {
    workspace: Workspace,
    open: Mutex<Option<Executor>>,
    /// This node's last reported standing with the relay, or `None` before it
    /// has reported.
    ///
    /// # Why this is held and not only emitted
    ///
    /// It was only emitted, and that lost it. The node starts in Tauri's
    /// `setup`, so it can settle its relay before the webview has finished
    /// registering listeners — and an event with nobody listening is gone. The
    /// panel then said "waiting for this node to report" forever, about a node
    /// that had already reported.
    ///
    /// So the event stays, for liveness, and this is the answer to the
    /// question. A consumer that can *ask* cannot miss the reply.
    relay: Mutex<Option<RelayStanding>>,
    /// The last voided-actions report, or `None` if no fork has healed here.
    ///
    /// Held for the same reason `relay` is, and one more: this report is not in
    /// the projection. Replayed state is the *winning* branch, so a consumer
    /// that missed the event has nowhere else to learn that something lost.
    reorg: Mutex<Option<dto::Reorg>>,
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

    /// Parses a category id out of the hex the webview holds.
    fn category(hex: &str) -> Result<kols_core::CategoryId, String> {
        intranet_crypto::from_hex(hex.trim())
            .and_then(|bytes| <[u8; 32]>::try_from(bytes.as_slice()).ok())
            .map(kols_core::CategoryId::from_bytes)
            .ok_or_else(|| "that is not a category id".to_owned())
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
        may_manage_channel: holds("chat:manage-channel:*"),
        may_invite: state.identity_holds(&identity.id(), &intranet_governance::Capability::ApproveNode),
        may_moderate: state
            .identity_holds(&identity.id(), &intranet_governance::Capability::ModerateContent),
        may_set_relays: state
            .identity_holds(&identity.id(), &intranet_governance::Capability::DefinePolicy),
    })
}

/// What this network designates as relays, and what this node cached.
///
/// A local read rather than a command, like [`waiting`]: replay is the authority
/// and asking it costs nothing.
#[tauri::command]
fn relays(app: tauri::State<'_, App>) -> Result<dto::Relays, String> {
    let standing = {
        let held = app.relay.lock().map_err(|_| "the relay lock is poisoned")?;
        held.as_ref().map(|standing| RelayStanding {
            reserved: standing.reserved.clone(),
            failures: standing.failures.clone(),
        })
    };
    app.with(|executor| {
        let store = executor.store();
        let identity = store.identity().map_err(|e| e.to_string())?;
        let state = store.state().map_err(|e| e.to_string())?;
        Ok(dto::Relays {
            designated: state.policy.bootstrap_relays.clone(),
            cached: store.relays(),
            may_set: state
                .identity_holds(&identity.id(), &intranet_governance::Capability::DefinePolicy),
            reported: standing.is_some(),
            failures: standing
                .as_ref()
                .map(|standing| standing.failures.clone())
                .unwrap_or_default(),
            reserved: standing.and_then(|standing| standing.reserved),
        })
    })
}

/// Designates this network's relays, replacing whatever it named before.
///
/// The gap this closes: the command, its gate and its executor have existed
/// since the terminal had them, and the window simply never submitted it — so a
/// founder who made a network here could not invite anybody and could not fix
/// it without a terminal (`STATUS.md` O12).
///
/// Replaces rather than appends, which is what `SetBootstrapRelays` means and
/// what the interface has to say plainly: this is the set, not an addition to
/// it.
#[tauri::command]
fn set_relays(
    handle: tauri::AppHandle,
    app: tauri::State<'_, App>,
    relays: String,
) -> Result<(), String> {
    let relays: Vec<String> = relays
        .split_whitespace()
        .filter(|address| !address.is_empty())
        .map(str::to_owned)
        .collect();
    if relays.is_empty() {
        return Err("give at least one relay address".to_owned());
    }
    // Checked here rather than only where it is dialled: this becomes a
    // governance entry every member replays, so a bad address is carried by
    // everybody and fails later, on somebody else's machine.
    let relays = relays
        .iter()
        .map(|relay| kols_node::parse_relay(relay))
        .collect::<Result<Vec<_>, _>>()?;
    let root = app.with(|executor| {
        executor
            .submit(Command::SetBootstrapRelays { relays })
            .map_err(|err| err.to_string())?;
        Ok(executor.store().root().to_path_buf())
    })?;

    // Restarted here rather than asked for. A relay is dialled when a node
    // starts, so designating one while a node is already running changes policy
    // and nothing else — and the interface would have had to tell the user to
    // go and reopen the network, which is a step that exists only because of
    // how this is implemented. Naming the next step is worse than taking it.
    //
    // The cost, stated: this drops whatever connections the node had. On the
    // path that matters it had none, because designating the first relay is
    // what a network does before it can reach anybody.
    start_node(&handle, app, root);
    Ok(())
}

/// Generates an identity for a relay, and hands back its backup phrase.
///
/// # Why this is in the window at all
///
/// It is the last thing in setting a network up that needed a terminal. A relay
/// will not start without an identity, `intranet-harness identity new` is how
/// one was made, and that is a tool in the protocol repository — so "no step
/// needs a terminal" was false for the one step a founder cannot skip.
///
/// # It is not this member's identity, and the interface must not let that blur
///
/// This is a *master seed*, for a machine, in a phrase the user will paste into
/// a hosting provider's configuration. The member's own seed is never shown,
/// never leaves the store, and has no interface at all. Anyone holding this
/// phrase can answer as this relay, which is why it is shown once and stored
/// nowhere: writing it down here would put a private key in the workspace for a
/// convenience nobody asked for.
#[tauri::command]
fn new_relay_identity() -> Result<String, String> {
    let master = intranet_identity::MasterSeed::generate().map_err(|err| err.to_string())?;
    master.to_backup_phrase().map_err(|err| err.to_string())
}

/// Restarts the node for the open network.
///
/// For the case [`set_relays`] cannot reach: a member learns of a relay
/// designated by *somebody else* through replay, and their node has been running
/// since before it existed. Same problem, one machine removed.
#[tauri::command]
fn restart_node(handle: tauri::AppHandle, app: tauri::State<'_, App>) -> Result<(), String> {
    let root = app.with(|executor| Ok(executor.store().root().to_path_buf()))?;
    start_node(&handle, app, root);
    Ok(())
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

/// The sidebar, in the order the network agrees on — spec 07 §1.6.
///
/// Ordered here rather than in the webview, because the default order is
/// normative and `kols_core::sidebar_order` is its tested implementation.
#[tauri::command]
fn sidebar(app: tauri::State<'_, App>) -> Result<Vec<dto::SidebarRow>, String> {
    app.with(|executor| {
        let store = executor.store();
        let state = store.state().map_err(|e| e.to_string())?;
        let (channels, _) = network::channels(store, &state).map_err(|e| e.to_string())?;
        let (categories, _) = network::categories(store, &state).map_err(|e| e.to_string())?;

        let ordered = kols_core::sidebar_order(
            &channels
                .values()
                .map(|c| kols_core::SidebarChannel {
                    id: c.id,
                    category: c.category,
                    position: c.position,
                })
                .collect::<Vec<_>>(),
            &categories
                .values()
                .map(|c| kols_core::SidebarCategory {
                    id: c.id,
                    position: c.position,
                })
                .collect::<Vec<_>>(),
        );

        Ok(ordered
            .into_iter()
            .filter_map(|row| match row {
                kols_core::SidebarRow::Channel(id) => {
                    channels.get(&id).map(|channel| dto::SidebarRow::Channel {
                        channel: dto::Channel::of(channel),
                    })
                }
                kols_core::SidebarRow::Category { id, channels: inner } => {
                    Some(dto::SidebarRow::Category {
                        id: to_hex(id.as_bytes()),
                        // Empty when nothing defined it. A channel may name a
                        // category with no definition, and the webview decides
                        // what to call that rather than this inventing a name.
                        name: categories
                            .get(&id)
                            .map(|c| c.name.clone())
                            .unwrap_or_default(),
                        position: categories.get(&id).and_then(|c| c.position),
                        channels: inner
                            .iter()
                            .filter_map(|c| channels.get(c))
                            .map(dto::Channel::of)
                            .collect(),
                    })
                }
            })
            .collect())
    })
}

/// The last voided-actions report, if a fork has healed here — Core §2.7.1.
///
/// A question rather than only an event, because the answer is not in replayed
/// state: replay follows the winning branch, so what lost leaves no trace there.
#[tauri::command]
fn reorg(app: tauri::State<'_, App>) -> Result<Option<dto::Reorg>, String> {
    app.reorg
        .lock()
        .map(|held| held.clone())
        .map_err(|_| "the reorg record is poisoned".to_owned())
}

/// Names and positions a category.
#[tauri::command]
fn create_category(app: tauri::State<'_, App>, name: String, position: u32) -> Result<(), String> {
    app.with(|executor| {
        executor
            .submit(Command::CreateCategory { name, position })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

fn category_change(
    app: &tauri::State<'_, App>,
    category: &str,
    change: kols_core::CategoryChange,
) -> Result<(), String> {
    let category = App::category(category)?;
    app.with(|executor| {
        executor
            .submit(Command::UpdateCategory { category, change })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

/// Renames a category.
#[tauri::command]
fn rename_category(app: tauri::State<'_, App>, category: String, name: String) -> Result<(), String> {
    category_change(&app, &category, kols_core::CategoryChange::Rename(name))
}

/// Moves a category among the other categories.
#[tauri::command]
fn move_category(app: tauri::State<'_, App>, category: String, position: u32) -> Result<(), String> {
    category_change(&app, &category, kols_core::CategoryChange::SetPosition(position))
}

/// Deletes a category.
///
/// Removes a name and a sort key, never a scope: channels naming it stay in it
/// and resolve exactly what they did before (spec 07 §1.8). A caller meaning
/// "and move its channels out" recategorises them first.
#[tauri::command]
fn delete_category(app: tauri::State<'_, App>, category: String) -> Result<(), String> {
    category_change(&app, &category, kols_core::CategoryChange::Delete)
}

fn channel_change(
    app: &tauri::State<'_, App>,
    channel: &str,
    change: kols_core::ChannelChange,
) -> Result<(), String> {
    let channel = App::channel(channel)?;
    app.with(|executor| {
        executor
            .submit(Command::UpdateChannel { channel, change })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

/// Renames a channel.
#[tauri::command]
fn rename_channel(app: tauri::State<'_, App>, channel: String, name: String) -> Result<(), String> {
    channel_change(&app, &channel, kols_core::ChannelChange::Rename(name))
}

/// Sets a channel's topic.
#[tauri::command]
fn set_channel_topic(app: tauri::State<'_, App>, channel: String, topic: String) -> Result<(), String> {
    channel_change(&app, &channel, kols_core::ChannelChange::SetTopic(topic))
}

/// Archives a channel: readable, not writable.
#[tauri::command]
fn archive_channel(app: tauri::State<'_, App>, channel: String) -> Result<(), String> {
    channel_change(&app, &channel, kols_core::ChannelChange::Archive)
}

/// Deletes a channel — hidden from listings, not erased.
#[tauri::command]
fn delete_channel(app: tauri::State<'_, App>, channel: String) -> Result<(), String> {
    channel_change(&app, &channel, kols_core::ChannelChange::Delete)
}

/// Moves a channel: into a category, or out of one, and to a position.
///
/// **Two governance entries, and not atomic.** The log has no transaction to put
/// them in (spec 07 §1.8), so a caller that sees this fail must assume either,
/// both or neither landed and read the sidebar back rather than guessing.
#[tauri::command]
fn move_channel(
    app: tauri::State<'_, App>,
    channel: String,
    category: Option<String>,
    position: u32,
) -> Result<(), String> {
    let target = match category.as_deref() {
        Some(hex) if !hex.is_empty() => Some(App::category(hex)?),
        _ => None,
    };
    let id = App::channel(&channel)?;

    // Only recategorise when the category actually changes. A no-op entry is not
    // free: every governance entry is replayed by every joiner forever, so
    // writing one that changes nothing spends everybody's replay to record that
    // somebody dragged a channel within the folder it was already in.
    let moved = app.with(|executor| {
        let store = executor.store();
        let state = store.state().map_err(|e| e.to_string())?;
        let (channels, _) = network::channels(store, &state).map_err(|e| e.to_string())?;
        Ok(channels.get(&id).and_then(|c| c.category) != target)
    })?;
    if moved {
        channel_change(
            &app,
            &channel,
            kols_core::ChannelChange::Recategorise(target),
        )?;
    }
    channel_change(&app, &channel, kols_core::ChannelChange::SetPosition(position))
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
    let me = executor.store().identity().map_err(|e| e.to_string())?.id();

    Ok(dto::Opened {
        channel: to_hex(channel.as_bytes()),
        messages: messages
            .iter()
            .map(|message| dto::Message::of(message, &names, &me))
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

/// Revises one of this member's own messages.
///
/// Only an author may (spec 07 §5.2). The interface offers it only on a
/// member's own messages, and the gate refuses it regardless — the first is
/// presentation, the second is the rule.
#[tauri::command]
fn edit_message(
    app: tauri::State<'_, App>,
    channel: String,
    message: String,
    body: String,
) -> Result<(), String> {
    let channel = App::channel(&channel)?;
    app.with(|executor| {
        let target = executor
            .resolve_message(&channel, &message)
            .map_err(|err| err.to_string())?;
        executor
            .submit(Command::EditMessage {
                channel,
                target,
                body: body.clone(),
            })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

/// Withdraws one of this member's own messages.
///
/// **Hidden, never unsent** (`design/01` §6). Withdrawal retracts no bytes
/// anybody already holds, and the interface says so where it renders one rather
/// than implying the message is gone.
#[tauri::command]
fn delete_message(
    app: tauri::State<'_, App>,
    channel: String,
    message: String,
) -> Result<(), String> {
    let channel = App::channel(&channel)?;
    app.with(|executor| {
        let target = executor
            .resolve_message(&channel, &message)
            .map_err(|err| err.to_string())?;
        executor
            .submit(Command::DeleteMessage { channel, target })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

/// Adds a reaction, or takes one back.
#[tauri::command]
fn react(
    app: tauri::State<'_, App>,
    channel: String,
    message: String,
    key: String,
    remove: bool,
) -> Result<(), String> {
    let channel = App::channel(&channel)?;
    app.with(|executor| {
        let target = executor
            .resolve_message(&channel, &message)
            .map_err(|err| err.to_string())?;
        executor
            .submit(Command::React {
                channel,
                target,
                key: key.clone(),
                remove,
            })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

/// Pins a message, or unpins it. Needs `chat:moderate`.
#[tauri::command]
fn pin(
    app: tauri::State<'_, App>,
    channel: String,
    message: String,
    remove: bool,
) -> Result<(), String> {
    let channel = App::channel(&channel)?;
    app.with(|executor| {
        let target = executor
            .resolve_message(&channel, &message)
            .map_err(|err| err.to_string())?;
        executor
            .submit(Command::Pin {
                channel,
                target,
                remove,
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

/// Mints an invite and hands back the one string a joiner needs.
///
/// The founder's last terminal step, and the reason this exists: a client that
/// can create a network, run a node and never bring anybody into it is not one
/// you can hand to somebody else.
#[tauri::command]
fn create_invite(app: tauri::State<'_, App>, uses: u32, hours: i64) -> Result<dto::Invite, String> {
    app.with(|executor| {
        match executor
            .submit(Command::CreateInvite {
                uses,
                valid_for_hours: hours,
            })
            .map_err(|err| err.to_string())?
        {
            Outcome::InviteCreated {
                invite,
                expires_at_millis,
                uses,
            } => Ok(dto::Invite {
                uri: kols_node::invite::to_uri_from_bytes(&invite),
                hours: (expires_at_millis - kols_node::chat::now_millis()) / 3_600_000,
                uses,
            }),
            other => Err(format!("minting an invite answered with {other:?}")),
        }
    })
}

/// Who redeemed an invite and is waiting to be admitted.
///
/// A local read of what the node wrote down rather than a command, for the same
/// reason `kols waiting` is: the waiting room is live state in the running node,
/// so this is stale by construction and the interface says so where it shows it.
#[tauri::command]
fn waiting(app: tauri::State<'_, App>) -> Result<Vec<dto::Waiting>, String> {
    app.with(|executor| {
        let store = executor.store();
        let identity = store.identity().map_err(|e| e.to_string())?;
        let state = store.state().map_err(|e| e.to_string())?;
        // The same capability admitting them needs. Seeing who is asking is not
        // a smaller question than letting them in.
        if !state.identity_holds(&identity.id(), &intranet_governance::Capability::ApproveNode) {
            return Ok(Vec::new());
        }
        Ok(store
            .waiting()
            .into_iter()
            .map(|identity| dto::Waiting {
                short: kols_node::parse_identity(&identity)
                    .map(|id| id.short())
                    .unwrap_or_else(|_| identity.clone()),
                identity,
            })
            .collect())
    })
}

/// Everybody in this network, and whether this node is connected to them.
///
/// A roster with a connection marker rather than a list of connections, because
/// a bare list of peers answers a question nobody asked: what a person wants to
/// know is who is here, and the connection is an attribute of each one.
///
/// The marker is honest about a narrow thing — `design/09` §2's hot/warm/cold
/// tiering does not exist, so this node holds connections to the peers it had
/// addresses for and not to every member. A member shown as not connected may
/// be away, unreachable from here, or simply never dialled.
#[tauri::command]
fn people(app: tauri::State<'_, App>) -> Result<Vec<dto::Member>, String> {
    app.with(|executor| {
        let store = executor.store();
        let me = store.identity().map_err(|e| e.to_string())?.id();
        let state = store.state().map_err(|e| e.to_string())?;
        let names = executor.names(&state).map_err(|e| e.to_string())?;
        let connected: std::collections::BTreeSet<String> =
            store.connected().into_iter().collect();

        let mut people: Vec<dto::Member> = state
            .groups
            .values()
            .flat_map(|group| group.members.keys().copied())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|identity| {
                let hex = to_hex(identity.verifying_key().as_bytes());
                dto::Member {
                    connected: connected.contains(&hex),
                    you: identity == me,
                    short: identity.short(),
                    name: names.of(&identity).map(str::to_owned),
                    identity: hex,
                }
            })
            .collect();
        // Named members first, then by name, so the list does not reshuffle as
        // people claim names and a stranger does not outrank somebody known.
        people.sort_by(|a, b| {
            b.name.is_some().cmp(&a.name.is_some()).then_with(|| {
                a.name
                    .as_deref()
                    .unwrap_or(&a.short)
                    .cmp(b.name.as_deref().unwrap_or(&b.short))
            })
        });
        Ok(people)
    })
}

/// Admits a waiting identity to the network.
#[tauri::command]
fn admit(app: tauri::State<'_, App>, identity: String) -> Result<(), String> {
    let identity = kols_node::parse_identity(&identity)?;
    app.with(|executor| {
        executor
            .submit(Command::AdmitMember { identity })
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

/// Redeems an invite, joining the network it names.
///
/// Outside the command vocabulary for the same reason creating one is: it makes
/// the state every command needs before any exists. The invite is the only thing
/// the joiner has, and everything after the first connection is ordinary sync
/// (Core §5.7).
#[tauri::command]
async fn join_network(
    handle: tauri::AppHandle,
    invite: String,
) -> Result<dto::Joined, String> {
    let credential = kols_node::invite::from_uri(&invite)?;
    let workspace = {
        let app = handle.state::<App>();
        Workspace::at(app.workspace.root().to_path_buf())
    };
    let path = workspace.path_for(&credential.network);

    let landed = kols_node::join::redeem(path.clone(), credential, 30, false).await?;

    // Open it either way. A waiting-room member holds an identity and nothing
    // else, and showing them that — rather than nothing — is the difference
    // between "you are waiting" and "something went wrong".
    let executor = Executor::open(path.clone()).map_err(|err| err.to_string())?;
    {
        let app = handle.state::<App>();
        *app.open
            .lock()
            .map_err(|_| "the workspace lock is poisoned")? = Some(executor);
    }
    start_node(&handle, handle.state::<App>(), path);

    Ok(match landed {
        kols_node::join::Landed::Admitted => dto::Joined {
            admitted: true,
            identity: String::new(),
        },
        kols_node::join::Landed::Waiting { identity } => dto::Joined {
            admitted: false,
            identity,
        },
    })
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
    // A new node has not reported yet, and the standing of the one being
    // replaced says nothing about it. Cleared here rather than when the new one
    // reports, so the gap reads as "asking" instead of as a stale answer.
    if let Ok(mut relay) = app.relay.lock() {
        *relay = None;
    }
    let previous = node.take();

    let emitter = handle.clone();
    let sink: kols_node::serve::Sink = std::sync::Arc::new(move |events: &[kols_api::Event]| {
        for event in events {
            // Named for what happened rather than carrying the payload: the
            // interface re-reads the channel, because `design/05` §3's third
            // property is that a consumer merges rather than appends, and the
            // cheapest way to hold to that is to render from the projection
            // every time.
            let name = match event {
                kols_api::Event::Records {
                    channel, records, ..
                } => {
                    // Two facts, because the interface wants different things
                    // from them: *something arrived here* means redraw, and *a
                    // message arrived here* is what makes a channel unread. A
                    // vote or an edit is activity and is not something somebody
                    // needs to be told to go and read.
                    let messages = records
                        .iter()
                        .any(|record| matches!(record.body, kols_core::RecordBody::Message { .. }));
                    // No author check: a record this member wrote is already in
                    // their store, so the absorb that follows reports nothing,
                    // and anything they write live goes to the channel they are
                    // looking at. Nothing here can mark your own post unread.
                    let _ = emitter.emit(
                        "kols://records",
                        (to_hex(channel.as_bytes()), messages),
                    );
                    continue;
                }
                kols_api::Event::Governance { .. } | kols_api::Event::Adopted { .. } => {
                    "kols://governance"
                }
                kols_api::Event::MemberKeyed { .. } | kols_api::Event::EpochRotated { .. } => {
                    "kols://keys"
                }
                kols_api::Event::JoinAnswered { .. } => "kols://joins",
                kols_api::Event::GovernanceReorg { mine, others } => {
                    // Recorded before it is emitted, like the relay standing:
                    // the emit makes it prompt, the record makes it reliable.
                    if let Ok(mut held) = emitter.state::<App>().reorg.lock() {
                        *held = Some(dto::Reorg {
                            mine: mine
                                .iter()
                                .map(|action| dto::VoidedAction {
                                    kind: action.kind.clone(),
                                    security_relevant: action.security_relevant,
                                })
                                .collect(),
                            others: *others,
                        });
                    }
                    let _ = emitter.emit("kols://reorg", ());
                    continue;
                }
                kols_api::Event::Degraded { reason } => {
                    let _ = emitter.emit("kols://degraded", reason.clone());
                    continue;
                }
                // The good news as well as the bad, which is the whole point:
                // before this, relay trouble reached the window and relay health
                // never did.
                kols_api::Event::Relay {
                    reserved,
                    failures,
                    // The count is not carried into the window: it reads the
                    // designated set from replay, which is the authority on it.
                    ..
                } => {
                    // Recorded before it is emitted, so that a consumer which
                    // missed the event can still ask. The emit is what makes it
                    // prompt; this is what makes it reliable.
                    if let Ok(mut held) = emitter.state::<App>().relay.lock() {
                        *held = Some(RelayStanding {
                            reserved: reserved.clone(),
                            failures: failures.clone(),
                        });
                    }
                    let _ = emitter.emit("kols://relay", ());
                    continue;
                }
            };
            let _ = emitter.emit(name, ());
        }
    });

    let failed = handle.clone();
    *node = Some(tauri::async_runtime::spawn(async move {
        // Stopping the previous node is *awaited*, not merely requested, and
        // that distinction is the whole reason this is here rather than above.
        //
        // Only one process may run a node per store, and the claim is released
        // when the serving future is dropped. `abort()` does not drop it — it
        // asks the task to stop at its next await point. Spawning the
        // replacement immediately therefore races the claim of the node being
        // replaced, and `hold_node` does not fail fast on a claim that looks
        // fresh: it waits the staleness window out, so the window would appear
        // to hang for half a minute and then work.
        //
        // Awaited in the new task rather than at the call site so nothing blocks
        // the thread the interface is on.
        if let Some(previous) = previous {
            previous.abort();
            let _ = previous.await;
        }

        let outcome = kols_node::serve::serve(
            root,
            // Dual-stack: TCP and QUIC over IPv4 and IPv6. See `serve`.
            "",
            &[],
            kols_node::serve::SEAL_TARGET_BYTES,
            true,
            kols_node::serve::LIVE_WINDOW_MILLIS,
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
            relay: Mutex::new(None),
            reorg: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            me,
            channels,
            sidebar,
            reorg,
            create_category,
            rename_category,
            move_category,
            delete_category,
            rename_channel,
            set_channel_topic,
            archive_channel,
            delete_channel,
            move_channel,
            open_channel,
            send_message,
            create_channel,
            set_name,
            create_invite,
            waiting,
            admit,
            networks,
            create_network,
            join_network,
            open_network,
            relays,
            people,
            set_relays,
            restart_node,
            edit_message,
            delete_message,
            react,
            pin,
            new_relay_identity
        ])
        .run(tauri::generate_context!())
        .expect("the window opens");
}
