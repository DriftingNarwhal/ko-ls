//! Channel entries against a real governance log — spec 07 §3.8, Core §2.2.
//!
//! # Why this exists separately from `channel_entries.rs`
//!
//! Those tests check the encoding and the two reader obligations against
//! hand-built values, which is the right shape for a wire contract and is blind
//! to one thing: whether an entry this client *builds* survives the protocol's
//! own authorization. It did not. A member holding `chat:create-channel:*` —
//! exactly what [`kols_core::capabilities::network_scoped`] registers, and the
//! grant that verb exists for — could not create a channel, because the entry
//! declared a channel-scoped name they did not hold and which nobody could have
//! registered in advance, the channel id not existing until the entry does.
//! Only a Founder, whose capability set is unrestricted, got through.
//!
//! The lesson is narrow and worth keeping: an extension capability is matched by
//! exact name, so what an entry declares has to be a name its author was really
//! granted, not the one that best describes the action.

use intranet_crypto::Timestamp;
use intranet_governance::{
    Capability, CapabilitySet, EntryBody, GovernanceLog, GovernanceState, GroupId, LogEntry,
    NetworkPolicy,
};
use intranet_identity::{MasterSeed, NetworkId, PerNetworkIdentity};
use kols_core::*;

const NET: NetworkId = NetworkId::from_bytes([7u8; 32]);

fn person(seed: u8) -> PerNetworkIdentity {
    MasterSeed::from_entropy([seed; 32])
        .identity_for(&NET)
        .expect("derives")
}

fn category() -> CategoryId {
    CategoryId::from_bytes([3u8; 32])
}

/// A network whose policy registers the chat vocabulary, as genesis must.
///
/// Without these entries a network cannot grant chat permissions at all: an
/// unregistered extension name is refused outright rather than assumed ordinary,
/// which is the fail-closed behaviour that keeps a governance-tier capability
/// from being slipped onto `everyone`.
fn genesis(founder: &PerNetworkIdentity) -> LogEntry {
    let mut policy = NetworkPolicy::conservative_default();
    for (name, tier) in kols_core::capabilities::network_scoped() {
        policy.extension_capabilities.insert(name, tier);
    }
    for (name, tier) in kols_core::capabilities::for_category(&category()) {
        policy.extension_capabilities.insert(name, tier);
    }
    LogEntry::create(
        founder,
        None,
        Timestamp::from_millis(0),
        EntryBody::Genesis {
            network: NET,
            policy,
            everyone_capabilities: [Capability::ReadContent].into_iter().collect(),
        },
    )
}

/// A log with a founder, plus `member` granted exactly `capabilities`.
fn network_granting(
    member: &PerNetworkIdentity,
    capabilities: impl IntoIterator<Item = Capability>,
) -> (GovernanceLog, intranet_crypto::Hash) {
    let founder = person(1);
    let mut log = GovernanceLog::new();
    let root = log.insert(genesis(&founder)).expect("genesis");

    let admitted = log
        .insert(LogEntry::create(
            &founder,
            Some(root),
            Timestamp::from_millis(1),
            EntryBody::MembershipChange {
                group: GroupId::everyone(),
                identity: member.id(),
                action: intranet_governance::MembershipAction::Add { via_invite: None },
            },
        ))
        .expect("admits the member");

    let group = GroupId::new("channel-makers");
    let defined = log
        .insert(LogEntry::create(
            &founder,
            Some(admitted),
            Timestamp::from_millis(2),
            EntryBody::DefineGroup {
                group: group.clone(),
                capabilities: CapabilitySet::explicit(capabilities),
            },
        ))
        .expect("defines the group");

    let head = log
        .insert(LogEntry::create(
            &founder,
            Some(defined),
            Timestamp::from_millis(3),
            EntryBody::MembershipChange {
                group,
                identity: member.id(),
                action: intranet_governance::MembershipAction::Add { via_invite: None },
            },
        ))
        .expect("adds them to it");

    (log, head)
}

fn replay(log: &GovernanceLog) -> Result<GovernanceState, intranet_governance::GovernanceError> {
    let chain: Vec<_> = log
        .canonical_chain()
        .iter()
        .filter_map(|hash| log.get(hash))
        .collect();
    GovernanceState::replay(chain)
}

