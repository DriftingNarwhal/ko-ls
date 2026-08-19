//! The boundary against a real governance log — `design/05` §3, property 1.
//!
//! # What these are for
//!
//! Not that `authorize` returns the answer its own match arms say it does; that
//! would test the code against itself. What is worth pinning is that the
//! boundary agrees with the thing it claims to enforce: a member holding
//! `chat:post:*` and nothing else can post and cannot pin, a category grant
//! reaches the channels in that category and no others, and a command's
//! sensitivity matches the tier the capability vocabulary actually assigns
//! rather than the one it felt like when it was written.
//!
//! Every state here is replayed from a real log rather than hand-built, for the
//! reason `channel_governance.rs` records: building values by hand is right for
//! a wire contract and blind to whether the protocol accepts what this client
//! produces.

use intranet_crypto::Timestamp;
use intranet_governance::{
    Capability, CapabilitySet, ContentType, EntryBody, GovernanceLog, GovernanceState, GroupId,
    LogEntry, MembershipAction, NetworkPolicy, PolicyValue, Tier,
};
use intranet_identity::{MasterSeed, NetworkId, PerNetworkIdentity};
use kols_api::{Actor, Authorized, Command, PlacementMap, Refusal, Sensitivity, authorize, placement};
use kols_core::{
    Attachment, CategoryId, ChannelChange, ChannelId, MessageId, Placement, Privacy, StateAuthority,
};

const NET: NetworkId = NetworkId::from_bytes([9u8; 32]);

fn person(seed: u8) -> PerNetworkIdentity {
    MasterSeed::from_entropy([seed; 32])
        .identity_for(&NET)
        .expect("derives")
}

fn channel(seed: u8) -> ChannelId {
    ChannelId::from_bytes([seed; 32])
}

fn category() -> CategoryId {
    CategoryId::from_bytes([200u8; 32])
}

/// One uncategorised channel, and one inside a category.
fn index() -> PlacementMap {
    let mut map = PlacementMap::new();
    map.insert(channel(1), placement(channel(1), None));
    map.insert(channel(2), placement(channel(2), Some(category())));
    map
}

