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

/// The URI these addresses actually produce, by minting and encoding an invite.
///
/// Estimated at first, from the address bytes plus a guess at the fixed fields.
/// The estimate was wrong by about 15% because `Enc` frames every length with a
/// fixed eight-byte `u64`, which a guess at "roughly 200 bytes of overhead"
/// does not account for. Measuring the real thing costs a keypair.
fn uri_length(addresses: &[String]) -> usize {
    let seed = intranet_identity::MasterSeed::from_entropy([7u8; 32]);
    let network = intranet_identity::NetworkId::from_bytes([9u8; 32]);
    let issuer = seed.identity_for(&network).expect("derives an identity");
    let invite = intranet_invite::Invite::issue(
        &issuer,
        addresses.to_vec(),
        intranet_invite::InviteSubject::Bearer,
        intranet_crypto::Timestamp::from_millis(0),
        intranet_crypto::Timestamp::from_millis(86_400_000),
        1,
    );
    kols_node::invite::to_uri(&invite).len()
}

#[test]
fn an_invite_for_a_real_machine_is_short_enough_to_send_somebody() {
    let listening = a_real_machine();
    let unselected: Vec<String> = listening.iter().map(ToString::to_string).collect();
    let selected = kols_node::addresses_for_an_invite(
        &listening,
        &["192.168.1.200".parse().expect("valid ip")],
    );

    // Two separate savings, and this measures the one that is still visible.
    // Carrying every address costs about a kilobyte of URI more than carrying
    // the nine worth carrying — the hypervisor subnets, Tailscale, and the
    // relay's container addresses.
    let (all, chosen) = (uri_length(&unselected), uri_length(&selected));
    assert!(
        all > chosen + 800,
        "selecting addresses must be worth having; {all} -> {chosen}"
    );

    // The other saving cannot be measured from here any more, because the
    // encoding it improved on no longer exists: an unselected invite under the
    // old wire format was about 4,450 characters, and factoring the shared
    // ending out is what took the rest (`intranet_invite::wire`).
    assert!(
        chosen < 1_400,
        "invite grew back to {chosen} characters:\n{selected:#?}"
    );
    assert_eq!(selected.len(), 9, "{selected:#?}");
    // Measured: 2,575 characters for all twenty-five, 1,324 for the nine.
}

#[test]
fn the_peer_id_is_no_longer_paid_for_once_per_address() {
    // It used to be more than half of what the addresses cost. The wire
    // encoding now writes the ending they share exactly once
    // (`intranet_invite::wire::put_addresses`), so the invite carries it a
    // single time however many addresses there are — which is what this
    // asserts, since the count is what used to scale.
    let after = kols_node::addresses_for_an_invite(
        &a_real_machine(),
        &["192.168.1.200".parse().expect("valid ip")],
    );
    let seed = intranet_identity::MasterSeed::from_entropy([7u8; 32]);
    let network = intranet_identity::NetworkId::from_bytes([9u8; 32]);
    let issuer = seed.identity_for(&network).expect("derives an identity");
    let invite = intranet_invite::Invite::issue(
        &issuer,
        after,
        intranet_invite::InviteSubject::Bearer,
        intranet_crypto::Timestamp::from_millis(0),
        intranet_crypto::Timestamp::from_millis(86_400_000),
        1,
    );
    let encoded = intranet_invite::encode_invite(&invite);
    let text = String::from_utf8_lossy(&encoded);

    assert_eq!(
        text.matches(ME).count(),
        1,
        "this node's peer id must appear once, not once per address"
    );
    // And what came back out is still every address that went in.
    assert_eq!(
        intranet_invite::decode_invite(&encoded)
            .expect("decodes")
            .bootstrap_addresses,
        invite.bootstrap_addresses
    );
}

/// What the same addresses cost before the wire encoding factored their shared
/// ending out. Kept so the headline number has something behind it.
#[test]
fn the_historical_figure_is_reproducible() {
    use intranet_crypto::Enc;

    let all: Vec<String> = a_real_machine().iter().map(ToString::to_string).collect();
    let mut whole = Enc::new();
    whole.seq(all.iter(), |e, address| {
        e.str(address);
    });
    let mut factored = Enc::new();
    factored.seq(std::iter::once(&all), |e, addresses| {
        // The shared ending, then each address without it — what `put_addresses`
        // does, reproduced here because it is private to the invite crate.
        let ending = format!("/p2p/{ME}");
        e.str(&ending);
        e.seq(addresses.iter(), |e, address| {
            e.str(&address[..address.len() - ending.len()]);
        });
    });

    // The URI grows 8/5 with the encoding, so the saving in characters is the
    // saving in bytes scaled by the same factor.
    let saved = (whole.finish().len() - factored.finish().len()) * 8 / 5;
    let now = uri_length(&all);
    assert!(
        (4_600..4_900).contains(&(now + saved)),
        "an unselected invite used to be about 4,750 characters; got {}",
        now + saved
    );
}
