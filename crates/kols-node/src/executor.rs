//! The one thing that runs a command — `design/05` §3.
//!
//! # Why this exists as its own layer
//!
//! `kols-api` answers whether a command may proceed and hands back an
//! `Authorized`. Something then has to *do* it, and for a while that was each
//! caller in turn: authorize, take the value apart, reach for the store. That
//! works and it scatters the interesting part — a second caller would copy the
//! sequence, and a third would copy it slightly differently.
//!
//! So there is one submit path. [`Executor::submit`] takes a [`Command`],
//! authorizes it against replayed state, and runs it. The `Authorized` never
//! leaves this module: [`Executor::run`] is what requires one, and nothing else
//! can produce one, so the check is not something a future caller can be
//! *asked* to remember.
//!
//! # It returns values, and prints nothing
//!
//! An executor that printed would be one no interface could reuse, which is the
//! whole reason the boundary exists. Every command produces an
//! [`Outcome`]; `main.rs` renders those to a terminal and a webview would render
//! the same values differently.
//!
//! # There is no trait here yet
//!
//! A `trait Executor` with one implementation is a claim about a second one. The
//! Tauri shell will want the same surface, and that is when the shape is known
//! rather than guessed.

use crate::chat::{next_hlc, now_millis, rebuild_log};
use crate::network;

/// Replayed channel state, as `network::channels` returns it.
type ChannelMap = std::collections::BTreeMap<ChannelId, network::Channel>;

/// The bounds a reader applies to one channel — spec 07 §4.3, §2.6.
///
/// Two sources, joined here because they are enforced together: the network's
/// policy carries the ceilings, and the channel's own definition carries the
/// slowmode a manager set (`design/01` §10.3). A channel replay does not know
/// about is read under the network's ceilings with slowmode off, which is the
/// safe direction — a channel this node cannot see the definition of should not
/// have records refused for a slowmode it is guessing at.
fn reader_limits(
    state: &intranet_governance::GovernanceState,
    channels: &ChannelMap,
    channel: &ChannelId,
) -> kols_core::ReaderLimits {
    let slowmode = channels.get(channel).map_or(0, |c| c.slowmode);
    kols_core::ReaderLimits::of(&kols_core::ChatPolicy::of(&state.policy), slowmode)
}
use crate::store::{Store, StoreError};
use intranet_crypto::Timestamp;
use intranet_governance::{EntryBody, GroupId, LogEntry, MembershipAction};
use intranet_identity::PerNetworkIdentityId;
use kols_api::{Actor, Authorized, Command, Outcome, PlacementMap, Refusal, authorize, placement};
use kols_core::{
    ChannelEntry, ChannelEntryBody, ChannelId, ChannelKind, ChannelView, MessageId, NameClaim,
    Names, Placement, Record, RecordBody, StateAuthority,
};
use std::path::PathBuf;

/// What can go wrong running a command that was allowed to run.
///
/// Separate from [`Refusal`], which is the boundary saying no. A refusal is an
/// answer; these are failures.
#[derive(Debug)]
pub enum ExecuteError {
    /// The boundary refused the command.
    Refused(Refusal),
    /// The store could not be read or written.
    Store(StoreError),
    /// A record or entry was built but the log or the log's rules rejected it.
    Rejected(String),
    /// Nothing matched what the caller named.
    NotFound(String),
}

impl std::fmt::Display for ExecuteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(refusal) => write!(f, "{refusal}"),
            Self::Store(err) => write!(f, "{err}"),
            Self::Rejected(why) => write!(f, "{why}"),
            Self::NotFound(what) => write!(f, "{what}"),
        }
    }
}

impl std::error::Error for ExecuteError {}

impl From<StoreError> for ExecuteError {
    fn from(err: StoreError) -> Self {
        Self::Store(err)
    }
}

impl From<Refusal> for ExecuteError {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}

/// Runs commands against one node's store.
pub struct Executor {
    store: Store,
}

impl Executor {
    /// Opens the store at `root`.
    pub fn open(root: PathBuf) -> Result<Self, ExecuteError> {
        Ok(Self {
            store: Store::open(root)?,
        })
    }

    /// Borrows the store, for the parts of `kols` that are not commands.
    ///
    /// `init`, `attach` and `whoami` are deliberately outside the command
    /// vocabulary: the first two create the state a command needs before there
    /// is any, and the third reads local state and asks the network nothing.
    pub const fn store(&self) -> &Store {
        &self.store
    }

    /// Authorizes a command and runs it.
    ///
    /// The one way in. Permission is resolved by replaying the governance log,
    /// never from anything the caller supplied — including the channel's
    /// category, which decides which grant applies and therefore comes from
    /// replay rather than from the command.
    pub fn submit(&self, command: Command) -> Result<Outcome, ExecuteError> {
        let identity = self.store.identity()?;
        let state = self.store.state()?;
        let (channels, _) = network::channels(&self.store, &state)?;
        let index: PlacementMap = channels
            .values()
            .map(|channel| (channel.id, placement(channel.id, channel.category)))
            .collect();
        let authority = StateAuthority { state: &state };
        let names = self.names(&state)?;

        let authorized = authorize(
            command,
            &Actor {
                identity: identity.id(),
                authority: &authority,
                state: &state,
                channels: &index,
                names: &names,
            },
        )?;

        self.run(authorized, &identity, &state, &index, &channels)
    }

