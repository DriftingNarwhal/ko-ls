//! What an invite for a real machine actually costs to paste.
//!
//! The address selection in `kols_node::addresses_for_an_invite` is tested for
//! *which* addresses it keeps beside the code it lives in. This is the other
//! half of the requirement, and the one that prompted it: an invite has to be
//! short enough for somebody to send in a chat message. It is measured on the
//! interface list of the machine that produced a 4,400-character invite —
//! Tailscale, three hypervisor subnets, three global IPv6 addresses and one
//! real LAN address, each doubled by TCP and QUIC, plus a relay announcing its
//! container addresses beside its public name.

use libp2p::Multiaddr;

const ME: &str = "12D3KooWMHZbUfFYuqe6NxXBFSg3aLzfTSa1B5QKGNKwMrWz5FaD";
const RELAY: &str = "12D3KooWAT1R2JjcZbnVUKLX8Xo1Qg5APTWMkpHarHY4Uo1YpGzT";

fn direct(host: &str, quic: bool) -> Multiaddr {
    let family = if host.contains(':') { "ip6" } else { "ip4" };
    let tail = if quic { "udp/65343/quic-v1" } else { "tcp/65343" };
    format!("/{family}/{host}/{tail}/p2p/{ME}")
        .parse()
        .expect("valid multiaddr")
}

fn circuit(hop: &str) -> Multiaddr {
    let family = if hop.chars().any(|c| c.is_ascii_alphabetic()) && !hop.contains(':') {
        "dns4"
    } else if hop.contains(':') {
        "ip6"
    } else {
        "ip4"
    };
    format!("/{family}/{hop}/tcp/4001/p2p/{RELAY}/p2p-circuit/p2p/{ME}")
        .parse()
        .expect("valid multiaddr")
}

fn a_real_machine() -> Vec<Multiaddr> {
    let hosts = [
        "2600:1700:a825:4800::3f",
        "2600:1700:a825:4800:634c:8370:a381:9f64",
        "2600:1700:a825:4800:d46f:a0b3:57f5:bbc7",
        "fd7a:115c:a1e0::4b36:9876",
        "100.101.152.117",
        "192.168.1.200",
        "192.168.56.1",
        "192.168.23.1",
        "192.168.220.1",
        "127.0.0.1",
        "fe80::1",
    ];
    let mut all = Vec::new();
    for host in hosts {
        all.push(direct(host, false));
        all.push(direct(host, true));
    }
    for hop in [
        "switchback.proxy.rlwy.net",
        "10.140.152.103",
        "fd12:a6a1:7ec6:1::9867",
    ] {
        all.push(circuit(hop));
    }
    all
}

/// The URI an address list produces, near enough to hold a bound against.
///
/// Base32 of the canonical bytes, which are the addresses plus a fixed ~200 for
/// the network id, issuer, subject, timestamps, use count and signature.
fn uri_length(addresses: &[String]) -> usize {
    let bytes: usize = addresses.iter().map(String::len).sum();
    kols_node::invite::SCHEME.len() + (bytes + 200).div_ceil(5) * 8
}

#[test]
fn an_invite_for_a_real_machine_is_short_enough_to_send_somebody() {
    let listening = a_real_machine();
    let before: Vec<String> = listening.iter().map(ToString::to_string).collect();
    let after = kols_node::addresses_for_an_invite(
        &listening,
        &["192.168.1.200".parse().expect("valid ip")],
    );

    // The number this started at, kept so the improvement is visible rather
    // than asserted against nothing.
    assert!(
        uri_length(&before) > 4_000,
        "the unfiltered invite was measured at ~4,450 characters; got {}",
        uri_length(&before)
    );

    // Roughly halved. Not a tight bound — it exists to fail if a change puts
    // the hypervisor subnets or the relay's container addresses back, each of
    // which costs hundreds of characters.
    let length = uri_length(&after);
    assert!(
        length < 2_200,
        "invite grew back to {length} characters:\n{after:#?}"
    );
    assert_eq!(after.len(), 9, "{after:#?}");
}

#[test]
fn what_is_left_is_mostly_the_peer_id_repeated() {
    // Recorded rather than fixed: the id appears once per address, and once
    // more on a circuit for the relay. Shortening this is a change to the wire
    // format of `intranet_invite::Invite` — the addresses would carry no
    // `/p2p/` suffix and the joiner would append the one the invite names — so
    // it is a decision above this crate. This test is here so the number is
    // known when that decision is taken.
    let after = kols_node::addresses_for_an_invite(
        &a_real_machine(),
        &["192.168.1.200".parse().expect("valid ip")],
    );
    let bytes: usize = after.iter().map(String::len).sum();
    let ids: usize = after
        .iter()
        .map(|address| address.matches("12D3KooW").count() * 52)
        .sum();

    assert!(
        ids * 2 > bytes,
        "peer ids are {ids} of {bytes} address bytes; if that is no longer the \
         majority, the note above about the wire format is out of date"
    );
}
