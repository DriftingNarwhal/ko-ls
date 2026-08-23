//! The client's composition layer — the store, the node, and the executor.
//!
//! # Why the name changed
//!
//! This was `kols-cli`, which described 853 of its lines and misdescribed the
//! rest. The window depends on this crate for everything except its own window:
//! [`serve::serve`] is the node loop it runs, [`store::Store`] is where its
//! state lives, [`executor::Executor`] is what its every command crosses. The
//! terminal is one *consumer* of this crate and not what it is (`design/00`
//! D30).
//!
//! # Why this is a library and not only a binary
//!
//! It was a binary alone until there was something worth testing that was not a
//! whole process. [`executor::Executor`] is that: it authorizes and runs every
//! command, and a test that had to spawn `kols` to reach it would be testing the
//! terminal's argument parsing at the same time — slower, and vague about which
//! half failed when it did.
//!
//! The binary is now argument parsing and rendering over this, which is also the
//! shape the desktop client needs: a different front end over the same submit
//! path, rather than a second copy of it.
//!
//! # What lives here rather than in `kols-core`
//!
//! Everything that touches a disk or a network. `kols-core` is I/O-free on
//! purpose, and `kols-api` reaches no store by design, so the composition has to
//! land somewhere and this is it — until `design/05` §5's `kols-store` exists and
//! takes the persistence half.

#![deny(missing_docs)]

pub mod chat;
pub mod executor;
pub mod invite;
pub mod join;
pub mod network;
mod secret;
pub mod serve;
pub mod store;
pub mod workspace;

/// Reads an identity id out of the 64 hex characters that display it.
///
/// In the library rather than in either front end, because both need it: a
/// terminal takes one as an argument and a window takes one from a click, and
/// two copies of a parser for the same 32 bytes is how they end up disagreeing
/// about what is valid.
pub fn parse_identity(hex: &str) -> Result<intranet_identity::PerNetworkIdentityId, String> {
    let bytes = intranet_crypto::from_hex(hex.trim())
        .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
        .ok_or("an identity is 64 hex characters")?;
    let key = intranet_crypto::VerifyingKey::from_bytes(bytes)
        .map_err(|_| "those bytes are not a valid identity key".to_owned())?;
    Ok(intranet_identity::PerNetworkIdentityId::from_verifying_key(key))
}

/// Checks a relay address before it becomes policy.
///
/// In the library for the same reason [`parse_identity`] is, and with a sharper
/// edge: designating a relay writes a governance entry **every member replays**,
/// so an address that is merely wrong does not fail for the person who typed it
/// — it is carried by everybody, and the symptom appears later on somebody
/// else's machine as a network that cannot introduce two peers.
///
/// Two things are checked. That it parses at all, and that it names a peer id:
/// a relay address without `/p2p/…` is dialable and unverifiable, so a node
/// reaching it has no way to know whether what answered is the relay the
/// network designated (Core §5.5).
pub fn parse_relay(address: &str) -> Result<String, String> {
    let address = address.trim();
    address
        .parse::<libp2p::Multiaddr>()
        .map_err(|_| format!("{address} is not an address — a relay looks like /dns4/host/tcp/443/p2p/12D3Koo…"))?;
    if !address.contains("/p2p/") {
        return Err(format!(
            "{address} names no peer id — without the /p2p/… part nothing can \
             verify that what answers there is the relay this network means"
        ));
    }
    Ok(address.to_owned())
}

/// 32 bytes from the OS.
pub fn random_32() -> Result<[u8; 32], String> {
    let mut bytes = [0u8; 32];
    intranet_crypto::random_bytes(&mut bytes)
        .map_err(|err| format!("could not read entropy: {err}"))?;
    Ok(bytes)
}

/// Whether an address is worth handing to somebody else.
///
/// A node listens on every interface it has, and most of them cannot be reached
/// from anywhere else: loopback answers only this machine, and an IPv6
/// link-local address is scoped to one link and needs a zone index a joiner does
/// not have. Both are legitimate to *listen* on and useless to *publish*, and an
/// invite carries whatever was published.
pub fn is_worth_publishing(address: &libp2p::Multiaddr) -> bool {
    address.iter().all(|part| match part {
        libp2p::multiaddr::Protocol::Ip4(ip) => !ip.is_loopback() && !ip.is_unspecified(),
        libp2p::multiaddr::Protocol::Ip6(ip) => {
            !ip.is_loopback() && !ip.is_unspecified() && !is_link_local(&ip)
        }
        _ => true,
    })
}

