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

/// Where a node's own lifecycle lines go — the relay, the dial, the hole punch.
///
/// # Why this exists beside `serve::Sink`
///
/// `Sink` already carries *events*, and `design/05` §3 deliberately leaves this
/// out of that vocabulary: the startup report is what the node **is** rather
/// than something that happened, and a sandboxed build gets no ambient host
/// access to report transport with. So these lines are not events and were
/// simply printed — which made [`serve::serve`] a layer that decides how
/// something looks, the one thing `Sink`'s own documentation says a second
/// interface cannot reuse.
///
/// **It stopped being only an inelegance on Windows.** A GUI-subsystem binary
/// launched from Explorer has no console, `GetStdHandle` hands back null, and
/// Rust's `print_to` panics on the write error rather than dropping it. So a
/// window that suppressed its console would have crashed on its first line of
/// output, and the fix for a stray terminal would have been worse than the
/// terminal. A front end with nowhere to print now passes [`quiet`].
pub type Report = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

/// A report that prints, for the terminal.
pub fn printing_report() -> Report {
    std::sync::Arc::new(|line: &str| println!("{line}"))
}

/// A report that goes nowhere, for a front end with no console to print to.
///
/// Not a loss of diagnostics in practice: what a window actually needs from
/// these — its relay standing, a degradation, whether it is keyed — already
/// reaches it as events, and reaches it more usefully than as text it would
/// have to parse.
pub fn quiet() -> Report {
    std::sync::Arc::new(|_line: &str| {})
}

/// Sends one line to a [`Report`], with `format!`'s syntax.
///
/// A macro rather than a call so a multi-line message stays a multi-line
/// message: every one of these was a `println!` and the conversion is a single
/// token at each site, which is what kept a mechanical change from becoming a
/// rewrite of twenty-five strings the daemon tests wait on.
#[macro_export]
macro_rules! say {
    ($report:expr, $($arg:tt)*) => {
        ($report)(&format!($($arg)*))
    };
}

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

/// How reachable an address is by somebody who was handed an invite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// A global IP or a name: anybody can try it.
    Anybody,
    /// A private IPv4 address, which answers somebody on the same LAN and
    /// nobody else. Two machines in one house are a case this must keep.
    SameLan,
    /// Reachable only from inside an overlay this joiner is not on — Tailscale's
    /// `100.64/10` and `fd7a:…`, and unique-local IPv6 generally.
    Overlay,
}

/// How far an address reaches, judged on the hop a dial actually starts with.
pub fn reach_of(address: &libp2p::Multiaddr) -> Reach {
    if intranet_transport::dial::first_hop_is_routable(address) {
        return Reach::Anybody;
    }
    for part in address.iter() {
        match part {
            // Everything past here names the peer beyond the relay, not the
            // hop being judged.
            libp2p::multiaddr::Protocol::P2pCircuit => break,
            libp2p::multiaddr::Protocol::Ip4(ip) if ip.is_private() => return Reach::SameLan,
            libp2p::multiaddr::Protocol::Ip4(_) | libp2p::multiaddr::Protocol::Ip6(_) => break,
            _ => {}
        }
    }
    Reach::Overlay
}

/// The most addresses an invite will carry.
///
/// Well under the wire ceiling of 32 (`intranet_invite::wire`), because that
/// bound exists to stop an attacker inflating a message and this one exists to
/// keep an invite short enough to paste. A node with more than a dozen
/// *reachable* addresses is a node whose extra addresses nobody will get to.
pub const INVITE_ADDRESS_LIMIT: usize = 12;

