//! The client's composition layer — the store, the node, and the executor.
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

/// 32 bytes from the OS.
pub fn random_32() -> Result<[u8; 32], String> {
    let mut bytes = [0u8; 32];
    intranet_crypto::random_bytes(&mut bytes)
        .map_err(|err| format!("could not read entropy: {err}"))?;
    Ok(bytes)
}