/// `fe80::/10`, which `std` has no stable predicate for.
fn is_link_local(ip: &std::net::Ipv6Addr) -> bool {
    ip.segments()[0] & 0xffc0 == 0xfe80
}

/// Whether an address is a relay circuit rather than a direct socket.
///
/// The distinction matters for what a node should say when one goes away: a
/// direct listener closing is ordinary, and a circuit closing means this node
/// stopped being reachable from behind somebody else's NAT.
pub fn is_circuit_address(address: &libp2p::Multiaddr) -> bool {
    address
        .iter()
        .any(|part| matches!(part, libp2p::multiaddr::Protocol::P2pCircuit))
}

#[cfg(test)]
mod address_tests {
    use super::is_worth_publishing;
    use libp2p::Multiaddr;

    fn addr(text: &str) -> Multiaddr {
        text.parse().expect("valid multiaddr")
    }

    #[test]
    fn a_routable_address_is_published() {
        assert!(is_worth_publishing(&addr("/ip4/203.0.113.7/tcp/4001")));
        assert!(is_worth_publishing(&addr("/ip6/2001:db8::1/udp/4001/quic-v1")));
        // A private LAN address still is: two machines on one network reach
        // each other with it, and tier 1 is meant to find that.
        assert!(is_worth_publishing(&addr("/ip4/192.168.1.200/tcp/65519")));
    }

    #[test]
    fn what_cannot_answer_anybody_else_is_not() {
        // Loopback answers only this machine, and shipped in every invite.
        assert!(!is_worth_publishing(&addr("/ip4/127.0.0.1/tcp/65519")));
        assert!(!is_worth_publishing(&addr("/ip6/::1/tcp/65519")));
        // Link-local is scoped to one link and needs a zone index the joiner
        // does not have. Binding dual-stack is what made these appear.
        assert!(!is_worth_publishing(&addr("/ip6/fe80::1/tcp/4001")));
        assert!(!is_worth_publishing(&addr("/ip4/0.0.0.0/tcp/4001")));
    }

    #[test]
    fn a_circuit_address_survives_the_filter() {
        // Its IP belongs to the relay rather than to this node, and it is the
        // one address a peer behind NAT can actually use.
        assert!(is_worth_publishing(&addr(
            "/ip4/66.33.22.230/tcp/55503/p2p/12D3KooWDq3KKteeKPBfkcz39RuaqnT49BjhMiKAcnrVDDbw4Vtn/p2p-circuit"
        )));
    }
}

#[cfg(test)]
mod relay_tests {
    use super::parse_relay;

    const GOOD: &str = "/dns4/monorail.proxy.rlwy.net/tcp/54321/p2p/12D3KooWKiD4mNjfhqTJrfnhCPTVy8N8gCLjTZs3fMLZ4kkFMBBu";

    #[test]
    fn a_relay_address_with_a_peer_id_is_accepted() {
        assert_eq!(parse_relay(GOOD).as_deref(), Ok(GOOD));
        assert_eq!(parse_relay(&format!("  {GOOD}  ")).as_deref(), Ok(GOOD));
    }

    #[test]
    fn an_address_without_a_peer_id_is_refused() {
        // The one that matters. It parses, it dials, and nothing can tell
        // whether what answered is the relay this network designated — so it
        // has to be caught here rather than at the point of use, where it looks
        // like an ordinary connection.
        let refused = parse_relay("/dns4/monorail.proxy.rlwy.net/tcp/54321")
            .expect_err("an address naming no peer id is not a relay");
        assert!(
            refused.contains("peer id"),
            "the refusal should say what is missing, said {refused:?}"
        );
    }

    #[test]
    fn something_that_is_not_an_address_is_refused() {
        for wrong in ["monorail.proxy.rlwy.net:54321", "https://example.com", ""] {
            assert!(
                parse_relay(wrong).is_err(),
                "{wrong:?} is not a multiaddr and should be refused"
            );
        }
    }
}
