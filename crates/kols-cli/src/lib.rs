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
pub mod network;
pub mod serve;
pub mod store;

/// 32 bytes from the OS.
pub fn random_32() -> Result<[u8; 32], String> {
    let mut bytes = [0u8; 32];
    intranet_crypto::random_bytes(&mut bytes)
        .map_err(|err| format!("could not read entropy: {err}"))?;
    Ok(bytes)
}
