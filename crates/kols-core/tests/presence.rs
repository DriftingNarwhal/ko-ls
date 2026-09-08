//! Presence on the wire — `design/01` §9, and the claim `design/09` §4.1 forbids.
//!
//! # What these are actually guarding
//!
//! Presence is the one signal in this client that is *about a person* rather
//! than about content, and it is the easiest place to accidentally publish more
//! than somebody agreed to. So the tests below are less about round-tripping
//! bytes than about three refusals:
//!
//! - an invisible member has **no wire representation at all**, because a beat
//!   saying "invisible" tells the network this node is running and hiding;
//! - a beat can only be made by the member it names, since gossip lets any
//!   subscriber republish anything they received;
//! - a beat is fresh by when it was *heard*, not by the date it carries, or one
//!   message could pin somebody as present forever.

use intranet_crypto::Hash;
use intranet_identity::{MasterSeed, NetworkId, PerNetworkIdentity};
use intranet_storage::EpochKey;
use kols_core::*;

fn network() -> NetworkId {
    NetworkId::from_bytes([9u8; 32])
}

fn identity(byte: u8) -> PerNetworkIdentity {
    MasterSeed::from_entropy([byte; 32])
        .identity_for(&network())
        .expect("an identity for this network")
}

fn epoch(byte: u8) -> EpochKey {
    EpochKey::from_bytes([byte; 32])
}

/// A beat survives the wire, and says exactly what its author said.
#[test]
fn a_beat_round_trips_and_names_the_member_who_signed_it() {
    let me = identity(1);
    let key = epoch(1);
    let rotation = Hash::from_bytes([4u8; 32]);

    let sealed = PresenceBeat::seal(&me, Beat::Idle, 1_000, &key, rotation);
    let wire = sealed.encode();
    let read = SealedBeat::decode(&wire).expect("decodes");
    let beat = read.open(&key).expect("opens");

    assert_eq!(beat.who, me.id());
    assert_eq!(beat.state, Beat::Idle);
    assert_eq!(beat.at, 1_000);
}

/// **A member outside the epoch learns nothing**, presence included.
///
/// The topic is derivable by anybody who knows the network id, and gossip
/// carries to every subscriber. Without the seal, sitting on that topic would
/// yield a live attendance register for the network.
#[test]
fn a_beat_is_unreadable_without_the_networks_key() {
    let me = identity(1);
    let sealed = PresenceBeat::seal(&me, Beat::Here, 1_000, &epoch(1), Hash::from_bytes([4u8; 32]));
    assert!(
        sealed.open(&epoch(2)).is_err(),
        "a different epoch key must not open a beat"
    );
}

/// **Nobody may announce anybody else.**
///
/// Every subscriber receives every beat and can republish it verbatim, so the
/// seal alone would let one member keep another shown as present long after they
/// had gone — or announce them as here while they had chosen to be invisible.
/// The signature is what makes the seal say *who*.
#[test]
fn a_beat_cannot_be_forged_or_re_dated_by_whoever_received_it() {
    let me = identity(1);
    let impostor = identity(2);
    let key = epoch(1);
    let rotation = Hash::from_bytes([4u8; 32]);

    // The impostor holds the network key — they are a member — and re-seals a
    // beat naming somebody else. Only the signature stands between that and a
    // roster the impostor writes.
    let mine = PresenceBeat::seal(&me, Beat::Here, 1_000, &key, rotation);
    let theirs = PresenceBeat::seal(&impostor, Beat::Here, 1_000, &key, rotation);
    assert_ne!(mine.sealed, theirs.sealed);
    assert_eq!(
        theirs.open(&key).expect("opens").who,
        impostor.id(),
        "a beat names its signer and nobody else"
    );

    // And the time is inside the signature, so a beat cannot be replayed under a
    // fresher date to keep somebody pinned as present.
    let later = PresenceBeat::seal(&me, Beat::Here, 9_000, &key, rotation);
    let (_, signature) = PresenceBeat::create(&me, Beat::Here, 1_000);
    let (_, other) = PresenceBeat::create(&me, Beat::Here, 9_000);
    assert_ne!(signature.as_bytes(), other.as_bytes());
    assert_eq!(later.open(&key).expect("opens").at, 9_000);
}

/// **Invisible has no wire form, and that is the whole setting.**
///
/// A beat saying "invisible" would tell every member of the network that this
/// node is running and has chosen to hide, which is most of what the setting is
/// for. So the type that goes out cannot express it: `Presence::Invisible`
/// yields no `Beat`, and there is nothing to publish.
#[test]
fn an_invisible_member_has_nothing_to_broadcast() {
    assert_eq!(Presence::Invisible.to_beat(), None);
    assert_eq!(Presence::Show(Beat::Busy).to_beat(), Some(Beat::Busy));

    // Round-tripping by name keeps the choice a member made, including the one
    // that sends nothing — this is what settings persists.
    for name in ["here", "idle", "busy", "invisible"] {
        assert_eq!(
            Presence::from_name(name).expect("a known state").name(),
            name
        );
    }
    assert_eq!(Presence::from_name("offline"), None, "there is no such state");
    assert_eq!(Presence::from_name("online"), None);
}

/// **An unknown state is refused rather than rounded down.**
///
/// A later client inventing a fourth state must not have it read as one of
/// these three — being shown as `here` when you said something else is the same
/// class of mistake as being shown as offline.
#[test]
fn an_unknown_state_is_refused_rather_than_guessed() {
    assert_eq!(Beat::from_code(1), Some(Beat::Here));
    assert_eq!(Beat::from_code(2), Some(Beat::Idle));
    assert_eq!(Beat::from_code(3), Some(Beat::Busy));
    assert_eq!(Beat::from_code(0), None);
    assert_eq!(Beat::from_code(4), None);

    // And the codes are pinned, because they are a wire format: reordering the
    // variants must not change what an older peer reads.
    assert_eq!(Beat::Here.code(), 1);
    assert_eq!(Beat::Idle.code(), 2);
    assert_eq!(Beat::Busy.code(), 3);
}

/// **Freshness is judged by when we heard, not by the date the beat carries.**
///
/// Clocks disagree, and `at` is signed by the sender — so a member whose clock
/// says next year could pin themselves as present indefinitely with one
/// message. What decides is this node's own observation.
#[test]
fn freshness_uses_this_nodes_clock_and_never_the_senders() {
    let heard = 1_000_000;
    assert!(PresenceBeat::fresh_at(heard, heard));
    assert!(PresenceBeat::fresh_at(heard, heard + FRESH_MILLIS - 1));
    assert!(!PresenceBeat::fresh_at(heard, heard + FRESH_MILLIS));

    // Two beats may be lost before somebody stops being shown as here, which is
    // what the window is sized for.
    assert!(PresenceBeat::fresh_at(heard, heard + 2 * BEAT_MILLIS));

    // Heard "in the future" — a clock that moved backwards, or a stored value
    // from a machine that was ahead. Not fresh, because the alternative is a
    // row that stays lit until the clock catches up.
    assert!(!PresenceBeat::fresh_at(heard, heard - 1));
}

/// The presence topic is its own, and is derived from the network.
#[test]
fn presence_rides_a_topic_of_its_own() {
    let other = NetworkId::from_bytes([10u8; 32]);
    assert_ne!(presence_topic(&network()), presence_topic(&other));
    assert_eq!(presence_topic(&network()), presence_topic(&network()));

    // Domain-separated from the channel topic, so presence for one network can
    // never land in a channel's mesh or the other way round.
    let channel = server_channel_id(&network(), &[3u8; 32]);
    assert_ne!(presence_topic(&network()), gossip_topic(&channel));
}
