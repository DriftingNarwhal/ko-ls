//! What crosses into the webview, and why it is not the domain types.
//!
//! # Why a separate set of shapes
//!
//! `kols-core`'s records have exactly one serialization, and it is normative:
//! spec 07 §3's canonical encoding, hand-written because a record's id is the
//! hash of those bytes and a derive macro's output is not a stable contract.
//! Putting `#[derive(Serialize)]` on the same types would create a second
//! serialization living beside the first, and the failure that invites is not
//! hypothetical — somebody eventually sends the convenient one over a wire and
//! discovers that ids no longer match.
//!
//! So the shell converts. These types are a *view*: strings where the domain has
//! hashes, flattened where the domain is precise, and free to change whenever
//! the interface wants something different, because nothing verifies against
//! them.
//!
//! # The interface never builds a command
//!
//! Nothing here deserializes. The webview names an intent with plain arguments —
//! a channel id, a body — and the shell builds the `kols_api::Command` from
//! them. That is not a weakening of `design/05` §3's first property: the command
//! still names its target explicitly, and the gate still resolves permission by
//! replay. It is one fewer place where a hostile front end can hand the core a
//! shape it did not expect.

use intranet_crypto::to_hex;
use kols_core::{Hlc, Privacy, RenderedMessage};
use serde::Serialize;

/// A channel, as the interface lists it.
#[derive(Debug, Serialize)]
pub struct Channel {
    /// Hex of the channel id — the handle the interface passes back.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its topic, possibly empty.
    pub topic: String,
    /// Whether it is restricted to a roster.
    pub private: bool,
    /// Whether it is archived: readable, not writable.
    pub archived: bool,
}

/// One reaction, collapsed to a count.
///
/// The domain carries *who* reacted; this carries how many. A roster of
/// reactors is a hover away and a count is what a message row shows, so sending
/// the set on every render would be paying for something nobody displays.
#[derive(Debug, Serialize)]
pub struct Reaction {
    /// The reaction itself.
    pub key: String,
    /// How many members hold it.
    pub count: usize,
}

/// A message, rendered.
#[derive(Debug, Serialize)]
pub struct Message {
    /// Hex of the record id.
    pub id: String,
    /// The author's display name, if they have claimed one.
    pub author: String,
    /// A short form of the author's per-network identity.
    ///
    /// Always sent, never optional. Spec 07 §8 makes this an obligation:
    /// uniqueness is decided on a key that does not fold confusables, so a name
    /// alone cannot distinguish two members and the interface must not let it
    /// try.
    pub author_id: String,
    /// The author's clock reading, as a wall time.
    pub at: String,
    /// Its current text, after any edits by its author.
    pub body: String,
    /// Whether its author revised it.
    pub edited: bool,
    /// Whether its author withdrew it.
    pub withdrawn: bool,
    /// Whether a moderator hid it.
    pub redacted: bool,
    /// Whether it is pinned.
    pub pinned: bool,
    /// Its reactions.
    pub reactions: Vec<Reaction>,
}

/// A channel, opened.
#[derive(Debug, Serialize)]
pub struct Opened {
    /// Hex of the channel id.
    pub channel: String,
    /// Its messages, merged and ordered.
    pub messages: Vec<Message>,
    /// How many authors contributed.
    pub authors: usize,
    /// How many records the reader refused, and why.
    ///
    /// Surfaced rather than dropped, for the reason `kols channel list` gives:
    /// a record this node refuses is one some other client may be showing, and
    /// silence would make the two look like they agree.
    pub refused: Vec<String>,
}

/// A network this client holds a store for.
#[derive(Debug, Serialize)]
pub struct Network {
    /// Hex of the network id — the handle the interface passes back.
    pub id: String,
    /// The local label its creator or joiner gave it.
    ///
    /// Local, and only ever local: spec 07 defines no policy key for a network's
    /// name, and inventing one is how two clients end up disagreeing about what a
    /// network is called.
    pub label: String,
    /// Whether this node holds an epoch key for it.
    ///
    /// A network without one is joined and unreadable, which is the ordinary
    /// state between being admitted and being keyed in rather than a fault.
    pub keyed: bool,
    /// Whether it is the one currently open.
    pub open: bool,
}

impl Network {
    /// Converts one known network.
    pub fn of(known: &kols_node::workspace::Known, open: Option<&str>) -> Self {
        Self {
            id: known.id.clone(),
            label: known.label.clone(),
            keyed: known.keyed,
            open: open == Some(known.id.as_str()),
        }
    }
}

