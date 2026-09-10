//! The direct-message invitation payload — spec 07 §6.2 (E10).
//!
//! # What these are for
//!
//! Core §5.1's carrier already proved the envelope: signed over sender,
//! namespace, kind and content, and bound to the connection it arrived on. What
//! it could not do is anything that needs to know what a DM invite *is*, and
//! that is all these test.
//!
//! The important case is not a broken signature — decoding catches that. It is a
//! proof carrying two genuine signatures over a **true statement about somebody
//! else**, which verifies perfectly and means nothing about the sender. Core §1.2
//! names it, and `verify_from` exists for it.

use intranet_crypto::Timestamp;
use intranet_identity::{CommonOwnershipProof, MasterSeed, NetworkId, PerNetworkIdentity};
use intranet_invite::{Invite, InviteSubject};
use kols_core::{DM_KIND, DM_NAMESPACE, DmInvite};

/// The network Alice and Bob already share.
const SHARED: NetworkId = NetworkId::from_bytes([1u8; 32]);
/// The conversation network Alice creates to talk to Bob.
const CONVERSATION: NetworkId = NetworkId::from_bytes([2u8; 32]);

fn person(seed: u8, network: &NetworkId) -> PerNetworkIdentity {
    MasterSeed::from_entropy([seed; 32])
        .identity_for(network)
        .expect("an identity")
}

fn invite_from(issuer: &PerNetworkIdentity) -> Invite {
    Invite::issue(
        issuer,
        vec!["/ip4/127.0.0.1/tcp/4001".to_string()],
        InviteSubject::Bearer,
        Timestamp::from_millis(0),
        Timestamp::from_millis(86_400_000),
        1,
    )
}

/// Alice, as she appears in the shared network and in the conversation she makes.
fn alice() -> (PerNetworkIdentity, PerNetworkIdentity) {
    (person(1, &SHARED), person(1, &CONVERSATION))
}

fn request_from_alice() -> DmInvite {
    let (here, there) = alice();
    let invite = invite_from(&there);
    let link = CommonOwnershipProof::create(&here, &there);
    DmInvite::new(invite, link)
}

#[test]
fn a_request_from_the_sender_it_claims_verifies() {
    let (alice_here, _) = alice();
    let request = request_from_alice();
    assert!(request.verify_from(&alice_here.id(), &SHARED).is_ok());
}

#[test]
fn the_namespace_and_kind_are_the_ones_the_carrier_is_told() {
    // Pinned so a rename cannot silently stop every existing sender being
    // understood: the carrier routes on these strings and does not decode the
    // payload, so a mismatch is a message nothing consumes.
    assert_eq!(DM_NAMESPACE, "chat");
    assert_eq!(DM_KIND, "dm-invite");
}

#[test]
fn a_request_forwarded_by_somebody_else_does_not_verify_as_theirs() {
    // Carol relays Alice's request. The carrier would refuse this at the
    // connection, but a client that ever obtained the bytes another way — from a
    // log, a backup, a second channel — must still not attribute them to Carol.
    let carol_here = person(3, &SHARED);
    let request = request_from_alice();
    assert!(request.verify_from(&carol_here.id(), &SHARED).is_err());
}

#[test]
fn a_proof_about_the_senders_own_other_identities_does_not_pass() {
    // **The forgery the whole function exists for.** Mallory owns both identities
    // in this proof, so both signatures are genuine and the statement is true. It
    // is simply not a statement about the invite being offered. A recipient that
    // only verified signatures would accept it.
    let mallory_here = person(9, &SHARED);
    let mallory_elsewhere = person(9, &NetworkId::from_bytes([7u8; 32]));
    let genuine_but_irrelevant =
        CommonOwnershipProof::create(&mallory_here, &mallory_elsewhere);
    assert!(
        genuine_but_irrelevant.verify().is_ok(),
        "the proof itself is real — that is the point"
    );

    // The invite is for a conversation Mallory really did mint.
    let mallory_conversation = person(9, &CONVERSATION);
    let request = DmInvite::new(invite_from(&mallory_conversation), genuine_but_irrelevant);

    assert!(
        request.verify_from(&mallory_here.id(), &SHARED).is_err(),
        "a valid proof about the wrong pair must not authenticate a request"
    );
}

#[test]
fn a_sender_cannot_offer_an_invite_they_did_not_mint() {
    // Alice proves who she is honestly, then hands over an invite to a network
    // somebody else controls. Without the second half of the check this passes,
    // and Bob joins a network on Alice's word that Alice does not run.
    let (alice_here, alice_there) = alice();
    let link = CommonOwnershipProof::create(&alice_here, &alice_there);
    let somebody_else = person(4, &CONVERSATION);
    let request = DmInvite::new(invite_from(&somebody_else), link);

    assert!(request.verify_from(&alice_here.id(), &SHARED).is_err());
}

#[test]
fn a_request_checked_against_the_wrong_shared_network_does_not_verify() {
    // The proof names the network it was made in, so a request lifted out of one
    // shared network and presented in another fails — which is what keeps a proof
    // from being reusable everywhere its subject happens to be a member.
    let (alice_here, _) = alice();
    let request = request_from_alice();
    let elsewhere = NetworkId::from_bytes([5u8; 32]);
    assert!(request.verify_from(&alice_here.id(), &elsewhere).is_err());
}

#[test]
fn a_conversation_that_is_the_shared_network_is_refused() {
    // Degenerate and worth refusing explicitly: §1.5's claim is that a
    // conversation is a *separate* network, and a proof linking one network to
    // itself would otherwise let that claim be skipped.
    let alice_here = person(1, &SHARED);
    let alice_again = person(1, &SHARED);
    let link = CommonOwnershipProof::create(&alice_here, &alice_again);
    let request = DmInvite::new(invite_from(&alice_again), link);
    assert!(request.verify_from(&alice_here.id(), &SHARED).is_err());
}

#[test]
fn a_request_round_trips_byte_identically() {
    let request = request_from_alice();
    let bytes = request.encode();
    let back = DmInvite::decode(&bytes).expect("decodes");
    assert_eq!(back, request);
    assert_eq!(back.encode(), bytes);

    // And still verifies after the round trip, which is what makes the encoding
    // useful rather than merely reversible.
    let (alice_here, _) = alice();
    assert!(back.verify_from(&alice_here.id(), &SHARED).is_ok());
}

#[test]
fn a_tampered_payload_fails_to_decode_rather_than_verifying_later() {
    let request = request_from_alice();
    let mut bytes = request.encode();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    assert!(
        DmInvite::decode(&bytes).is_err(),
        "a broken proof is caught at the boundary, not carried inward"
    );
}

#[test]
fn the_decoded_invite_is_the_one_that_was_sent() {
    let request = request_from_alice();
    let sent = request.invite().clone();
    let back = DmInvite::decode(&request.encode()).expect("decodes");
    assert_eq!(back.invite().network, sent.network);
    assert_eq!(back.invite().issuer, sent.issuer);
    assert_eq!(back.invite().bootstrap_addresses, sent.bootstrap_addresses);
}
