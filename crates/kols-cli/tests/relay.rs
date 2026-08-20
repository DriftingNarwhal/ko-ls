//! A reservation has to end in a *usable* circuit — Core §5.2–5.5.
//!
//! # Why this test exists
//!
//! Nothing asserted it. The protocol's own relay tests count `ReservationGranted`
//! on the relay side, and its wildcard tests explicitly filter circuit addresses
//! *out* to compare source ports. So "the relay granted a reservation" was
//! covered and "the member can now be reached" was not — and those come apart
//! for a reason that is easy to walk into:
//!
//! **A relay promotes its listen addresses to external addresses only when they
//! are not loopback**, which is correct, because 127.0.0.1 is not something
//! another host can dial. libp2p builds the address list it returns in a
//! reservation from external addresses alone. So a relay on loopback grants
//! every reservation, logs that it granted them, reports healthy — and hands
//! back no address, leaving the member with no circuit listener and no way to
//! know why.
//!
//! That is not a bug in either side. It is a configuration that looks like it is
//! working from every vantage point except this one, which is what makes it
//! worth a test.

use intranet_identity::{MasterSeed, NetworkId};
use intranet_transport::{MemberNode, NodeEvent, RelayNode};
use libp2p::multiaddr::Protocol;
use std::time::Duration;

mod common;
use common::patience;

fn person(seed: u8, net: u8) -> intranet_identity::PerNetworkIdentity {
    MasterSeed::from_entropy([seed; 32])
        .identity_for(&NetworkId::from_bytes([net; 32]))
        .expect("derives")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reservation_produces_a_circuit_listener() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let relay_identity = person(1, 42);
    let mut relay = RelayNode::new(&relay_identity).unwrap();
    relay
        .listen_on("/ip4/0.0.0.0/tcp/0".parse().unwrap())
        .unwrap();

    // A routable address, not loopback: a relay only promotes non-loopback
    // listen addresses to external ones, and libp2p builds the address list it
    // returns in a reservation from external addresses alone. A loopback relay
    // therefore grants reservations that carry no address, which is correct and
    // useless.
    let relay_addr = tokio::time::timeout(patience(Duration::from_secs(5)), async {
        loop {
            if let NodeEvent::Listening(address) = relay.next_event().await
                && !address.to_string().contains("127.0.0.1")
            {
                return address;
            }
        }
    })
    .await
    .expect("the relay listens");
    let relay_full = relay_addr.with(Protocol::P2p(relay_identity.peer_id()));

    // A member of a *different* network, which is the real shape: a relay is
    // not a member of what it serves.
    let member_identity = person(2, 7);
    let mut member = MemberNode::new(&member_identity).unwrap();
    member
        .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .unwrap();

    // Drive the relay in the background, the way a separate process would.
    tokio::spawn(async move {
        loop {
            let event = relay.next_event().await;
            println!("RELAY  event: {event:?}");
        }
    });

    member.reserve_via_relay(relay_full).await.unwrap();

    let granted = member.await_reservation().await;
    assert!(granted, "await_reservation reported no circuit listener");
}
