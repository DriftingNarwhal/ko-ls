//! Roles, grants and assignment, through the executor — `design/02` §1–§4.
//!
//! # Why these run in-process
//!
//! Unlike `records.rs` and `flow.rs`, nothing here needs a second process or a
//! wire. What is under test is that a grant written by [`Command::SetPermission`]
//! is the same grant [`kols_core::holds`] resolves — a question about one
//! store's replay, and one this can ask directly by building a network and
//! reading state back.
//!
//! That property is the one nothing else would catch. A capability is a string
//! in a group's set, so a writer that spelled a scope differently from the
//! resolver produces a grant that resolves for nobody and reports no error at
//! any layer: an absent grant and a misspelled one are the same observation
//! (`design/02` §3 — denials are absent grants, never negative ones).

use intranet_governance::{Capability, GroupId, Tier};
use kols_api::Command;
use kols_core::{Placement, Scope, holds};
use kols_node::executor::Executor;
use kols_node::workspace::Workspace;
use std::path::PathBuf;

/// A scratch workspace holding one network, removed on drop.
///
/// `Drop` rather than the end of a test, because `Drop` runs on an unwind and a
/// failing test is exactly when scratch would otherwise be left behind — this
/// container's storage is the host's.
struct Lab {
    root: PathBuf,
    executor: Executor,
}

impl Lab {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "kols-perm-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("makes a workspace");