    /// Runs a command that has been through the gate.
    ///
    /// Private, and takes an [`Authorized`] rather than a [`Command`]. Both
    /// together are what make the check unavoidable rather than remembered:
    /// there is no other way to reach this, and no way to build its argument.
    fn run(
        &self,
        authorized: Authorized,
        identity: &intranet_identity::PerNetworkIdentity,
        state: &intranet_governance::GovernanceState,
        index: &PlacementMap,
        channels: &ChannelMap,
    ) -> Result<Outcome, ExecuteError> {
        match authorized.into_command() {
            Command::OpenChannel { channel, .. } => {
                self.open_channel(channel, state, index, channels)
            }

            Command::SendMessage {
                channel,
                body,
                reply_to,
                attachments,
            } => self.write(
                channel,
                RecordBody::Message {
                    body,
                    reply_to,
                    attachments,
                },
                identity,
                state,
                channels,
            ),

            Command::EditMessage {
                channel,
                target,
                body,
            } => {
                self.require_own_message(&channel, &target, &identity.id())?;
                self.write(channel, RecordBody::Edit { target, body }, identity, state, channels)
            }

            Command::DeleteMessage { channel, target } => {
                self.require_own_message(&channel, &target, &identity.id())?;
                self.write(channel, RecordBody::Tombstone { target }, identity, state, channels)
            }

            Command::React {
                channel,
                target,
                key,
                remove,
            } => self.write(
                channel,
                RecordBody::Reaction {
                    target,
                    key,
                    remove,
                },
                identity,
                state,
                channels,
            ),

            Command::Pin {
                channel,
                target,
                remove,
            } => self.write(channel, RecordBody::Pin { target, remove }, identity, state, channels),

            Command::CreateChannel {
                name,
                category,
                privacy,
                topic,
            } => self.create_channel(name, category, privacy, topic, identity, state),

            Command::UpdateChannel { channel, change } => {
                self.update_channel(channel, change, identity, state)
            }

            Command::CreateCategory { name, position } => {
                self.create_category(name, position, identity, state)
            }

            Command::UpdateCategory { category, change } => {
                self.update_category(category, change, identity, state)
            }

            Command::SetName { name } => {
                let claim = NameClaim::new(name)
                    .map_err(|refusal| ExecuteError::Rejected(refusal.to_string()))?;
                let display = claim.name.clone();

                // Held across reading the head and writing, like every other
                // append: a claim landing as a sibling of something the daemon
                // wrote would fork the log, and fork-choice would void one.
                let _lock = self.store.lock()?;
                let head = self.store.head()?;
                let entry = LogEntry::create(
                    identity,
                    head,
                    Timestamp::from_millis(now_millis()),
                    claim.to_app_entry(),
                );
                self.store.append_entry(&entry)?;

                // Replay rather than trust. Two members can claim one name
                // concurrently, and which one binds is the log's order rather
                // than who returned first — so success is what replay says.
                let state = self.store.state()?;
                let names = self.names(&state)?;
                if names.of(&identity.id()) != Some(display.as_str()) {
                    return Err(ExecuteError::Rejected(
                        "the claim was written but replay did not bind it — somebody else \
                         holds that name"
                            .to_owned(),
                    ));
                }
                Ok(Outcome::NameClaimed { name: display })
            }

            Command::CreateInvite {
                uses,
                valid_for_hours,
            } => {
                // An invite with no bootstrap address cannot establish a
                // connection, which is the one job it exists to do — so this
                // refuses rather than minting a credential that goes nowhere.
                // A network needs a relay before it can invite anybody, because
                // an invite's addresses are what a joiner dials and two people
                // behind NAT cannot dial each other (Core §5.5). Refused here
                // rather than producing a credential that works only for
                // somebody already on the same LAN.
                if state.policy.bootstrap_relays.is_empty() {
                    return Err(ExecuteError::Rejected(
                        "this network designates no relay, so an invite would only reach \
                         somebody who can already dial this machine. Run one with \
                         `intranet-harness relay`, or deploy DI-Relay, then \
                         `kols relay set <its address>`"
                            .to_owned(),
                    ));
                }

                let addresses = self.store.addresses();
                if addresses.is_empty() {
                    return Err(ExecuteError::Rejected(
                        "this node has never recorded an address to be reached on. \
                         Run `kols serve` once, which is also what makes the network \
                         reachable for whoever redeems this"
                            .to_owned(),
                    ));
                }

                // Said rather than silently shipped: without a circuit the
                // invite carries only addresses that work on this LAN, and the
                // failure lands on the joiner as a timeout they cannot diagnose.
                if !addresses.iter().any(|address| address.contains("p2p-circuit")) {
                    return Err(ExecuteError::Rejected(
                        "this node holds no relay circuit, so an invite would carry only \
                         addresses reachable from this network. Check `kols serve`'s output \
                         for whether it reserved one"
                            .to_owned(),
                    ));
                }

                let issued = now_millis();
                let expires = issued.saturating_add(valid_for_hours.saturating_mul(3_600_000));
                let invite = intranet_invite::Invite::issue(
                    identity,
                    addresses,
                    // Bearer rather than a named identity: whoever is being
                    // invited does not have a per-network identity yet, since
                    // it is derived from the network id they are about to learn.
                    intranet_invite::InviteSubject::Bearer,
                    Timestamp::from_millis(issued),
                    Timestamp::from_millis(expires),
                    uses,
                );

                Ok(Outcome::InviteCreated {
                    invite: intranet_invite::encode_invite(&invite),
                    expires_at_millis: expires,
                    uses,
                })
            }

            Command::SetBootstrapRelays { relays } => {
                let mut policy = state.policy.clone();
                policy.bootstrap_relays = relays.clone();

                let _lock = self.store.lock()?;
                let head = self
                    .store
                    .head()?
                    .ok_or_else(|| ExecuteError::Rejected("this network has no genesis".to_owned()))?;
                let entry = LogEntry::create(
                    identity,
                    Some(head),
                    Timestamp::from_millis(now_millis()),
                    EntryBody::PolicyChange { policy },
                );
                self.store.append_entry(&entry)?;

                // Replay rather than trust: a policy change the log accepts
                // structurally is still refused by replay if the author did not
                // hold `define-policy`.
                let after = self.store.state()?;
                if after.policy.bootstrap_relays != relays {
                    return Err(ExecuteError::Rejected(
                        "the change was written but replay did not apply it".to_owned(),
                    ));
                }
                // Cached immediately, because reading policy needs a synced log
                // and syncing needs a connection — which is what these are for.
                self.store.set_relays(&relays)?;

                Ok(Outcome::BootstrapRelaysSet { relays })
            }

            Command::AdmitMember { identity: who } => {
                self.change_membership(who, true, identity, GroupId::everyone())
            }

            Command::RevokeMember { identity: who } => {
                self.change_membership(who, false, identity, GroupId::everyone())
            }

            Command::SetNetworkName { name } => self.set_network_name(name, identity, state),

            Command::SetChatSetting { setting, value } => {
                self.set_chat_setting(setting, value, identity)
            }

            Command::SetAdmissionMode { mode } => self.set_admission_mode(mode, identity),

            Command::CreateRole { group } => self.create_role(group, identity),

            Command::SetPermission {
                group,
                verb,
                scope,
                grant,
            } => self.set_permission(group, &verb, scope, grant, identity),

            Command::SetRoleMember {
                group,
                identity: who,
                member,
            } => self.change_membership(who, member, identity, group),
        }
    }