fn definition(channel: ChannelId, category: Option<CategoryId>) -> ChannelEntry {
    ChannelEntry::new(
        channel,
        ChannelEntryBody::Definition {
            name: "general".to_owned(),
            category,
            kind: ChannelKind::Text,
            privacy: Privacy::Public,
            topic: String::new(),
            slowmode: 0,
        },
    )
}

/// Appends `entry` authored by `author` and replays, returning the verdict.
fn publish(
    log: &mut GovernanceLog,
    parent: intranet_crypto::Hash,
    author: &PerNetworkIdentity,
    body: EntryBody,
) -> Result<GovernanceState, intranet_governance::GovernanceError> {
    log.insert(LogEntry::create(
        author,
        Some(parent),
        Timestamp::from_millis(10),
        body,
    ))
    .expect("insert is structural; authorization is replay's job");
    replay(log)
}

#[test]
fn a_network_wide_grant_lets_an_ordinary_member_create_a_channel() {
    // The regression this file exists for. `chat:create-channel:*` is what
    // `network_scoped()` registers and the whole reason the verb is Ordinary; if
    // holding it does not let you create a channel, the verb authorizes nothing
    // and only Founders ever make channels.
    let member = person(2);
    let (mut log, head) = network_granting(
        &member,
        [Capability::extension("chat:create-channel:*".to_owned())],
    );
    let state = replay(&log).expect("the network replays");

    let entry = definition(server_channel_id(&NET, &[9u8; 32]), None);
    let body = entry
        .to_app_entry(&state, &member.id(), None)
        .expect("a member holding the network-wide grant can declare something");

    assert_eq!(
        match &body {
            EntryBody::AppEntry { required, .. } => required.clone(),
            _ => panic!("expected an application entry"),
        },
        Capability::extension("chat:create-channel:*".to_owned()),
        "the declaration must name what this author actually holds"
    );

    publish(&mut log, head, &member, body).expect("and the network accepts it");
}

#[test]
fn a_category_grant_lets_a_member_create_a_channel_in_that_category() {
    // The scope a network is expected to administer with (`design/02` §4). A
    // definition takes its category from the entry, since the channel does not
    // exist in replayed state yet.
    let member = person(3);
    let (mut log, head) = network_granting(
        &member,
        kols_core::capabilities::for_category(&category())
            .into_keys()
            .filter(|name| name.starts_with("chat:create-channel:"))
            .map(Capability::extension),
    );
    let state = replay(&log).expect("replays");

    let entry = definition(server_channel_id(&NET, &[10u8; 32]), Some(category()));
    let body = entry
        .to_app_entry(&state, &member.id(), None)
        .expect("the category grant authorizes a channel placed in it");

    publish(&mut log, head, &member, body).expect("the network accepts it");
}

#[test]
fn a_member_holding_nothing_relevant_is_refused_before_publishing() {
    // Refusing here is worth more than refusing at replay: an entry that names a
    // capability its author lacks is one every node rejects, so producing it puts
    // a permanently dead entry in the log and tells the author nothing.
    let member = person(4);
    let (log, _) = network_granting(
        &member,
        [Capability::extension("chat:post:*".to_owned())],
    );
    let state = replay(&log).expect("replays");

    let entry = definition(server_channel_id(&NET, &[11u8; 32]), None);
    assert!(
        matches!(
            entry.to_app_entry(&state, &member.id(), None),
            Err(ChannelRefusal::WrongCapability { .. })
        ),
        "a posting grant must not produce a publishable channel definition"
    );
}

