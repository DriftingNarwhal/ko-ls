//! The bounds a reader enforces — spec 07 §2.6 and §4.3, `design/01` §10.
//!
//! These exist because the rules they cover were, until now, enforced by the
//! *author's* client and nobody else — which §4.3 says plainly is not
//! enforcement at all:
//!
//! > a record past the ceiling is refused by readers, so a local limit would
//! > mean two members rendering different histories from the same records
//!
//! Every test below therefore builds records **directly**, never through the
//! executor. A test that went through the writer could not tell a rule that is
//! enforced from one that is merely never violated, which is exactly how this
//! gap survived: the writer refuses to produce the records, so nothing ever
//! handed the reader one to refuse.

use intranet_crypto::Timestamp;
use intranet_governance::{
    Capability, ContentType, EntryBody, GovernanceState, GroupId, LogEntry, MembershipAction,
    NetworkPolicy,
};
use intranet_identity::{MasterSeed, NetworkId, PerNetworkIdentity};
use kols_core::*;

fn network() -> NetworkId {
    NetworkId::from_bytes([7u8; 32])
}

fn identity(byte: u8) -> PerNetworkIdentity {
    MasterSeed::from_entropy([byte; 32])
        .identity_for(&network())
        .expect("derives")
}

fn channel() -> ChannelId {
    server_channel_id(&network(), &[9u8; 32])
}

fn placement() -> Placement {
    Placement {
        channel: channel(),
        category: None,
    }
}

fn state(members: &[&PerNetworkIdentity]) -> GovernanceState {
    let founder = identity(1);
    let mut policy = NetworkPolicy::conservative_default();
    policy
        .content_type_allowlist
        .insert(ContentType::new(CHAT_LOG_CONTENT_TYPE));
    policy
        .extension_capabilities
        .extend(kols_core::capabilities::namespaces());

    let mut chain = vec![LogEntry::create(
        &founder,
        None,
        Timestamp::from_millis(0),
        EntryBody::Genesis {
            network: network(),
            policy,
            everyone_capabilities: [
                Capability::ReadContent,
                Capability::publish(CHAT_LOG_CONTENT_TYPE),
                Capability::extension("chat:post:*"),
            ]
            .into_iter()
            .collect(),
        },
    )];
    for (n, member) in members.iter().enumerate() {
        let parent = chain.last().expect("genesis").hash();
        chain.push(LogEntry::create(
            &founder,
            Some(parent),
            Timestamp::from_millis(10 + n as i64),
            EntryBody::MembershipChange {
                group: GroupId::everyone(),
                identity: member.id(),
                action: MembershipAction::Add { via_invite: None },
            },
        ));
    }
    GovernanceState::replay(&chain).expect("replays")
}

fn message(text: &str) -> RecordBody {
    RecordBody::Message {
        body: text.to_owned(),
        reply_to: None,
        attachments: Vec::new(),
    }
}

const START: i64 = 1_700_000_000_000;

/// Ceilings that bite, so a test can be about the rule rather than about volume.
fn strict() -> ReaderLimits {
    ReaderLimits {
        message_rate_per_minute: 3,
        reaction_rate_per_minute: 2,
        message_max_bytes: 32,
        max_future_skew_millis: 300_000,
        slowmode_seconds: 0,
    }
}

/// `n` messages from one author, `gap` milliseconds apart.
fn burst(who: &PerNetworkIdentity, n: u32, gap: i64) -> Vec<Record> {
    (0..n)
        .map(|i| {
            Record::create(
                who,
                channel(),
                Hlc::new(START + i64::from(i) * gap, 0),
                message(&format!("m{i}")),
            )
        })
        .collect()
}

fn view_of(records: Vec<Record>, state: &GovernanceState, limits: &ReaderLimits) -> ChannelView {
    let mut view = ChannelView::new(placement());
    view.admit(records, &StateAuthority { state }, limits);
    view
}

// ── §4.3: the ceiling is a validity rule, so the reader applies it ──────

#[test]
fn a_reader_refuses_what_the_author_wrote_past_the_ceiling() {
    // The whole point: these records are validly signed by a current member who
    // holds `chat:post`. Every per-record check passes. Only the rate refuses
    // them, and only the reader is asking.
    let alice = identity(2);
    let state = state(&[&alice]);
    let limits = strict();

    let view = view_of(burst(&alice, 10, 1_000), &state, &limits);
    let rendered = view.render(&limits, START + 60_000);

    assert_eq!(
        rendered.len(),
        3,
        "the ceiling is 3 a minute and ten arrived inside one",
    );

    let withheld = view.withheld(&limits, START + 60_000);
    assert_eq!(withheld.refused.len(), 7);
    assert!(
        withheld
            .refused
            .values()
            .all(|r| *r == Rejection::TooFast),
        "refused for rate, not for anything else",
    );
}