/// Where a redeemed invite left this member.
#[derive(Debug, Serialize)]
pub struct Joined {
    /// Whether they were admitted outright, or are waiting for an admin.
    ///
    /// Both are successful joins (Core §2.4). A client that treated waiting as a
    /// failure would report a network screening its members — working exactly as
    /// configured — as though something had broken.
    pub admitted: bool,
    /// Their identity here, for whoever will admit them. Empty when admitted.
    pub identity: String,
}

/// A freshly minted invite, ready to be handed to somebody.
///
/// The URI rather than the bytes, because carrying one is presentation and this
/// is the shell — `kols-api` hands back the protocol's bytes and declines to
/// decide whether they travel as a link, a QR code or a pasted string.
#[derive(Debug, Serialize)]
pub struct Invite {
    /// `intranet-chat://join/…`, the whole of what a joiner needs.
    pub uri: String,
    /// Roughly how many hours it has left, for saying so out loud.
    pub hours: i64,
    /// How many identities may still be admitted with it.
    pub uses: u32,
}

/// Somebody who redeemed an invite and is waiting to be let in.
#[derive(Debug, Serialize)]
pub struct Waiting {
    /// The full identity, which is what admitting them names.
    pub identity: String,
    /// The short form, for showing.
    pub short: String,
}

/// Who this member is here, and what they may do.
///
/// The permission flags exist so the interface can hide controls it should not
/// offer (`design/09` §5) — and hiding is presentation, never enforcement. Every
/// command is re-checked on receipt regardless of what this said.
/// What this network designates as relays, and what this node cached.
///
/// Both, because they answer different questions and disagree in exactly the
/// case that is hard to debug: a node whose cache names a relay that is gone
/// behaves differently from one that never had a relay, and only the pair shows
/// which you have (`STATUS.md` O13).
#[derive(Debug, Serialize)]
pub struct Relays {
    /// What replay says this network uses, in order.
    pub designated: Vec<String>,
    /// What this node wrote down locally and will dial before it has synced.
    pub cached: Vec<String>,
    /// Whether this member may change them — `define-policy`.
    pub may_set: bool,
}

#[derive(Debug, Serialize)]
pub struct Me {
    /// A short form of this member's identity in this network.
    pub identity: String,
    /// This member's display name here, if they have claimed one.
    pub name: Option<String>,
    /// Hex of the network id.
    pub network: String,
    /// The local label for this network.
    pub label: String,
    /// Whether this node holds an epoch key at all.
    pub has_key: bool,
    /// Whether this member may post.
    pub may_post: bool,
    /// Whether this member may define channels.
    pub may_create_channel: bool,
    /// Whether this member may create invites.
    pub may_invite: bool,
    /// Whether this member may designate relays — `define-policy`.
    ///
    /// Separate from [`Me::may_invite`] even though a founder holds both, because
    /// they come apart for everybody else: `approve-node` can be delegated to a
    /// moderator who has no business rewriting policy.
    pub may_set_relays: bool,
}

impl Message {
    /// Converts one rendered message, resolving its author's name.
    pub fn of(message: &RenderedMessage, names: &kols_core::Names) -> Self {
        Self {
            id: to_hex(message.id.as_bytes()),
            author: names
                .of(&message.author)
                .map_or_else(|| message.author.short(), str::to_owned),
            author_id: message.author.short(),
            at: stamp(message.hlc),
            body: message.body.clone(),
            edited: message.edited,
            withdrawn: message.withdrawn,
            redacted: message.redacted,
            pinned: message.pinned,
            reactions: message
                .reactions
                .iter()
                .map(|(key, who)| Reaction {
                    key: key.clone(),
                    count: who.len(),
                })
                .collect(),
        }
    }
}

impl Channel {
    /// Converts one channel as replay understands it.
    pub fn of(channel: &kols_node::network::Channel) -> Self {
        Self {
            id: to_hex(channel.id.as_bytes()),
            name: channel.name.clone(),
            topic: channel.topic.clone(),
            private: channel.privacy == Privacy::Private,
            archived: channel.archived,
        }
    }
}