    /// Writes this network's name into app-layer policy — D32, spec 07 §1.7.
    fn set_network_name(
        &self,
        name: String,
        identity: &intranet_identity::PerNetworkIdentity,
        state: &intranet_governance::GovernanceState,
    ) -> Result<Outcome, ExecuteError> {
        let mut policy = state.policy.clone();
        let trimmed = name.trim().to_owned();
        if trimmed.is_empty() {
            // Removed rather than stored empty. Spec 07 §1.7: a network with no
            // name declared *has* no name, and an empty string sitting in policy
            // is a declaration that it is called nothing — a different claim,
            // and one every joiner would replay forever.
            policy.app_policy.remove(kols_core::keys::NETWORK_NAME);
        } else {
            policy.app_policy.insert(
                kols_core::keys::NETWORK_NAME.to_owned(),
                intranet_governance::PolicyValue::Text(trimmed.clone()),
            );
        }

        self.append_policy(policy, identity)?;

        let after = self.store.state()?;
        let bound = kols_core::ChatPolicy::of(&after.policy)
            .network_name()
            .unwrap_or_default()
            .to_owned();
        if bound != trimmed {
            return Err(ExecuteError::Rejected(
                "the change was written but replay did not apply it".to_owned(),
            ));
        }
        Ok(Outcome::NetworkNamed { name: trimmed })
    }

    /// Writes one chat setting into app-layer policy — spec 07 §4.3, §2.8.
    ///
    /// The lock is held across the read as well as the write, for the reason
    /// [`Executor::set_permission`] holds it: `PolicyChange` carries the whole
    /// record, so building the new one from state replayed before the lock would
    /// let two concurrent holders each write a policy derived from what they
    /// saw, and the second would silently revert the first's setting. The verify
    /// below cannot catch that — it asks about the key this call changed, which
    /// is the one that survived.
    fn set_chat_setting(
        &self,
        setting: kols_core::ChatSetting,
        value: i64,
        identity: &intranet_identity::PerNetworkIdentity,
    ) -> Result<Outcome, ExecuteError> {
        let _lock = self.store.lock()?;
        let state = self.store.state()?;

        let key = setting.key();
        // The default is the same thing as absence (Core §2.6.2), so writing it
        // explicitly would freeze today's number into a network that would
        // otherwise pick up a revised one. Removing the key is the honest way to
        // say "whatever this application ships".
        let mut policy = state.policy.clone();
        let already = policy
            .app_policy_int(key, setting.default_value());
        if value == setting.default_value() {
            policy.app_policy.remove(key);
        } else {
            policy
                .app_policy
                .insert(key.to_owned(), intranet_governance::PolicyValue::Int(value));
        }

        // A no-op entry is not free: every governance entry is replayed by every
        // joiner forever. The same reasoning `set_permission` and `move_channel`
        // apply.
        if already == value && policy.app_policy == state.policy.app_policy {
            return Ok(Outcome::ChatSettingSet { key, value });
        }

        self.write_policy(policy, identity)?;

        let after = self.store.state()?;
        if after.policy.app_policy_int(key, setting.default_value()) != value {
            return Err(ExecuteError::Rejected(
                "the change was written but replay did not apply it".to_owned(),
            ));
        }
        Ok(Outcome::ChatSettingSet { key, value })
    }