#[test]
fn the_window_slides_rather_than_counting_forever() {
    // Ten messages two minutes apart are not a flood, and a reader that counted
    // an author's records rather than their *recent* records would refuse seven
    // of them.
    let alice = identity(2);
    let state = state(&[&alice]);
    let limits = strict();

    let view = view_of(burst(&alice, 10, 120_000), &state, &limits);
    assert_eq!(view.render(&limits, START + 10 * 120_000).len(), 10);
}

#[test]
fn one_author_going_too_fast_costs_nobody_else() {
    let alice = identity(2);
    let bob = identity(3);
    let state = state(&[&alice, &bob]);
    let limits = strict();

    let mut records = burst(&alice, 10, 1_000);
    records.extend(burst(&bob, 2, 1_000).into_iter().map(|mut r| {
        // Same author-relative timing, different author: rebuild so the
        // signature stays honest.
        r = Record::create(&bob, channel(), r.hlc, r.body.clone());
        r
    }));

    let view = view_of(records, &state, &limits);
    let rendered = view.render(&limits, START + 60_000);
    assert_eq!(
        rendered.iter().filter(|m| m.author == bob.id()).count(),
        2,
        "the window is per author, so Bob is unaffected by Alice",
    );
}

#[test]
fn a_refused_record_does_not_poison_the_ones_behind_it() {
    // A burst of ten, then quiet, then one more well outside the window. The
    // last one must render: refused records do not occupy a slot, so an author
    // who once went too fast is not silenced afterwards.
    let alice = identity(2);
    let state = state(&[&alice]);
    let limits = strict();

    let mut records = burst(&alice, 10, 1_000);
    records.push(Record::create(
        &alice,
        channel(),
        Hlc::new(START + 600_000, 0),
        message("much later"),
    ));

    let rendered = view_of(records, &state, &limits).render(&limits, START + 700_000);
    assert_eq!(rendered.len(), 4);
    assert_eq!(rendered.last().expect("the late one").body, "much later");
}

/// **The property the rule exists for.** Two members holding the same records
/// must refuse the same ones, whatever order those records arrived in.
#[test]
fn every_arrival_order_refuses_the_same_records() {
    let alice = identity(2);
    let bob = identity(3);
    let state = state(&[&alice, &bob]);
    let limits = strict();

    let mut records = burst(&alice, 12, 3_000);
    records.extend(
        (0..8u32).map(|i| {
            Record::create(
                &bob,
                channel(),
                Hlc::new(START + i64::from(i) * 4_000, 0),
                message(&format!("b{i}")),
            )
        }),
    );

    let baseline = view_of(records.clone(), &state, &limits);
    let expected = baseline.render(&limits, START + 120_000);
    let expected_refused = baseline.withheld(&limits, START + 120_000).refused;
    assert!(!expected_refused.is_empty(), "the ceiling has to bite");

    for seed in 0..40u32 {
        // Deterministic shuffle, so a failure names a reproducible order.
        let mut shuffled = records.clone();
        let mut x = seed | 1;
        for i in (1..shuffled.len()).rev() {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            shuffled.swap(i, (x >> 16) as usize % (i + 1));
        }

        let view = view_of(shuffled, &state, &limits);
        assert_eq!(
            view.render(&limits, START + 120_000),
            expected,
            "seed {seed} rendered differently",
        );
        assert_eq!(
            view.withheld(&limits, START + 120_000).refused,
            expected_refused,
            "seed {seed} refused a different set",
        );
    }
}

#[test]
fn a_reaction_is_counted_against_its_own_ceiling() {
    let alice = identity(2);
    let state = state(&[&alice]);
    let limits = strict();

    let target = Record::create(&alice, channel(), Hlc::new(START, 0), message("hello"));
    let target_id = target.id();
    let mut records = vec![target];
    // Five reactions inside a minute, against a reaction ceiling of two.
    records.extend((0..5u32).map(|i| {
        Record::create(
            &alice,
            channel(),
            Hlc::new(START + 1_000 + i64::from(i) * 1_000, 0),
            RecordBody::Reaction {
                target: target_id,
                key: format!("k{i}"),
                remove: false,
            },
        )
    }));

    let view = view_of(records, &state, &limits);
    let rendered = view.render(&limits, START + 60_000);
    assert_eq!(
        rendered[0].reactions.len(),
        2,
        "three of the five were past the reaction ceiling and must not land",
    );
}