fn genesis(founder: &PerNetworkIdentity, profile: Option<&str>) -> LogEntry {
    let mut policy = NetworkPolicy::conservative_default();
    for (name, tier) in kols_core::capabilities::namespaces() {
        policy.extension_capabilities.insert(name, tier);
    }
    if let Some(profile) = profile {
        policy.app_policy.insert(
            kols_core::keys::PROFILE.to_owned(),
            PolicyValue::Text(profile.to_owned()),
        );
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

fn network_granting(
    member: &PerNetworkIdentity,
    capabilities: impl IntoIterator<Item = Capability>,
) -> GovernanceState {
    network_granting_in(member, capabilities, None)
}

fn network_granting_in(
    member: &PerNetworkIdentity,
    capabilities: impl IntoIterator<Item = Capability>,
    profile: Option<&str>,
) -> GovernanceState {
    let founder = person(1);
    let mut log = GovernanceLog::new();
    let root = log.insert(genesis(&founder, profile)).expect("genesis");

    let admitted = log
        .insert(LogEntry::create(
            &founder,
            Some(root),
            Timestamp::from_millis(1),
            EntryBody::MembershipChange {
                group: GroupId::everyone(),
                identity: member.id(),
                action: MembershipAction::Add { via_invite: None },
            },
        ))
        .expect("admits");

    let group = GroupId::new("grantees");
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
        .expect("defines");

    log.insert(LogEntry::create(
        &founder,
        Some(defined),
        Timestamp::from_millis(3),
        EntryBody::MembershipChange {
            group,
            identity: member.id(),
            action: MembershipAction::Add { via_invite: None },
        },
    ))
    .expect("joins");

    let chain: Vec<_> = log
        .canonical_chain()
        .iter()
        .filter_map(|hash| log.get(hash))
        .collect();
    GovernanceState::replay(chain).expect("replays")
}

fn extension(name: &str) -> Capability {
    Capability::extension(name.to_owned())
}

/// Both gates `may_post` asks about — `design/02` §2.1, Core §2.8.
fn poster() -> Vec<Capability> {
    vec![
        extension("chat:post:*"),
        Capability::publish(ContentType::new(kols_core::CHAT_LOG_CONTENT_TYPE)),
    ]
}

fn send(channel: ChannelId, body: &str) -> Command {
    Command::SendMessage {
        channel,
        body: body.to_owned(),
        reply_to: None,
        attachments: Vec::new(),
    }
}

fn check(
    state: &GovernanceState,
    who: &PerNetworkIdentity,
    command: Command,
) -> Result<Command, Refusal> {
    let authority = StateAuthority { state };
    let index = index();
    let actor = Actor {
        identity: who.id(),
        authority: &authority,
        state,
        channels: &index,
    };
    authorize(command, &actor).map(Authorized::into_command)
}

// --------------------------------------------------------------- permission

#[test]
fn a_poster_may_post() {
    let member = person(2);
    let state = network_granting(&member, poster());
    assert!(check(&state, &member, send(channel(1), "hello")).is_ok());
}

#[test]
fn posting_needs_both_gates() {
    // `chat:post` without `publish:chat-log` is Core §2.8's two gates, and the
    // failure the CLI's genesis notes: the right to write a log at all is
    // separate from the right to write this channel's.
    let member = person(2);
    let state = network_granting(&member, [extension("chat:post:*")]);
    assert!(matches!(
        check(&state, &member, send(channel(1), "hello")),
        Err(Refusal::NotPermitted { .. })
    ));
}

#[test]
fn a_poster_may_not_pin() {
    let member = person(2);
    let state = network_granting(&member, poster());
    assert!(matches!(
        check(
            &state,
            &member,
            Command::Pin {
                channel: channel(1),
                target: MessageId::from_bytes([0u8; 32]),
                remove: false,
            },
        ),
        Err(Refusal::NotPermitted {
            needs: "chat:moderate",
            ..
        })
    ));
}

#[test]
fn a_moderator_may_pin() {
    let member = person(2);
    let state = network_granting(&member, [extension("chat:moderate:*")]);
    assert!(
        check(
            &state,
            &member,
            Command::Pin {
                channel: channel(1),
                target: MessageId::from_bytes([0u8; 32]),
                remove: false,
            },
        )
        .is_ok()
    );
}

#[test]
fn a_category_grant_reaches_that_category_and_no_further() {
    let member = person(2);
    let cat = intranet_crypto::to_hex(category().as_bytes());
    let state = network_granting(
        &member,
        [
            extension(&format!("chat:post:cat:{cat}")),
            Capability::publish(ContentType::new(kols_core::CHAT_LOG_CONTENT_TYPE)),
        ],
    );
    assert!(
        check(&state, &member, send(channel(2), "inside")).is_ok(),
        "the channel in the category is covered"
    );
    assert!(
        matches!(
            check(&state, &member, send(channel(1), "outside")),
            Err(Refusal::NotPermitted { .. })
        ),
        "a channel outside it is not"
    );
}

#[test]
fn a_channel_override_reaches_one_channel() {
    let member = person(2);
    let one = intranet_crypto::to_hex(channel(1).as_bytes());
    let state = network_granting(
        &member,
        [
            extension(&format!("chat:post:{one}")),
            Capability::publish(ContentType::new(kols_core::CHAT_LOG_CONTENT_TYPE)),
        ],
    );
    assert!(check(&state, &member, send(channel(1), "here")).is_ok());
    assert!(matches!(
        check(&state, &member, send(channel(2), "not here")),
        Err(Refusal::NotPermitted { .. })
    ));
}

#[test]
fn creating_a_channel_resolves_at_scope_not_at_the_channel() {
    // The id does not exist until the entry that mints it, so only a category or
    // network grant can ever authorize a definition.
    let member = person(2);
    let state = network_granting(&member, [extension("chat:create-channel:*")]);
    assert!(
        check(
            &state,
            &member,
            Command::CreateChannel {
                name: "general".to_owned(),
                category: None,
                privacy: Privacy::Public,
                topic: String::new(),
            },
        )
        .is_ok()
    );
}

#[test]
fn admission_is_the_protocols_own_capability() {
    let member = person(2);
    let joiner = person(3);

    let without = network_granting(&member, poster());
    assert!(matches!(
        check(
            &without,
            &member,
            Command::AdmitMember {
                identity: joiner.id()
            },
        ),
        Err(Refusal::NotPermitted {
            needs: "approve-node",
            ..
        })
    ));

    let with = network_granting(&member, [Capability::ApproveNode]);
    assert!(
        check(
            &with,
            &member,
            Command::AdmitMember {
                identity: joiner.id()
            },
        )
        .is_ok()
    );
}

#[test]
fn an_unknown_channel_is_refused_before_any_permission_question() {
    let member = person(2);
    let state = network_granting(&member, poster());
    assert_eq!(
        check(&state, &member, send(channel(99), "hello")),
        Err(Refusal::NoSuchChannel(channel(99)))
    );
}

// ------------------------------------------------------------------- bounds

#[test]
fn an_empty_message_is_refused() {
    let member = person(2);
    let state = network_granting(&member, poster());
    assert_eq!(
        check(&state, &member, send(channel(1), "   ")),
        Err(Refusal::Empty("message"))
    );
}

#[test]
fn a_message_over_the_networks_ceiling_is_refused_here_first() {
    // `design/01` §10.2: the author's own client enforces the ceiling, so a user
    // is told rather than watching every reader silently refuse the record.
    let member = person(2);
    let state = network_granting(&member, poster());
    let ceiling = kols_core::ChatPolicy::of(&state.policy).message_max_bytes();
    assert!(matches!(
        check(&state, &member, send(channel(1), &"x".repeat(ceiling + 1))),
        Err(Refusal::TooLarge { field: "message", limit, .. }) if limit == ceiling
    ));
}

#[test]
fn too_many_attachments_are_refused() {
    let member = person(2);
    let state = network_granting(&member, poster());
    let limit = kols_core::ChatPolicy::of(&state.policy).attachment_max_count();
    let attachment = Attachment {
        manifest_cid: intranet_crypto::Hash::from_bytes([1u8; 32]),
        byte_len: 1,
        media_type: "image/png".to_owned(),
        name: "a.png".to_owned(),
    };
    assert!(matches!(
        check(
            &state,
            &member,
            Command::SendMessage {
                channel: channel(1),
                body: "look".to_owned(),
                reply_to: None,
                attachments: vec![attachment; limit + 1],
            },
        ),
        Err(Refusal::TooMany {
            field: "attachments",
            ..
        })
    ));
}

#[test]
fn slowmode_is_bounded_by_the_networks_ceiling() {
    let member = person(2);
    let state = network_granting(&member, [extension("chat:manage-channel:*")]);
    let ceiling = kols_core::ChatPolicy::of(&state.policy).slowmode_max_seconds();
    assert!(matches!(
        check(
            &state,
            &member,
            Command::UpdateChannel {
                channel: channel(1),
                change: ChannelChange::SetSlowmode(ceiling as u32 + 1),
            },
        ),
        Err(Refusal::TooLarge {
            field: "slowmode",
            ..
        })
    ));
}

#[test]
fn a_conversation_network_has_no_channels_to_create() {
    // `design/03` §4.1, enforced rather than presented.
    let member = person(2);
    let state = network_granting_in(
        &member,
        [extension("chat:create-channel:*")],
        Some("conversation"),
    );
    assert_eq!(
        check(
            &state,
            &member,
            Command::CreateChannel {
                name: "general".to_owned(),
                category: None,
                privacy: Privacy::Public,
                topic: String::new(),
            },
        ),
        Err(Refusal::NotAServer)
    );
}

// ------------------------------------------------------------------ consent

fn every_command() -> Vec<Command> {
    let target = MessageId::from_bytes([0u8; 32]);
    vec![
        Command::OpenChannel {
            channel: channel(1),
            before: None,
            limit: 50,
        },
        send(channel(1), "hello"),
        Command::EditMessage {
            channel: channel(1),
            target,
            body: "revised".to_owned(),
        },
        Command::DeleteMessage {
            channel: channel(1),
            target,
        },
        Command::React {
            channel: channel(1),
            target,
            key: "+1".to_owned(),
            remove: false,
        },
        Command::Pin {
            channel: channel(1),
            target,
            remove: false,
        },
        Command::CreateChannel {
            name: "general".to_owned(),
            category: None,
            privacy: Privacy::Public,
            topic: String::new(),
        },
        Command::UpdateChannel {
            channel: channel(1),
            change: ChannelChange::Archive,
        },
        Command::AdmitMember {
            identity: person(3).id(),
        },
        Command::RevokeMember {
            identity: person(3).id(),
        },
    ]
}

#[test]
fn sensitivity_agrees_with_the_capability_vocabulary() {
    // The drift test that gives `Sensitivity` its meaning. Re-tiering a verb in
    // `design/02` §2.2 and forgetting this classification fails here, rather
    // than in a consent prompt that quietly stopped appearing.
    let tiers: std::collections::BTreeMap<&str, Tier> =
        kols_core::capabilities::VERBS.iter().copied().collect();

    for command in every_command() {
        let Some(verb) = command.verb() else {
            assert_eq!(
                command.sensitivity(),
                Sensitivity::Governs,
                "{} is gated on a protocol governance capability",
                command.name()
            );
            continue;
        };
        let tier = tiers
            .get(verb)
            .unwrap_or_else(|| panic!("chat:{verb} is registered"));
        let expected = match (tier, command.sensitivity()) {
            (Tier::Governance, _) => Sensitivity::Governs,
            // Reading signs nothing; every other ordinary verb writes a record.
            (Tier::Ordinary, Sensitivity::Local) => Sensitivity::Local,
            (Tier::Ordinary, _) => Sensitivity::Signs,
        };
        assert_eq!(
            command.sensitivity(),
            expected,
            "{} needs chat:{verb}, which is {tier:?}",
            command.name()
        );
    }
}

#[test]
fn only_reads_are_local() {
    // App Hosting §3.3 as a test: a sandboxed build satisfies it by prompting
    // for everything that is not `Local`, so anything that signs must not be.
    for command in every_command() {
        let signs = !matches!(command, Command::OpenChannel { .. });
        assert_eq!(
            command.sensitivity() != Sensitivity::Local,
            signs,
            "{} signs: {signs}",
            command.name()
        );
    }
}

#[test]
fn every_command_names_itself() {
    let mut names: Vec<_> = every_command().iter().map(Command::name).collect();
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), count, "two commands share a name");
}

#[test]
fn placement_comes_from_replay_not_from_the_caller() {
    // A category grant is reached because replay says the channel is in that
    // category — not because anything crossing the boundary said so. There is no
    // field in which to say it.
    let member = person(2);
    let cat = intranet_crypto::to_hex(category().as_bytes());
    let state = network_granting(
        &member,
        [
            extension(&format!("chat:post:cat:{cat}")),
            Capability::publish(ContentType::new(kols_core::CHAT_LOG_CONTENT_TYPE)),
        ],
    );

    // Channel 1 is outside the category in `index()`, and refused there.
    assert!(matches!(
        check(&state, &member, send(channel(1), "hello")),
        Err(Refusal::NotPermitted { .. })
    ));

    // The same command, against replay that puts channel 1 inside it.
    let authority = StateAuthority { state: &state };
    let mut moved = PlacementMap::new();
    moved.insert(
        channel(1),
        Placement {
            channel: channel(1),
            category: Some(category()),
        },
    );
    let actor = Actor {
        identity: member.id(),
        authority: &authority,
        state: &state,
        channels: &moved,
    };
    assert!(authorize(send(channel(1), "hello"), &actor).is_ok());
}