    /// Chooses how joiners are admitted — Core §2.4.
    fn set_admission_mode(
        &self,
        mode: intranet_governance::AdmissionMode,
        identity: &intranet_identity::PerNetworkIdentity,
    ) -> Result<Outcome, ExecuteError> {
        let _lock = self.store.lock()?;
        let state = self.store.state()?;
        if state.policy.admission_mode == mode {
            return Ok(Outcome::AdmissionModeSet { mode });
        }

        let mut policy = state.policy.clone();
        policy.admission_mode = mode;
        self.write_policy(policy, identity)?;

        // Replay rather than trust, and here it is load-bearing rather than
        // belt-and-braces: the protocol refuses an incoherent pairing on replay
        // (Core §2.6), so a change the gate let through can still be dropped —
        // and reporting success would tell somebody their network now admits
        // automatically when it does not.
        let after = self.store.state()?;
        if after.policy.admission_mode != mode {
            return Err(ExecuteError::Rejected(
                "the change was written but replay did not apply it — this network's \
                 governance model does not permit that admission mode"
                    .to_owned(),
            ));
        }
        Ok(Outcome::AdmissionModeSet { mode })
    }

    /// Creates a role holding nothing — `design/02` §1.
    fn create_role(
        &self,
        group: GroupId,
        identity: &intranet_identity::PerNetworkIdentity,
    ) -> Result<Outcome, ExecuteError> {
        self.append_group(
            group.clone(),
            intranet_governance::CapabilitySet::explicit([]),
            identity,
        )?;

        let after = self.store.state()?;
        if !after.groups.contains_key(&group) {
            return Err(ExecuteError::Rejected(
                "the entry was written but replay did not produce the role".to_owned(),
            ));
        }
        Ok(Outcome::RoleCreated { group })
    }

    /// Grants or withdraws one verb at one scope — `design/05` §3's `SetPermission`.
    ///
    /// # Read, modify, write, and why the window is held shut
    ///
    /// `DefineGroup` carries a whole capability set, so changing one grant means
    /// reading the current set and writing it back with one member added or
    /// removed. Two managers doing that concurrently would each write a set
    /// built from what they read, and the one that lands second silently
    /// reverts the other's change — the log would not fork, and nothing would
    /// report a lost grant.
    ///
    /// The store's append lock is what closes that window on this node, and the
    /// verify-by-replay below is what catches the case it cannot: a manager on
    /// another node writing between this read and this append. It cannot repair
    /// that — nothing here can — but it refuses to report success for it.
    fn set_permission(
        &self,
        group: GroupId,
        verb: &str,
        scope: kols_core::Scope,
        grant: bool,
        identity: &intranet_identity::PerNetworkIdentity,
    ) -> Result<Outcome, ExecuteError> {
        let capability = scope.capability(verb);

        // **Taken before the set is read, not merely before it is written.** The
        // state this was authorized against was replayed at the top of `submit`,
        // and building the new set from *that* would make this a read-modify-write
        // straddling the lock: two managers on this node would each write a set
        // built from what they read, the second would land, and the first's grant
        // would be gone with nothing reporting it. The verify below cannot catch
        // that either — it asks about the capability this call changed, which is
        // exactly the one that survived.
        //
        // So the read moves inside. Held across read and append, the pair is
        // atomic on this node, and what remains is the genuinely distributed
        // case: a manager on another node writing in the same window. Nothing
        // here can repair that, and the log records both entries in an order
        // every reader agrees on.
        let _lock = self.store.lock()?;
        let state = self.store.state()?;

        let current = state
            .groups
            .get(&group)
            .ok_or_else(|| ExecuteError::NotFound(format!("no role called {group:?} here")))?;

        let mut set = match &current.capabilities {
            intranet_governance::CapabilitySet::Explicit(held) => held.clone(),
            // Refused at the gate already; refused again rather than silently
            // replacing `All` with whatever this happens to enumerate.
            intranet_governance::CapabilitySet::All => {
                return Err(ExecuteError::Rejected(format!(
                    "{group} holds every capability, so there is no set to edit"
                )));
            }
        };

        let changed = if grant {
            set.insert(capability.clone())
        } else {
            set.remove(&capability)
        };
        // A no-op entry is not free: every governance entry is replayed by every
        // joiner forever, so writing one that changes nothing spends everybody's
        // replay to record that somebody clicked a checkbox that was already in
        // that position. The same reasoning `move_channel` applies to a channel
        // that did not move.
        if !changed {
            return Ok(Outcome::PermissionSet {
                group,
                capability: scope.name(verb),
                granted: grant,
            });
        }

        self.write_group(
            group.clone(),
            intranet_governance::CapabilitySet::Explicit(set),
            identity,
        )?;

        // Replay rather than trust. An entry the log accepts structurally is
        // still refused by replay if the author did not hold `define-group`, and
        // — the case this is really for — Core §2.4's `everyone` ceiling is
        // enforced by the protocol, so a governance-tier grant that slipped past
        // the gate is dropped here rather than reported as applied.
        let after = self.store.state()?;
        let holds = after
            .groups
            .get(&group)
            .is_some_and(|found| found.capabilities.grants(&capability));
        if holds != grant {
            return Err(ExecuteError::Rejected(
                "the entry was written but replay did not apply it — the network refused \
                 that grant"
                    .to_owned(),
            ));
        }

        Ok(Outcome::PermissionSet {
            group,
            capability: scope.name(verb),
            granted: grant,
        })
    }

