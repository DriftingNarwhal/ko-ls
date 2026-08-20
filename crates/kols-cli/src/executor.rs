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

        self.run(authorized, &identity, &state, &index)
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
    ) -> Result<Outcome, ExecuteError> {
        match authorized.into_command() {
            Command::OpenChannel { channel, .. } => self.open_channel(channel, state, index),

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
            ),

            Command::EditMessage {
                channel,
                target,
                body,
            } => {
                self.require_own_message(&channel, &target, &identity.id())?;
                self.write(channel, RecordBody::Edit { target, body }, identity, state)
            }

            Command::DeleteMessage { channel, target } => {
                self.require_own_message(&channel, &target, &identity.id())?;
                self.write(channel, RecordBody::Tombstone { target }, identity, state)
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
            ),

            Command::Pin {
                channel,
                target,
                remove,
            } => self.write(channel, RecordBody::Pin { target, remove }, identity, state),

            Command::CreateChannel {
                name,
                category,
                privacy,
                topic,
            } => self.create_channel(name, category, privacy, topic, identity, state),

            Command::UpdateChannel { channel, change } => {
                self.update_channel(channel, change, identity, state)
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

            Command::AdmitMember { identity: who } => {
                self.change_membership(who, true, identity)
            }

            Command::RevokeMember { identity: who } => {
                self.change_membership(who, false, identity)
            }
        }
    }

    fn open_channel(
        &self,
        channel: ChannelId,
        state: &intranet_governance::GovernanceState,
        index: &PlacementMap,
    ) -> Result<Outcome, ExecuteError> {
        let placement = index
            .get(&channel)
            .copied()
            .unwrap_or(Placement { channel, category: None });
        let mut view = ChannelView::new(placement);
        let authority = StateAuthority { state };

        let records = self.store.records(&channel)?;
        let authors: std::collections::BTreeSet<_> =
            records.iter().map(|record| record.author).collect();
        view.admit(records, &authority);

        Ok(Outcome::Opened {
            channel,
            messages: view.render(),
            rejected: view
                .rejected()
                .iter()
                .map(|(id, rejection)| (*id, *rejection))
                .collect(),
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
    ) -> Result<Outcome, ExecuteError> {
        let mut log = rebuild_log(&self.store, identity, channel, state)
            .map_err(ExecuteError::Rejected)?;
        let hlc = next_hlc(&log, now_millis());
        self.require_within_rate(&channel, &body, hlc, identity, state)?;
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

    fn change_membership(
        &self,
        target: PerNetworkIdentityId,
        admit: bool,
        identity: &intranet_identity::PerNetworkIdentity,
    ) -> Result<Outcome, ExecuteError> {
        if !admit && target == identity.id() {
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
                group: GroupId::everyone(),
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
        let after = self.store.state()?;
        if after.is_member(&target) != admit {
            return Err(ExecuteError::Rejected(
                "the entry was written but replay did not apply it".to_owned(),
            ));
        }

        Ok(Outcome::MembershipChanged {
            identity: target,
            admitted: admit,
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
                "that message is somebody else's. You can only revise or withdraw your own —                  a moderator hides another member's with a redaction instead"
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
    ) -> Result<(), ExecuteError> {
        let policy = kols_core::ChatPolicy::of(&state.policy);
        let class = body.class();
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
                "you're going too fast — this network allows {ceiling} of these a minute,                  and you have written {recent} in the last one"
            )));
        }
        Ok(())
    }

    /// Who holds which display name, folded out of the canonical chain.
    ///
    /// Rebuilt per submit rather than cached, for the same reason the CLI
    /// replays the governance log on every invocation: a cache is a second
    /// answer to a question replay already answers, and the two disagree
    /// exactly when it matters. `STATUS` §6's projection is where this stops
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
