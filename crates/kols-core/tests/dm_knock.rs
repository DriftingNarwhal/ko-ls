//! The knock that wakes a sleeping conversation — spec 07 §6.2 (D41).
//!
//! Its bytes are frozen below: a knock crosses machines, so a change to its
//! encoding is a wire break between two versions of the client, never a value
//! to re-bless.

use intranet_identity::NetworkId;
use kols_core::{DM_KNOCK_KIND, DM_NAMESPACE, DmKnock};

const CONVERSATION: NetworkId = NetworkId::from_bytes([2u8; 32]);

#[test]
fn the_namespace_and_kind_are_the_ones_the_carrier_is_told() {
    // The receiving node declares exactly this pair (Core v1.8 §5.1); a
    // mismatch would be answered `Unsupported` everywhere.
    assert_eq!(DM_NAMESPACE, "chat");
    assert_eq!(DM_KNOCK_KIND, "dm-knock");
}

#[test]
fn a_knock_round_trips() {
    let knock = DmKnock {
        conversation: CONVERSATION,
    };
    assert_eq!(DmKnock::decode(&knock.encode()).unwrap(), knock);
}

#[test]
fn a_knock_encodes_to_its_frozen_bytes() {
    let bytes = DmKnock {
        conversation: CONVERSATION,
    }
    .encode();
    assert_eq!(hex(&bytes), FROZEN);
}

#[test]
fn an_invite_does_not_decode_as_a_knock() {
    // Separate domain tags, so one payload can never be read as the other even
    // if a sender labelled it with the wrong kind.
    let bytes = DmKnock {
        conversation: CONVERSATION,
    }
    .encode();
    assert!(kols_core::DmInvite::decode(&bytes).is_err());
}

#[test]
fn trailing_bytes_are_refused() {
    let mut bytes = DmKnock {
        conversation: CONVERSATION,
    }
    .encode();
    bytes.push(0);
    assert!(DmKnock::decode(&bytes).is_err());
}

/// The domain tag's length as a big-endian u64, the tag, then the
/// conversation's 32 bytes.
const FROZEN: &str = "000000000000001e696e7472616e65742e776972652e636861742d646d2d6b6e6f636b2e7631\
                      0202020202020202020202020202020202020202020202020202020202020202";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