    /// Appends a `DefineGroup` entry, taking the store's append lock.
    fn append_group(
        &self,
        group: GroupId,
        capabilities: intranet_governance::CapabilitySet,
        identity: &intranet_identity::PerNetworkIdentity,
    ) -> Result<(), ExecuteError> {
        let _lock = self.store.lock()?;
        self.write_group(group, capabilities, identity)
    }

    /// The append itself, for a caller already holding the lock.
    ///
    /// Split out because [`Executor::set_permission`] has to hold the lock
    /// across its own read as well as its write, and taking it twice would
    /// deadlock or — worse, depending on the lock — quietly not.
    fn write_group(
        &self,
        group: GroupId,
        capabilities: intranet_governance::CapabilitySet,
        identity: &intranet_identity::PerNetworkIdentity,
    ) -> Result<(), ExecuteError> {
        let head = self
            .store
            .head()?
            .ok_or_else(|| ExecuteError::Rejected("this network has no genesis".to_owned()))?;
        let entry = LogEntry::create(
            identity,
            Some(head),
            Timestamp::from_millis(now_millis()),
            EntryBody::DefineGroup {
                group,
                capabilities,
            },
        );
        self.store.append_entry(&entry)?;
        Ok(())
    }

    /// Appends a `PolicyChange` entry, taking the store's append lock.
    fn append_policy(
        &self,
        policy: intranet_governance::NetworkPolicy,
        identity: &intranet_identity::PerNetworkIdentity,
    ) -> Result<(), ExecuteError> {
        let _lock = self.store.lock()?;
        self.write_policy(policy, identity)
    }

    /// The append itself, for a caller already holding the lock.
    ///
    /// Split for the reason `write_group` is: a policy edit derived from what it
    /// read has to hold the lock across both, and taking it twice would deadlock
    /// or — depending on the lock — quietly not.
    fn write_policy(
        &self,
        policy: intranet_governance::NetworkPolicy,
        identity: &intranet_identity::PerNetworkIdentity,
    ) -> Result<(), ExecuteError> {
        let head = self
            .store
            .head()?
            .ok_or_else(|| ExecuteError::Rejected("this network has no genesis".to_owned()))?;
        let entry = LogEntry::create(
            identity,
            Some(head),
            Timestamp::from_millis(now_millis()),
            EntryBody::PolicyChange { policy },
        );
        self.store.append_entry(&entry)?;
        Ok(())
    }

    fn open_channel(
        &self,
        channel: ChannelId,
        state: &intranet_governance::GovernanceState,
        index: &PlacementMap,
        channels: &ChannelMap,
    ) -> Result<Outcome, ExecuteError> {
        let placement = index
            .get(&channel)
            .copied()
            .unwrap_or(Placement { channel, category: None });
        let mut view = ChannelView::new(placement);
        let authority = StateAuthority { state };
        let limits = reader_limits(state, channels, &channel);

        let records = self.store.records(&channel)?;
        let authors: std::collections::BTreeSet<_> =
            records.iter().map(|record| record.author).collect();
        view.admit(records, &authority, &limits);

        // Both halves of the refusal set, and they arrive from different places
        // on purpose. `rejected` holds what failed a check about *one* record —
        // a bad signature, a non-member, an oversized body. `withheld.refused`
        // holds what failed a rule about the whole set, which cannot be decided
        // as records land without making the verdict depend on arrival order.
        //
        // Held records are deliberately **not** in here. They are dated ahead of
        // this node's clock and will render on their own within a few minutes
        // (§2.6), and reporting them as refusals would be the interface saying
        // something it knows to be untrue.
        let at = now_millis();
        let withheld = view.withheld(&limits, at);
        let mut rejected: Vec<_> = view
            .rejected()
            .iter()
            .map(|(id, rejection)| (*id, *rejection))
            .collect();
        rejected.extend(withheld.refused.iter().map(|(id, why)| (*id, *why)));

        Ok(Outcome::Opened {
            channel,
            messages: view.render(&limits, at),
            rejected,
            authors: authors.len(),
        })
    }