/// The addresses an invite should carry, out of everything this node listens on.
///
/// # Why this is a selection and not a filter
///
/// A node listens on every interface it has, and an invite used to carry all of
/// them. One real machine produced twenty-three: three global IPv6 addresses and
/// one real LAN address, and beside them a Tailscale pair, three virtual-adapter
/// subnets left by VirtualBox and two hypervisors, four circuit addresses
/// through a relay's private container IPs — each doubled by TCP and QUIC. That
/// is a 4,300-character invite of which the great majority cannot answer
/// anybody, and the length is not the only cost: a joiner dials them in order,
/// and circuit addresses through one relay share a connection, so the dead ones
/// are actively harmful (Core §5.2).
///
/// Three rules, in order of how much they remove:
///
/// 1. **An overlay address is dropped.** Somebody on your tailnet does not need
///    your invite to find you, and somebody who is not on it can never use the
///    address. This is the one the machine above was asked about by name.
/// 2. **A LAN address survives only if it is the one this machine actually uses.**
///    Nothing in `192.168.56.1` says VirtualBox and nothing in `192.168.1.200`
///    says real; the routing table is what knows the difference, so `preferred`
///    is asked rather than the text (see `preferred_source_addresses`). If it
///    knows nothing, every LAN address is kept — a long invite is a worse
///    outcome than a house where two machines cannot find each other.
///
///    This asks about *this machine's* interfaces, so it is asked of direct
///    addresses only. A circuit's hop is the relay's address, and the question
///    there is a different one: **a relay already offered at an address a
///    stranger can reach is not also offered at a private one.** That drops the
///    container address a hosted relay leaks, while a relay whose only hop is
///    private — a member relaying on the LAN — is kept, because for that
///    network it is the way in.
/// 3. **What is left is capped**, reachable-by-anybody first, so a pathological
///    interface list cannot produce an invite nobody can paste.
///
/// Loopback, link-local and unspecified are gone before any of this, by
/// `is_worth_publishing`: they are not a matter of degree.
pub fn addresses_for_an_invite(
    listening: &[libp2p::Multiaddr],
    preferred: &[std::net::IpAddr],
) -> Vec<String> {
    let worth: Vec<&libp2p::Multiaddr> = listening
        .iter()
        .filter(|address| is_worth_publishing(address))
        .filter(|address| reach_of(address) != Reach::Overlay)
        .collect();

    // The relays this node has a reservation on that are reachable from
    // outside. A relay announcing a private address beside a public one is the
    // deployment that caused this: the extra hop is not a second way in, it is
    // the same way in written wrongly, and dialling it can cancel the requests
    // queued behind it (Core §5.2).
    let reachable_relays: std::collections::BTreeSet<libp2p::PeerId> = worth
        .iter()
        .filter(|address| reach_of(address) == Reach::Anybody)
        .filter_map(|address| relay_peer(address))
        .collect();

    // Asked over direct addresses only, and before pruning: if the routing
    // table named an address this node is not listening on, it has told us
    // nothing about these, and dropping the lot on that basis would leave a
    // same-LAN join with nothing to dial.
    let preferred_is_useful = worth
        .iter()
        .filter(|address| !is_circuit_address(address) && reach_of(address) == Reach::SameLan)
        .any(|address| first_ip(address).is_some_and(|ip| preferred.contains(&ip)));

    let mut chosen: Vec<&libp2p::Multiaddr> = worth
        .into_iter()
        .filter(|address| {
            if reach_of(address) == Reach::Anybody {
                return true;
            }
            match relay_peer(address) {
                // A circuit. Its hop names the relay rather than this machine,
                // so the routing table has nothing to say about it — the
                // question is only whether the same relay was also offered at
                // an address a stranger can reach. If it was not, this is a
                // relay on the LAN and the only way in.
                Some(relay) => !reachable_relays.contains(&relay),
                // A LAN address of this machine's own, kept only if it is the
                // one this machine actually uses.
                None => {
                    !preferred_is_useful
                        || first_ip(address).is_some_and(|ip| preferred.contains(&ip))
                }
            }
        })
        .collect();

    // Only decides what the cap keeps. The joiner orders what it receives for
    // itself (`dial::order_candidates`), which is where dialling order is
    // settled.
    chosen.sort_by_key(|address| u8::from(reach_of(address) != Reach::Anybody));
    chosen.truncate(INVITE_ADDRESS_LIMIT);
    chosen.iter().map(ToString::to_string).collect()
}

/// The relay a circuit address goes through, which is the peer named just
/// before `/p2p-circuit`. `None` for a direct address.
fn relay_peer(address: &libp2p::Multiaddr) -> Option<libp2p::PeerId> {
    let mut relay = None;
    for part in address.iter() {
        match part {
            libp2p::multiaddr::Protocol::P2p(peer) => relay = Some(peer),
            libp2p::multiaddr::Protocol::P2pCircuit => return relay,
            _ => {}
        }
    }
    None
}

/// The IP a dial to this address would start at, if it starts at one.
fn first_ip(address: &libp2p::Multiaddr) -> Option<std::net::IpAddr> {
    address.iter().find_map(|part| match part {
        libp2p::multiaddr::Protocol::Ip4(ip) => Some(std::net::IpAddr::V4(ip)),
        libp2p::multiaddr::Protocol::Ip6(ip) => Some(std::net::IpAddr::V6(ip)),
        _ => None,
    })
}

