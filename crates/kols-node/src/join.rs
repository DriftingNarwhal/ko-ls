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

/// Where a redeemed invite left this node.
///
/// Both are successful joins (Core §2.4), and a client that treated the second
/// as a failure would report a network working as configured as though
/// something had gone wrong.
#[derive(Debug, Clone)]
pub enum Landed {
    /// Admitted outright — the network auto-admits.
    Admitted,
    /// Given a waiting-room place, pending an admin.
    Waiting {
        /// This member's identity here, as hex, for whoever will admit them.
        identity: String,
    },
}

/// Redeems an invite from a terminal, printing what happened.
pub fn run(root: PathBuf, uri: &str, timeout_secs: u64) -> Result<(), String> {
    let invite = crate::invite::from_uri(uri)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("could not start a runtime: {err}"))?;

    println!("redeeming an invite to {}", invite.network.short());
    let landed = runtime.block_on(redeem(root, invite, timeout_secs, true))?;

    println!();
    match landed {
        Landed::Admitted => {
            println!("admitted to the network");
            println!();
            println!("Next: `kols name <name>` claims a display name, and `kols serve`");
            println!("syncs the log and asks to be keyed in.");
        }
        Landed::Waiting { identity } => {
            println!("waiting to be admitted");
            println!();
            println!("This network screens its members, so the invite got you a connection");
            println!("and an identity here and nothing else — no content, no keys — until");
            println!("somebody admits you. Ask a member to run:");
            println!();
            println!("  kols admit {identity}");
            println!();
            println!("Then `kols serve` syncs the log and asks to be keyed in.");
        }
    }
    Ok(())
}

/// Redeems an invite, creating this node's store for the network it names.
///
/// `chatty` is whether to narrate progress. A terminal wants the dialling
/// reported as it happens, because a join that is going to fail spends its whole
/// timeout looking identical to one that is about to work; a window shows a
/// spinner and the outcome.
pub async fn redeem(
    root: PathBuf,
    invite: intranet_invite::Invite,
    timeout_secs: u64,
    chatty: bool,
) -> Result<Landed, String> {
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

    if chatty {
        println!("  you       {}", identity.id().short());
        println!("  issued by {}", invite.issuer.short());
    }

    let mut node = MemberNode::new(&identity).map_err(|err| format!("could not start: {err}"))?;
    node.listen_on(
        "/ip4/0.0.0.0/tcp/0"
            .parse()
            .expect("a literal multiaddr parses"),
    )
    .map_err(|err| err.to_string())?;

    // **Every address is attempted, and one failing no longer ends the join.**
    //
    // This used to `?` on each parse and each dial, which reads as strict and is
    // exactly backwards here. An invite carries the issuer's LAN addresses *and*
    // its relay circuit, and the circuit is always last — `order_candidates`
    // puts it there deliberately, and it arrives last anyway because a
    // reservation completes after the direct listeners come up. So the one
    // address that can reach somebody on another network was the one an earlier
    // failure stopped this from ever dialling, and the relay saw no connection
    // attempt at all.
    //
    // Invisible on one LAN, which is where this was tested: there the first
    // address works and the circuit is never needed. It shows up only between
    // two networks, which is the case Core §5.5 exists for.
    let mut candidates = Vec::new();
    let mut refused = Vec::new();
    for address in &invite.bootstrap_addresses {
        match address.parse::<Multiaddr>() {
            Ok(parsed) => candidates.push(parsed),
            Err(err) => refused.push(format!("{address} ({err})")),
        }
    }

    // Ordered here rather than taken in the invite's order, because dialling
    // them one at a time meant `dial_candidates` only ever saw a single address
    // and its ordering never applied: direct before circuit, IPv6 before IPv4.
    let mut dialed = Vec::new();
    for parsed in intranet_transport::dial::order_candidates(candidates) {
        match node.dial_candidates([parsed.clone()]) {
            Ok(()) => {
                if chatty {
                    println!("  dialing   {parsed}");
                }
                dialed.push(parsed.to_string());
            }
            Err(err) => refused.push(format!("{parsed} ({err})")),
        }
    }

    if dialed.is_empty() {
        return Err(format!(
            "not one of the invite's address(es) could be dialled, so nothing was contacted \
             — not the issuer, and not their relay: {}",
            refused.join("; ")
        ));
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
    // **Why each dial failed, which the transport reports and this used to
    // discard.** `DialFailed` carries the reason, the match below ended in a
    // catch-all, and the join then reported a bare "nobody answered" — so a
    // relay that refused the connection, a name that would not resolve and a
    // port a network silently drops all looked identical from the joining end.
    // That is the difference between a person saying "it did not work" and
    // saying what did not work.
    let mut failures: Vec<String> = Vec::new();

    loop {
        let event = tokio::select! {
            event = node.next_event() => event,
            () = tokio::time::sleep_until(deadline) => {
                // Says what was actually tried. The previous wording named two
                // causes and left the interesting third invisible: an address
                // that could not be dialled at all reached nobody, and a joiner
                // reading "nobody answered" had no way to tell a silent relay
                // from one that was never contacted.
                let mut why =
                    format!("nobody answered within {timeout_secs}s. Dialled: {}", dialed.join(", "));
                if !refused.is_empty() {
                    why.push_str(&format!(". Could not be dialled: {}", refused.join("; ")));
                }
                if !dialed.iter().any(|address| address.contains("p2p-circuit")) {
                    why.push_str(
                        ". None of these was a relay circuit, so they only reach somebody on \
                         the same network as the issuer — the invite may have been minted \
                         without one",
                    );
                }
                if failures.is_empty() {
                    // Nothing failed and nothing answered: the connections were
                    // still in flight when time ran out, which is what a network
                    // silently dropping the packets looks like from here.
                    why.push_str(
                        ". Nothing reported a failure either, so the connections were still \
                         outstanding — which is what a network that drops the traffic rather \
                         than refusing it looks like from this end",
                    );
                } else {
                    why.push_str(&format!(". Failures: {}", failures.join("; ")));
                }
                why.push_str(
                    ". Otherwise the addresses may be stale, or the node that issued the \
                     invite may not be running",
                );
                return Err(why);
            }
        };

        match event {
            NodeEvent::Connected { peer, .. } if !asked => {
                if chatty {
                    println!("connected to {peer}");
                }
                // Asking the issuer, whom the invite names. Any member holding
                // `approve-node` could answer, but the issuer is the one this
                // invite came from and is therefore the one known to be willing.
                node.request_join(invite.issuer, invite.clone(), &identity);
                asked = true;
            }

            NodeEvent::Admitted { .. } => return Ok(Landed::Admitted),

            NodeEvent::AwaitingAdmission { .. } => {
                return Ok(Landed::Waiting {
                    identity: intranet_crypto::to_hex(
                        identity.id().verifying_key().as_bytes(),
                    ),
                });
            }

            NodeEvent::JoinRefused { reason, .. } => {
                return Err(format!("the join was refused: {reason}"));
            }

            NodeEvent::DialFailed { peer, error } => {
                let who = peer.map_or_else(|| "an address".to_owned(), |peer| peer.to_string());
                let line = format!("{who}: {error}");
                // Deduplicated because a single unreachable address is retried
                // and would otherwise fill the refusal with one repeated line.
                if !failures.contains(&line) {
                    if chatty {
                        println!("  no answer {line}");
                    }
                    failures.push(line);
                }
            }

            _ => {}
        }
    }
}