    /// Appends one signed record to this member's log for a channel.
    ///
    /// Every record-producing command lands here, because at this layer they are
    /// the same act: a record this member signed, in the one log they can write.
    fn write(
        &self,
        channel: ChannelId,
        body: RecordBody,
        identity: &intranet_identity::PerNetworkIdentity,
        state: &intranet_governance::GovernanceState,
        channels: &ChannelMap,
    ) -> Result<Outcome, ExecuteError> {
        let mut log = rebuild_log(&self.store, identity, channel, state)
            .map_err(ExecuteError::Rejected)?;
        let hlc = next_hlc(&log, now_millis());
        self.require_within_rate(&channel, &body, hlc, identity, state, channels)?;
        let record = Record::create(identity, channel, hlc, body);
        let id = record.id();
        let stored = record.clone();

        let published = log
            .append(identity, record, state)
            .map_err(|err| ExecuteError::Rejected(format!("the record was refused: {err}")))?;
        self.store.put_record(&channel, &stored)?;

        Ok(Outcome::Wrote {
            record: id,
            moved: published.new_bytes(),
            total: published.total_bytes(),
        })
    }

    fn create_channel(
        &self,
        name: String,
        category: Option<kols_core::CategoryId>,
        privacy: kols_core::Privacy,
        topic: String,
        identity: &intranet_identity::PerNetworkIdentity,
        state: &intranet_governance::GovernanceState,
    ) -> Result<Outcome, ExecuteError> {
        // The nonce is what makes two channels of the same name distinct objects
        // rather than one; the id derives from it and the network id, so nobody
        // can mint an id that collides with an existing channel's.
        let nonce = crate::random_32().map_err(ExecuteError::Rejected)?;
        let channel = kols_core::server_channel_id(self.store.network(), &nonce);

        let entry = ChannelEntry::new(
            channel,
            ChannelEntryBody::Definition {
                name: name.clone(),
                category,
                kind: ChannelKind::Text,
                privacy,
                topic,
                slowmode: 0,
            },
        );
        self.append_channel_entry(&entry, category.as_ref(), identity, state)?;

        // Replay before claiming success: an entry the log accepts structurally
        // can still be refused by replay, and reporting a channel that does not
        // exist would be worse than failing.
        let state = self.store.state()?;
        let (channels, _) = network::channels(&self.store, &state)?;
        if !channels.contains_key(&channel) {
            return Err(ExecuteError::Rejected(
                "the entry was written but replay did not produce the channel".to_owned(),
            ));
        }

        Ok(Outcome::ChannelCreated {
            channel,
            name,
            privacy,
        })
    }

    fn create_category(
        &self,
        name: String,
        position: u32,
        identity: &intranet_identity::PerNetworkIdentity,
        state: &intranet_governance::GovernanceState,
    ) -> Result<Outcome, ExecuteError> {
        // Same shape as a channel's id and separated only by its domain tag,
        // which is what keeps a category-scoped grant from resolving against a
        // channel derived over the same inputs (spec 07 §3.2).
        let nonce = crate::random_32().map_err(ExecuteError::Rejected)?;
        let category = kols_core::category_id(self.store.network(), &nonce);

        let entry = ChannelEntry::new(
            category,
            ChannelEntryBody::CategoryDefinition {
                name: name.clone(),
                position,
            },
        );
        // No enclosing category to pass: nothing encloses a category, and a
        // definition is authorized network-wide or not at all (spec 07 §1.8).
        self.append_channel_entry(&entry, None, identity, state)?;

        let state = self.store.state()?;
        let (categories, _) = network::categories(&self.store, &state)?;
        if !categories.contains_key(&category) {
            return Err(ExecuteError::Rejected(
                "the entry was written but replay did not produce the category".to_owned(),
            ));
        }

        Ok(Outcome::CategoryCreated { category, name })
    }

    fn update_category(
        &self,
        category: kols_core::CategoryId,
        change: kols_core::CategoryChange,
        identity: &intranet_identity::PerNetworkIdentity,
        state: &intranet_governance::GovernanceState,
    ) -> Result<Outcome, ExecuteError> {
        let entry = ChannelEntry::new(category, ChannelEntryBody::CategoryUpdate { change });
        // A category's own scope is the one that can authorize an update, so it
        // is passed as the placement rather than looked up: unlike a channel,
        // there is nowhere else it could sit.
        self.append_channel_entry(&entry, Some(&category), identity, state)?;
        Ok(Outcome::CategoryUpdated { category })
    }

    fn update_channel(
        &self,
        channel: ChannelId,
        change: kols_core::ChannelChange,
        identity: &intranet_identity::PerNetworkIdentity,
        state: &intranet_governance::GovernanceState,
    ) -> Result<Outcome, ExecuteError> {
        // Where replay says this channel sits, since a category grant authorizes
        // an entry against a channel inside it and the entry does not restate
        // where the channel lives.
        let (channels, _) = network::channels(&self.store, state)?;
        let category = channels.get(&channel).and_then(|found| found.category);

        let entry = ChannelEntry::new(channel, ChannelEntryBody::Update { change });
        self.append_channel_entry(&entry, category.as_ref(), identity, state)?;
        Ok(Outcome::ChannelUpdated { channel })
    }