#[test]
fn creating_does_not_confer_managing() {
    // The split the capability decision turns on, checked where it binds rather
    // than in isolation: a member who may create channels must not thereby be
    // able to change a private channel's roster, which is the governance-tier
    // power `chat:manage-channel` guards.
    let member = person(5);
    let (log, _) = network_granting(
        &member,
        [Capability::extension("chat:create-channel:*".to_owned())],
    );
    let state = replay(&log).expect("replays");
    let channel = server_channel_id(&NET, &[12u8; 32]);

    let roster = ChannelEntry::new(
        channel,
        ChannelEntryBody::Membership {
            action: MembershipAction::Add,
            identity: person(6).id(),
        },
    );
    assert!(
        roster.declaration(&state, &member.id(), None).is_none(),
        "create-channel must not authorize a roster change"
    );

    assert!(
        definition(channel, None)
            .declaration(&state, &member.id(), None)
            .is_some(),
        "while the same member can still create one"
    );
}

#[test]
fn an_entry_declaring_the_wrong_capability_is_refused_by_the_reader_after_the_log_accepted_it() {
    // The gap that makes the reader check necessary, demonstrated end to end
    // rather than argued. A member granted `chat:post:*` builds a channel
    // definition by hand declaring that, the protocol checks they hold it — they
    // do — and the entry is accepted into the log. Only a reader that understands
    // `chat` knows a definition needed `chat:create-channel`.
    let member = person(7);
    let (mut log, head) = network_granting(
        &member,
        [Capability::extension("chat:post:*".to_owned())],
    );

    let entry = definition(server_channel_id(&NET, &[13u8; 32]), None);
    let forged = EntryBody::AppEntry {
        namespace: CHAT_NAMESPACE.to_owned(),
        kind: "channel-definition".to_owned(),
        required: Capability::extension("chat:post:*".to_owned()),
        payload: entry.encode(),
    };

    let state = publish(&mut log, head, &member, forged.clone())
        .expect("the protocol accepts it: the author does hold what it declared");
    let _ = state;

    assert!(
        matches!(
            ChannelEntry::read(&forged, NetworkProfile::Server, None),
            Err(ChannelRefusal::WrongCapability { .. })
        ),
        "and every conformant chat reader refuses it anyway"
    );
}

#[test]
fn a_conversation_network_accepts_the_entry_and_every_reader_still_refuses_it() {
    // The other honest limit, likewise end to end. The protocol carries `chat`
    // payloads without decoding them, so it cannot know a conversation has no
    // channels to define — the entry lands in the log and is refused on the way
    // out, by every client that understands the namespace.
    let member = person(8);
    let (mut log, head) = network_granting(
        &member,
        [Capability::extension("chat:create-channel:*".to_owned())],
    );
    let state = replay(&log).expect("replays");

    let entry = definition(server_channel_id(&NET, &[14u8; 32]), None);
    let body = entry
        .to_app_entry(&state, &member.id(), None)
        .expect("declares something they hold");

    publish(&mut log, head, &member, body.clone())
        .expect("the protocol has no basis to refuse it");

    assert_eq!(
        ChannelEntry::read(&body, NetworkProfile::Conversation, None),
        Err(ChannelRefusal::NotAServer),
        "the rejection is the reader's job, and this is it being done"
    );
}

#[test]
fn the_capability_names_this_client_registers_are_the_names_it_declares() {
    // Registration and declaration are built by different functions from the
    // same convention, and an extension capability resolves by exact name — so a
    // one-character drift between them would make every chat grant unresolvable,
    // with a `UnregisteredExtensionCapability` at replay and no earlier signal.
    let registered = kols_core::capabilities::network_scoped();
    let channel = server_channel_id(&NET, &[15u8; 32]);

    for entry in [
        definition(channel, None),
        ChannelEntry::new(
            channel,
            ChannelEntryBody::Update {
                change: ChannelChange::Archive,
            },
        ),
    ] {
        let network_wide = entry
            .acceptable(None)
            .into_iter()
            .find(|capability| match capability {
                Capability::Extension(name) => name.ends_with(":*"),
                _ => false,
            })
            .expect("every kind has a network-wide form");
        let Capability::Extension(name) = &network_wide else {
            unreachable!("filtered above")
        };
        assert!(
            registered.contains_key(name),
            "{name} is declared but never registered"
        );
    }

    for name in kols_core::capabilities::for_category(&category()).keys() {
        assert!(
            name.starts_with("chat:") && name.contains(":cat:"),
            "category registration shape changed: {name}"
        );
    }
}