/// An author's clock reading, as a wall time.
///
/// A reading is what the author's clock said, not what this node's says, and
/// `design/01` §4 is explicit that without a central sequencer no chat system
/// can do better. Displaying it as a time is honest as long as nothing claims it
/// is authoritative.
fn stamp(hlc: Hlc) -> String {
    let secs = hlc.wall_millis / 1000;
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use kols_core::MessageId;
    use std::collections::{BTreeMap, BTreeSet};

    fn identity(seed: u8) -> intranet_identity::PerNetworkIdentityId {
        intranet_identity::MasterSeed::from_entropy([seed; 32])
            .identity_for(&intranet_identity::NetworkId::from_bytes([1u8; 32]))
            .expect("derives")
            .id()
    }

    /// A registry with nobody in it, so an author falls back to their id.
    ///
    /// Which is the case worth having as the default in these: the fallback is
    /// what a member with no claimed name renders as, and it must stay legible.
    fn no_names() -> kols_core::Names {
        kols_core::Names::new()
    }

    fn rendered() -> RenderedMessage {
        RenderedMessage {
            id: MessageId::from_bytes([0xab; 32]),
            author: identity(2),
            hlc: Hlc::new(3_723_000, 0),
            body: "hello".to_owned(),
            reply_to: None,
            attachments: Vec::new(),
            edited: false,
            withdrawn: false,
            redacted: false,
            reactions: BTreeMap::new(),
            pinned: false,
        }
    }

    #[test]
    fn a_message_carries_the_flags_the_interface_draws_differently() {
        let mut message = rendered();
        message.edited = true;
        message.pinned = true;
        let view = Message::of(&message, &no_names());

        assert!(view.edited && view.pinned);
        assert!(!view.withdrawn && !view.redacted);
        assert!(view.id.starts_with("abab"), "{}", view.id);
    }

    #[test]
    fn withdrawn_and_redacted_stay_distinct() {
        // Different facts about different actors: the author withdrew it, or a
        // moderator hid it. `design/01` §6 keeps them apart and so does the
        // interface, because a client should be able to say which happened.
        let mut withdrawn = rendered();
        withdrawn.withdrawn = true;
        let mut redacted = rendered();
        redacted.redacted = true;

        let a = Message::of(&withdrawn, &no_names());
        let b = Message::of(&redacted, &no_names());
        assert!(a.withdrawn && !a.redacted);
        assert!(b.redacted && !b.withdrawn);
    }

    #[test]
    fn reactions_collapse_to_counts_and_keep_their_order() {
        let mut message = rendered();
        message.reactions.insert(
            "+1".to_owned(),
            BTreeSet::from([identity(2), identity(3), identity(4)]),
        );
        message
            .reactions
            .insert("eyes".to_owned(), BTreeSet::from([identity(5)]));

        let view = Message::of(&message, &no_names());
        assert_eq!(view.reactions.len(), 2);
        assert_eq!(view.reactions[0].key, "+1");
        assert_eq!(view.reactions[0].count, 3);
        assert_eq!(view.reactions[1].count, 1);
    }

    #[test]
    fn the_body_crosses_unchanged() {
        // No escaping, no truncation, no markdown. The interface sets it as
        // text rather than markup, so anything done to it here would be a
        // second, quieter place for it to go wrong.
        let mut message = rendered();
        message.body = "<script>alert(1)</script> & \"quotes\"".to_owned();
        assert_eq!(Message::of(&message, &no_names()).body, message.body);
    }

    #[test]
    fn an_author_with_a_name_still_carries_their_id() {
        // Spec 07 §8: uniqueness does not fold confusables, so a name alone
        // cannot tell two members apart and the view must not let it try.
        let message = rendered();
        let mut names = kols_core::Names::new();
        names.apply(
            message.author,
            &kols_core::NameClaim::new("ada").expect("valid"),
        );

        let view = Message::of(&message, &names);
        assert_eq!(view.author, "ada");
        assert_eq!(view.author_id, message.author.short());
        assert_ne!(view.author, view.author_id);
    }

    #[test]
    fn an_author_with_no_name_renders_as_their_id() {
        let message = rendered();
        let view = Message::of(&message, &no_names());
        assert_eq!(view.author, message.author.short());
        assert_eq!(view.author, view.author_id);
    }

    #[test]
    fn a_clock_reading_renders_as_a_wall_time() {
        // 3,723,000 ms is 01:02:03 past the hour.
        assert_eq!(Message::of(&rendered(), &no_names()).at, "01:02:03");
    }
}
