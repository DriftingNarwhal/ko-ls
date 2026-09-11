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

// # No console window on Windows
//
// A Rust binary defaults to the console subsystem, so launching this one from
// Explorer opened a terminal behind the window — cosmetic, and not what a
// product looks like (`design/00` D30: the window is the product, the terminal
// is a development tool). `windows` is the GUI subsystem and closes that.
//
// **Release only, and the gate is not caution.** A debug build keeps its console
// so that running it from a terminal still shows what the node is doing, which
// is where this gets developed.
//
// **This attribute alone would have been worse than the terminal it removes.**
// A GUI-subsystem process launched from Explorer has no console at all, so
// `GetStdHandle` returns null, Rust's stdio turns that into a write error, and
// `print_to` *panics* rather than dropping the line — the window would have
// crashed on the node's first line of output, on the one platform this cannot
// be run from. So it landed together with `kols_node::Report`: the node loop
// hands its lifecycle lines to whoever is listening, the terminal prints them,
// and this passes `quiet`. Nothing in this process writes to stdout.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
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
    /// Every network's node — `design/09` §2, `kols_node::nodes`.
    ///
    /// One handle used to live here, for "the node running for the open
    /// network", and switching networks dropped the task for the one being
    /// left. That is right for a client that shows one network and wrong for one
    /// that *belongs* to several: a member in a dozen networks received in one
    /// of them, and a conversation — a network neither party is usually looking
    /// at — could neither be reached nor deliver.
    ///
    /// Dropping this stops everything, which is still the whole shutdown
    /// protocol; there is simply more than one thing to stop.
    nodes: Mutex<kols_node::nodes::Nodes>,
    /// The network the member is looking at, which is the only thing that makes
    /// a node hot rather than warm.
    in_view: Mutex<Option<intranet_identity::NetworkId>>,
    /// Whether this application can be reached after its last window closes.
    ///
    /// True when a tray icon was built (`design/09` §1.11). **False is not a
    /// smaller version of true**: with no tray there is no way back to a hidden
    /// window and no way to quit short of killing the process, so closing the
    /// last window has to end the application instead — which is what it did
    /// before the tray existed, and is honest.
    outlives_windows: Mutex<bool>,
    /// This installation's claim on being the running application.
    ///
    /// # Held here so that quitting can release it
    ///
    /// It would be simpler to leave it in the task that beats it, and that has
    /// a bug in it: `handle.exit` ends the process without dropping a running
    /// task, so the claim files would outlive the application by their whole
    /// staleness window — and **quit followed immediately by relaunch would
    /// find a fresh heartbeat with nobody behind it**, exit as a second
    /// instance, and look like an application that will not start. The node
    /// claim survives the same gap because waiting six seconds for a node is a
    /// pause; waiting six seconds for a window is a bug report.
    claim: Mutex<Option<kols_node::workspace::AppClaim>>,
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

    // The local label follows the network's own name once this node knows it.
    //
    // The picker cannot afford to answer this properly — it lists every network
    // in the workspace and the name lives in replayed policy, so showing it
    // there means replaying every store's log to draw a list. The label is a
    // file, which is what makes the picker cheap, so it is kept as a cache of
    // the name rather than as a second opinion about it.
    //
    // A joiner has no label at all: nothing on the join path ever set one, so
    // networks somebody was invited to listed as an id. This is where they get
    // one, and it corrects itself if the network is renamed later.
    let network_name = kols_core::ChatPolicy::of(&state.policy)
        .network_name()
        .map(str::to_owned);
    let label = match (&network_name, store.label()) {
        (Some(name), held) if held.as_deref() != Some(name.as_str()) => {
            // Best effort: a label that cannot be written is a picker entry that
            // still says the id, which is what it said before.
            let _ = store.set_label(name);
            name.clone()
        }
        (_, held) => held.unwrap_or_default(),
    };

    Ok(dto::Me {
        identity: identity.id().short(),
        name: names.of(&identity.id()).map(str::to_owned),
        network: to_hex(store.network().as_bytes()),
        label,
        is_member: state.is_member(&identity.id()),
        has_key: store.epoch_key().is_ok(),
        may_post: holds("chat:post:*"),
        may_create_channel: holds("chat:create-channel:*"),
        may_manage_channel: holds("chat:manage-channel:*"),
        may_invite: state.identity_holds(&identity.id(), &intranet_governance::Capability::ApproveNode),
        may_moderate: state
            .identity_holds(&identity.id(), &intranet_governance::Capability::ModerateContent),
        may_set_relays: state
            .identity_holds(&identity.id(), &intranet_governance::Capability::DefinePolicy),
        may_define_group: state
            .identity_holds(&identity.id(), &intranet_governance::Capability::DefineGroup),
        // Any role at all, so the tab is offered. Which ones is per role, since
        // `manage-membership:<group>` is dynamically tiered (Core §2.4).
        may_assign_role: state.groups.keys().any(|group| {
            state.identity_holds(
                &identity.id(),
                &intranet_governance::Capability::manage_membership(group.clone()),
            )
        }),
        network_name,
        admission_mode: match state.policy.admission_mode {
            intranet_governance::AdmissionMode::AutoAdmit => "auto".to_owned(),
            intranet_governance::AdmissionMode::ExplicitIntake => "intake".to_owned(),
        },
        member_vote: matches!(
            state.policy.governance_model,
            intranet_governance::GovernanceModel::MemberVote { .. }
        ),
    })
}

/// This network's chat settings, as they currently stand.
///
/// Served from `kols_core::ChatSetting` rather than restated here, so a setting
/// added to the vocabulary appears in the interface instead of needing a second
/// list to remember. The summaries are the interface's own — they say what a
/// number *bounds*, which the vocabulary has no field for.
#[tauri::command]
fn settings(app: tauri::State<'_, App>) -> Result<Vec<dto::Setting>, String> {
    app.with(|executor| {
        let state = executor.store().state().map_err(|e| e.to_string())?;
        Ok(kols_core::ChatSetting::ALL
            .iter()
            .map(|setting| {
                let key = setting.key();
                dto::Setting {
                    id: format!("{setting:?}"),
                    key: key.to_owned(),
                    label: label_of(*setting).to_owned(),
                    summary: summary_of(*setting).to_owned(),
                    value: state.policy.app_policy_int(key, setting.default_value()),
                    default: setting.default_value(),
                    explicit: state.policy.app_policy(key).is_some(),
                    unit: match setting.unit() {
                        kols_core::Unit::PerMinute => "per-minute",
                        kols_core::Unit::Bytes => "bytes",
                        kols_core::Unit::Count => "count",
                        kols_core::Unit::Millis => "millis",
                        kols_core::Unit::Seconds => "seconds",
                        kols_core::Unit::Days => "days",
                    }
                    .to_owned(),
                    zero_means: match setting.zero_means() {
                        kols_core::ZeroMeaning::NoLimit => "no limit at all",
                        kols_core::ZeroMeaning::Forever => "kept forever",
                        kols_core::ZeroMeaning::Zero => "a real bound of zero",
                        kols_core::ZeroMeaning::RefusesEverything => {
                            "a real bound of zero, which refuses every message"
                        }
                    }
                    .to_owned(),
                    retention: matches!(
                        setting,
                        kols_core::ChatSetting::RetainMessagesDays
                            | kols_core::ChatSetting::RetainAttachmentsDays
                    ),
                }
            })
            .collect())
    })
}

/// A short label for one setting.
const fn label_of(setting: kols_core::ChatSetting) -> &'static str {
    use kols_core::ChatSetting as S;
    match setting {
        S::MessageRate => "messages a minute",
        S::ReactionRate => "reactions a minute",
        S::MessageMaxBytes => "message size",
        S::AttachmentMaxBytes => "attachment size",
        S::AttachmentMaxCount => "attachments a message",
        S::SegmentMaxBytes => "segment size",
        S::MaxFutureSkewMillis => "clock skew allowed",
        S::SlowmodeMaxSeconds => "longest slowmode",
        S::RetainMessagesDays => "keep messages",
        S::RetainAttachmentsDays => "keep attachments",
    }
}

/// What one setting bounds, and why a network would move it.
const fn summary_of(setting: kols_core::ChatSetting) -> &'static str {
    use kols_core::ChatSetting as S;
    match setting {
        S::MessageRate => {
            "Messages, edits and withdrawals one member may write in one channel per minute.              Counted over the author's own clock readings, so every node reaches the same              verdict. This bounds flooding; it is not for pacing conversation, which is a              channel's own slowmode."
        }
        S::ReactionRate => "Reactions and pins one member may write in one channel per minute.",
        S::MessageMaxBytes => "The largest a single message or edit may be, as UTF-8.",
        S::AttachmentMaxBytes => {
            "The largest a single attachment may be. This spends other members' disks: at              replication factor three, one 25 MiB file costs 75 MiB across the network."
        }
        S::AttachmentMaxCount => "How many attachments may ride on one message.",
        S::SegmentMaxBytes => {
            "The largest published segment a reader will fetch. Without it one member could              make every reader pull an arbitrarily large object."
        }
        S::MaxFutureSkewMillis => {
            "How far ahead of your clock a record may claim to be. Anything further is held              and rendered when your clock reaches it — never dropped."
        }
        S::SlowmodeMaxSeconds => {
            "The longest slowmode a channel manager may set. The ceiling on a knob delegated              to whoever moderates, so calming one channel needs no authority over policy."
        }
        S::RetainMessagesDays => {
            "How long message history stays maintained. Past it, segments stop being              re-wrapped on rotation and go dark to anyone who did not already hold them."
        }
        S::RetainAttachmentsDays => {
            "How long attachments stay maintained. Separate from messages because the costs              are not comparable — a heavy week of files outweighs years of text."
        }
    }
}

