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
    /// Hex of the category it sits in, if any.
    pub category: Option<String>,
    /// Its position among its siblings, if it has ever been given one.
    pub position: Option<u32>,
}

/// One row of the sidebar, in the order the network agrees on.
///
/// **Ordered here rather than in the webview.** Spec 07 §1.6's default is
/// normative — every node must compute the same one — so it is produced by
/// `kols_core::sidebar_order`, which is the tested implementation of that rule.
/// Sorting in JavaScript would be a second implementation of a normative rule
/// living beside the first, which is the arrangement this module's own opening
/// paragraph exists to refuse.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SidebarRow {
    /// A channel in no category. These come before every category.
    Channel {
        /// The channel.
        channel: Channel,
    },
    /// A category and the channels inside it, in their own order.
    Category {
        /// Hex of the category id.
        id: String,
        /// What to call it. Empty when nothing ever defined this category —
        /// a channel may name one that has no definition (spec 07 §1.8).
        name: String,
        /// Its position among the other categories, if it has one. Sent because
        /// the interface needs somewhere to put the next folder, and guessing
        /// would pile every new one on the same position.
        position: Option<u32>,
        /// Its channels, ordered.
        channels: Vec<Channel>,
    },
}

/// One action a reconciliation undid — Core §2.7.1 point 5.
#[derive(Debug, Clone, Serialize)]
pub struct VoidedAction {
    /// A short label for what it was.
    pub kind: String,
    /// Whether losing it re-opens a gap rather than merely losing an edit.
    pub security_relevant: bool,
}

