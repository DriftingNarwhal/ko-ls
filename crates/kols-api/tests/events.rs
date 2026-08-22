//! Property 3 — events are idempotent and re-deliverable (`design/05` §3).
//!
//! # What is actually being claimed
//!
//! Not that the emitter is careful. The emitter cannot be: the live path may be
//! lossy, the durable path carries the same records again, and a record that
//! arrived over gossip is *also* inside the segment that follows it. Duplicate
//! and out-of-order delivery is the normal case rather than the failure case.
//!
//! So the property belongs to the consumer, and it comes down to one word:
//! **merge, never append.** These tests hold a consumer to it, by taking the
//! payload of an [`Event::Records`] and putting it through the same
//! [`ChannelView`] a client would.

use intranet_crypto::Timestamp;
use intranet_governance::{
    Capability, ContentType, EntryBody, GovernanceLog, GovernanceState, LogEntry, MembershipAction,
    NetworkPolicy,
};
use intranet_identity::{MasterSeed, NetworkId, PerNetworkIdentity};
use kols_api::{Arrival, Event};
use kols_core::{

    ChannelId, ChannelView, Hlc, Placement, Record, RecordBody, RenderedMessage, StateAuthority,
};
/// Limits that refuse nothing, for tests about something other than §4.3.
///
/// Spelled out at each call rather than defaulted, because `ReaderLimits` has
/// no `Default` on purpose — a reader that silently enforced nothing is the
/// state this parameter exists to end.
const LAX: kols_core::ReaderLimits = kols_core::ReaderLimits::unbounded();

const NET: NetworkId = NetworkId::from_bytes([11u8; 32]);

fn person(seed: u8) -> PerNetworkIdentity {
    MasterSeed::from_entropy([seed; 32])
        .identity_for(&NET)
        .expect("derives")
}

fn channel() -> ChannelId {
    ChannelId::from_bytes([5u8; 32])
}

fn placement() -> Placement {
    Placement {
        channel: channel(),
        category: None,
    }
}

/// A network where both people may post.
fn state(members: &[&PerNetworkIdentity]) -> GovernanceState {
    let founder = person(1);
    let mut policy = NetworkPolicy::conservative_default();
    for (name, tier) in kols_core::capabilities::namespaces() {
        policy.extension_capabilities.insert(name, tier);
    }
    let mut log = GovernanceLog::new();
    let mut head = log
        .insert(LogEntry::create(
            &founder,
            None,
            Timestamp::from_millis(0),
            EntryBody::Genesis {
                network: NET,
                policy,
                everyone_capabilities: [
                    Capability::ReadContent,
                    Capability::publish(ContentType::new(kols_core::CHAT_LOG_CONTENT_TYPE)),
                    Capability::extension("chat:post:*".to_owned()),
                ]
                .into_iter()
                .collect(),
            },
        ))
        .expect("genesis");

    for (index, member) in members.iter().enumerate() {
        head = log
            .insert(LogEntry::create(
                &founder,
                Some(head),
                Timestamp::from_millis(index as i64 + 1),
                EntryBody::MembershipChange {
                    group: intranet_governance::GroupId::everyone(),
                    identity: member.id(),
                    action: MembershipAction::Add { via_invite: None },
                },
            ))
            .expect("admits");
    }

    let chain: Vec<_> = log
        .canonical_chain()
        .iter()
        .filter_map(|hash| log.get(hash))
        .collect();
    GovernanceState::replay(chain).expect("replays")
}

fn message(who: &PerNetworkIdentity, at: i64, text: &str) -> Record {
    Record::create(
        who,
        channel(),
        Hlc::new(at, 0),
        RecordBody::Message {
            body: text.to_owned(),
            reply_to: None,
            attachments: Vec::new(),
        },
    )
}

/// A consumer: it merges every event it is handed, and renders from the merge.
///
/// Deliberately the whole implementation. There is no bookkeeping to get right,
/// which is the point — a consumer that appended would need to deduplicate, and
/// this one gets it from `ChannelView` being a function of the record set.
fn apply(events: &[Event], authority: &StateAuthority<'_>) -> Vec<RenderedMessage> {
    let mut view = ChannelView::new(placement());
    for event in events {
        let Event::Records { records, .. } = event else {
            continue;
        };
        view.admit(records.clone(), authority, &LAX);
    }
    view.render(&LAX, 0)
}

