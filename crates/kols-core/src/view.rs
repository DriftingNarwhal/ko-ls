//! Merging author logs into one channel history — `design/01` §4, §5.
//!
//! # The correctness claim of the whole design
//!
//! Records arrive in any order, from any number of logs, over two paths (live
//! gossip and durable fetch) that make no promises about sequence. What every
//! member must nonetheless see is the *same* history. That holds because
//! rendering is a pure function of the record set:
//!
//! - order is computed from each record's own clock and hash, never from
//!   arrival;
//! - applying a record is commutative with respect to arrival order, because
//!   every effect is resolved after sorting rather than as records land;
//! - a record that is invalid is invalid on every node, because validity is a
//!   question about replayed governance state rather than local opinion.
//!
//! So a partition, a permutation, a duplicate delivery and a missing live
//! message all converge on the same rendering once the record sets agree — which
//! the durable path guarantees they eventually do.

use crate::{
    Attachment, Authority, ChannelId, Hlc, MessageId, Placement, Record, RecordBody,
};
use intranet_identity::PerNetworkIdentityId;
use std::collections::{BTreeMap, BTreeSet};

/// Why a record was not applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// Its signature did not verify.
    BadSignature,
    /// It named a channel this view does not cover.
    WrongChannel,
    /// Its author is not a current member of the network.
    NotAMember,
    /// Its author may not post here.
    NotPermitted,
    /// It edits or withdraws a message its author did not write.
    NotTheAuthor,
    /// It redacts, but its author held no moderation authority.
    NotAModerator,
    /// Its target does not exist in this view.
    UnknownTarget,
}

/// One message as it currently stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedMessage {
    /// The message's identifier.
    pub id: MessageId,
    /// Who wrote it.
    pub author: PerNetworkIdentityId,
    /// When they say they wrote it.
    pub hlc: Hlc,
    /// Its current text, after any edits by its author.
    pub body: String,
    /// What it replies to.
    pub reply_to: Option<MessageId>,
    /// Files published with it.
    pub attachments: Vec<Attachment>,
    /// Whether its author has since revised it.
    pub edited: bool,
    /// Whether its author withdrew it.
    pub withdrawn: bool,
    /// Whether a moderator hid it.
    ///
    /// Kept distinct from `withdrawn` because they are different facts about
    /// different actors, and a client should be able to say which happened.
    pub redacted: bool,
    /// Reactions, by key, each naming who reacted.
    pub reactions: BTreeMap<String, BTreeSet<PerNetworkIdentityId>>,
    /// Whether it is currently pinned.
    pub pinned: bool,
}

impl RenderedMessage {
    /// Whether a conformant client should display this message's content.
    ///
    /// Hiding is all this can mean. Neither withdrawal nor redaction retracts
    /// bytes anybody already holds, and `design/05` §5 requires the UI say so
    /// rather than imply otherwise.
    pub const fn is_visible(&self) -> bool {
        !self.withdrawn && !self.redacted
    }
}

/// A channel's history, merged from every log that contributes to it.
#[derive(Debug, Clone)]
pub struct ChannelView {
    channel: ChannelId,
    placement: Placement,
    records: BTreeMap<(Hlc, MessageId), Record>,
    rejected: Vec<(MessageId, Rejection)>,
}

impl ChannelView {
    /// Opens an empty view of a channel.
    pub fn new(placement: Placement) -> Self {
        Self {
            channel: placement.channel,
            placement,
            records: BTreeMap::new(),
            rejected: Vec::new(),
        }
    }

    /// Admits records, in any order, from any source.
    ///
    /// Idempotent: a record delivered twice occupies the same key and replaces
    /// itself, which is what lets the live and durable paths overlap freely.
    pub fn admit<A: Authority>(
        &mut self,
        records: impl IntoIterator<Item = Record>,
        authority: &A,
    ) {
        for record in records {
            let id = record.id();
            if let Err(reason) = self.check(&record, authority) {
                self.rejected.push((id, reason));
                continue;
            }
            self.records.insert((record.hlc, id), record);
        }
    }

    /// Checks a record against replayed state, before it can affect anything.
    fn check<A: Authority>(&self, record: &Record, authority: &A) -> Result<(), Rejection> {
        if record.channel != self.channel {
            return Err(Rejection::WrongChannel);
        }
        record
            .verify_signature()
            .map_err(|_| Rejection::BadSignature)?;
        if !authority.is_member(&record.author) {
            return Err(Rejection::NotAMember);
        }
        match &record.body {
            RecordBody::Redaction {
                governance_head, ..
            } => {
                if !authority.may_moderate_at(&record.author, &self.placement, governance_head) {
                    return Err(Rejection::NotAModerator);
                }
            }
            _ => {
                if !authority.may_post(&record.author, &self.placement) {
                    return Err(Rejection::NotPermitted);
                }
            }
        }
        Ok(())
    }

    /// Records that were refused, with why.
    ///
    /// Surfaced rather than silently dropped: a refusal is usually a permission
    /// change somebody needs to know about, not noise.
    pub fn rejected(&self) -> &[(MessageId, Rejection)] {
        &self.rejected
    }

    /// How many records this view holds.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether this view holds nothing.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Renders the channel.
    ///
    /// A pure function of the admitted record set: same records in, same bytes
    /// out, on every node, regardless of the order they arrived in. Everything
    /// below resolves *after* sorting, which is what makes that true.
    pub fn render(&self) -> Vec<RenderedMessage> {
        let mut messages: BTreeMap<MessageId, RenderedMessage> = BTreeMap::new();
        let mut order: Vec<MessageId> = Vec::new();

        // Pass one: messages, in merge order, so later passes can target them.
        for ((hlc, id), record) in &self.records {
            if let RecordBody::Message {
                body,
                reply_to,
                attachments,
            } = &record.body
            {
                order.push(*id);
                messages.insert(
                    *id,
                    RenderedMessage {
                        id: *id,
                        author: record.author,
                        hlc: *hlc,
                        body: body.clone(),
                        reply_to: *reply_to,
                        attachments: attachments.clone(),
                        edited: false,
                        withdrawn: false,
                        redacted: false,
                        reactions: BTreeMap::new(),
                        pinned: false,
                    },
                );
            }
        }

        // Pass two: everything that modifies a message. Applied in merge order,
        // so the last edit wins deterministically and a remove that sorts after
        // an add wins over it.
        for record in self.records.values() {
            match &record.body {
                RecordBody::Message { .. } => {}
                RecordBody::Edit { target, body } => {
                    if let Some(message) = messages.get_mut(target)
                        && message.author == record.author
                    {
                        message.body = body.clone();
                        message.edited = true;
                    }
                }
                RecordBody::Tombstone { target } => {
                    if let Some(message) = messages.get_mut(target)
                        && message.author == record.author
                    {
                        message.withdrawn = true;
                    }
                }
                RecordBody::Reaction {
                    target,
                    key,
                    remove,
                } => {
                    if let Some(message) = messages.get_mut(target) {
                        let who = message.reactions.entry(key.clone()).or_default();
                        if *remove {
                            who.remove(&record.author);
                        } else {
                            who.insert(record.author);
                        }
                        if who.is_empty() {
                            message.reactions.remove(key);
                        }
                    }
                }
                RecordBody::Pin { target, remove } => {
                    if let Some(message) = messages.get_mut(target) {
                        message.pinned = !remove;
                    }
                }
                RecordBody::Redaction { target, .. } => {
                    if let Some(message) = messages.get_mut(target) {
                        message.redacted = true;
                    }
                }
            }
        }

        order
            .into_iter()
            .filter_map(|id| messages.remove(&id))
            .collect()
    }
}