#[test]
fn an_oversized_body_is_refused_by_the_reader() {
    // `check_bounds` implemented this the day the encoding landed and was called
    // by nothing but its own tests. The gate refuses an oversized body on the
    // way out; this is the half that matters against a client that does not.
    let alice = identity(2);
    let state = state(&[&alice]);
    let limits = strict();

    let view = view_of(
        vec![Record::create(
            &alice,
            channel(),
            Hlc::new(START, 0),
            message(&"x".repeat(64)),
        )],
        &state,
        &limits,
    );

    assert!(view.render(&limits, START).is_empty());
    assert_eq!(view.rejected()[0].1, Rejection::TooLarge);
}

// ── §2.6: held, not dropped ────────────────────────────────────────────

#[test]
fn a_future_dated_record_is_held_and_then_renders() {
    let alice = identity(2);
    let state = state(&[&alice]);
    let limits = strict(); // 300s of tolerated skew

    let view = view_of(
        vec![Record::create(
            &alice,
            channel(),
            Hlc::new(START + 3_600_000, 0),
            message("an hour ahead"),
        )],
        &state,
        &limits,
    );

    assert!(
        view.render(&limits, START).is_empty(),
        "an hour ahead of a 300s tolerance is not rendered yet",
    );
    assert_eq!(view.withheld(&limits, START).held.len(), 1);

    // Held, not dropped: the record is still here, and still not a refusal.
    assert_eq!(view.len(), 1, "the record set is unchanged");
    assert!(
        view.rejected().is_empty() && view.withheld(&limits, START).refused.is_empty(),
        "a hold is temporary and must never be reported as a refusal",
    );

    // And it renders once local time reaches it, with no further delivery.
    let rendered = view.render(&limits, START + 3_600_000);
    assert_eq!(rendered.len(), 1);
    assert_eq!(rendered[0].body, "an hour ahead");
}

#[test]
fn a_record_inside_the_skew_tolerance_renders_now() {
    let alice = identity(2);
    let state = state(&[&alice]);
    let limits = strict();

    // A minute ahead, against 300s of tolerance: ordinary clock drift.
    let view = view_of(
        vec![Record::create(
            &alice,
            channel(),
            Hlc::new(START + 60_000, 0),
            message("slightly fast clock"),
        )],
        &state,
        &limits,
    );
    assert_eq!(view.render(&limits, START).len(), 1);
}

#[test]
fn a_record_dated_in_the_past_sorts_where_it_claims_and_is_not_held() {
    // §2.6 is explicit that only the *future* is held. A backdated record is
    // admitted and cannot displace history, because rendering is a function of
    // the set rather than of arrival.
    let alice = identity(2);
    let state = state(&[&alice]);
    let limits = strict();

    let view = view_of(
        vec![
            Record::create(&alice, channel(), Hlc::new(START, 0), message("second")),
            Record::create(
                &alice,
                channel(),
                Hlc::new(START - 86_400_000, 0),
                message("first, claimed yesterday"),
            ),
        ],
        &state,
        &limits,
    );

    let rendered = view.render(&limits, START);
    assert_eq!(rendered.len(), 2);
    assert_eq!(rendered[0].body, "first, claimed yesterday");
}

/// **The composition, which is the whole argument of `design/01` §10.2.**
///
/// > The obvious evasion — claiming timestamps spaced far apart while actually
/// > sending fast — defeats itself: records dated ahead of the receiver's clock
/// > are held until local time reaches them. An author who lies about pacing to
/// > escape the ceiling gets exactly the pacing they claimed.
///
/// Neither rule delivers that alone. The rate ceiling is computed over claimed
/// readings, so spreading them out passes it; the hold is what makes the lie
/// cost the liar. This is the test that fails if either half is removed.
#[test]
fn spacing_claimed_timestamps_buys_exactly_the_pacing_claimed() {
    let alice = identity(2);
    let state = state(&[&alice]);
    let limits = strict();

    // Twenty messages, each claiming to be a minute after the last — written
    // all at once, at `START`. The rate ceiling has nothing to object to.
    let records = burst(&alice, 20, 60_000);
    let view = view_of(records, &state, &limits);
    assert!(
        view.withheld(&limits, START).refused.is_empty(),
        "spread-out claims pass the rate ceiling, which is why the hold matters",
    );

    // But the reader shows them at the pace they claimed, not at the pace they
    // were sent. At `START` only the ones inside the skew tolerance are visible.
    assert_eq!(
        view.render(&limits, START).len(),
        6,
        "300s of tolerance is five minutes of claims, plus the one at START",
    );
    assert_eq!(view.render(&limits, START + 10 * 60_000).len(), 16);
    assert_eq!(view.render(&limits, START + 3_600_000).len(), 20);
}

