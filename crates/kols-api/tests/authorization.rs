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
    let names = kols_core::Names::new();
    let actor = Actor {
        identity: who.id(),
        authority: &authority,
        state,
        channels: &index,
        names: &names,
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

// ------------------------------------------------------------- permissions

#[test]
fn granting_needs_define_group() {
    // `design/02` §1's asymmetry, at the gate: deciding what a role *can do* is
    // a higher bar than deciding who holds it, and holding every chat verb in
    // the vocabulary buys neither.
    let member = person(2);
    let state = network_granting(&member, [extension("chat:manage-channel:*")]);
    assert!(matches!(
        check(
            &state,
            &member,
            Command::SetPermission {
                group: GroupId::new("grantees"),
                verb: "post".to_owned(),
                scope: kols_core::Scope::Network,
                grant: true,
            },
        ),
        Err(Refusal::NotPermitted { .. })
    ));
}

#[test]
fn a_define_group_holder_may_grant() {
    let member = person(2);
    let state = network_granting(&member, [Capability::DefineGroup]);
    assert!(
        check(
            &state,
            &member,
            Command::SetPermission {
                group: GroupId::new("grantees"),
                verb: "post".to_owned(),
                scope: kols_core::Scope::Network,
                grant: true,
            },
        )
        .is_ok()
    );
}

#[test]
fn everyone_may_never_be_granted_a_governance_tier_verb() {
    // Core §2.4's hardcoded ceiling, refused before an entry every node would
    // reject is signed. The protocol enforces it too — this is the client
    // declining to spend a governance entry finding that out.
    let member = person(2);
    let state = network_granting(&member, [Capability::DefineGroup]);
    for verb in ["manage-channel", "moderate"] {
        assert!(
            matches!(
                check(
                    &state,
                    &member,
                    Command::SetPermission {
                        group: GroupId::everyone(),
                        verb: verb.to_owned(),
                        scope: kols_core::Scope::Network,
                        grant: true,
                    },
                ),
                Err(Refusal::EveryoneCeiling { .. })
            ),
            "chat:{verb} is governance-tier and must not reach everyone"
        );
    }
}

#[test]
fn everyone_may_be_granted_an_ordinary_verb() {
    // The other half, and the one that would be lost by refusing on the group
    // rather than on the tier: `everyone` holding `chat:post` is the ordinary
    // configuration, not an exception to a rule.
    let member = person(2);
    let state = network_granting(&member, [Capability::DefineGroup]);
    assert!(
        check(
            &state,
            &member,
            Command::SetPermission {
                group: GroupId::everyone(),
                verb: "post".to_owned(),
                scope: kols_core::Scope::Network,
                grant: true,
            },
        )
        .is_ok()
    );
}

#[test]
fn a_governance_tier_verb_may_be_taken_off_everyone() {
    // Withdrawal is not refused where granting is. Refusing both would make the
    // invariant unfixable on a network that somehow has such a grant — the
    // repair would be the one action the client would not perform.
    let member = person(2);
    let state = network_granting(&member, [Capability::DefineGroup]);
    assert!(
        check(
            &state,
            &member,
            Command::SetPermission {
                group: GroupId::everyone(),
                verb: "moderate".to_owned(),
                scope: kols_core::Scope::Network,
                grant: false,
            },
        )
        .is_ok()
    );
}

#[test]
fn a_verb_outside_the_vocabulary_is_refused() {
    // An unregistered extension name is refused at replay (Core §2.2.1), so a
    // typo would otherwise produce a grant that resolves for nobody and reports
    // nothing — an absent grant and a misspelled one are the same observation.
    let member = person(2);
    let state = network_granting(&member, [Capability::DefineGroup]);
    assert!(matches!(
        check(
            &state,
            &member,
            Command::SetPermission {
                group: GroupId::new("grantees"),
                verb: "pots".to_owned(),
                scope: kols_core::Scope::Network,
                grant: true,
            },
        ),
        Err(Refusal::UnknownVerb(_))
    ));
}

#[test]
fn an_unrestricted_role_is_not_edited_a_verb_at_a_time() {
    // `Founders` holds `CapabilitySet::All`, which includes capabilities defined
    // later. There is no explicit set to take one out of, and replacing `All`
    // with whatever a checkbox enumerated would silently drop the rest.
    let member = person(2);
    let state = network_granting(&member, [Capability::DefineGroup]);
    assert!(matches!(
        check(
            &state,
            &member,
            Command::SetPermission {
                group: GroupId::founders(),
                verb: "post".to_owned(),
                scope: kols_core::Scope::Network,
                grant: false,
            },
        ),
        Err(Refusal::Unrestricted { .. })
    ));
}

#[test]
fn creating_a_role_that_exists_is_refused() {
    // `DefineGroup` both creates and redefines, so without this a "create" would
    // replace an existing role's capability set with an empty one — the
    // destructive reading of a button that says it adds something.
    let member = person(2);
    let state = network_granting(&member, [Capability::DefineGroup]);
    assert!(matches!(
        check(
            &state,
            &member,
            Command::CreateRole {
                group: GroupId::new("grantees"),
            },
        ),
        Err(Refusal::RoleExists(_))
    ));
}

#[test]
fn assigning_a_role_needs_manage_membership_for_that_role() {
    // The dynamically-tiered capability (Core §2.4). `define-group` is the
    // higher bar and deliberately does not imply this one: what a role can do
    // and who holds it are separate questions with separate answers.
    let member = person(2);
    let state = network_granting(&member, [Capability::DefineGroup]);
    assert!(matches!(
        check(
            &state,
            &member,
            Command::SetRoleMember {
                group: GroupId::new("grantees"),
                identity: person(3).id(),
                member: true,
            },
        ),
        Err(Refusal::NotPermitted { .. })
    ));

    let holder = person(4);
    let state = network_granting(
        &holder,
        [Capability::manage_membership(GroupId::new("grantees"))],
    );
    assert!(
        check(
            &state,
            &holder,
            Command::SetRoleMember {
                group: GroupId::new("grantees"),
                identity: person(3).id(),
                member: true,
            },
        )
        .is_ok()
    );
}

#[test]
fn a_grant_is_named_the_way_resolution_reads_it() {
    // The property the whole feature rests on, and the one nothing else would
    // catch: a grant that spelled its scope differently from the resolver would
    // resolve for nobody, and an absent grant looks exactly like a misspelled
    // one. `Scope::capability` is the single construction both sides use, so
    // this pins that they agree rather than that the string looks right.
    let member = person(2);
    for (scope, expected) in [
        (kols_core::Scope::Network, "chat:post:*".to_owned()),
        (
            kols_core::Scope::Category(category()),
            format!("chat:post:cat:{}", intranet_crypto::to_hex(category().as_bytes())),
        ),
        (
            kols_core::Scope::Channel(channel(2)),
            format!("chat:post:{}", intranet_crypto::to_hex(channel(2).as_bytes())),
        ),
    ] {
        assert_eq!(scope.name("post"), expected);

        // And the resolver finds it, granted under exactly that name.
        let state = network_granting(&member, [scope.capability("post"), poster()[1].clone()]);
        assert!(
            kols_core::holds(
                &state,
                &member.id(),
                "post",
                &placement(channel(2), Some(category())),
            ),
            "a grant at {scope:?} must resolve for the channel it covers"
        );
    }
}

#[test]
fn a_network_name_is_bounded() {
    let member = person(2);
    let state = network_granting(&member, [Capability::DefinePolicy]);
    assert!(matches!(
        check(
            &state,
            &member,
            Command::SetNetworkName {
                name: "n".repeat(kols_core::MAX_NETWORK_NAME_BYTES + 1),
            },
        ),
        Err(Refusal::TooLarge { .. })
    ));
    // Empty is a real state rather than a refusal: spec 07 §1.7 says a network
    // with no name declared has no name, and clients must not invent one.
    assert!(
        check(
            &state,
            &member,
            Command::SetNetworkName {
                name: String::new(),
            },
        )
        .is_ok()
    );
}

#[test]
fn changing_a_setting_needs_define_policy() {
    let member = person(2);
    let state = network_granting(&member, [extension("chat:manage-channel:*")]);
    assert!(matches!(
        check(
            &state,
            &member,
            Command::SetChatSetting {
                setting: kols_core::ChatSetting::MessageRate,
                value: 10,
            },
        ),
        Err(Refusal::NotPermitted { .. })
    ));
}

#[test]
fn a_negative_setting_is_refused_and_zero_is_not() {
    // Negative is refused because storing it does nothing visible: every reader
    // falls back to the default on a value it cannot use, so the change would be
    // replayed forever and read as the number it was meant to replace.
    //
    // Zero is allowed for every one of these, and means something different for
    // each — no ceiling, forever, or a real bound of zero. Refusing it would
    // take away the only way a network can say "no attachments".
    let member = person(2);
    let state = network_granting(&member, [Capability::DefinePolicy]);

    for setting in kols_core::ChatSetting::ALL {
        assert!(
            matches!(
                check(
                    &state,
                    &member,
                    Command::SetChatSetting { setting, value: -1 },
                ),
                Err(Refusal::Negative { .. })
            ),
            "{setting:?} must refuse a negative"
        );
        assert!(
            check(
                &state,
                &member,
                Command::SetChatSetting { setting, value: 0 },
            )
            .is_ok(),
            "{setting:?} must accept zero"
        );
    }
}

#[test]
fn every_setting_has_a_distinct_namespaced_key() {
    // The property `ChatSetting` exists for. An app-policy key is *not* refused
    // when unrecognised (Core §2.6.2), so a duplicated or mistyped key would be
    // written, replayed and silently ignored — with the setting it was meant to
    // change still reading as its default. Nothing else would catch it.
    let mut keys = std::collections::BTreeSet::new();
    for setting in kols_core::ChatSetting::ALL {
        assert!(
            keys.insert(setting.key()),
            "{setting:?} shares a key with another setting"
        );
        assert!(
            setting.key().starts_with("chat:"),
            "{setting:?} must be namespaced, or two applications collide"
        );
    }
    assert_eq!(keys.len(), kols_core::ChatSetting::ALL.len());
}

#[test]
fn auto_admit_is_refused_where_a_vote_decides_admission() {
    // Core §2.6's one incompatible pairing, refused where policy is set rather
    // than where a joiner is turned away. The protocol refuses it on replay too;
    // this is the client declining to spend a governance entry finding out.
    let member = person(2);
    let mut state = network_granting(&member, [Capability::DefinePolicy]);
    state.policy.governance_model = intranet_governance::GovernanceModel::MemberVote {
        electorate: GroupId::everyone(),
        quorum: 2,
        window_millis: 72 * 3_600_000,
    };

    assert!(matches!(
        check(
            &state,
            &member,
            Command::SetAdmissionMode {
                mode: intranet_governance::AdmissionMode::AutoAdmit,
            },
        ),
        Err(Refusal::IncoherentAdmission)
    ));
    // Explicit intake is the coherent pairing and stays available — refusing
    // both would leave a member-vote network unable to state its own mode.
    assert!(
        check(
            &state,
            &member,
            Command::SetAdmissionMode {
                mode: intranet_governance::AdmissionMode::ExplicitIntake,
            },
        )
        .is_ok()
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
        Command::SetName {
            name: "ada".to_owned(),
        },
        Command::CreateCategory {
            name: "staff".to_owned(),
            position: 0,
        },
        Command::UpdateCategory {
            category: CategoryId::from_bytes([4u8; 32]),
            change: kols_core::CategoryChange::Delete,
        },
        Command::CreateInvite {
            uses: 1,
            valid_for_hours: 24,
        },
        Command::SetBootstrapRelays {
            relays: vec!["/ip4/198.51.100.7/tcp/4001".to_owned()],
        },
        Command::AdmitMember {
            identity: person(3).id(),
        },
        Command::RevokeMember {
            identity: person(3).id(),
        },
        Command::SetNetworkName {
            name: "the workshop".to_owned(),
        },
        Command::SetChatSetting {
            setting: kols_core::ChatSetting::MessageRate,
            value: 20,
        },
        Command::SetAdmissionMode {
            mode: intranet_governance::AdmissionMode::AutoAdmit,
        },
        Command::CreateRole {
            group: GroupId::new("Moderators"),
        },
        Command::SetPermission {
            group: GroupId::new("Moderators"),
            verb: "moderate".to_owned(),
            scope: kols_core::Scope::Network,
            grant: true,
        },
        Command::SetRoleMember {
            group: GroupId::new("Moderators"),
            identity: person(3).id(),
            member: true,
        },
    ]
}

/// Fails to compile when a variant is added to `Command`.
///
/// `every_command` above is hand-written and can therefore drift from the enum
/// it claims to sample — which it had, silently, by four commands, until this
/// was written. That is the same drift `design/05` §3 records against its own
/// boundary list, and it has the same fix: check it mechanically, which is
/// cheap, rather than by intention, which is not reliable.
///
/// The match is exhaustive and carries no wildcard arm, so a new command stops
/// this suite compiling until somebody has read this. **If you added an arm
/// here, add a sample to `every_command`** — the consent classification is only
/// tested for the commands that list contains.
fn _every_variant_is_sampled(command: &Command) {
    match command {
        Command::OpenChannel { .. }
        | Command::SendMessage { .. }
        | Command::EditMessage { .. }
        | Command::DeleteMessage { .. }
        | Command::React { .. }
        | Command::Pin { .. }
        | Command::CreateChannel { .. }
        | Command::UpdateChannel { .. }
        | Command::CreateCategory { .. }
        | Command::UpdateCategory { .. }
        | Command::SetName { .. }
        | Command::CreateInvite { .. }
        | Command::SetBootstrapRelays { .. }
        | Command::AdmitMember { .. }
        | Command::RevokeMember { .. }
        | Command::SetNetworkName { .. }
        | Command::SetChatSetting { .. }
        | Command::SetAdmissionMode { .. }
        | Command::CreateRole { .. }
        | Command::SetPermission { .. }
        | Command::SetRoleMember { .. } => (),
    }
}

#[test]
fn every_command_has_a_sample() {
    // The other half of `_every_variant_is_sampled`: that one makes adding a
    // variant impossible to miss, and this one makes leaving it out of the list
    // impossible to miss. Neither alone is enough — the first compiles happily
    // with the sample list untouched.
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for command in every_command() {
        assert!(
            seen.insert(command.name()),
            "{} is sampled twice, which hides whichever one drifts",
            command.name()
        );
    }
    assert_eq!(
        seen.len(),
        21,
        "every_command samples {} of Command's variants — update both this count \
         and the list when the boundary grows",
        seen.len()
    );
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
    let names = kols_core::Names::new();
    let actor = Actor {
        identity: member.id(),
        authority: &authority,
        state: &state,
        channels: &moved,
        names: &names,
    };
    assert!(authorize(send(channel(1), "hello"), &actor).is_ok());
}

#[test]
fn a_name_already_held_is_refused_at_the_boundary() {
    // The uniqueness question is answered from replay, like the channel index
    // beside it — so an interface cannot reach past it by claiming otherwise.
    let member = person(2);
    let other = person(4);
    let state = network_granting(&member, [extension("chat:set-name:*")]);

    let mut names = kols_core::Names::new();
    names.apply(other.id(), &kols_core::NameClaim::new("ada").expect("valid"));

    let authority = StateAuthority { state: &state };
    let index = index();
    let actor = Actor {
        identity: member.id(),
        authority: &authority,
        state: &state,
        channels: &index,
        names: &names,
    };

    let taken = authorize(
        Command::SetName {
            name: "ADA".to_owned(), // same key, different spelling
        },
        &actor,
    );
    assert!(
        matches!(taken, Err(Refusal::Name(kols_core::NameRefusal::Taken { .. }))),
        "a held name was claimable"
    );

    assert!(
        authorize(
            Command::SetName {
                name: "grace".to_owned()
            },
            &actor
        )
        .is_ok()
    );
}

#[test]
fn claiming_a_name_needs_the_capability_everyone_is_given() {
    let member = person(2);
    let state = network_granting(&member, poster()); // no chat:set-name
    let names = kols_core::Names::new();
    let authority = StateAuthority { state: &state };
    let index = index();
    let actor = Actor {
        identity: member.id(),
        authority: &authority,
        state: &state,
        channels: &index,
        names: &names,
    };

    assert!(matches!(
        authorize(
            Command::SetName {
                name: "ada".to_owned()
            },
            &actor
        ),
        Err(Refusal::NotPermitted { .. })
    ));
}

// ---------------------------------------------------------------- categories

fn create_category() -> Command {
    Command::CreateCategory {
        name: "Ops".to_owned(),
        position: 0,
    }
}

#[test]
fn creating_a_category_needs_the_network_wide_grant() {
    let member = person(2);
    let state = network_granting(
        &member,
        [Capability::extension("chat:manage-channel:*".to_owned())],
    );
    assert!(check(&state, &member, create_category()).is_ok());
}

#[test]
fn a_category_scoped_grant_cannot_create_a_category() {
    // Spec 07 §1.8: a definition is scoped `*` or not at all. Scoping one to the
    // category it defines is circular — that scope becomes grantable only once
    // the entry exists — so holding every category grant in the network still
    // does not let somebody mint a new one.
    let member = person(2);
    let cat = intranet_crypto::to_hex(category().as_bytes());
    let state = network_granting(
        &member,
        [Capability::extension(format!("chat:manage-channel:cat:{cat}"))],
    );
    assert!(matches!(
        check(&state, &member, create_category()),
        Err(Refusal::NotPermitted { .. })
    ));
}

#[test]
fn creating_a_channel_does_not_let_somebody_create_a_category() {
    // `create-channel` is Ordinary and expected to be granted widely. Categories
    // are the furniture permissions bind to, and ride the governance-tier verb.
    let member = person(2);
    let state = network_granting(
        &member,
        [Capability::extension("chat:create-channel:*".to_owned())],
    );
    assert!(matches!(
        check(&state, &member, create_category()),
        Err(Refusal::NotPermitted { .. })
    ));
}

#[test]
fn updating_a_category_accepts_its_own_scope() {
    let member = person(2);
    let cat = intranet_crypto::to_hex(category().as_bytes());
    let state = network_granting(
        &member,
        [Capability::extension(format!("chat:manage-channel:cat:{cat}"))],
    );
    assert!(
        check(
            &state,
            &member,
            Command::UpdateCategory {
                category: category(),
                change: kols_core::CategoryChange::SetPosition(4),
            },
        )
        .is_ok()
    );
}

#[test]
fn category_commands_are_governance_tier() {
    // D26: a command's consent class follows the tier of the capability it needs,
    // not how consequential making a folder feels.
    assert_eq!(create_category().sensitivity(), Sensitivity::Governs);
    assert_eq!(
        Command::UpdateCategory {
            category: category(),
            change: kols_core::CategoryChange::Delete,
        }
        .sensitivity(),
        Sensitivity::Governs,
    );
}

#[test]
fn a_conversation_network_refuses_categories() {
    // The profile rule reaches structure of every kind: a conversation has one
    // implied channel and nothing to file it under.
    let member = person(2);
    let state = network_granting_in(
        &member,
        [Capability::extension("chat:manage-channel:*".to_owned())],
        Some("conversation"),
    );
    assert!(matches!(
        check(&state, &member, create_category()),
        Err(Refusal::NotAServer)
    ));
}

#[test]
fn an_empty_category_name_is_refused() {
    let member = person(2);
    let state = network_granting(
        &member,
        [Capability::extension("chat:manage-channel:*".to_owned())],
    );
    assert!(matches!(
        check(
            &state,
            &member,
            Command::CreateCategory {
                name: "   ".to_owned(),
                position: 0,
            },
        ),
        Err(Refusal::Empty("category name"))
    ));
}
