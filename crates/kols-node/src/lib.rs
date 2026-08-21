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
//! takes the persistence half (`STATUS` §6).

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