/// The addresses this machine would send from, one per family.
///
/// Connecting a UDP socket sends no packet: it asks the routing table which
/// interface would carry one to that destination, and the answer is that
/// interface's own address. That is the only thing on this machine that can
/// tell a real LAN address from a hypervisor's, which is why an invite asks.
///
/// Returns what it can. A machine with no IPv6 route, or none at all, is not an
/// error here — `addresses_for_an_invite` keeps every LAN address when this
/// says nothing useful.
pub fn preferred_source_addresses() -> Vec<std::net::IpAddr> {
    // Documentation ranges (RFC 5737, RFC 3849). Nothing is sent to them; they
    // are addresses to *route to*, and they route the same way as anything else
    // off this link — down the default route.
    [("0.0.0.0:0", "192.0.2.1:9"), ("[::]:0", "[2001:db8::1]:9")]
        .into_iter()
        .filter_map(|(bind, probe)| {
            let socket = std::net::UdpSocket::bind(bind).ok()?;
            socket.connect(probe).ok()?;
            socket.local_addr().ok().map(|address| address.ip())
        })
        .collect()
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
mod invite_address_tests {
    use super::{addresses_for_an_invite, reach_of, Reach, INVITE_ADDRESS_LIMIT};
    use libp2p::Multiaddr;

    const ME: &str = "12D3KooWMHZbUfFYuqe6NxXBFSg3aLzfTSa1B5QKGNKwMrWz5FaD";
    const RELAY: &str = "12D3KooWAT1R2JjcZbnVUKLX8Xo1Qg5APTWMkpHarHY4Uo1YpGzT";

    fn addr(text: &str) -> Multiaddr {
        text.parse().expect("valid multiaddr")
    }

    /// A direct listen address, as `serve` records it — peer id appended.
    fn direct(host: &str) -> Multiaddr {
        let family = if host.contains(':') { "ip6" } else { "ip4" };
        addr(&format!("/{family}/{host}/tcp/65343/p2p/{ME}"))
    }

    /// A circuit through a relay reachable at `hop`.
    fn circuit(hop: &str) -> Multiaddr {
        let family = if host_is_name(hop) {
            "dns4"
        } else if hop.contains(':') {
            "ip6"
        } else {
            "ip4"
        };
        addr(&format!(
            "/{family}/{hop}/tcp/4001/p2p/{RELAY}/p2p-circuit/p2p/{ME}"
        ))
    }

    fn host_is_name(host: &str) -> bool {
        host.chars().any(|c| c.is_ascii_alphabetic()) && !host.contains(':')
    }

    /// Everything one real machine listened on, which is what prompted this.
    /// Only the TCP half is listed; QUIC doubled every direct address.
    fn a_real_machine() -> Vec<Multiaddr> {
        vec![
            direct("2600:1700:a825:4800::3f"),
            direct("2600:1700:a825:4800:634c:8370:a381:9f64"),
            direct("2600:1700:a825:4800:d46f:a0b3:57f5:bbc7"),
            direct("fd7a:115c:a1e0::4b36:9876"), // Tailscale
            direct("100.101.152.117"),           // Tailscale
            direct("192.168.1.200"),             // the real LAN
            direct("192.168.56.1"),              // VirtualBox
            direct("192.168.23.1"),              // a hypervisor
            direct("192.168.220.1"),             // another one
            direct("127.0.0.1"),
            direct("fe80::1"),
            circuit("switchback.proxy.rlwy.net"),
            circuit("10.140.152.103"), // the relay's container address
            circuit("fd12:a6a1:7ec6:1::9867"),
        ]
    }

    fn lan() -> Vec<std::net::IpAddr> {
        vec!["192.168.1.200".parse().expect("valid ip")]
    }

    #[test]
    fn an_overlay_address_never_reaches_the_person_you_invited() {
        // Both halves of Tailscale. Somebody on the tailnet does not need the
        // invite to find this machine; somebody who is not on it never can.
        assert_eq!(reach_of(&direct("100.101.152.117")), Reach::Overlay);
        assert_eq!(reach_of(&direct("fd7a:115c:a1e0::4b36:9876")), Reach::Overlay);
        // A private relay hop is not overlay — a member relaying on the LAN
        // looks exactly like this, and `addresses_for_an_invite` is what tells
        // the two apart, by whether the same relay was also offered publicly.
        assert_eq!(reach_of(&circuit("10.140.152.103")), Reach::SameLan);

        assert_eq!(reach_of(&direct("2600:1700:a825:4800::3f")), Reach::Anybody);
        assert_eq!(reach_of(&circuit("switchback.proxy.rlwy.net")), Reach::Anybody);
        assert_eq!(reach_of(&direct("192.168.1.200")), Reach::SameLan);
    }

    #[test]
    fn a_real_machines_invite_names_only_what_can_answer() {
        let chosen = addresses_for_an_invite(&a_real_machine(), &lan());

        let expected = [
            "2600:1700:a825:4800::3f",
            "2600:1700:a825:4800:634c:8370:a381:9f64",
            "2600:1700:a825:4800:d46f:a0b3:57f5:bbc7",
            "switchback.proxy.rlwy.net",
            "192.168.1.200",
        ];
        for host in expected {
            assert!(
                chosen.iter().any(|address| address.contains(host)),
                "{host} must survive, got {chosen:#?}"
            );
        }
        // The hypervisors, Tailscale, the relay's container addresses, and what
        // was never publishable to begin with.
        for host in [
            "192.168.56.1",
            "192.168.23.1",
            "192.168.220.1",
            "100.101.152.117",
            "fd7a:",
            "10.140.152.103",
            "fd12:",
            "127.0.0.1",
            "fe80:",
        ] {
            assert!(
                !chosen.iter().any(|address| address.contains(host)),
                "{host} must not survive, got {chosen:#?}"
            );
        }
        assert_eq!(chosen.len(), expected.len(), "{chosen:#?}");
    }

    #[test]
    fn a_lan_relay_survives_even_though_a_lan_relay_hop_is_not_routable() {
        // A network whose only relay is a member on the same LAN is legitimate,
        // and dropping its circuit would leave that network with nothing. The
        // hop is private rather than overlay, so it is kept — and, being a
        // circuit, it is not subject to the preferred-source pruning either.
        let chosen = addresses_for_an_invite(&[circuit("192.168.1.7")], &lan());
        assert_eq!(chosen.len(), 1, "{chosen:#?}");
    }

    #[test]
    fn a_relay_offered_publicly_is_not_also_offered_privately() {
        // The deployment that caused all this: one relay announcing its
        // container addresses beside its public name, so the invite carried
        // four circuits through the same peer and the joiner dialled the dead
        // ones first. Same relay peer, so the private hops are redundant.
        let chosen = addresses_for_an_invite(
            &[
                circuit("switchback.proxy.rlwy.net"),
                circuit("10.140.152.103"),
                circuit("fd12:a6a1:7ec6:1::9867"),
            ],
            &lan(),
        );
        assert_eq!(chosen.len(), 1, "{chosen:#?}");
        assert!(chosen[0].contains("switchback"), "{chosen:#?}");
    }

    #[test]
    fn a_lan_relay_is_kept_beside_this_machines_own_lan_address() {
        // The pruning in the test above is about this machine's interfaces. A
        // relay on the LAN is a *different* machine, so knowing which of our
        // own addresses is real says nothing about it — and it is this
        // network's only way in.
        let chosen = addresses_for_an_invite(
            &[direct("192.168.1.200"), direct("192.168.56.1"), circuit("192.168.1.7")],
            &lan(),
        );
        assert!(chosen.iter().any(|address| address.contains("192.168.1.7")), "{chosen:#?}");
        assert!(chosen.iter().any(|address| address.contains("192.168.1.200")), "{chosen:#?}");
        assert!(!chosen.iter().any(|address| address.contains("192.168.56.1")), "{chosen:#?}");
    }

    #[test]
    fn a_routing_table_that_says_nothing_useful_keeps_every_lan_address() {
        // A machine routing its default through Tailscale reports a `100.64/10`
        // source, which matches none of these. Pruning on that basis would
        // leave two machines in one house unable to find each other, so an
        // answer that explains nothing prunes nothing.
        let listening = a_real_machine();
        let elsewhere = vec!["100.101.152.117".parse().expect("valid ip")];

        let chosen = addresses_for_an_invite(&listening, &elsewhere);
        for host in ["192.168.1.200", "192.168.56.1", "192.168.220.1"] {
            assert!(
                chosen.iter().any(|address| address.contains(host)),
                "{host} must survive, got {chosen:#?}"
            );
        }
        // Failing open is about LAN addresses only; the overlay is still gone.
        assert!(!chosen.iter().any(|address| address.contains("100.101")));

        // And the same when nothing at all could be probed.
        assert_eq!(chosen, addresses_for_an_invite(&listening, &[]));
    }

    #[test]
    fn an_invite_stays_short_enough_to_paste() {
        // Every one of these is reachable, so nothing above the cap removes
        // them; the cap is what stops an invite nobody can paste.
        let listening: Vec<Multiaddr> = (0..40)
            .map(|n| direct(&format!("2600:1700:a825:4800::{n:x}")))
            .chain(std::iter::once(direct("192.168.1.200")))
            .collect();

        let chosen = addresses_for_an_invite(&listening, &lan());
        assert_eq!(chosen.len(), INVITE_ADDRESS_LIMIT);
        // What the cap dropped is what somebody was least likely to reach.
        assert!(
            chosen.iter().all(|address| !address.contains("192.168")),
            "{chosen:#?}"
        );
    }

    #[test]
    fn the_routing_table_names_an_address_this_machine_holds() {
        // Not asserting *which* — that is the point of asking the OS. Only that
        // the probe answers on an ordinary machine rather than silently
        // returning nothing and failing open forever.
        let preferred = super::preferred_source_addresses();
        assert!(
            preferred.iter().any(|ip| !ip.is_loopback() && !ip.is_unspecified()),
            "no usable source address: {preferred:?}"
        );
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
