//! Redeeming an invite — Core §5.6–5.7, `design/02` §6.
//!
//! # Why this is its own command and not part of `serve`
//!
//! Joining ends. It runs a node because presenting an invite needs a
//! connection, and it stops as soon as the network has answered — admitted,
//! waiting, or refused. `serve` is the opposite shape: it runs until stopped.
//!
//! # What the invite's job is, and where it ends
//!
//! §5.7: "the invite's job ends at the first connection". It carries addresses
//! to dial and enough to verify who issued it, and everything after — the
//! governance log, the capability ledger, the epoch key — is ordinary
//! post-connection sync that `serve` does. So this deliberately does not fetch
//! anything. It gets the joiner a place in the network and gets out of the way.

use crate::store::Store;
use intranet_transport::{MemberNode, NodeEvent};
use libp2p::Multiaddr;
use std::path::PathBuf;
use std::time::Duration;

/// Redeems an invite, creating this node's store for the network it names.
pub fn run(root: PathBuf, uri: &str, timeout_secs: u64) -> Result<(), String> {
    let invite = crate::invite::from_uri(uri)?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("could not start a runtime: {err}"))?;
    runtime.block_on(join(root, invite, timeout_secs))
}

async fn join(
    root: PathBuf,
    invite: intranet_invite::Invite,
    timeout_secs: u64,
) -> Result<(), String> {
    // The identity is derived from the network id the invite names, so it can
    // only exist once the invite has been read. This is the same work `attach`
    // does, minus having to be told the network id by hand.
    let store = match Store::open(root.clone()) {
        Ok(existing) => {
            if existing.network() != &invite.network {
                return Err(format!(
                    "there is already a different network at {}. Use --home to keep them apart",
                    existing.root().display()
                ));
            }
            existing
        }
        Err(_) => {
            let entropy = crate::random_32()?;
            Store::create(root, invite.network, entropy).map_err(|e| e.to_string())?
        }
    };
    let identity = store.identity().map_err(|e| e.to_string())?;

    println!("redeeming an invite to {}", invite.network.short());
    println!("  you       {}", identity.id().short());
    println!("  issued by {}", invite.issuer.short());

    let mut node = MemberNode::new(&identity).map_err(|err| format!("could not start: {err}"))?;
    node.listen_on(
        "/ip4/0.0.0.0/tcp/0"
            .parse()
            .expect("a literal multiaddr parses"),
    )
    .map_err(|err| format!("could not listen: {err}"))?;

    for address in &invite.bootstrap_addresses {
        let parsed: Multiaddr = address
            .parse()
            .map_err(|err| format!("the invite carries an unusable address {address:?}: {err}"))?;
        node.dial_candidates([parsed])
            .map_err(|err| format!("could not dial {address}: {err}"))?;
        println!("  dialing   {address}");
    }

    // Remembered before the answer rather than after: these are how this node
    // reaches the network from now on, and they are worth keeping even if the
    // join is refused — the refusal may be "not yet", and the next attempt
    // should not need the invite again.
    store
        .set_peers(&invite.bootstrap_addresses)
        .map_err(|e| e.to_string())?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    let mut asked = false;

    loop {
        let event = tokio::select! {
            event = node.next_event() => event,
            () = tokio::time::sleep_until(deadline) => {
                return Err(format!(
                    "nobody answered within {timeout_secs}s. The addresses in the invite may be \\
                     stale, or the node that issued it may not be running"
                ));
            }
        };

        match event {
            NodeEvent::Connected { peer, .. } if !asked => {
                println!("connected to {peer}");
                // Asking the issuer, whom the invite names. Any member holding
                // `approve-node` could answer, but the issuer is the one this
                // invite came from and is therefore the one known to be willing.
                node.request_join(invite.issuer, invite.clone(), &identity);
                asked = true;
            }

            NodeEvent::Admitted { entry, .. } => {
                println!();
                println!("admitted to the network");
                println!("  entry     {}", &intranet_crypto::to_hex(entry.as_bytes())[..16]);
                println!();
                println!("Next: `kols name <name>` claims a display name, and `kols serve`");
                println!("syncs the log and asks to be keyed in.");
                return Ok(());
            }

            NodeEvent::AwaitingAdmission { .. } => {
                println!();
                println!("waiting to be admitted");
                println!();
                println!("This network screens its members, so the invite got you a connection");
                println!("and an identity here and nothing else — no content, no keys — until");
                println!("somebody admits you. Ask a member to run:");
                println!();
                println!(
                    "  kols admit {}",
                    intranet_crypto::to_hex(identity.id().verifying_key().as_bytes())
                );
                println!();
                println!("Then `kols serve` syncs the log and asks to be keyed in.");
                return Ok(());
            }

            NodeEvent::JoinRefused { reason, .. } => {
                return Err(format!("the join was refused: {reason}"));
            }

            _ => {}
        }
    }
}