/// What a healed fork undid.
///
/// Not derived from replayed state and it cannot be: replay follows the winning
/// branch, so an action that lost leaves nothing there to find. This is the only
/// record of it that reaches the interface.
#[derive(Debug, Clone, Serialize)]
pub struct Reorg {
    /// This member's own voided actions, which are the ones they can resubmit.
    pub mine: Vec<VoidedAction>,
    /// How many other members' actions the same reconciliation voided.
    pub others: usize,
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
    /// Whether this member is one of them.
    ///
    /// Carried rather than inferred, because reacting is a toggle and the
    /// interface has to know which way it is about to go. A client that guessed
    /// would add a reaction the member already holds, which is a no-op the
    /// member reads as a broken button.
    pub mine: bool,
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
    /// Whether this member wrote it.
    ///
    /// What gates revising and withdrawing in the interface. The command is
    /// authorized on receipt regardless — spec 07 §5.2 lets only an author
    /// revise their own — so this decides what is *offered*, not what is
    /// allowed.
    pub mine: bool,
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
/// One member of this network, and whether this node is talking to them.
#[derive(Debug, Serialize)]
pub struct Member {
    /// Their identity in this network, as hex.
    pub identity: String,
    /// A short form of it, for display beside a name.
    pub short: String,
    /// Their display name here, if they have claimed one.
    pub name: Option<String>,
    /// Whether this node currently has a connection to them.
    ///
    /// **Not "online".** There is no routing (Core §5.2) and this client dials
    /// the peers it has addresses for rather than every member, so `false`
    /// covers three different situations — away, unreachable from here, or
    /// simply never dialled — and nothing on this machine distinguishes them.
    pub connected: bool,
    /// Whether this is the member reading the list.
    pub you: bool,
}

/// What this network designates as relays, and what this node cached.
///
/// Both, because they answer different questions and disagree in exactly the
/// case that is hard to debug: a node whose cache names a relay that is gone
/// behaves differently from one that never had a relay, and only the pair shows
/// which you have.
#[derive(Debug, Serialize)]
pub struct Relays {
    /// What replay says this network uses, in order.
    pub designated: Vec<String>,
    /// What this node wrote down locally and will dial before it has synced.
    pub cached: Vec<String>,
    /// Whether this member may change them — `define-policy`.
    pub may_set: bool,
    /// Whether the running node has reported its relay standing yet.
    ///
    /// Distinguishes "no circuit" from "not asked yet", which look identical
    /// from the interface and mean opposite things: the first is a fault to act
    /// on, the second is a node that has been up for two seconds.
    pub reported: bool,
    /// The relay a circuit was reserved on, when one was.
    pub reserved: Option<String>,
    /// Why each designated relay did not work, in the order tried.
    ///
    /// Shown verbatim. These name the *cause* — unreachable, or reached and
    /// announcing nothing — which need opposite fixes and which a status line
    /// cannot distinguish.
    pub failures: Vec<String>,
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
    /// Whether this member may rename, move, archive or delete structure —
    /// network-wide `chat:manage-channel`.
    ///
    /// An approximation in the same direction as `may_moderate`, and for the
    /// same reason: `chat:manage-channel` is grantable per category and per
    /// channel, and this does not see those. Somebody who manages one category
    /// is offered no controls here and would be authorized if they had them.
    /// Under-offering is the safe way to be wrong — the command is re-checked on
    /// receipt either way, so a hidden control costs a discovery rather than a
    /// refusal after the fact.
    pub may_manage_channel: bool,
    /// Whether this member may create invites.
    pub may_invite: bool,
    /// Whether this member may pin — network-wide `moderate-content`.
    ///
    /// An approximation, deliberately. `chat:moderate` is also grantable per
    /// channel, and this does not see that — so a channel moderator is offered
    /// no pin button and the command would still succeed if they used one. The
    /// error runs the safe way: this hides a control somebody may in fact hold,
    /// rather than offering one that will be refused.
    pub may_moderate: bool,
    /// Whether this member may designate relays — `define-policy`.
    ///
    /// Separate from [`Me::may_invite`] even though a founder holds both, because
    /// they come apart for everybody else: `approve-node` can be delegated to a
    /// moderator who has no business rewriting policy.
    pub may_set_relays: bool,
    /// Whether this member may create roles and change what they hold — `define-group`.
    ///
    /// Deliberately not the same question as [`Me::may_assign_role`]. Core §2.2
    /// makes defining what a role *can do* a higher bar than deciding *who holds
    /// it*, and `design/02` §1 asks the interface to reflect that asymmetry
    /// rather than flatten it into one "admin" flag.
    pub may_define_group: bool,
    /// Whether this member may put anybody in any role at all.
    ///
    /// True when at least one role's `manage-membership` resolves for them, so
    /// the tab is offered rather than hidden. Which roles specifically is per
    /// role and answered by [`Role::may_assign`], because the tier of
    /// `manage-membership:<group>` is dynamic (Core §2.4) and one flag cannot
    /// carry it.
    pub may_assign_role: bool,
    /// How joiners are admitted — `auto` or `intake` (Core §2.4).
    pub admission_mode: String,
    /// Whether this network decides admission by member vote — Core §2.6.
    ///
    /// Carried because it is what makes auto-admit unavailable: the two cannot
    /// be combined, and an interface that offered the choice anyway would be
    /// offering a change the protocol refuses on replay.
    pub member_vote: bool,
    /// This network's name as every member sees it — D32, spec 07 §1.7.
    ///
    /// Distinct from [`Me::label`], which is local and this installation's alone.
    /// `None` means no name is declared, which is a real state: clients must not
    /// invent one.
    pub network_name: Option<String>,
}

/// One role, what it holds, and who is in it — `design/02` §1.
///
/// "Role" rather than "group" at this boundary on purpose. The protocol's word
/// is group and this is the same object; the interface says role because that is
/// what the concept maps to for somebody who has used a chat server, and
/// `design/00` §1 makes that mapping the whole design.
#[derive(Debug, Serialize)]
pub struct Role {
    /// Its name, which is also its identifier — group ids are names (Core §2.1).
    pub id: String,
    /// Whether this is one of the two groups genesis creates (Core §2.3, §2.4).
    ///
    /// `Founders` and `everyone` cannot be created or removed, and `everyone`
    /// additionally may never hold a governance-tier capability. Marked so the
    /// interface can say why a control is missing rather than just omitting it.
    pub implicit: bool,
    /// Whether it holds every capability there is — `Founders`, by default.
    ///
    /// Its grants cannot be edited one at a time: there is no explicit set to
    /// take a verb out of, and replacing `All` with whatever a checkbox
    /// enumerates would silently drop everything nobody listed.
    pub unrestricted: bool,
    /// Whether this is the `everyone` group, which carries Core §2.4's ceiling.
    pub everyone: bool,
    /// The chat verbs it holds, with the scope each is held at.
    pub grants: Vec<Grant>,
    /// Capabilities it holds that are the protocol's own rather than chat's.
    ///
    /// Shown, never edited here. `approve-node` and the rest are the network's
    /// governance rather than this application's vocabulary, and a checkbox
    /// grid built for chat verbs would misrepresent them as the same kind of
    /// thing. Listing them is still necessary — a role whose powers were half
    /// displayed would read as weaker than it is.
    pub protocol_grants: Vec<String>,
    /// Who holds it.
    pub members: Vec<Member>,
    /// Whether this member may add or remove its members.
    ///
    /// Per role, because `manage-membership:<group>` is the one capability whose
    /// tier is dynamic (Core §2.4) — governance-tier exactly when the target
    /// role holds governance power — so the answer genuinely differs per row.
    pub may_assign: bool,
}

/// One chat verb held at one scope.
#[derive(Debug, Serialize)]
pub struct Grant {
    /// The verb, from `design/02` §2.2.
    pub verb: String,
    /// `network`, `category` or `channel`.
    pub scope: String,
    /// Hex of the category or channel id, or empty at network scope.
    pub scope_id: String,
    /// What to call the scope on screen, resolved from replayed state.
    ///
    /// A grant can name a channel or category that no longer exists — deleting
    /// either leaves grants against its id untouched, since a capability is a
    /// string in a group's set and nothing sweeps them. Resolved here so the
    /// interface can say *"a channel that is gone"* rather than rendering a
    /// bare hash nobody can place.
    pub scope_label: String,
    /// Whether the verb is governance-tier — `design/02` §2.2.
    pub governance: bool,
}

/// A scope a grant can be bound at, for the picker.
#[derive(Debug, Serialize)]
pub struct ScopeOption {
    /// `network`, `category` or `channel`.
    pub kind: String,
    /// Hex of the id, or empty at network scope.
    pub id: String,
    /// What to call it.
    pub label: String,
}

/// One network setting, as it currently stands — spec 07 §4.3, §2.8.
#[derive(Debug, Serialize)]
pub struct Setting {
    /// A stable handle the interface passes back, matching `ChatSetting`.
    pub id: String,
    /// The policy key, shown so a value can be matched against the spec.
    pub key: String,
    /// A short human label.
    pub label: String,
    /// What it bounds, in a sentence.
    pub summary: String,
    /// Its value now, which is the default when nothing was ever set.
    pub value: i64,
    /// The shipped default, so the interface can offer a way back to it.
    pub default: i64,
    /// Whether the network has actually set it, or is riding the default.
    ///
    /// Worth distinguishing: a network that never chose a number picks up a
    /// revised default, and one that wrote the same number does not. The
    /// interface should not make those look identical.
    pub explicit: bool,
    /// `per-minute`, `bytes`, `count`, `millis`, `seconds` or `days`.
    pub unit: String,
    /// What setting this to zero would do, in a few words.
    ///
    /// Carried per setting because the answer differs — no limit, forever, or a
    /// real bound of zero — and that asymmetry is the thing most likely to be
    /// got wrong (`ChatSetting::zero_means`).
    pub zero_means: String,
    /// Whether this is a retention window rather than a ceiling.
    ///
    /// Retention refuses nothing: it decides what a network keeps maintaining.
    /// Grouped separately so it never reads as something a record can exceed.
    pub retention: bool,
}

/// One verb in the vocabulary, with what it means and what it costs.
#[derive(Debug, Serialize)]
pub struct Verb {
    /// The verb itself.
    pub name: String,
    /// Whether it is governance-tier.
    pub governance: bool,
    /// What holding it lets somebody do, in a sentence.
    pub summary: String,
}

impl Message {
    /// Converts one rendered message, resolving its author's name.
    ///
    /// `me` is the viewer, which decides what the interface offers on this
    /// message rather than what it says about it.
    pub fn of(
        message: &RenderedMessage,
        names: &kols_core::Names,
        me: &intranet_identity::PerNetworkIdentityId,
    ) -> Self {
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
                    mine: who.contains(me),
                })
                .collect(),
            mine: &message.author == me,
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
            category: channel.category.map(|c| to_hex(c.as_bytes())),
            position: channel.position,
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
        let view = Message::of(&message, &no_names(), &identity(9));

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

        let a = Message::of(&withdrawn, &no_names(), &identity(9));
        let b = Message::of(&redacted, &no_names(), &identity(9));
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

        let view = Message::of(&message, &no_names(), &identity(9));
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
        assert_eq!(Message::of(&message, &no_names(), &identity(9)).body, message.body);
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

        let view = Message::of(&message, &names, &identity(9));
        assert_eq!(view.author, "ada");
        assert_eq!(view.author_id, message.author.short());
        assert_ne!(view.author, view.author_id);
    }

    #[test]
    fn an_author_with_no_name_renders_as_their_id() {
        let message = rendered();
        let view = Message::of(&message, &no_names(), &identity(9));
        assert_eq!(view.author, message.author.short());
        assert_eq!(view.author, view.author_id);
    }

    #[test]
    fn a_clock_reading_renders_as_a_wall_time() {
        // 3,723,000 ms is 01:02:03 past the hour.
        assert_eq!(Message::of(&rendered(), &no_names(), &identity(9)).at, "01:02:03");
    }
}