    /// Writes a channel entry, declaring whatever its author actually holds.
    fn append_channel_entry(
        &self,
        entry: &ChannelEntry,
        category: Option<&kols_core::CategoryId>,
        identity: &intranet_identity::PerNetworkIdentity,
        state: &intranet_governance::GovernanceState,
    ) -> Result<(), ExecuteError> {
        // The declaration is chosen from what the author holds rather than from
        // what best describes the action, because the protocol verifies the
        // author holds exactly what the entry declared.
        let body = entry
            .to_app_entry(state, &identity.id(), category)
            .map_err(|refusal| ExecuteError::Rejected(refusal.to_string()))?;

        // Held across reading the head and writing, so this cannot land as a
        // sibling of something the daemon appended in between — which forks the
        // log, and fork-choice then voids one of them.
        let _lock = self.store.lock()?;
        let head = self.store.head()?;
        let log_entry = LogEntry::create(identity, head, Timestamp::from_millis(now_millis()), body);
        self.store.append_entry(&log_entry)?;
        Ok(())
    }

    /// Adds or removes an identity from one group.
    ///
    /// `group` rather than always `everyone`, since a role is a group like any
    /// other (`design/02` §1) and assigning one is the same act as admitting
    /// somebody to the network — the difference is entirely which group and
    /// therefore which `manage-membership` the gate asked for.
    fn change_membership(
        &self,
        target: PerNetworkIdentityId,
        admit: bool,
        identity: &intranet_identity::PerNetworkIdentity,
        group: GroupId,
    ) -> Result<Outcome, ExecuteError> {
        // Only for network membership. Taking yourself out of a *role* is
        // ordinary — stepping down from Moderators is a thing people do, and
        // `design/02` §5 says plainly there is no hierarchy protecting anyone —
        // whereas leaving `everyone` is leaving the network, which strands it.
        if !admit && target == identity.id() && group.is_everyone() {
            return Err(ExecuteError::Rejected(
                "that is you. Removing yourself would leave the network unmanaged \
                 by the only node that can rotate its key"
                    .to_owned(),
            ));
        }

        let _lock = self.store.lock()?;
        let head = self
            .store
            .head()?
            .ok_or_else(|| ExecuteError::Rejected("this network has no genesis".to_owned()))?;
        let entry = LogEntry::create(
            identity,
            Some(head),
            Timestamp::from_millis(now_millis()),
            EntryBody::MembershipChange {
                group: group.clone(),
                identity: target,
                action: if admit {
                    MembershipAction::Add { via_invite: None }
                } else {
                    // Non-cascading, the protocol's default and the right one:
                    // anyone this member admitted was validly admitted at the
                    // time, and cascading is a visible choice rather than
                    // something a command does on your behalf (Core §2.5).
                    MembershipAction::Remove { cascade: None }
                },
            },
        );
        self.store.append_entry(&entry)?;

        // Replay rather than trust. An entry the log accepts structurally is
        // still refused by replay if the actor did not hold the capability, and
        // reporting success there would tell somebody they had admitted a person
        // who is not a member.
        //
        // Asked of the *group* rather than of network membership, because those
        // are different questions once roles exist: somebody removed from
        // Moderators is still a member, so `is_member` would report the removal
        // as having failed when it landed exactly as asked.
        let after = self.store.state()?;
        let in_group = after
            .groups
            .get(&group)
            .is_some_and(|found| found.contains(&target));
        if in_group != admit {
            return Err(ExecuteError::Rejected(
                "the entry was written but replay did not apply it".to_owned(),
            ));
        }

        Ok(Outcome::MembershipChanged {
            identity: target,
            admitted: admit,
            group,
        })
    }

    /// Refuses an edit or withdrawal aimed at somebody else's message.
    ///
    /// The boundary cannot ask this — authorship is a fact about the record set
    /// rather than about replayed state, and `kols-api` reaches no store. The
    /// executor does, so the refusal happens before a record is signed rather
    /// than after every reader has ignored it.
    ///
    /// Note what this is *not*: enforcement. Nobody can write into another
    /// author's log, so an edit naming a stranger's message was always going to
    /// be discarded on read (`design/01` §6). This is the author's own client
    /// telling them, which is the same division `design/01` §10.2 draws for
    /// rate limits.
    fn require_own_message(
        &self,
        channel: &ChannelId,
        target: &MessageId,
        actor: &PerNetworkIdentityId,
    ) -> Result<(), ExecuteError> {
        let records = self.store.records(channel)?;
        let found = records.iter().find(|record| &record.id() == target);
        match found {
            Some(record) if &record.author == actor => Ok(()),
            Some(_) => Err(ExecuteError::Rejected(
                "that message is somebody else's. You can only revise or withdraw your own — \
                 a moderator hides another member's with a redaction instead"
                    .to_owned(),
            )),
            None => Err(ExecuteError::NotFound(
                "no message here with that id".to_owned(),
            )),
        }
    }

