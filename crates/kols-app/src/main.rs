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
        network_name: kols_core::ChatPolicy::of(&state.policy)
            .network_name()
            .map(str::to_owned),
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

/// Removes this installation's store for a network — permanently.
///
/// **Forgetting, not leaving**, and the command is named for what it does.
/// Membership is governance state, so there is no resigning: the log every other
/// member replays is untouched, and to them nothing happened. What goes is this
/// machine's copy — and the seed with it, which *is* the identity, so a later
/// join arrives as a stranger rather than as the member the log already names.
///
/// Refuses the network that is currently open rather than quietly closing it: a
/// node is running for that one, and removing live MLS state out from under a
/// running node is how key material goes missing with no step reporting it.
#[tauri::command]
fn forget_network(app: tauri::State<'_, App>, network: String) -> Result<(), String> {
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
    if open_here {
        return Err(
            "that network is open. Switch to another one first — its node is running, and              removing the store under it is how a key group goes missing quietly"
                .to_owned(),
        );
    }

    app.workspace.forget(&id)
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
            &kols_node::serve::Output {
                events: &sink,
                // Nowhere, and deliberately. A GUI-subsystem binary has no
                // console, so a `println!` here would not be ignored — Rust
                // panics on the write error — and the window would crash on the
                // node's first line of output. What this window actually needs
                // from those lines reaches it as events instead.
                report: &kols_node::quiet(),
            },
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
            forget_network
        ])
        .run(tauri::generate_context!())
        .expect("the window opens");
}