        let workspace = Workspace::at(root.clone());
        let store = workspace
            .create_at(root.join("net"), name, Vec::new())
            .expect("creates a network");
        let executor = Executor::open(store.root().to_path_buf()).expect("opens");
        Self { root, executor }
    }

    fn state(&self) -> intranet_governance::GovernanceState {
        self.executor.store().state().expect("replays")
    }

    fn me(&self) -> intranet_identity::PerNetworkIdentityId {
        self.executor.store().identity().expect("has a seed").id()
    }

    fn run(&self, command: Command) -> Result<(), String> {
        self.executor
            .submit(command)
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    /// A member who is not the founder, admitted to `everyone`.
    ///
    /// **Resolution must be asked about somebody ordinary.** The founder is the
    /// sole member of `Founders`, which holds `CapabilitySet::All` — so every
    /// `holds` question about them answers true whatever any role does, and a
    /// test using them would pass its positive assertions for the wrong reason
    /// and fail every negative one. Found by writing it that way first.
    fn subject(&self) -> intranet_identity::PerNetworkIdentityId {
        let network = self.executor.store().network();
        let who = intranet_identity::MasterSeed::from_entropy([77u8; 32])
            .identity_for(network)
            .expect("derives")
            .id();
        if !self.state().is_member(&who) {
            self.run(Command::AdmitMember { identity: who })
                .expect("admits");
        }
        who
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn role(name: &str) -> GroupId {
    GroupId::new(name)
}

fn grant(group: &GroupId, verb: &str, scope: Scope, grant: bool) -> Command {
    Command::SetPermission {
        group: group.clone(),
        verb: verb.to_owned(),
        scope,
        grant,
    }
}

#[test]
fn a_new_role_holds_nothing() {
    // Both senses of nothing, and deliberately: `define-group` and
    // `manage-membership` are separate acts at separate bars (Core §2.2), so a
    // command that created a role *and* filled it would collapse an asymmetry
    // the protocol is built around.
    let lab = Lab::new("empty");
    lab.run(Command::CreateRole { group: role("Mods") })
        .expect("creates");

    let state = lab.state();
    let made = state.groups.get(&role("Mods")).expect("replay produced it");
    assert!(
        matches!(&made.capabilities, intranet_governance::CapabilitySet::Explicit(set) if set.is_empty()),
        "a new role holds no capabilities"
    );
    assert!(made.members.is_empty(), "a new role holds no members");
}

#[test]
fn a_grant_resolves_at_the_scope_it_was_written_at() {
    // The property the whole feature rests on. Written through the command and
    // read through the resolver, so the two spellings must agree — a test that
    // built the capability itself and asserted on the string would pass while
    // the two drifted.
    let lab = Lab::new("scopes");
    let channel = kols_core::ChannelId::from_bytes([7u8; 32]);
    let category = kols_core::CategoryId::from_bytes([8u8; 32]);
    let elsewhere = kols_core::ChannelId::from_bytes([9u8; 32]);

    let here = Placement {
        channel,
        category: Some(category),
    };
    let there = Placement {
        channel: elsewhere,
        category: None,
    };

    let who = lab.subject();
    lab.run(Command::CreateRole { group: role("Team") })
        .expect("creates");
    lab.run(Command::SetRoleMember {
        group: role("Team"),
        identity: who,
        member: true,
    })
    .expect("joins");

    // `moderate` deliberately: `everyone` holds post, read and set-name at
    // genesis, so a scope test written on one of those would resolve true from
    // the network default and prove nothing about the scope under test.
    assert!(
        !holds(&lab.state(), &who, "moderate", &here),
        "the subject starts holding nothing here"
    );

    // Network scope reaches everywhere.
    lab.run(grant(&role("Team"), "moderate", Scope::Network, true))
        .expect("grants");
    assert!(holds(&lab.state(), &who, "moderate", &here));
    assert!(holds(&lab.state(), &who, "moderate", &there));
    lab.run(grant(&role("Team"), "moderate", Scope::Network, false))
        .expect("withdraws");
    assert!(!holds(&lab.state(), &who, "moderate", &here));

    // Category scope reaches the channels in it, and no others. This is
    // `design/02` §4's whole reason for existing, so it is asserted in both
    // directions rather than only the positive one.
    lab.run(grant(
        &role("Team"),
        "moderate",
        Scope::Category(category),
        true,
    ))
    .expect("grants");
    assert!(holds(&lab.state(), &who, "moderate", &here));
    assert!(
        !holds(&lab.state(), &who, "moderate", &there),
        "a category grant must not reach a channel outside it"
    );
    lab.run(grant(
        &role("Team"),
        "moderate",
        Scope::Category(category),
        false,
    ))
    .expect("withdraws");

    // Channel scope reaches exactly one channel.
    lab.run(grant(
        &role("Team"),
        "moderate",
        Scope::Channel(channel),
        true,
    ))
    .expect("grants");
    assert!(holds(&lab.state(), &who, "moderate", &here));
    assert!(
        !holds(&lab.state(), &who, "moderate", &there),
        "a channel grant must not reach another channel"
    );
}

#[test]
fn withdrawing_a_grant_leaves_the_others_alone() {
    // `DefineGroup` carries a whole capability set, so every edit is a
    // read-modify-write. The failure this guards is the one that would be
    // silent: writing back a set built from a stale read, and reverting a grant
    // nobody meant to touch.
    let lab = Lab::new("partial");
    lab.run(Command::CreateRole { group: role("Team") })
        .expect("creates");
    lab.run(Command::SetRoleMember {
        group: role("Team"),
        identity: lab.me(),
        member: true,
    })
    .expect("joins");

    for verb in ["post", "read", "moderate"] {
        lab.run(grant(&role("Team"), verb, Scope::Network, true))
            .expect("grants");
    }
    lab.run(grant(&role("Team"), "read", Scope::Network, false))
        .expect("withdraws");

    let state = lab.state();
    let held = state.groups.get(&role("Team")).expect("exists");
    for (verb, expected) in [("post", true), ("read", false), ("moderate", true)] {
        assert_eq!(
            held.capabilities
                .grants(&Scope::Network.capability(verb)),
            expected,
            "chat:{verb} after withdrawing only chat:read"
        );
    }
}

#[test]
fn a_role_member_is_added_and_removed_without_leaving_the_network() {
    // The distinction that made `change_membership` take a group: somebody
    // taken out of a role is still a member, so verifying the removal against
    // network membership would report a landed change as having failed — and
    // the wording would claim they had been removed from the network.
    let lab = Lab::new("assign");
    lab.run(Command::CreateRole { group: role("Team") })
        .expect("creates");
    lab.run(Command::SetRoleMember {
        group: role("Team"),
        identity: lab.me(),
        member: true,
    })
    .expect("joins");
    assert!(lab.state().groups[&role("Team")].contains(&lab.me()));

    lab.run(Command::SetRoleMember {
        group: role("Team"),
        identity: lab.me(),
        member: false,
    })
    .expect("leaves");
    assert!(!lab.state().groups[&role("Team")].contains(&lab.me()));
    assert!(
        lab.state().is_member(&lab.me()),
        "leaving a role is not leaving the network"
    );
}

// ---------------------------------------------------------------------------
// Leaving a network — Core §2.5.1, `design/02` §6.5
// ---------------------------------------------------------------------------

#[test]
fn the_only_revoke_node_holder_cannot_leave() {
    // The guard that replaced a blanket refusal of every self-removal. This is
    // the case that refusal *meant*: a network whose last `revoke-node` holder
    // walks out has nobody left who can rotate the epoch, so nobody who can
    // ever remove anybody — including the member who just left with a copy of
    // the key material.
    let lab = Lab::new("last-holder");
    let refusal = lab
        .run(Command::LeaveNetwork)
        .expect_err("the only founder holds the only `revoke-node`");
    assert!(
        refusal.contains("revoke-node"),
        "the refusal has to say which capability strands the network: {refusal}"
    );
    assert!(
        lab.state().is_member(&lab.me()),
        "a refused departure must leave membership exactly as it was"
    );
}

#[test]
fn a_member_who_is_not_the_last_holder_may_leave() {
    // The same founder, once somebody else can rotate. Nothing else changed —
    // which is the point of asking the question about the network rather than
    // about who is asking.
    let lab = Lab::new("hand-over");
    let heir = lab.subject();
    lab.run(Command::SetRoleMember {
        group: GroupId::founders(),
        identity: heir,
        member: true,
    })
    .expect("a founder may make another");

    lab.run(Command::LeaveNetwork).expect("now it is ordinary");
    assert!(
        !lab.state().is_member(&lab.me()),
        "leaving means out of every group, not out of one"
    );
    assert!(
        lab.state().is_member(&heir),
        "leaving is self-directed and touches nobody else"
    );
}

#[test]
fn leaving_writes_one_entry_per_group() {
    // Core §2.5.1 declines to make leaving `everyone` imply the rest, so this
    // is several entries rather than one. Asserted because the alternative —
    // writing only the `everyone` removal — leaves the departed member holding
    // roles in replayed state, which is the state the whole item exists to
    // avoid.
    let lab = Lab::new("every-group");
    let heir = lab.subject();
    lab.run(Command::SetRoleMember {
        group: GroupId::founders(),
        identity: heir,
        member: true,
    })
    .expect("hands over `revoke-node`");
    lab.run(Command::CreateRole { group: role("Team") })
        .expect("creates");
    lab.run(Command::SetRoleMember {
        group: role("Team"),
        identity: lab.me(),
        member: true,
    })
    .expect("joins");

    let held: Vec<GroupId> = lab
        .state()
        .groups
        .iter()
        .filter(|(_, found)| found.contains(&lab.me()))
        .map(|(id, _)| id.clone())
        .collect();
    // `Founders` and the role, and **not** `everyone` — genesis puts the
    // founder in the implicit root group and leaves `everyone` empty (Core
    // §2.3, §2.4). Worth asserting rather than assuming: a departure that only
    // wrote the `everyone` removal would be a no-op for exactly this member.
    assert_eq!(
        held,
        vec![GroupId::founders(), role("Team")],
        "the groups a founder is actually in"
    );

    lab.run(Command::LeaveNetwork).expect("leaves");
    for group in &held {
        assert!(
            !lab.state().groups[group].contains(&lab.me()),
            "still in {group} after leaving"
        );
    }
}

#[test]
fn stepping_out_of_one_role_is_not_leaving_and_is_not_guarded() {
    // The regression the new guard could most easily introduce. `would_strand`
    // asks whether the capability survives the departure, and a founder leaving
    // an ordinary role keeps it through `Founders` — so this must stay the
    // ordinary act it was, with no mention of stranding anything.
    let lab = Lab::new("step-down");
    lab.run(Command::CreateRole { group: role("Team") })
        .expect("creates");
    lab.run(Command::SetRoleMember {
        group: role("Team"),
        identity: lab.me(),
        member: true,
    })
    .expect("joins");

    lab.run(Command::SetRoleMember {
        group: role("Team"),
        identity: lab.me(),
        member: false,
    })
    .expect("stepping out of a role is not leaving the network");
    assert!(lab.state().is_member(&lab.me()));
}

#[test]
fn the_everyone_ceiling_holds_at_the_executor_too() {
    // The gate refuses this before anything is signed, and the protocol refuses
    // it on replay. Asserted here because those are two different mechanisms and
    // only one of them is this client's — a change that loosened the gate must
    // still not be able to produce the grant.
    let lab = Lab::new("ceiling");
    let refusal = lab
        .run(grant(
            &GroupId::everyone(),
            "manage-channel",
            Scope::Network,
            true,
        ))
        .expect_err("a governance-tier verb may never reach everyone");
    assert!(
        refusal.contains("governance"),
        "the refusal should say why, not merely refuse: {refusal}"
    );
    assert!(
        !lab.state().groups[&GroupId::everyone()]
            .capabilities
            .grants(&Scope::Network.capability("manage-channel")),
        "everyone must not hold it however the refusal was reported"
    );
}

#[test]
fn every_verb_in_the_vocabulary_can_be_granted_and_resolved() {
    // A sweep rather than a sample, because the failure mode is per verb: a
    // capability is a string, and one verb spelled wrong in one direction would
    // pass every test written about a different one.
    let lab = Lab::new("sweep");
    let channel = kols_core::ChannelId::from_bytes([3u8; 32]);
    let placement = Placement {
        channel,
        category: None,
    };
    let who = lab.subject();
    lab.run(Command::CreateRole { group: role("All") })
        .expect("creates");
    lab.run(Command::SetRoleMember {
        group: role("All"),
        identity: who,
        member: true,
    })
    .expect("joins");

    for (verb, _) in kols_core::capabilities::VERBS {
        lab.run(grant(&role("All"), verb, Scope::Channel(channel), true))
            .unwrap_or_else(|err| panic!("granting chat:{verb} failed: {err}"));

        // Both halves, because either alone is weak. The set check pins the
        // exact name the log now carries; the resolution check pins that the
        // resolver reads that same name. A verb `everyone` already holds would
        // satisfy the second on its own.
        let state = lab.state();
        assert!(
            state.groups[&role("All")]
                .capabilities
                .grants(&Scope::Channel(channel).capability(verb)),
            "chat:{verb} did not land in the role's set under the name it was written with"
        );
        assert!(
            holds(&state, &who, verb, &placement),
            "chat:{verb} was granted at this channel and did not resolve there"
        );
    }
}

#[test]
fn the_vocabulary_agrees_with_what_everyone_may_hold() {
    // The tier table is the one place a verb's tier lives, and the ceiling keys
    // off it. Pinned so that re-tiering a verb without thinking about `everyone`
    // fails here rather than in a network that quietly accepted the grant.
    let lab = Lab::new("tiers");
    for (verb, tier) in kols_core::capabilities::VERBS {
        let outcome = lab.run(grant(&GroupId::everyone(), verb, Scope::Network, true));
        match tier {
            Tier::Governance => assert!(
                outcome.is_err(),
                "chat:{verb} is governance-tier and must be refused on everyone"
            ),
            Tier::Ordinary => outcome
                .unwrap_or_else(|err| panic!("chat:{verb} is ordinary and was refused: {err}")),
        }
    }
}

#[test]
fn a_setting_round_trips_through_replay() {
    // Written by the command and read by the accessor, so the two must agree on
    // the key — which is the whole reason `ChatSetting::key` exists. A test that
    // wrote the key itself would pass while the accessor read a different one.
    let lab = Lab::new("settings");
    for setting in kols_core::ChatSetting::ALL {
        let value = setting.default_value() + 7;
        lab.run(Command::SetChatSetting { setting, value })
            .unwrap_or_else(|err| panic!("setting {setting:?} failed: {err}"));

        let state = lab.state();
        assert_eq!(
            state
                .policy
                .app_policy_int(setting.key(), setting.default_value()),
            value,
            "{setting:?} did not come back under the key it was written with"
        );
    }
}

#[test]
fn setting_a_value_back_to_its_default_removes_the_key() {
    // The default *is* absence (Core §2.6.2), so writing it explicitly would
    // freeze today's number into a network that would otherwise pick up a
    // revised one. Removing the key is the honest way to say "whatever this
    // application ships".
    let lab = Lab::new("default");
    let setting = kols_core::ChatSetting::MessageRate;

    lab.run(Command::SetChatSetting { setting, value: 5 })
        .expect("sets");
    assert!(lab.state().policy.app_policy(setting.key()).is_some());

    lab.run(Command::SetChatSetting {
        setting,
        value: setting.default_value(),
    })
    .expect("resets");
    assert!(
        lab.state().policy.app_policy(setting.key()).is_none(),
        "setting the default back must remove the key, not write it"
    );
    // And the effective value is unchanged either way, which is the point.
    assert_eq!(
        kols_core::ChatPolicy::of(&lab.state().policy).message_rate_per_minute(),
        setting.default_value()
    );
}

#[test]
fn one_setting_does_not_disturb_another() {
    // `PolicyChange` carries the whole record, so every edit is a
    // read-modify-write. This is the failure that would be silent: a second
    // setting written from a stale read, reverting the first.
    let lab = Lab::new("adjacent");
    lab.run(Command::SetChatSetting {
        setting: kols_core::ChatSetting::MessageRate,
        value: 11,
    })
    .expect("sets");
    lab.run(Command::SetChatSetting {
        setting: kols_core::ChatSetting::ReactionRate,
        value: 22,
    })
    .expect("sets");

    let policy = lab.state().policy;
    let chat = kols_core::ChatPolicy::of(&policy);
    assert_eq!(chat.message_rate_per_minute(), 11);
    assert_eq!(chat.reaction_rate_per_minute(), 22);
    // And nothing else in the record moved — relays are the neighbour most
    // likely to be lost, since they live in the same policy and are set from
    // the same panel.
    assert!(policy.bootstrap_relays.is_empty());
}

#[test]
fn retention_reads_zero_as_forever_and_a_rate_reads_it_as_no_limit() {
    // The asymmetry `ChatSetting::zero_means` exists to make sayable, asserted
    // against the accessors rather than against the comment. Spec 07 §2.8 fixes
    // the retention direction normatively — a corrupted value must not start
    // discarding history — and the rate check returns early on a non-positive
    // ceiling, which is the opposite reading of the same number.
    let lab = Lab::new("zero");
    for setting in [
        kols_core::ChatSetting::RetainMessagesDays,
        kols_core::ChatSetting::RetainAttachmentsDays,
    ] {
        lab.run(Command::SetChatSetting { setting, value: 0 })
            .expect("sets");
    }
    lab.run(Command::SetChatSetting {
        setting: kols_core::ChatSetting::MessageRate,
        value: 0,
    })
    .expect("sets");

    let policy = lab.state().policy;
    let chat = kols_core::ChatPolicy::of(&policy);
    assert_eq!(chat.retain_messages(), kols_core::Retention::Forever);
    assert_eq!(chat.retain_attachments(), kols_core::Retention::Forever);
    assert_eq!(chat.message_rate_per_minute(), 0);
}

#[test]
fn admission_mode_changes_and_comes_back_from_replay() {
    let lab = Lab::new("admission");
    assert_eq!(
        lab.state().policy.admission_mode,
        intranet_governance::AdmissionMode::ExplicitIntake,
        "a network is created conservative"
    );

    lab.run(Command::SetAdmissionMode {
        mode: intranet_governance::AdmissionMode::AutoAdmit,
    })
    .expect("switches");
    assert_eq!(
        lab.state().policy.admission_mode,
        intranet_governance::AdmissionMode::AutoAdmit
    );

    lab.run(Command::SetAdmissionMode {
        mode: intranet_governance::AdmissionMode::ExplicitIntake,
    })
    .expect("switches back");
    assert_eq!(
        lab.state().policy.admission_mode,
        intranet_governance::AdmissionMode::ExplicitIntake
    );
}

#[test]
fn a_founder_holds_define_group_and_an_ordinary_member_does_not() {
    // The gate's own question, asked against a real network rather than a
    // hand-built state: the founder is the sole member of `Founders`, which
    // holds every capability, and nothing else grants `define-group`.
    let lab = Lab::new("bar");
    let state = lab.state();
    assert!(state.identity_holds(&lab.me(), &Capability::DefineGroup));
    assert!(
        !state.identity_holds(
            &intranet_identity::MasterSeed::from_entropy([42u8; 32])
                .identity_for(&state.network)
                .expect("derives")
                .id(),
            &Capability::DefineGroup,
        ),
        "somebody who is not a member holds nothing"
    );
}