    /// Refuses a record that would exceed this network's rate ceiling.
    ///
    /// Computed over **the author's own HLC readings**, not arrival time, which
    /// is what makes it the same verdict on every node (`design/01` §10.2). The
    /// author's client enforcing it first is the point: a user typing too fast
    /// is told, rather than having their records silently refused by everybody
    /// else. Reader-side enforcement remains the backstop against a modified
    /// client, and is not this.
    fn require_within_rate(
        &self,
        channel: &ChannelId,
        body: &RecordBody,
        hlc: kols_core::Hlc,
        identity: &intranet_identity::PerNetworkIdentity,
        state: &intranet_governance::GovernanceState,
        channels: &ChannelMap,
    ) -> Result<(), ExecuteError> {
        let policy = kols_core::ChatPolicy::of(&state.policy);
        let class = body.class();

        // Slowmode first, since where it is set at all it is the stricter of the
        // two (`design/01` §10.3) — and it is the one somebody is more likely to
        // be surprised by, a channel having been calmed since they last posted.
        let slowmode = channels.get(channel).map_or(0, |c| c.slowmode);
        if class == kols_core::RecordClass::Message && slowmode > 0 {
            let interval = i64::from(slowmode).saturating_mul(1_000);
            let last = self
                .store
                .own_records(channel, &identity.id())?
                .iter()
                .filter(|record| record.body.class() == kols_core::RecordClass::Message)
                .map(|record| record.hlc.wall_millis)
                .max();
            if let Some(previous) = last
                && hlc.wall_millis.saturating_sub(previous) < interval
            {
                let wait = (interval - (hlc.wall_millis - previous)).div_euclid(1_000) + 1;
                return Err(ExecuteError::Rejected(format!(
                    "this channel is in slowmode at {slowmode}s — about {wait}s to go"
                )));
            }
        }
        let ceiling = match class {
            kols_core::RecordClass::Message => policy.message_rate_per_minute(),
            kols_core::RecordClass::Reaction => policy.reaction_rate_per_minute(),
            // Control records are governed by capability instead, and a reserved
            // one never reaches here — the encoder refuses it.
            kols_core::RecordClass::Control | kols_core::RecordClass::Reserved => return Ok(()),
        };
        if ceiling <= 0 {
            return Ok(());
        }

        const WINDOW_MILLIS: i64 = 60_000;
        let since = hlc.wall_millis.saturating_sub(WINDOW_MILLIS);
        let recent = self
            .store
            .own_records(channel, &identity.id())?
            .iter()
            .filter(|record| record.body.class() == class && record.hlc.wall_millis > since)
            .count();

        if recent as i64 >= ceiling {
            return Err(ExecuteError::Rejected(format!(
                "you're going too fast — this network allows {ceiling} of these a minute, \
                 and you have written {recent} in the last one"
            )));
        }
        Ok(())
    }

    /// Who holds which display name, folded out of the canonical chain.
    ///
    /// Rebuilt per submit rather than cached, for the same reason the CLI
    /// replays the governance log on every invocation: a cache is a second
    /// answer to a question replay already answers, and the two disagree
    /// exactly when it matters. `design/05` §5's projection is where this stops
    /// being recomputed.
    pub fn names(&self, state: &intranet_governance::GovernanceState) -> Result<Names, ExecuteError> {
        let log = self.store.log()?;
        let entries: Vec<_> = log
            .canonical_chain()
            .iter()
            .filter_map(|hash| log.get(hash))
            .collect();
        Ok(kols_core::replay_names(entries, state))
    }

    /// Finds a channel by name, or by the leading hex of its id.
    pub fn resolve_channel(&self, needle: &str) -> Result<ChannelId, ExecuteError> {
        let state = self.store.state()?;
        let (channels, _) = network::channels(&self.store, &state)?;
        network::resolve(&channels, needle)
            .map(|channel| channel.id)
            .ok_or_else(|| {
                ExecuteError::NotFound(format!(
                    "no channel matching {needle:?}. `kols channel list`"
                ))
            })
    }

    /// Finds a message in a channel by the leading hex of its id.
    ///
    /// Ambiguity is an error rather than a first match: acting on the wrong
    /// message because two ids share a prefix is not a failure a user would
    /// notice until after it happened.
    pub fn resolve_message(
        &self,
        channel: &ChannelId,
        prefix: &str,
    ) -> Result<MessageId, ExecuteError> {
        let prefix = prefix.trim().to_ascii_lowercase();
        if prefix.is_empty() {
            return Err(ExecuteError::NotFound("name a message".to_owned()));
        }
        let matches: Vec<_> = self
            .store
            .records(channel)?
            .iter()
            .map(kols_core::Record::id)
            .filter(|id| intranet_crypto::to_hex(id.as_bytes()).starts_with(&prefix))
            .collect();

        match matches.as_slice() {
            [one] => Ok(*one),
            [] => Err(ExecuteError::NotFound(format!(
                "no message here starts with {prefix:?}"
            ))),
            many => Err(ExecuteError::NotFound(format!(
                "{} messages here start with {prefix:?} — give more of the id",
                many.len()
            ))),
        }
    }
}