// ── §10.3: slowmode is the stricter of the two ─────────────────────────

#[test]
fn slowmode_paces_an_author_the_network_ceiling_would_allow() {
    let alice = identity(2);
    let state = state(&[&alice]);
    let mut limits = strict();
    limits.message_rate_per_minute = 100; // the network ceiling is not the bound here
    limits.slowmode_seconds = 30;

    // Ten messages five seconds apart: comfortably under 100 a minute, and well
    // over one every thirty seconds.
    let view = view_of(burst(&alice, 10, 5_000), &state, &limits);
    let rendered = view.render(&limits, START + 60_000);

    assert_eq!(rendered.len(), 2, "45s of messages at one per 30s");
    assert!(
        view.withheld(&limits, START + 60_000)
            .refused
            .values()
            .all(|r| *r == Rejection::Slowmode),
        "refused for slowmode, which is a different fact from going too fast",
    );
}

#[test]
fn slowmode_off_is_zero_and_bounds_nothing() {
    let alice = identity(2);
    let state = state(&[&alice]);
    let mut limits = strict();
    limits.message_rate_per_minute = 100;
    limits.slowmode_seconds = 0;

    assert_eq!(
        view_of(burst(&alice, 10, 5_000), &state, &limits)
            .render(&limits, START + 60_000)
            .len(),
        10,
    );
}

#[test]
fn slowmode_is_per_author_not_per_channel() {
    // A slow channel slows each member down; it does not make members take
    // turns. Getting this wrong would make a busy channel unusable for everyone
    // as soon as one person posted.
    let alice = identity(2);
    let bob = identity(3);
    let state = state(&[&alice, &bob]);
    let mut limits = strict();
    limits.message_rate_per_minute = 100;
    limits.slowmode_seconds = 30;

    let records = vec![
        Record::create(&alice, channel(), Hlc::new(START, 0), message("a")),
        Record::create(&bob, channel(), Hlc::new(START + 1_000, 0), message("b")),
    ];
    assert_eq!(
        view_of(records, &state, &limits)
            .render(&limits, START + 60_000)
            .len(),
        2,
    );
}

// ── the escape hatch, and what it costs ────────────────────────────────

#[test]
fn unbounded_limits_refuse_nothing_and_hold_nothing() {
    // `ReaderLimits::unbounded` exists for reading a set whose network policy is
    // genuinely unavailable. It must be inert rather than lenient-by-accident,
    // including for a record claiming the end of time — which is where an
    // unchecked `now + skew` would overflow into the past and admit it.
    let alice = identity(2);
    let state = state(&[&alice]);
    let limits = ReaderLimits::unbounded();

    let records = vec![
        Record::create(&alice, channel(), Hlc::new(START, 0), message("now")),
        Record::create(&alice, channel(), Hlc::new(i64::MAX, 0), message("never")),
    ];
    let view = view_of(records, &state, &limits);

    assert!(view.withheld(&limits, START).is_empty());
    assert_eq!(view.render(&limits, START).len(), 2);
}

#[test]
fn a_far_future_claim_is_held_rather_than_wrapping_into_the_past() {
    // The arithmetic hazard, asserted directly: `now + max_future_skew` is a
    // sum an author can choose the other side of.
    let alice = identity(2);
    let state = state(&[&alice]);
    let limits = strict();

    let view = view_of(
        vec![Record::create(
            &alice,
            channel(),
            Hlc::new(i64::MAX, 0),
            message("the end of time"),
        )],
        &state,
        &limits,
    );
    assert_eq!(view.withheld(&limits, START).held.len(), 1);
    assert!(view.render(&limits, START).is_empty());
}