fn records_event(records: Vec<Record>, arrival: Arrival) -> Event {
    Event::Records {
        channel: channel(),
        records,
        arrival,
    }
}

#[test]
fn the_same_event_delivered_twice_changes_nothing() {
    let (alice, bob) = (person(2), person(3));
    let state = state(&[&alice, &bob]);
    let authority = StateAuthority { state: &state };

    let event = records_event(
        vec![message(&alice, 10, "hello"), message(&bob, 11, "hi")],
        Arrival::Head,
    );

    let once = apply(std::slice::from_ref(&event), &authority);
    let twice = apply(&[event.clone(), event], &authority);
    assert_eq!(once, twice);
    assert_eq!(once.len(), 2);
}

#[test]
fn a_record_that_arrives_live_and_again_in_a_segment_is_one_message() {
    // The delivery pattern the design guarantees rather than an edge case: the
    // live path pushes a record as it is written, and the durable path carries
    // the same record inside the segment that follows. A consumer that appended
    // would show the message twice, every time, for every message.
    let alice = person(2);
    let state = state(&[&alice]);
    let authority = StateAuthority { state: &state };

    let record = message(&alice, 10, "said once");
    let rendered = apply(
        &[
            records_event(vec![record.clone()], Arrival::Live),
            records_event(vec![record], Arrival::Head),
        ],
        &authority,
    );

    assert_eq!(rendered.len(), 1, "one record became {} messages", rendered.len());
    assert_eq!(rendered[0].body, "said once");
}

#[test]
fn events_out_of_order_render_the_same_as_in_order() {
    // Arrival order is not rendering order: order is computed from the merged
    // set (`design/01` §4), so a backfill that lands after the head it precedes
    // sorts where its clock says rather than where it arrived.
    let (alice, bob) = (person(2), person(3));
    let state = state(&[&alice, &bob]);
    let authority = StateAuthority { state: &state };

    let old = records_event(
        vec![message(&alice, 10, "earliest")],
        Arrival::Backfill { segments: 2 },
    );
    let new = records_event(vec![message(&bob, 20, "latest")], Arrival::Head);

    let forwards = apply(&[old.clone(), new.clone()], &authority);
    let backwards = apply(&[new, old], &authority);

    assert_eq!(forwards, backwards);
    assert_eq!(forwards[0].body, "earliest");
    assert_eq!(forwards[1].body, "latest");
}

#[test]
fn a_gap_costs_nothing_once_it_is_filled() {
    // Live delivery may simply be missed — `design/01` §7 says nothing depends
    // on it. A consumer that saw only the later record and then received the
    // earlier one in a backfill ends up where it would have been anyway.
    let alice = person(2);
    let state = state(&[&alice]);
    let authority = StateAuthority { state: &state };

    let first = message(&alice, 10, "missed live");
    let second = message(&alice, 11, "seen live");

    let whole = apply(
        &[records_event(
            vec![first.clone(), second.clone()],
            Arrival::Head,
        )],
        &authority,
    );
    let gapped = apply(
        &[
            records_event(vec![second], Arrival::Live),
            records_event(vec![first], Arrival::Backfill { segments: 1 }),
        ],
        &authority,
    );

    assert_eq!(whole, gapped);
}

#[test]
fn arrival_says_how_it_got_here_and_nothing_about_where_it_sorts() {
    // Stated as a test because it is the thing a client is most likely to get
    // wrong: `Arrival` exists for notification and progress, never for ordering.
    let alice = person(2);
    let state = state(&[&alice]);
    let authority = StateAuthority { state: &state };

    let record = message(&alice, 10, "same either way");
    let live = apply(
        &[records_event(vec![record.clone()], Arrival::Live)],
        &authority,
    );
    let backfilled = apply(
        &[records_event(
            vec![record],
            Arrival::Backfill { segments: 9 },
        )],
        &authority,
    );

    assert_eq!(live, backfilled);
}