/// Parses the handle the interface passes back for a setting.
fn setting_of(id: &str) -> Result<kols_core::ChatSetting, String> {
    kols_core::ChatSetting::ALL
        .iter()
        .find(|setting| format!("{setting:?}") == id)
        .copied()
        .ok_or_else(|| format!("{id:?} is not a setting"))
}

/// Changes one of this network's chat settings.
#[tauri::command]
fn set_chat_setting(app: tauri::State<'_, App>, setting: String, value: i64) -> Result<(), String> {
    let setting = setting_of(&setting)?;
    app.with(|executor| {
        executor
            .submit(Command::SetChatSetting { setting, value })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

/// What this machine offers this network, and whether anybody has said.
///
/// Both halves are sent because they are different answers (`Store::storage_offered`):
/// nothing set means the shipped default is in force and would follow a revised
/// one, while zero is a member having opted out. Rendering them identically
/// would make opting out look like never having chosen.
#[tauri::command]
fn contribution(app: tauri::State<'_, App>) -> Result<dto::Contribution, String> {
    app.with(|executor| {
        let chosen = executor.store().contribution();
        let offer = chosen.unwrap_or(kols_node::serve::DEFAULT_CONTRIBUTION);
        Ok(dto::Contribution {
            storage_offered: offer.storage_offered,
            upload_offered: offer.upload_offered,
            download_offered: offer.download_offered,
            relay_willing: offer.relay_willing,
            is_default: chosen.is_none(),
            storage_used: executor.store().duty_bytes(),
            storage_total: executor.store().stored_bytes(),
            reachable: executor.store().reachable(),
        })
    })
}

/// Sets what this machine contributes here — Core §4.3.
#[tauri::command]
fn set_contribution(
    app: tauri::State<'_, App>,
    storage_offered: u64,
    upload_offered: u64,
    download_offered: u64,
    relay_willing: bool,
) -> Result<(), String> {
    app.with(|executor| {
        executor
            .submit(Command::SetContribution {
                storage_offered,
                upload_offered,
                download_offered,
                relay_willing,
            })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

/// Says what to tell this network about you — `design/01` §9.
///
/// One of `here`, `idle`, `busy` or `invisible`, and refused otherwise: an
/// unrecognised value would be read back as *never chosen*, and never chosen is
/// visible. There is no `offline`, because there is no observation that would
/// justify the word (`design/09` §4.1).
#[tauri::command]
fn set_presence(app: tauri::State<'_, App>, state: String) -> Result<(), String> {
    app.with(|executor| {
        executor
            .submit(Command::SetPresence { state })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

/// Asks this node to collect history older than the oldest message on screen.
///
/// Bounded by a reading rather than open-ended: history is a backwards chain, so
/// without a stopping point the only choices are one hop or all of it. The
/// oldest message the member can currently see is exactly the right boundary —
/// it is where their view stops.
#[tauri::command]
fn fetch_history(app: tauri::State<'_, App>, channel: String, before_millis: i64) -> Result<(), String> {
    let channel = App::channel(&channel)?;
    app.with(|executor| {
        executor
            .submit(Command::FetchHistory {
                channel,
                before: kols_core::Hlc::new(before_millis, 0),
            })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

/// Starts serving everything this installation belongs to, once somebody has
/// logged in.
///
/// **Where the startup logic went.** This used to run before the window existed,
/// which is now too early by construction: with the seeds wrapped there is
/// nothing to open until an account is unlocked.
///
/// # It opens nothing, and it used to — two defects from the same leftover
///
/// This began as *resume the network you were in*, from before the workspace
/// window existed, and it held on to two habits that became wrong when D40 made
/// a window a view.
///
/// It **marked a network as in view** and held its executor. Nothing was showing
/// it: unlocking arrives at the list (`09` §1.13), so the row drew itself as the
/// open one and offered *leave* where it meant *forget*, for a network nobody
/// had opened. In-view is now set by `open_network` alone, which is the only
/// thing that puts a network on a screen.
///
/// And it gave up unless there was **exactly one** network — a single-network
/// relic that quietly meant a member with two networks unlocked and served
/// neither until they clicked one. `09` §2 makes every joined network warm
/// whether or not anybody is looking at it, which is the whole reason a
/// conversation nobody has open still arrives.
#[tauri::command]
fn resume(handle: tauri::AppHandle, app: tauri::State<'_, App>) -> Result<bool, String> {
    let any = !app.workspace.list().is_empty();
    reconcile_nodes(&handle, &app);
    Ok(any)
}

/// Exports every identity on this disk, sealed under its own passphrase.
///
/// **Portability rather than recovery** (`design/02` §6.3). The seed is the
/// member and lives on one disk: the account password protects it from somebody
/// who takes the laptop, and nothing protects it from the laptop dying.
///
/// Its passphrase is deliberately **not** the account password. This file leaves
/// the machine, so the machine's protection does not travel with it — and one
/// secret losing both the keyring and the thing kept in case the keyring is lost
/// would defeat the point.
#[tauri::command]
fn export_bundle(
    app: tauri::State<'_, App>,
    passphrase: String,
    path: String,
) -> Result<usize, String> {
    if passphrase.is_empty() {
        return Err("a passphrase is required — this file is every identity here".to_owned());
    }
    let entries = app.workspace.to_bundle()?;
    let sealed = kols_node::bundle::seal(&entries, &passphrase).map_err(|err| err.to_string())?;
    // Written `0600` like every other secret this program puts on disk. It is
    // sealed as well, and the two are independent: the permissions are what
    // stand between another account on this machine and the file, and the
    // passphrase is what stands between anybody at all and its contents.
    kols_node::secret::write_private(std::path::Path::new(&path), &sealed)
        .map_err(|err| err.to_string())?;
    Ok(entries.len())
}

/// Restores networks from a bundle.
#[tauri::command]
fn import_bundle(
    app: tauri::State<'_, App>,
    passphrase: String,
    path: String,
) -> Result<dto::Restored, String> {
    let raw = std::fs::read(&path).map_err(|err| format!("could not read that file: {err}"))?;
    let entries = kols_node::bundle::open(&raw, &passphrase).map_err(|err| err.to_string())?;
    let outcome = app.workspace.from_bundle(&entries)?;
    Ok(dto::Restored {
        added: outcome.added,
        skipped: outcome.skipped,
        refused: outcome.refused,
    })
}

/// Whether this installation has an account, and whether it is open.
///
/// The first thing the window asks, before it asks anything else, because every
/// other answer depends on it: with a locked installation there is no seed to
/// derive an identity from and therefore no network to open.
#[tauri::command]
fn account_state(app: tauri::State<'_, App>) -> dto::AccountState {
    let root = app.workspace.root();
    dto::AccountState {
        exists: kols_node::account::Account::exists(root),
        unlocked: kols_node::account::is_unlocked(root),
        username: kols_node::account::Account::username(root),
        unprotected: app.workspace.unprotected(),
    }
}

/// Makes this installation's account and wraps whatever was in the clear.
///
/// **Wrapping happens here rather than being offered**, because the account
/// exists to make the seeds unreadable and one that left them as they were would
/// be a password protecting nothing (`design/02` §6.3).
#[tauri::command]
fn create_account(
    app: tauri::State<'_, App>,
    username: String,
    password: String,
) -> Result<usize, String> {
    let root = app.workspace.root().to_path_buf();
    let account = kols_node::account::create_process(&root, &username, &password)
        .map_err(|err| err.to_string())?;
    Ok(app.workspace.adopt_all(&account))
}

/// Opens it.
#[tauri::command]
fn unlock(app: tauri::State<'_, App>, password: String) -> Result<usize, String> {
    let root = app.workspace.root().to_path_buf();
    let account =
        kols_node::account::unlock_process(&root, &password).map_err(|err| err.to_string())?;
    // An installation that skipped a release still gets wrapped, on the next
    // unlock rather than on a step somebody has to remember.
    Ok(app.workspace.adopt_all(&account))
}

/// Locks the interface, and deliberately does not stop the node.
///
/// `design/02` §6.3: *nobody at my keyboard can act as me* and *I want to
/// disappear from the network* are different requests, and only the first one is
/// being made. The node keeps serving what it holds, keeps its relay
/// reservation and keeps answering for its member; quitting the application is
/// how somebody makes the second request.
#[tauri::command]
fn lock(handle: tauri::AppHandle, _app: tauri::State<'_, App>) {
    kols_node::account::lock_process();
    // **The lock reaches every window** — `design/09` §1.12. It hid one window
    // when there was one; a network window left on screen after a lock would
    // leave this installation's messages readable to somebody at the keyboard,
    // which is the first of the three things `02` §6.3 says the lock protects.
    //
    // Closed rather than hidden, because this window is cheap to reopen and
    // because a hidden window holding a network's drawn state is a copy of that
    // state sitting behind a lock that did not clear it.
    if let Some(window) = handle.get_webview_window(NETWORK_WINDOW) {
        let _ = window.close();
    }
    if let Some(window) = handle.get_webview_window(WORKSPACE_WINDOW) {
        let _ = window.set_focus();
    }
}

/// Hides the workspace window without ending the application.
///
/// What closing it does too (`design/09` §1.11) — this is the button on the
/// notice that says so, so that the first close is an informed one rather than
/// a surprise.
#[tauri::command]
fn hide_workspace(handle: tauri::AppHandle) -> Result<(), String> {
    let Some(window) = handle.get_webview_window(WORKSPACE_WINDOW) else {
        return Ok(());
    };
    window.hide().map_err(|err| err.to_string())
}

/// Stops every node and ends the application — the deliberate way out.
///
/// The only path that runs a shutdown properly: claims and relay reservations
/// are released rather than left to expire (`05` §1.1). Offered beside the
/// notice because the moment somebody learns that closing is not quitting is
/// the moment they may want to actually quit.
#[tauri::command]
fn quit_app(handle: tauri::AppHandle) {
    stop_node(&handle);
    handle.exit(0);
}

/// Every conversation this installation holds, at whatever stage — `09` §1.4.
///
/// # Gathered across networks, which is the workspace window's whole point
///
/// A conversation is arranged inside a shared network, so what is pending lives
/// in *that* network's store (spec 07 §6.2 permits the two parties to hold a
/// request and nobody else). A member should not have to open each network in
/// turn to find out who has asked to talk to them, so this asks all of them.
///
/// Local knowledge only, and worth saying which kind: the correlation between a
/// member's networks exists in this directory and nowhere else (`03` §4.6). No
/// peer learns it, and nothing here is published.
#[tauri::command]
fn conversations(app: tauri::State<'_, App>) -> Result<Vec<dto::Conversation>, String> {
    let stores: Vec<kols_node::store::Store> = app
        .workspace
        .list()
        .into_iter()
        .filter_map(|known| kols_node::store::Store::open(known.path).ok())
        .collect();

    let mut found = Vec::new();
    for store in &stores {
        let shared = to_hex(store.network().as_bytes());
        // A request waiting on an answer. Every one of these verified before it
        // reached the disk (spec 07 §6.2), so there is no unverified state to
        // render and no badge to put on one.
        for (who, _) in store.requests() {
            found.push(dto::Conversation {
                network: String::new(),
                who: to_hex(who.verifying_key().as_bytes()),
                shared: shared.clone(),
                state: "asked".to_owned(),
                label: name_in(store, &who),
            });
        }
        // One this member offered and the daemon has not delivered, or has
        // delivered and nobody has answered — which are the same row, because
        // nothing tells the sender which (`09` §1.8).
        for (who, network) in store.offered_conversations() {
            found.push(dto::Conversation {
                network: to_hex(network.as_bytes()),
                who: to_hex(who.verifying_key().as_bytes()),
                shared: shared.clone(),
                state: "offered".to_owned(),
                label: name_in(store, &who),
            });
        }
    }

    // And the ones that exist: a conversation network on this disk, named by
    // where it was arranged rather than by its own id.
    for store in &stores {
        if store.cached_profile() != Some(kols_core::NetworkProfile::Conversation) {
            continue;
        }
        let (shared, who) = match store.origin() {
            Some(origin) => origin,
            // A conversation with no origin recorded borrows no relay and can
            // say nothing about who it is with (`Store::origin`). Listed
            // anyway, because it is on this disk and hiding it would be worse.
            None => {
                found.push(dto::Conversation {
                    network: to_hex(store.network().as_bytes()),
                    who: String::new(),
                    shared: String::new(),
                    state: "joined".to_owned(),
                    label: store.label().unwrap_or_default(),
                });
                continue;
            }
        };
        let label = stores
            .iter()
            .find(|other| other.network() == &shared)
            .map(|other| name_in(other, &who))
            .unwrap_or_else(|| to_hex(who.verifying_key().as_bytes())[..8].to_owned());
        found.push(dto::Conversation {
            network: to_hex(store.network().as_bytes()),
            who: to_hex(who.verifying_key().as_bytes()),
            shared: to_hex(shared.as_bytes()),
            state: "joined".to_owned(),
            label,
        });
    }
    Ok(found)
}

/// What to call somebody, in the network the name was claimed in.
///
/// **A name is never sufficient on its own** (spec 07 §8, §3.9.1): the
/// uniqueness key deliberately does not fold confusables, so an interface has
/// to render enough identity beside a name to tell two lookalikes apart. This
/// returns the name; the row puts the identity beside it, which is where a
/// person is actually deciding who they are talking to.
fn name_in(store: &kols_node::store::Store, who: &intranet_identity::PerNetworkIdentityId) -> String {
    let short = to_hex(who.verifying_key().as_bytes())[..8].to_owned();
    let Ok(state) = store.state() else {
        return short;
    };
    let Ok(executor) = Executor::open(store.root().to_path_buf()) else {
        return short;
    };
    executor
        .names(&state)
        .ok()
        .and_then(|names| names.of(who).map(str::to_owned))
        .unwrap_or(short)
}

/// Offers a conversation to a member of a network this installation is in.
#[tauri::command]
fn start_conversation(
    app: tauri::State<'_, App>,
    network: String,
    with: String,
) -> Result<String, String> {
    let shared = app.workspace.open(&network)?;
    let with = kols_node::parse_identity(&with)?;
    let started = kols_node::dm::start(&app.workspace, &shared, &with, None)?;
    Ok(to_hex(started.conversation.as_bytes()))
}

/// Accepts a request, which joins the conversation's network.
#[tauri::command]
async fn accept_conversation(
    handle: tauri::AppHandle,
    network: String,
    from: String,
) -> Result<String, String> {
    let from = kols_node::parse_identity(&from)?;
    let shared = kols_node::parse_network(&network)?;
    let workspace = {
        let app = handle.state::<App>();
        Workspace::at(app.workspace.root().to_path_buf())
    };
    let conversation = kols_node::dm::accept(&workspace, &shared, &from).await?;
    // The new network is one this installation belongs to, so it is warm like
    // every other (`09` §2) — and a conversation neither party has open is
    // exactly the case that has to keep running.
    reconcile_nodes(&handle, &handle.state::<App>());
    Ok(to_hex(conversation.as_bytes()))
}

/// Declines a request, which tells nobody — spec 07 §6.2.
#[tauri::command]
fn decline_conversation(
    app: tauri::State<'_, App>,
    network: String,
    from: String,
) -> Result<(), String> {
    let shared = app.workspace.open(&network)?;
    kols_node::dm::decline(&shared, &kols_node::parse_identity(&from)?)
}

/// The label a conversation window is drawn under.
///
/// One window per conversation, each its own view (`design/09` §1.6) — unlike
/// the single network window, because several conversations at once is the
/// shape's whole advantage and they are small. Every label must appear in
/// `capabilities/default.json`, or the window gets an empty allow-list and no
/// node event ever reaches it, silently (`design/05` §1).
fn conversation_label(network: &str) -> String {
    format!("conversation-{}", &network[..16.min(network.len())])
}

/// An executor for a network that is not the one in view.
///
/// # Why conversations do not use the open executor
///
/// `App::open` is the network a member is *looking at*, and a conversation
/// window is looking at a different one — possibly three of them at once. So
/// each call opens an executor for the network it names, which is exactly what
/// the terminal does on every invocation and is cheap for the same reason: the
/// store caches its replayed state, so this is a directory read rather than a
/// replay (`design/05` §5).
///
/// It crosses the same boundary either way. `Executor::submit` authorizes and
/// then runs, and there is no second path into it — `05` §3's first property is
/// held by a type rather than by which caller happens to be asking.
fn executor_for(app: &App, network: &str) -> Result<Executor, String> {
    let store = app.workspace.open(network)?;
    let root = store.root().to_path_buf();
    drop(store);
    Executor::open(root).map_err(|err| err.to_string())
}

/// Draws a conversation in a window of its own, creating or raising it.
#[tauri::command]
fn open_conversation(
    handle: tauri::AppHandle,
    app: tauri::State<'_, App>,
    network: String,
    label: String,
) -> Result<(), String> {
    // Opened for its side effects: it refuses a network this disk does not
    // hold, which is the check worth making before a window exists to report
    // it in.
    let _ = executor_for(&app, &network)?;

    let shown = if label.trim().is_empty() {
        network[..8.min(network.len())].to_owned()
    } else {
        label.trim().to_owned()
    };
    let title = format!("{shown} — a conversation");
    let name = conversation_label(&network);

    if let Some(existing) = handle.get_webview_window(&name) {
        let _ = existing.set_focus();
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        &handle,
        name,
        tauri::WebviewUrl::App(format!("conversation.html?network={network}").into()),
    )
    .title(title)
    // Small, because a conversation is one implied channel and a person
    // (`09` §1.6) — a window sized for a channel rail and a roster would be
    // claiming to hold things a conversation does not have.
    .inner_size(460.0, 620.0)
    .min_inner_size(320.0, 360.0)
    .disable_drag_drop_handler()
    .build()
    .map_err(|err| format!("could not open a window for this conversation: {err}"))?;
    Ok(())
}

/// Who a conversation is with, and whether it can still be reached.
#[tauri::command]
fn conversation_who(app: tauri::State<'_, App>, network: String) -> Result<dto::Conversation, String> {
    let id = kols_node::parse_network(&network)?;
    let store = app
        .workspace
        .store_for(&id)
        .ok_or("that conversation is not on this disk")?;

    let (shared, who) = store.origin().unzip();
    let label = match (shared, who) {
        (Some(shared), Some(who)) => app
            .workspace
            .store_for(&shared)
            .map(|other| name_in(&other, &who))
            .unwrap_or_else(|| to_hex(who.verifying_key().as_bytes())[..8].to_owned()),
        _ => store.label().unwrap_or_default(),
    };

    // **Whether the rendezvous can still be borrowed** — D39, and `09` §1.9's
    // owed sentence. The permission is recomputed rather than stored, so this
    // is the answer *now*: both parties still members of the network this was
    // arranged in. A conversation that has stopped connecting and one where
    // nobody is talking render identically, and only the first is worth saying.
    let reachable = app.workspace.borrowable_relay(&store).is_some();
    Ok(dto::Conversation {
        network,
        who: who
            .map(|who| to_hex(who.verifying_key().as_bytes()))
            .unwrap_or_default(),
        shared: shared.map(|it| to_hex(it.as_bytes())).unwrap_or_default(),
        state: if reachable { "joined".to_owned() } else { "adrift".to_owned() },
        label,
    })
}

/// Reads a conversation's one implied channel.
#[tauri::command]
fn conversation_read(
    app: tauri::State<'_, App>,
    network: String,
    window: Option<dto::WindowArg>,
) -> Result<dto::Opened, String> {
    let id = kols_node::parse_network(&network)?;
    // **Derived, never declared** (spec 07 §3.6): a conversation has exactly
    // one channel and nothing announces it, so both ends compute the same id
    // from the network. There is nothing to look up and nothing to disagree
    // about.
    let channel = kols_core::conversation_channel_id(&id);
    let executor = executor_for(&app, &network)?;
    open_one(&executor, channel, window.unwrap_or_default().resolve())
}

/// Says something in a conversation.
#[tauri::command]
fn conversation_send(
    app: tauri::State<'_, App>,
    network: String,
    body: String,
) -> Result<(), String> {
    let id = kols_node::parse_network(&network)?;
    let channel = kols_core::conversation_channel_id(&id);
    executor_for(&app, &network)?
        .submit(Command::SendMessage {
            channel,
            body,
            reply_to: None,
            attachments: Vec::new(),
        })
        .map(|_| ())
        .map_err(|err| err.to_string())
}

/// Raises the window listing everything this installation belongs to.
///
/// The way back from a network, and the shell's to do rather than the
/// document's: a document asking to be shown a window it does not own is a
/// document with an opinion about window management.
#[tauri::command]
fn show_workspace(handle: tauri::AppHandle) -> Result<(), String> {
    let Some(window) = handle.get_webview_window(WORKSPACE_WINDOW) else {
        return Ok(());
    };
    // Unhidden rather than recreated: closing this window hides it (below), so
    // it is always there to be shown and its state survives being put away.
    window.show().map_err(|err| err.to_string())?;
    window.unminimize().map_err(|err| err.to_string())?;
    window.set_focus().map_err(|err| err.to_string())
}

/// The ceiling on everything this installation stores, and what it is using.
///
/// A workspace command rather than a network one, and outside the `kols-api`
/// vocabulary for the reason creating a network is: that boundary is per
/// network, and a disk does not know how many networks are on it.
#[tauri::command]
fn storage_ceiling(app: tauri::State<'_, App>) -> Result<dto::StorageCeiling, String> {
    Ok(dto::StorageCeiling {
        ceiling: app.workspace.ceiling(),
        used: app.workspace.stored_bytes(),
        networks: app.workspace.list().len(),
    })
}

/// Sets the ceiling on everything this installation stores.
#[tauri::command]
fn set_storage_ceiling(app: tauri::State<'_, App>, bytes: u64) -> Result<(), String> {
    app.workspace.set_ceiling(bytes)
}

/// Chooses how joiners are admitted.
#[tauri::command]
fn set_admission_mode(app: tauri::State<'_, App>, mode: String) -> Result<(), String> {
    let mode = match mode.as_str() {
        "auto" => intranet_governance::AdmissionMode::AutoAdmit,
        "intake" => intranet_governance::AdmissionMode::ExplicitIntake,
        other => return Err(format!("{other:?} is not an admission mode")),
    };
    app.with(|executor| {
        executor
            .submit(Command::SetAdmissionMode { mode })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

/// Every role, what it holds, and who is in it — `design/02` §1.
///
/// One read rather than a call per role: the whole answer comes out of a single
/// replay, and asking per row would replay the log once per role for a question
/// one pass already settled.
#[tauri::command]
fn roles(app: tauri::State<'_, App>) -> Result<Vec<dto::Role>, String> {
    app.with(|executor| {
        let store = executor.store();
        let me = store.identity().map_err(|e| e.to_string())?.id();
        let state = store.state().map_err(|e| e.to_string())?;
        let names = executor.names(&state).map_err(|e| e.to_string())?;
        let (channels, _) = network::channels(store, &state).map_err(|e| e.to_string())?;
        let (categories, _) = network::categories(store, &state).map_err(|e| e.to_string())?;

        // Every scope name this network could have granted at, so a grant can be
        // rendered as the thing it names rather than as a hash.
        let mut labels: std::collections::BTreeMap<String, (String, String)> =
            std::collections::BTreeMap::new();
        for channel in channels.values() {
            labels.insert(
                to_hex(channel.id.as_bytes()),
                ("channel".to_owned(), format!("#{}", channel.name)),
            );
        }
        for category in categories.values() {
            labels.insert(
                to_hex(category.id.as_bytes()),
                ("category".to_owned(), category.name.clone()),
            );
        }

        Ok(state
            .groups
            .values()
            .map(|group| {
                let unrestricted =
                    matches!(group.capabilities, intranet_governance::CapabilitySet::All);
                let mut grants = Vec::new();
                let mut protocol_grants = Vec::new();

                if let intranet_governance::CapabilitySet::Explicit(held) = &group.capabilities {
                    for capability in held {
                        match capability {
                            intranet_governance::Capability::Extension(full) => {
                                match parse_grant(full, &labels) {
                                    Some(grant) => grants.push(grant),
                                    // An extension outside the chat vocabulary.
                                    // Shown as a protocol grant rather than
                                    // dropped: a role whose powers were half
                                    // displayed reads as weaker than it is.
                                    None => protocol_grants.push(full.clone()),
                                }
                            }
                            other => protocol_grants.push(describe_capability(other)),
                        }
                    }
                }
                grants.sort_by(|a, b| (&a.verb, &a.scope_label).cmp(&(&b.verb, &b.scope_label)));
                protocol_grants.sort();

                let mut members: Vec<_> = group
                    .members
                    .keys()
                    .map(|who| dto::Member {
                        identity: to_hex(who.verifying_key().as_bytes()),
                        short: who.short(),
                        name: names.of(who).map(str::to_owned),
                        // Not asked here. Whether a node has a connection to
                        // somebody is a question about this moment and belongs
                        // to the roster (`design/09` §4.1); a role's membership
                        // is replayed state and does not change when a socket
                        // does.
                        connected: false,
                        // Nor this, for the same reason: what somebody is
                        // telling the network right now is not part of who
                        // holds a role.
                        presence: None,
                        you: *who == me,
                    })
                    .collect();
                members.sort_by(|a, b| {
                    (a.name.is_none(), &a.name, &a.short)
                        .cmp(&(b.name.is_none(), &b.name, &b.short))
                });

                dto::Role {
                    id: group.id.to_string(),
                    implicit: group.id.is_everyone()
                        || group.id.as_str() == intranet_governance::FOUNDERS,
                    unrestricted,
                    everyone: group.id.is_everyone(),
                    grants,
                    protocol_grants,
                    members,
                    may_assign: state.identity_holds(
                        &me,
                        &intranet_governance::Capability::manage_membership(group.id.clone()),
                    ),
                }
            })
            .collect())
    })
}

/// Splits a chat capability name back into a verb and a scope.
///
/// The inverse of `kols_core::Scope::name`, and the only place that inversion
/// happens. Returns `None` for anything outside the chat vocabulary, which is
/// how an extension belonging to some other consuming spec — or a verb this
/// build does not know — is kept out of a grid built for chat verbs.
fn parse_grant(
    full: &str,
    labels: &std::collections::BTreeMap<String, (String, String)>,
) -> Option<dto::Grant> {
    let rest = full.strip_prefix("chat:")?;
    let (verb, scope) = rest.split_once(':')?;
    if !kols_core::is_verb(verb) {
        return None;
    }
    let governance = kols_core::capabilities::VERBS
        .iter()
        .any(|(name, tier)| *name == verb && *tier == intranet_governance::Tier::Governance);

    let (kind, id, label) = if scope == "*" {
        ("network".to_owned(), String::new(), "network-wide".to_owned())
    } else if let Some(id) = scope.strip_prefix("cat:") {
        let label = labels.get(id).map_or_else(
            // A grant can outlive what it names: deleting a category or channel
            // leaves grants against its id in place, because a capability is a
            // string in a set and nothing sweeps them. Saying so beats a bare
            // hash, and beats hiding a grant that still resolves.
            || "a category that is gone".to_owned(),
            |(_, label)| label.clone(),
        );
        ("category".to_owned(), id.to_owned(), label)
    } else {
        let label = labels.get(scope).map_or_else(
            || "a channel that is gone".to_owned(),
            |(_, label)| label.clone(),
        );
        ("channel".to_owned(), scope.to_owned(), label)
    };

    Some(dto::Grant {
        verb: verb.to_owned(),
        scope: kind,
        scope_id: id,
        scope_label: label,
        governance,
    })
}

/// Names a protocol capability for display.
fn describe_capability(capability: &intranet_governance::Capability) -> String {
    use intranet_governance::Capability as C;
    match capability {
        C::ApproveNode => "approve-node".to_owned(),
        C::RevokeNode => "revoke-node".to_owned(),
        C::DefineGroup => "define-group".to_owned(),
        C::DefinePolicy => "define-policy".to_owned(),
        C::DefineContentPolicy => "define-content-policy".to_owned(),
        C::ModerateContent => "moderate-content".to_owned(),
        C::AuditReputation => "audit-reputation".to_owned(),
        C::ReadContent => "read-content".to_owned(),
        C::ManageMembership(group) => format!("manage-membership:{group}"),
        C::Publish(content_type) => format!("publish:{content_type}"),
        C::Extension(name) => name.clone(),
    }
}

/// The verbs a grant may name, with what each one costs.
///
/// Served from `kols_core::capabilities::VERBS` rather than restated in the
/// webview, so re-tiering a verb in `design/02` §2.2 moves the interface with
/// it instead of leaving a second copy to drift. The summaries are the
/// interface's own — they say what holding one *does*, which the vocabulary
/// table has no field for and should not grow one.
#[tauri::command]
fn verbs() -> Vec<dto::Verb> {
    kols_core::capabilities::VERBS
        .iter()
        .map(|(name, tier)| dto::Verb {
            name: (*name).to_owned(),
            governance: *tier == intranet_governance::Tier::Governance,
            summary: match *name {
                "post" => "write, revise and withdraw messages",
                "read" => "read what is written",
                "create-channel" => "define new channels",
                "manage-channel" => "rename, move, archive and delete channels, and set who is in a private one",
                "moderate" => "hide other members' messages, and pin",
                "set-name" => "claim a display name here",
                "connect-voice" => "join a voice channel and hear it",
                "speak-voice" => "transmit in a voice channel",
                // Total by construction rather than defaulted: a verb added to
                // the vocabulary should arrive here with a sentence, and an
                // empty arm would ship it with a placeholder nobody noticed.
                other => other,
            }
            .to_owned(),
        })
        .collect()
}

/// Every scope a grant can bind at, in the order the sidebar shows them.
#[tauri::command]
fn scopes(app: tauri::State<'_, App>) -> Result<Vec<dto::ScopeOption>, String> {
    app.with(|executor| {
        let store = executor.store();
        let state = store.state().map_err(|e| e.to_string())?;
        let (channels, _) = network::channels(store, &state).map_err(|e| e.to_string())?;
        let (categories, _) = network::categories(store, &state).map_err(|e| e.to_string())?;

        let mut out = vec![dto::ScopeOption {
            kind: "network".to_owned(),
            id: String::new(),
            label: "network-wide".to_owned(),
        }];
        // Categories before channels, because `design/02` §4 makes the category
        // the scope a grant is expected to bind at and the channel the override.
        // Ordering the picker that way is the cheapest place to say so.
        let mut cats: Vec<_> = categories.values().collect();
        cats.sort_by(|a, b| (a.position, &a.name).cmp(&(b.position, &b.name)));
        for category in cats {
            out.push(dto::ScopeOption {
                kind: "category".to_owned(),
                id: to_hex(category.id.as_bytes()),
                label: category.name.clone(),
            });
        }
        let mut chans: Vec<_> = channels.values().filter(|c| !c.archived).collect();
        chans.sort_by(|a, b| a.name.cmp(&b.name));
        for channel in chans {
            out.push(dto::ScopeOption {
                kind: "channel".to_owned(),
                id: to_hex(channel.id.as_bytes()),
                label: format!("#{}", channel.name),
            });
        }
        Ok(out)
    })
}

/// Names this network, for every member — D32.
#[tauri::command]
fn set_network_name(app: tauri::State<'_, App>, name: String) -> Result<(), String> {
    app.with(|executor| {
        executor
            .submit(Command::SetNetworkName { name: name.clone() })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

/// Creates a role, holding nothing.
#[tauri::command]
fn create_role(app: tauri::State<'_, App>, name: String) -> Result<(), String> {
    let group = intranet_governance::GroupId::new(name.trim());
    app.with(|executor| {
        executor
            .submit(Command::CreateRole {
                group: group.clone(),
            })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

/// Grants or withdraws one verb at one scope.
#[tauri::command]
fn set_permission(
    app: tauri::State<'_, App>,
    role: String,
    verb: String,
    scope: String,
    scope_id: String,
    grant: bool,
) -> Result<(), String> {
    // Rebuilt from the kind and the id rather than parsed from a name, so the
    // interface never hands across a capability string of its own. The one
    // construction stays `kols_core::Scope::name`, on both sides.
    let scope = match scope.as_str() {
        "network" => kols_core::Scope::Network,
        "category" => kols_core::Scope::Category(App::category(&scope_id)?),
        "channel" => kols_core::Scope::Channel(App::channel(&scope_id)?),
        other => return Err(format!("{other:?} is not a scope")),
    };
    app.with(|executor| {
        executor
            .submit(Command::SetPermission {
                group: intranet_governance::GroupId::new(role.clone()),
                verb: verb.clone(),
                scope,
                grant,
            })
            .map(|_| ())
            .map_err(|err| err.to_string())
    })
}

/// Puts an identity in a role, or takes them out.
#[tauri::command]
fn set_role_member(
    app: tauri::State<'_, App>,
    role: String,
    identity: String,
    member: bool,
) -> Result<(), String> {
    let who = kols_node::parse_identity(&identity)?;
    app.with(|executor| {
        executor
            .submit(Command::SetRoleMember {
                group: intranet_governance::GroupId::new(role.clone()),
                identity: who,
                member,
            })
            .map(|_| ())
            .map_err(|err| err.to_string())
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

/// Relays another of this member's networks already designates — D29, O11.
///
/// Asked *before* the designation, because the point of it is that the member
/// decides. The client warns and never refuses (`design/09` §3): a refusal is
/// unenforceable anyway, since nothing stops a founder naming one address in two
/// networks and the relay cannot know it is being reused, and it would block a
/// member legitimately relaying on their own LAN for two of their own networks.
///
/// **Two commands rather than one taking the network as an argument.** They
/// differ only in what is excluded from the comparison, and which to exclude is
/// a fact about which moment is asking — so the shell decides it and the webview
/// supplies only the addresses somebody typed. A front end that named the
/// network to exclude could hide the warning by naming the wrong one, which is
/// the same reason `authorize` looks a channel's category up rather than
/// accepting it on the command (`design/05` §3).
#[tauri::command]
fn shared_relays(
    app: tauri::State<'_, App>,
    relays: String,
) -> Result<Vec<dto::SharedRelay>, String> {
    let network = app.with(|executor| Ok(*executor.store().network()))?;
    Ok(app
        .workspace
        .shared_relays(Some(&network), &addresses(&relays))
        .into_iter()
        .map(dto::SharedRelay::of)
        .collect())
}

/// The same question for a network that does not exist yet.
///
/// Creating one with a relay is a designation like any other, and it is the
/// first one most people make. Nothing is excluded, because there is no network
/// to exclude — and excluding the *open* one instead would hide an overlap with
/// the network the member is looking at while they make a second.
#[tauri::command]
fn shared_relays_for_new_network(
    app: tauri::State<'_, App>,
    relays: String,
) -> Result<Vec<dto::SharedRelay>, String> {
    Ok(app
        .workspace
        .shared_relays(None, &addresses(&relays))
        .into_iter()
        .map(dto::SharedRelay::of)
        .collect())
}

/// Splits what somebody typed into addresses, validating nothing.
///
/// Both designation paths parse and refuse properly on the way in; this one is
/// asked *before* that, about text still being edited, so it must answer for
/// whatever it is given rather than refuse. An address too malformed to carry a
/// peer id is simply not compared (`Workspace::shared_relays`).
fn addresses(relays: &str) -> Vec<String> {
    relays
        .split_whitespace()
        .filter(|address| !address.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Designates this network's relays, replacing whatever it named before.
///
/// The gap this closes: the command, its gate and its executor have existed
/// since the terminal had them, and the window simply never submitted it — so a
/// founder who made a network here could not invite anybody and could not fix
/// it without a terminal.
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
    let relays = addresses(&relays);
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
    let designated = app.with(|executor| {
        executor
            .submit(Command::SetBootstrapRelays { relays })
            .map_err(|err| err.to_string())?;
        Ok(*executor.store().network())
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
    //
    // **A restart rather than a reconcile.** Reconciling is idempotent and would
    // leave a running node exactly as it is, which is right everywhere else and
    // wrong here: the point is to make it read a relay set it only consults at
    // startup. Stopping it first is what turns the next reconcile into a start.
    restart_one(&handle, &app, designated);
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
    let network = app.with(|executor| Ok(*executor.store().network()))?;
    restart_one(&handle, &app, network);
    Ok(())
}

/// Stops one network's node so the reconcile that follows starts it again.
///
/// The only thing that needs a restart rather than a reconcile: a node reads its
/// relay set once, at startup, so a designation made while it runs changes policy
/// and reaches nothing until it comes back. Everywhere else, reconciling is both
/// sufficient and cheaper — a running node stays running.
fn restart_one(
    handle: &tauri::AppHandle,
    app: &tauri::State<'_, App>,
    network: intranet_identity::NetworkId,
) {
    if let Ok(mut nodes) = app.nodes.lock() {
        nodes.stop(&network);
    }
    reconcile_nodes(handle, app);
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

/// Opens a channel and renders the range a reader is holding.
///
/// `window` is what the interface already has, and absent means it has nothing —
/// an ordinary open, which answers with the newest page. The cursors in it were
/// issued by a previous answer and are opaque to the interface by design
/// (`design/09` §4.4): a front end that could build a cursor could build a wrong
/// one, and a clock reading is exactly the shape that invites arithmetic.
#[tauri::command]
fn open_channel(
    app: tauri::State<'_, App>,
    channel: String,
    window: Option<dto::WindowArg>,
) -> Result<dto::Opened, String> {
    let channel = App::channel(&channel)?;
    let window = window.unwrap_or_default().resolve();
    app.with(|executor| open_one(executor, channel, window))
}

fn open_one(
    executor: &Executor,
    channel: ChannelId,
    window: kols_core::Window,
) -> Result<dto::Opened, String> {
    let outcome = executor
        .submit(Command::OpenChannel { channel, window })
        .map_err(|err| err.to_string())?;

    let Outcome::Opened {
        messages,
        rejected,
        authors,
        more_history,
        oldest,
        newest,
        older,
        newer,
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
        more_history,
        oldest: oldest.map(kols_core::Cursor::to_token),
        newest: newest.map(kols_core::Cursor::to_token),
        older,
        newer,
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
        // **Freshness is applied here rather than in the store**, because this
        // is the layer that is answering a question. The store keeps
        // observations, and an observation that has gone stale is still an
        // observation; what expires is the *claim* built on it.
        let now = kols_node::chat::now_millis();
        let heard: std::collections::BTreeMap<String, String> = store
            .beats()
            .into_iter()
            .filter(|(_, _, at)| kols_core::PresenceBeat::fresh_at(*at, now))
            .map(|(who, state, _)| (who, state.name().to_owned()))
            .collect();

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
                    // **Your own row is your own choice, not an observation.**
                    // This node ignores its own beats — a roster that counted
                    // them would answer a different question — so there is
                    // nothing heard to report here, and what somebody wants to
                    // see beside their own name is what the network is being
                    // told. That includes `invisible`, which is the one state
                    // nobody else can see and the one worth being sure about.
                    presence: if identity == me {
                        Some(
                            store
                                .presence()
                                .unwrap_or(kols_core::Presence::Show(kols_core::Beat::Here))
                                .name()
                                .to_owned(),
                        )
                    } else {
                        heard.get(&hex).cloned()
                    },
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
    let relays = addresses(&relay);

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
    let id = *executor.store().network();
    *app.open.lock().map_err(|_| "the workspace lock is poisoned")? = Some(executor);
    if let Ok(mut view) = app.in_view.lock() {
        *view = Some(id);
    }
    let _ = path;
    reconcile_nodes(&handle, &app);
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

    // A pasted invite is a server's: accepting a conversation goes through the
    // direct-message flow, which says so (`design/09` §1.7).
    let landed = kols_node::join::redeem(
        path.clone(),
        credential,
        30,
        false,
        kols_core::NetworkProfile::Server,
    )
    .await?;

    // Open it either way. A waiting-room member holds an identity and nothing
    // else, and showing them that — rather than nothing — is the difference
    // between "you are waiting" and "something went wrong".
    let executor = Executor::open(path.clone()).map_err(|err| err.to_string())?;
    let joined = *executor.store().network();
    {
        let app = handle.state::<App>();
        *app.open
            .lock()
            .map_err(|_| "the workspace lock is poisoned")? = Some(executor);
        if let Ok(mut view) = app.in_view.lock() {
            *view = Some(joined);
        }
    }
    let _ = path;
    reconcile_nodes(&handle, &handle.state::<App>());

    Ok(match landed {
        kols_node::join::Landed::Admitted => dto::Joined {
            admitted: true,
            identity: String::new(),
            answered: true,
        },
        kols_node::join::Landed::Waiting { identity } => dto::Joined {
            admitted: false,
            identity,
            answered: true,
        },
        // The node is already started above, which is the whole point: nothing
        // here knows whether this member was admitted, and the one thing that
        // can find out is a sync. Reporting a failure and closing the network
        // would spend the invite on the retry it invites (O21).
        kols_node::join::Landed::Unanswered { identity, .. } => dto::Joined {
            admitted: false,
            identity,
            answered: false,
        },
    })
}

/// Leaves a network and removes this installation's store for it — permanently.
///
/// **Two acts, in an order that cannot be reversed.** The departure entry is
/// signed by the seed the deletion destroys (Core §2.5.1), so it is written and
/// handed to the running node *first*; then the node is stopped and the store
/// goes. Doing it the other way round produces a member the network can never
/// be told about, which is the state `design/02` §6.5 exists to end.
///
/// **The open network is now the good case rather than the refused one.** This
/// used to insist the network be closed first, which guaranteed no node was
/// running and therefore that nothing could ever be announced — the requirement
/// was exactly backwards once there was something to announce. Forgetting a
/// closed network still works and still deletes everything; it simply cannot
/// tell anybody, and says so rather than reporting the same success.
///
/// What comes back is what actually happened rather than what was attempted.
/// [`dto::Forgotten`] carries whether a departure went out and how many members
/// this node was connected to when it did — who *could* have heard, since
/// gossip acknowledges nothing and presenting that number as receipt would be
/// the same lie one layer along.
#[tauri::command]
fn forget_network(
    handle: tauri::AppHandle,
    app: tauri::State<'_, App>,
    network: String,
) -> Result<dto::Forgotten, String> {
    let id = intranet_crypto::from_hex(network.trim())
        .and_then(|bytes| <[u8; 32]>::try_from(bytes.as_slice()).ok())
        .map(intranet_identity::NetworkId::from_bytes)
        .ok_or("that is not a network id")?;

    let open_here = app
        .open
        .lock()
        .map_err(|_| "the workspace lock is poisoned")?
        .as_ref()
        .is_some_and(|executor| *executor.store().network() == id);

    if !open_here {
        app.workspace.forget(&id)?;
        return Ok(dto::Forgotten {
            announced: false,
            reached: 0,
            reason: "this network was not open, so no node was running to tell it. Its \
                     members still have you in their logs"
                .to_owned(),
        });
    }

    // Written before anything is torn down, and refused loudly if it cannot be:
    // the last `revoke-node` holder is stopped here rather than left to delete
    // their store and strand the network silently.
    let reached = app.with(|executor| {
        executor
            .submit(Command::LeaveNetwork)
            .map_err(|err| err.to_string())?;
        Ok(executor.store().connected().len())
    })?;

    // One tick of the node's two-second refresh is what carries the entry from
    // the store into the node's own log and out to its peers. Waited for rather
    // than assumed, because the alternative is aborting the node between the
    // append and the adoption — which writes a departure into a store that is
    // about to be deleted and tells nobody at all.
    std::thread::sleep(ANNOUNCE_GRACE);

    stop_node(&handle);
    *app.open
        .lock()
        .map_err(|_| "the workspace lock is poisoned")? = None;

    app.workspace.forget(&id)?;
    Ok(dto::Forgotten {
        announced: true,
        reached,
        reason: if reached == 0 {
            "no member was connected, so the departure is written and nobody has heard \
             it. It cannot be sent again — the seed that signed it is gone"
                .to_owned()
        } else {
            String::new()
        },
    })
}

/// The label of the window a network is drawn in — `design/09` §1.5, D40.
///
/// One, reused as the member switches, with a second available on request
/// later. Every label this application creates has to appear in
/// `capabilities/default.json`, or the window gets an empty allow-list and no
/// node event ever reaches it — silently (`design/05` §1).
const NETWORK_WINDOW: &str = "network";

/// The label of the window listing everything this installation belongs to.
const WORKSPACE_WINDOW: &str = "workspace";

/// What a network window's title says — D36, `design/09` §6.5.
///
/// # Composed here rather than in the document, which is the point of it
///
/// D36 takes the network and the identity out of the themeable document
/// because a theme that makes one network resemble another does not cause
/// confusion, it causes a message written into the wrong network. A title the
/// *document* sets is a title the document can set wrongly, so it is only
/// outside the theme's reach in the sense that matters if the shell is the one
/// composing it — which is what this does. The interface supplies a number
/// (`set_unread`) and never a name: a document lying about a count is
/// harmless, and one lying about which network you are in is the whole risk.
///
/// The network's own name (D32) rather than the local label where it has one,
/// because that is what every member sees and what travels with the network.
fn network_title(store: &kols_node::store::Store, unread: usize) -> String {
    let replayed = store.state().ok();
    let named = replayed
        .as_ref()
        .and_then(|state| kols_core::ChatPolicy::of(&state.policy).network_name().map(str::to_owned))
        .filter(|name| !name.trim().is_empty())
        .or_else(|| store.label().filter(|label| !label.trim().is_empty()))
        .unwrap_or_else(|| to_hex(store.network().as_bytes())[..8].to_owned());

    // Which member you are *here*. Identities are per network (Core §1.2), so
    // this is not decoration: the same person is a different member in each,
    // and two windows open at once is exactly when that matters.
    let me = store
        .identity()
        .map(|identity| identity.id().short())
        .unwrap_or_else(|_| "not keyed".to_owned());

    if unread > 0 {
        format!("{named} ({unread}) — {me}")
    } else {
        format!("{named} — {me}")
    }
}

/// Draws a network in the network window, creating or raising it.
///
/// **The title is set before the window is shown, and cleared before a reuse
/// redraws.** `design/09` §1.5: a reused window turns D36's spoof into a
/// temporal one — click, the content redraws, the title lags, and the next
/// message goes to the network that was there a moment ago. §1 already
/// requires the screen cleared rather than overwritten; the native title is
/// part of what is cleared.
fn show_network_window(
    handle: &tauri::AppHandle,
    store: &kols_node::store::Store,
) -> Result<(), String> {
    let title = network_title(store, 0);
    let network = to_hex(store.network().as_bytes());

    if let Some(existing) = handle.get_webview_window(NETWORK_WINDOW) {
        // Cleared first, so nothing names the outgoing network over the
        // incoming one's content.
        let _ = existing.set_title("ko-ls");
        // **An event rather than `eval`.** Injecting a script to tell a window
        // which network it is showing would be this shell writing code into a
        // document it spends a CSP keeping other people's code out of, with the
        // network id interpolated into that script — which is the shape of every
        // injection bug there has ever been. The event channel is already how
        // every other push reaches the interface.
        existing
            .emit("kols://network", &network)
            .map_err(|err| err.to_string())?;
        existing.set_title(&title).map_err(|err| err.to_string())?;
        let _ = existing.set_focus();
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        handle,
        NETWORK_WINDOW,
        tauri::WebviewUrl::App(format!("index.html?network={network}").into()),
    )
    .title(title)
    .inner_size(1100.0, 720.0)
    .min_inner_size(640.0, 480.0)
    // The same default that had to be turned off for the one window this
    // application used to have: Tauri installs a native drag handler that takes
    // the drag before the page sees it, so HTML5 drag-and-drop — which is how
    // channels are reordered — never fires (`design/05` §1).
    .disable_drag_drop_handler()
    .build()
    .map_err(|err| format!("could not open a window for this network: {err}"))?;
    Ok(())
}

/// Folds the interface's unread count into the network window's title.
///
/// **The document supplies a number and never a name**, which is the whole of
/// how D36 survives having a count in the title at all. `network_title` composes
/// the string from replayed state; this only says how many. A document that
/// lied about the count would be a document lying to its own member about their
/// own messages, which is harmless; a document that could write the *network*
/// into the title could make one network wear another's name, which is the
/// failure D36 exists to prevent.
///
/// `design/09` §1.3: a network window's title carries its own network and never
/// a total across all of them — the cross-network count belongs on the
/// workspace window, which names no network and cannot be confused for one.
#[tauri::command]
fn set_unread(handle: tauri::AppHandle, app: tauri::State<'_, App>, unread: usize) -> Result<(), String> {
    let Some(window) = handle.get_webview_window(NETWORK_WINDOW) else {
        return Ok(());
    };
    let open = app.open.lock().map_err(|_| "the workspace lock is poisoned")?;
    let Some(executor) = open.as_ref() else {
        return Ok(());
    };
    window
        .set_title(&network_title(executor.store(), unread))
        .map_err(|err| err.to_string())
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
    let id = *store.network();
    show_network_window(&handle, &store)?;
    drop(store);
    let executor = Executor::open(root).map_err(|err| err.to_string())?;
    *app.open.lock().map_err(|_| "the workspace lock is poisoned")? = Some(executor);
    if let Ok(mut view) = app.in_view.lock() {
        *view = Some(id);
    }
    // **Reconciled rather than restarted.** Opening a network used to stop the
    // node for the one being left; now it raises one to hot and lowers the other
    // to warm, and neither is torn down — a tier change between two running
    // tiers is a label, and dropping a node's connections to change one would
    // cost its member every message that arrived during the reconnect.
    reconcile_nodes(&handle, &app);
    Ok(())
}

/// Brings the running nodes in line with what the member has joined.
///
/// `design/09` §2's policy, applied: the network in view is hot, everything else
/// joined is warm, and only a network the member set aside is cold — woken on
/// the poll rather than abandoned. Idempotent, so this is safe to call from
/// anywhere that might have changed the answer and from a tick that does not
/// know whether anything did.
///
/// This replaced a function that ran exactly one node and stopped it when the
/// member looked elsewhere. Everything below is the same forwarding it always
/// did, with one addition that is not cosmetic: **an event says which network it
/// came from**. It did not need to while a single node reported.
fn reconcile_nodes(handle: &tauri::AppHandle, app: &tauri::State<'_, App>) {
    let Ok(mut nodes) = app.nodes.lock() else {
        return;
    };
    let in_view = app.in_view.lock().ok().and_then(|view| *view);

    let specs: Vec<kols_node::nodes::Spec> = app
        .workspace
        .list()
        .into_iter()
        .filter_map(|known| {
            let store = kols_node::store::Store::open(known.path.clone()).ok()?;
            Some(kols_node::nodes::Spec {
                network: *store.network(),
                root: known.path,
                set_aside: store.is_set_aside(),
            })
        })
        .collect();

    let emitter = handle.clone();
    let sink: kols_node::nodes::TaggedSink = std::sync::Arc::new(
        move |network: &intranet_identity::NetworkId, events: &[kols_api::Event]| {
        let from = network.short();
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
                    // The network first, because a consumer has to know whether
                    // this event is even about the network it is showing before
                    // it looks at anything else in the payload.
                    let _ = emitter.emit(
                        "kols://records",
                        (from.clone(), to_hex(channel.as_bytes()), messages),
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
                // **Redrawn from the roster rather than applied from the
                // payload**, like everything else here: the beat is already in
                // the store with the time it was heard, and freshness has to be
                // recomputed anyway — a member stops being here because nothing
                // arrived, which is not an event and can never be one.
                kols_api::Event::MemberPresence { .. } => "kols://presence",
                // Re-read from the store rather than carried, for the same
                // reason as everything else here — the request is on disk
                // (spec 07 §6.2 lets exactly the two parties keep one), and a
                // consumer that rendered from this payload would be appending
                // where it should be merging. The workspace window's
                // conversations group listens for this and asks again
                // (`09` §1.8): without it a verified request sat on the disk
                // until something else happened to redraw that list.
                kols_api::Event::DirectMessageRequest { .. } => "kols://conversations",
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
                    let _ = emitter.emit("kols://relay", from.clone());
                    continue;
                }
            };
            // **Every event now says where it came from.** `Event` carries no
            // network id — it never needed one while a single node reported —
            // and a consumer that could not tell a record in a conversation from
            // one in a server would render each into the other. Tagged here
            // rather than in the type, so `design/05` §3's vocabulary is
            // unchanged for anything that still watches one network.
            let _ = emitter.emit(name, from.clone());
        }
    });

    // The relay standing belongs to whichever network is in view, and a node
    // that has not reported yet says nothing about it. Cleared on reconcile
    // rather than when a new one reports, so the gap reads as *asking* instead
    // of as a stale answer about a different network.
    if let Ok(mut relay) = app.relay.lock() {
        *relay = None;
    }

    nodes.reconcile(
        &specs,
        in_view.as_ref(),
        &sink,
        kols_node::serve::SEAL_TARGET_BYTES,
        std::time::Instant::now(),
    );
}

fn main() {
    let workspace = Workspace::at(Workspace::default_root());

    // **A second launch raises the windows that already exist** — `design/09`
    // §1.1, and the second door §1.11 needs. The application outlives its
    // windows, so launching it again is the ordinary way back when a tray icon
    // is not where somebody expects one; without this, that gesture would
    // start a second copy which then loses every store claim to the first, one
    // network at a time.
    //
    // Before Tauri, deliberately: the cheapest possible answer is to not build
    // a window at all, and a member who double-clicked an icon should see the
    // application come forward rather than watch a second one start and
    // vanish.
    let claim = match workspace.hold_application() {
        kols_node::workspace::Launch::First(claim) => claim,
        // Not a refusal, and it must not be reported as one: from where the
        // member is standing the application came to the front, which is what
        // they asked for.
        kols_node::workspace::Launch::Second => return,
    };

    // **Nothing is opened here, and no node is started.** `design/02` §6.3: a
    // node runs only once somebody has logged in, and that half is what decides
    // what the password protects. Starting one before then would require the
    // seeds to be unwrappable without it — at which point the password protects
    // nothing at rest and O7 is a lock over an unlocked door.
    //
    // So the window asks `account_state` first and drives what follows: a first
    // run, a login, or — once unlocked — `resume`, which is where the single
    // network that used to be opened here is opened instead.
    let app = tauri::Builder::default()
        .manage(App {
            workspace,
            open: Mutex::new(None),
            // **The runtime is handed over rather than looked up**, and that
            // is a fix rather than a style. `tokio::spawn` finds the runtime
            // *entered on the calling thread* and panics when there is none —
            // and a Tauri command is a synchronous caller with none entered, so
            // selecting a network took the window down with "there is no reactor
            // running". A handle works from any thread.
            nodes: Mutex::new(kols_node::nodes::Nodes::new(
                tauri::async_runtime::handle().inner().clone(),
            )),
            in_view: Mutex::new(None),
            outlives_windows: Mutex::new(false),
            claim: Mutex::new(Some(claim)),
            relay: Mutex::new(None),
            reorg: Mutex::new(None),
        })
        .on_window_event(|window, event| {
            let tauri::WindowEvent::CloseRequested { api, .. } = event else {
                return;
            };
            // A network window closes for real. It is a view of one network and
            // is created again the moment one is opened, so there is nothing to
            // keep — and keeping it would mean a hidden window holding a
            // network's drawn state, which is what `lock` refuses for the same
            // reason (`design/09` §1.12).
            if window.label() != WORKSPACE_WINDOW {
                return;
            }

            // Nothing to hide into. Let the close proceed, and `ExitRequested`
            // below will stop the nodes on the way out.
            let app = window.state::<App>();
            if !app.outlives_windows.lock().is_ok_and(|it| *it) {
                return;
            }

            // **The workspace window hides rather than closing**, because the
            // application outlives it (`09` §1.11) and a window that was
            // destroyed would have to be rebuilt, losing whatever was on screen
            // for no gain.
            api.prevent_close();

            let workspace = Workspace::at(app.workspace.root().to_path_buf());
            if workspace.told_closing_is_not_quitting() {
                let _ = window.hide();
                return;
            }
            // **Said once, the first time, and before the window goes.** A
            // member who closed every window may reasonably believe they shut
            // the application down, and it is still serving other members'
            // content (`05` §5.1). Shown rather than hidden-then-announced,
            // because a notice nobody can see is not one — and it carries the
            // way out, since the moment somebody learns closing is not quitting
            // is the moment they may want to quit.
            let _ = workspace.remember_closing_is_not_quitting();
            let _ = window.emit("kols://still-running", ());
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
            account_state,
            resume,
            export_bundle,
            import_bundle,
            create_account,
            unlock,
            lock,
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
            set_unread,
            show_workspace,
            conversations,
            open_conversation,
            conversation_who,
            conversation_read,
            conversation_send,
            start_conversation,
            accept_conversation,
            decline_conversation,
            hide_workspace,
            quit_app,
            relays,
            people,
            set_relays,
            shared_relays,
            shared_relays_for_new_network,
            restart_node,
            edit_message,
            delete_message,
            react,
            pin,
            new_relay_identity,
            roles,
            verbs,
            scopes,
            set_network_name,
            create_role,
            set_permission,
            set_role_member,
            settings,
            set_chat_setting,
            set_admission_mode,
            contribution,
            set_contribution,
            set_presence,
            storage_ceiling,
            set_storage_ceiling,
            fetch_history,
            forget_network
        ])
        .build(tauri::generate_context!())
        .expect("the window opens");

    // **A tick, because a poll is a schedule and a schedule needs something to
    // ask it.** `reconcile` is idempotent and decides everything from the state
    // it is handed, so calling it on a timer costs nothing while nothing has
    // changed — and it is the only thing that will ever wake a network the
    // member set aside (`design/09` §2's poll interval). Without this, cold
    // would be the "no node, ever" this was just corrected away from.
    //
    // A minute, against a ten-minute poll: fine enough that a wake is not late
    // by a noticeable fraction of its interval, coarse enough to be free.
    // **The claim is beaten and the ask is watched on the same second.** A
    // `stat` and a small write per second, which is bounded in the sense `09`
    // §4.4 means it: the work does not grow with anything. It is separate from
    // the supervisor's minute because a member who has just double-clicked an
    // icon is waiting, and a minute is not an answer.
    {
        let handle = app.handle().clone();
        tauri::async_runtime::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                tick.tick().await;
                let asked = {
                    let state = handle.state::<App>();
                    let Ok(held) = state.claim.lock() else { continue };
                    let Some(claim) = held.as_ref() else {
                        // Released on the way out. Nothing left to answer for.
                        return;
                    };
                    // Losing it stops nothing (see `AppClaim::beat`): the holder
                    // is whoever the disk says, and if it is no longer this
                    // process then raising its window is not its job.
                    claim.beat() && claim.asked_to_show()
                };
                if asked {
                    let _ = show_workspace(handle.clone());
                }
            }
        });
    }

    {
        let handle = app.handle().clone();
        tauri::async_runtime::spawn(async move {
            let mut tick = tokio::time::interval(SUPERVISOR_TICK);
            // The first tick fires immediately and would race the login that has
            // not happened yet, which is deliberate elsewhere and wrong here.
            tick.tick().await;
            loop {
                tick.tick().await;
                // Nothing runs before somebody has logged in (`02` §6.3), and
                // `list` is empty until the workspace is unlocked, so this is a
                // no-op until then rather than a thing to guard.
                reconcile_nodes(&handle, &handle.state::<App>());
            }
        });
    }

    // **The tray, and what it makes true** — `design/09` §1.11, D40.
    //
    // A window is a view: closing one must not set a network aside or stop its
    // node, which would reintroduce the defect `v0.13.0` was cut to fix while
    // looking like a feature. So the application outlives its windows and
    // quitting is a separate, explicit act — `00` §6 already decided that
    // *nobody at my keyboard can act as me* and *I want to disappear from the
    // network* are different requests, and `05` §5.1 has this machine holding
    // replica duty for other people that a close must not silently drop.
    {
        let handle = app.handle().clone();
        let show = tauri::menu::MenuItem::with_id(&handle, "show", "open ko-ls", true, None::<&str>)
            .expect("a menu item");
        let quit = tauri::menu::MenuItem::with_id(&handle, "quit", "quit", true, None::<&str>)
            .expect("a menu item");
        let menu = tauri::menu::Menu::with_items(&handle, &[&show, &quit]).expect("a menu");

        // **Two items, and no more, because the tray is reachable while locked.**
        // `02` §6.3 says the lock stops somebody at the keyboard seeing which
        // networks this installation belongs to — and a tray menu listing them
        // would read them out without the password. So it never lists anything:
        // there is nothing here to keep in step with the lock, which is a
        // stronger arrangement than one that hides the list at the right moment
        // (`design/09` §1.12).
        let tray = tauri::tray::TrayIconBuilder::with_id("kols")
            .icon(app.default_window_icon().expect("a bundled icon").clone())
            .tooltip("ko-ls — still running")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(|handle, event| match event.id().as_ref() {
                "show" => {
                    let _ = show_workspace(handle.clone());
                }
                // The one deliberate stop, and the only path that runs a
                // shutdown properly: every node stopped and awaited, so claims
                // and relay reservations are released rather than left to
                // expire (`05` §1.1).
                "quit" => {
                    stop_node(handle);
                    handle.exit(0);
                }
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                // Clicking the icon is the ordinary way back, and is what
                // somebody tries first.
                if let tauri::tray::TrayIconEvent::Click {
                    button: tauri::tray::MouseButton::Left,
                    button_state: tauri::tray::MouseButtonState::Up,
                    ..
                } = event
                {
                    let _ = show_workspace(tray.app_handle().clone());
                }
            })
            .build(&handle);

        // **A tray that could not be built must not be fatal, and must not be
        // assumed either.** Without one there is no way back to a hidden
        // window and no way to quit but killing the process — so the promise
        // in §1.11 inverts: closing the last window ends the application, which
        // is the behaviour before this slice and is honest. Recorded in the
        // state so the close handler and the notice both follow it rather than
        // each deciding for themselves.
        if let Err(err) = &tray {
            eprintln!("no tray icon ({err}); closing the last window will quit");
        }
        if let Ok(mut outlives) = handle.state::<App>().outlives_windows.lock() {
            *outlives = tray.is_ok();
        }
    }

    app.run(|handle, event| {
        match event {
            // **Closing the last window does not end the application.** Tauri
            // exits when no window is left, which was right while there was one
            // window and is the *close is going offline* behaviour `09` §1.11
            // refuses now that there are several. The tray keeps it reachable
            // and `quit` is the way out.
            //
            // `api.prevent_exit()` is not reached by the tray's own quit,
            // which calls `exit` after stopping the nodes: that path passes
            // through here with a code, and `ExitRequested` carries none for a
            // window-driven exit.
            tauri::RunEvent::ExitRequested { api, code, .. } => {
                let outlives = handle
                    .state::<App>()
                    .outlives_windows
                    .lock()
                    .is_ok_and(|it| *it);
                if code.is_none() && outlives {
                    api.prevent_exit();
                } else {
                    stop_node(handle);
                }
            }
            // The nodes are stopped on the way out however the exit was asked
            // for, including a signal or the session ending — the one thing
            // `05` §1.1 says a deliberate stop can do that a crash cannot.
            tauri::RunEvent::Exit => stop_node(handle),
            _ => {}
        }
    });
}

/// How often the supervisor re-decides which nodes should be running.
///
/// Idempotent, so this is cheap while nothing has changed — and it is the only
/// thing that wakes a network the member set aside, whose poll would otherwise
/// never come round.
const SUPERVISOR_TICK: std::time::Duration = std::time::Duration::from_secs(60);

/// How long to wait for the node to stop before leaving without it.
///
/// The loop it is in `select!`s on several timers and the swarm, so it reaches a
/// cancellation point within a tick under any normal load. This bound exists for
/// the case where it does not — closing a window must never be the thing that
/// hangs, and everything below is recoverable by expiry anyway.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_millis(1500);

/// How long a departure is given to reach the node before the store is deleted.
///
/// The node adopts what the store gained on a two-second refresh, so this is
/// that plus room for a slow tick. A wait rather than a handshake because there
/// is nothing to shake hands with: gossip acknowledges nothing, and the honest
/// report is how many peers were connected rather than how many received it.
const ANNOUNCE_GRACE: std::time::Duration = std::time::Duration::from_millis(5000);

/// Stops the node before the process goes.
///
/// # Why this is worth doing rather than letting the process exit
///
/// It is not about corruption — `Store` writes atomically, so a process that
/// vanishes mid-write leaves the previous contents rather than half of the new
/// ones, and that has to hold whatever this function does: a power cut and a
/// force-quit are not going to call it.
///
/// It is about the two things that are held rather than written. **The node
/// claim** is released on drop and otherwise expires on a six-second timer, so
/// a process that just ends makes the next launch sit and wait for a claim
/// nobody holds — the app opens, and the node behind it does not start for
/// several seconds. **The relay reservation** is likewise a slot on somebody
/// else's machine, held until it times out.
///
/// Aborting the task drops the future, and dropping the future drops what it
/// owns — the claim among it. So this waits: an abort that is never polled has
/// dropped nothing, and returning immediately would leave exactly the state this
/// exists to avoid.
fn stop_node(handle: &tauri::AppHandle) {
    // **The application claim goes first, and before anything that can block.**
    // `handle.exit` does not drop a running task, so a claim left to its
    // staleness window would make a relaunch within six seconds exit as a
    // second instance with nobody to raise — an application that appears not to
    // start. Dropping it here removes the files (`AppClaim::drop`), and doing
    // it before the nodes are awaited means a slow shutdown does not hold the
    // next launch hostage.
    if let Ok(mut held) = handle.state::<App>().claim.lock() {
        drop(held.take());
    }
    // **Every node, not one.** The reasoning above is unchanged and now applies
    // several times over: a member closing the window may be holding a dozen
    // claims and a dozen relay reservations, and leaving each to expire is a
    // dozen networks that cannot be reopened for six seconds and a dozen slots
    // held on other people's machines.
    let app = handle.state::<App>();
    let stopping = {
        let Ok(mut nodes) = app.nodes.lock() else {
            return;
        };
        nodes.stop_all()
    };
    if stopping.is_empty() {
        return;
    }
    // **Awaited, not merely requested**, for the reason above: abort asks a task
    // to stop and the claim goes when the future is dropped. Bounded, and the
    // failure mode of the bound being hit is what the six-second expiry covers.
    //
    // One grace for the whole set rather than one apiece — closing a window must
    // not take a dozen timeouts — so they are awaited together.
    let _ = tauri::async_runtime::block_on(async {
        tokio::time::timeout(SHUTDOWN_GRACE, async {
            for task in stopping {
                let _ = task.await;
            }
        })
        .await
    });
}
