# ko-ls — Implementation Status

**Updated:** 2026-08-21 (documentation pass — `design/06` described the opposite of what E2 landed, and a divergence from Core §1.1 had never been written down)
**Phase:** P1 — two nodes talk live and durably, a joiner reads back through sealed
history, and the boundary carries commands in and events out
**Design:** [`design/`](design/) — `00`–`08` at v1.0, `09` at v0.3. **`distributed-intranet/specs/07` is normative** where it and the design set overlap.

This file is the answer to "where are we?". It is updated in the same change that moves
work, never afterwards from memory — a status file that lags is worse than none, because
it is believed.

---

## 0. Resuming After a Break

**Read this section, then §1. Everything else is reference.** This section describes the
present. What happened, and why, is §9 — do not read it to find out where things stand.

### What this is

A Discord-shaped chat client on the Distributed Intranet protocol. A "server" is a network:
its own governance log, membership, epoch key chain and DHT namespace. Read
[`design/00`](design/00-overview.md) for why anything is the way it is; **`../distributed-intranet/specs/07` is
normative** where it and the design set overlap.

Two repositories, side by side, both on `main` and pushed:

| Repo | Remote |
|---|---|
| `ko-ls` (this one) | `DriftingNarwhal/ko-ls` (public) |
| `../distributed-intranet` | `DriftingNarwhal/distributed-intranet` — spec 07 plus E2, E4, E5, E9, E11, E12, MLS persistence, invite serialization and `NetworkPolicy.bootstrap_relays` |

The client builds against the sibling checkout by **path dependency** while the extensions
still move, so a fresh machine needs both cloned as siblings. `.devcontainer/` lives in *this*
repo and builds both, plus the Tauri toolchain — open the `ko-ls` folder in it, not the parent.

### The current milestone

**A client that can be handed to somebody else, so two people on entirely separate networks
can talk, using a bootstrap relay and no VPS.** The user's first test is two of their own
laptops, one on a mobile hotspot.

What that needs, and where it stands:

| | |
|---|---|
| Relay reachability — a network designates relays, a node reserves a circuit, invites carry it | **done** |
| One string to join — `kols invite` / `kols join` | **done** |
| A window that creates, joins, opens and runs a node for a network | **done** |
| **A Windows build** | **Both binaries cross-compile**, and `kols.exe` runs — network created, seed correctly restricted. `kols-desktop.exe` is built and unrun |
| Minting an invite from the window | **done**, with the waiting room and admitting beside it. No step of the flow now needs a terminal |

### Start here tomorrow

1. `cargo test` — **192 here, 649 in `../distributed-intranet`** — and
   `cargo clippy --workspace --all-targets` clean in both. Both trees were left green; if they
   are not, fix that before anything else. Clippy on the Windows target is a *second* run,
   `cargo clippy -p kols-node --target x86_64-pc-windows-gnu`, and it caught something the
   Linux one cannot see, so it is worth making when `src/secret.rs` changes.
2. **The two-machine test is the only thing left, and it needs a relay nobody has deployed.**
   [`docs/two-machine-test.md`](docs/two-machine-test.md) is the step-by-step, written to be
   followed on two machines and deleted once it is either routine or wrong.
   Both machines are behind NAT, so they cannot reach each other directly (Core §5.5) — one
   bootstrap relay on a routable address is a standing dependency, not a convenience.
   DI-Relay deploys one; `intranet-harness relay` is the local equivalent and is useless
   here, because it has to be reachable from both. Everything either end does after that is
   built and unproven across real networks.
3. **`kols-desktop` builds in CI and ships in the release** — Windows and Apple Silicon macOS,
   installer and portable binary each. It cannot be *built* in this container (Tauri wants a
   platform toolchain here) but that stopped being a blocker when the workflow started building
   it. What is unproven is the window as a window: it has been opened once, before it was wired,
   and no flow has been run through it. `design/07` §2 S3 records what the Linux toolchain
   needed.

### Building, and where tests run

**Tests run here. GitHub builds.** `cargo test --workspace` and
`cargo clippy --workspace --all-targets` in this container before every commit, plus
`taskset -c 0,1 cargo test -p kols-node --test two_nodes` when timing is in question — a starved
machine loses races a 24-core one wins every time, and that is how the keying bug surfaced.
**Clean up afterwards**: the container's storage is the host's, and a day of cross-compiles took
`target/` to 11 GB.

`.github/workflows/release.yml` is the only workflow, and it runs **only when a build is
wanted** — a `v*` tag publishes one, a `workflow_dispatch` produces the artifacts without
publishing. Not on every push: nothing in it belongs on the path of an ordinary commit.

It builds `windows-latest` and `macos-latest` (Apple Silicon), and each leg
runs the narrow set of tests that can only run there — `cargo test -p kols-node --lib`, the store
and the seed — followed by a check on the **artifact itself** that the seed it writes is
readable by nobody else. That last one exists because the Rust tests for it are
`#[cfg(all(test, unix))]`: the Windows DACL path compiles out of them, so a `cargo test` leg on
Windows would prove nothing about the one function written specifically for Windows.

Deliberately not in CI: the `two_nodes` daemon suite. It tests merge, gossip and backfill —
none of it platform-specific, all of it timing-sensitive — and it belongs where its output is
complete and a failure can be reproduced in the same minute it appears.

### What to watch out for

- **`kols invite` needs a relay and a circuit**, and refuses without either. Run one with
  `cargo run -p intranet-harness -- relay --seed 1 --network 42 --listen /ip4/0.0.0.0/tcp/4001`,
  or deploy DI-Relay. **Bind it to a routable address, never loopback** — a relay only
  promotes non-loopback listen addresses to external ones, and libp2p builds a reservation's
  address list from external addresses alone, so a loopback relay grants reservations that
  carry nothing and everything downstream looks broken for the wrong reason.
- **Only one process may run a node per network.** The window runs one now, so `kols serve`
  on the same store is refused. The claim expires after 30 seconds without a heartbeat, so a
  crash costs a pause rather than a stuck store.
- **Nobody has looked hard at the interface.** The X forwarding into this container is
  intermittent, so the layout is unreviewed beyond the user confirming the picker renders.
  `design/09` §7's navigation question is open by default, not by decision.
- The scratchpad note about `ko-ls/target`: the dev container now mounts a named volume there,
  so whatever sits at `ko-ls/target` on the host is shadowed once it is rebuilt, and wants
  deleting from outside the container.

## 1. Right Now

| | |
|---|---|
| **Working on** | Milestone: a client two people on separate networks can use, **tested through the window alone**. E14 landed, so a joiner that goes unanswered now keeps asking instead of stranding. Both binaries build for Windows and macOS in CI and v0.1.0 publishes them. The window now does relay setup too (O12/O13 closed 2026-08-21), so no step of the flow needs a terminal. Honest caveat: the window **has** been launched and rendered roughly correctly, but that was before anything was wired to it, so no flow has ever been exercised through it. What is left is a relay reachable from both machines |
| **Blocked on** | Nothing |
| **Runnable** | **`kols-desktop`** — *the product* (D30). A window that creates a network or joins one by invite, runs a node for it, **generates a relay identity**, designates relays and reports whether one granted a circuit, lists channels, renders one, posts, **reacts, revises, withdraws and pins**, mints an invite and admits from the waiting room, updating as records arrive. Every step of the two-machine test is reachable from it and none needs a terminal. **Launched once, before it was wired — it rendered, and no flow has been run through it.** **`kols`** — *a development tool, not a product surface*: init (with `--relay`), relay list/set, invite, join, waiting, attach, admit, revoke, name, serve, post, read, edit, delete, react, pin, and channel create/list/rename/topic/slowmode/archive. `cargo test` — 192 tests here and 649 in `../distributed-intranet`, clippy clean in both; `scripts/cross-check.sh` for big-endian, `taskset -c 0,1` for the starved case |
| **Next decision needed from the user** | Nothing blocking |

---

## 2. Finalization

| Item | State | Notes |
|---|---|---|
| F1 record encoding | **Done** | `design/08-record-encoding.md`, normative |
| F2 spec 07 in protocol repo | **Done** | `distributed-intranet/specs/07-chat-application-spec.md`, committed there. README and CLAUDE.md updated — the repo no longer claims six specs |
| F3 design set → v1.0 | **Done** | `00`–`08` at v1.0. The review pass demoted `08` from normative (its content is upstream now), refreshed the roadmap and scale claims, and turned `05` §8's test plan into a table with real state per row. `09` was written after that pass and is at v0.3 — it describes an interface that is a first pass rather than a settled one, so v1.0 would be a claim about work that has not been reviewed |

## 3. Setup

| Item | State | Notes |
|---|---|---|
| S1 client repo | **Done** | `/workspaces/ko-ls/ko-ls`, design moved in, crates building. Committed and pushed to `DriftingNarwhal/ko-ls` on `main`, in step with `origin` — §0's table is the current statement of that |
| S2 protocol changes on `main` | In progress | E9 (Core §2.6.2), E2 (Core §2.7.2), E5, E4, E11 (Core §2.2.1) and E12 (Core §5.1.1) landed, each with spec text, implementation and tests together. **E14 is P1 and outstanding** — found by a bug rather than by planning; E7, E10 and E13 are P2 |
| S3 Tauri environment | **Done** | Node 24 LTS, webkit2gtk 4.1 and the GTK stack, bundler tools, a generated UTF-8 locale — all in `.devcontainer/Dockerfile`, so a rebuild has them rather than each machine acquiring them by hand. Proven by building a Tauri v2 app and watching a window map |

## 4. Protocol Extensions

Tracked against `design/06-protocol-extensions.md`. Landing rule: spec text, implementation
and tests together, with `cargo test --workspace` and `cargo clippy --workspace --all-targets`
both green.

| # | Extension | State |
|---|---|---|
| E1 | Extension capability registry | **Withdrawn** — already implemented upstream, needs configuration only |
| E2 | Channel governance entries | **Landed, generalised** — one `AppEntry` variant (Core §2.7.2) rather than four chat-shaped ones; chat records become payloads |
| E3 | Derived pointer ids | **Withdrawn** — `PointerId::from_bytes` is already public; derivation lives in `kols-core::ids` |
| E4 | Gossipsub live delivery | **Landed** — Core §5.1 carries gossipsub; sealed payloads per spec 07 §5.2/§6.1 |
| E5 | Media fan-out at the relay | **Landed early** — Real-Time §2.2.1; `Recipient::{One, Participants}`, envelope domain tag now `v2`. Relay resource ceilings landed with it (§2.2.2, `media_limits`) |
| E6 | QUIC datagram media path | Not started (P3) |
| E7 | Channel-scoped MLS groups | Not started (P2) |
| E8 | Track metadata in media payloads | Not started (P4) |
| E9 | App-layer policy map | **Landed** — `PolicyValue`, namespaced keys, Core §2.6.2; client accessors in `kols-core::policy` |
| E10 | Direct DM invite delivery | Not started (P2) |
| E11 | Namespace registration for extension capabilities | **Landed** — Core §2.2.1; one registry entry per verb covers every scope of it |
| E12 | Optional peer discovery | **Landed, narrowed** — Core §5.1.1; a node MAY be built without Kademlia and mDNS. Asked as *tiered liveness*; only the behaviour set was the protocol's, so the hot/warm/cold tiering stayed client-side |
| E13 | Cross-network connection bootstrap | **Load-bearing since O11, not merely convenient.** From `design/09` §3. If a relay cannot be shared between networks and every direct message is its own network, then without this a conversation across NAT needs its own deployed relay — which nobody will do, so it means no DMs at all. It was scoped as removing friction; it is now the mechanism (P2) |
| E14 | Idempotent epoch-key delivery | **Landed** — Core §3.5.1; a repeat request **replaces** the requester's leaf rather than adding a second one. Asked as key re-delivery, which would have restored no group state |
| E15 | Independent per-network seeds | **New** — from reading `design/06` against Core §1.1, which specifies one master seed with per-network *derivation*. D28 generates fresh entropy per network instead, and has since before the decision was recorded. Spec text only; blocks nothing, and belongs beside O7 |

## 5. Client Crates

| Crate | State |
|---|---|
| `kols-core` | **Encoding, author logs, merge, collision recovery, chat policy, channel structure** — records/segments/ids, `AuthorLog` incl. `rebase`, `ChannelView`, permissions, capability vocabulary, `ChatPolicy`, `ChannelEntry`. 88 tests |
| `kols-net` | **Publish and fetch** — stores/announces chunks, accepts pointers, reassembles segments. Two live two-node tests |
| `kols-api` | **The whole boundary** — `Command`, `Sensitivity`, `Refusal` and `authorize` returning an `Authorized` nothing else can construct, going in; `Outcome` and `Event` coming out. All three of `design/05` §3's properties are now held. 26 tests |
| `kols-node` | **`kols`, its node daemon, and the executor** — a library now, with the binary as argument parsing and rendering over it. Creates a network, admits and keys in joiners, serves and fetches content, writes every record kind, renders a merged view across authors. `secret` restricts a written seed to this user on both platforms and refuses when it cannot. Tests that drive the real binaries, ten of them over a live wire between two processes |
| `kols-app` | **The desktop shell** — a Tauri v2 window over the boundary, holding a *workspace* of networks and an `Executor` for whichever is open, with the view types the webview receives and the handlers that build commands from plain arguments — including minting an invite, reading the waiting room and admitting from it. `kols-desktop`. 7 tests |
| `kols-ui` | **The interface** — HTML, CSS and one script, holding no keys, no sockets and no files. Creates and picks networks, answers `design/09` §4's first two questions, gates its chrome on permission, and carries the doorway an `approve-node` holder uses to invite and admit |
| `kols-store` | Not created |
| `kols-media` | Not created |

Crates are created when there is code for them, not in advance — an empty crate is a
claim that something exists.

## 6. What Is Owed

Work that is known, named and not yet done. Distinct from §4's protocol extensions, which
are scoped and sequenced; these are debts the client took on deliberately, each recorded
where it was incurred so it is not rediscovered as a surprise.

**Nothing here blocks anything else.** Where an item exists because something it depends on
does not, the dependency is named — an owed item with no stated reason is just a thing
somebody forgot.

| # | Owed | Why it is not done | Where it is recorded |
|---|---|---|---|
| O1 | **Commands for direct messages, search, voice and stage** | Each has a line in `design/05` §3 and no code behind it. P2 for the first two, P3 and P4 for the rest | `kols-api::Command`'s own documentation |
| O2 | **`Discovery::Off` for conversation-profile networks** — the client half of E12, and **load-bearing for privacy rather than merely leaner since D29** | `kols init` writes no `chat:network-profile` key, so every network the CLI creates is a `server` and there is no conversation network to build the leaner node for. `kols_core::policy::conversation_genesis_values` exists for one; only a test calls it. The escalation *was* E13's relay fallback carrying a conversation; Core §5.2's correction of 2026-08-22 removes that fallback entirely, so what remains is the rendezvous. `Discovery::Off` still matters and for the same reason: with discovery on, a DM node meeting a peer at the shared network's relay lands in that relay's routing table, which is the shared-routing-table correlation D29 forbids — a shorter exposure than a carried conversation, and the same kind | `design/06` §12, `design/09` §3 |
| O3 | **`may_moderate_at` answers from current state, ignoring the head it is given** | Checking authority *as of* a governance head needs the log rather than one replayed snapshot. The difference shows only when a moderator is demoted after acting: this retroactively invalidates their past redactions, where `design/01` §6 says they should stand. Pinning is now judged against current state deliberately (`may_moderate_now`), which is a different question rather than the same one approximated | `kols-core::permissions`, flagged in the type's own docs |
| O4 | **`kols-store` does not exist.** `kols-node` carries its own file-backed store instead of the SQLite projection `design/05` §5 describes | Nothing has needed a projection yet — the CLI replays the log on every invocation, which is slow and correct. The projection is worth building when something renders fast enough to notice | `design/05` §5 |
| O5 | **The executor rebuilds an author's whole log to append one record** | `rebuild_log` replays every record this member wrote in a channel on every write, which is correct — the segment is a pure function of the sequence — and is linear in a log that only grows. It is the same work O4's projection exists to stop repeating, and wants measuring before it is optimised rather than after | `kols-node::chat::rebuild_log` |
| O6 | **The window has no presence.** It mints invites, shows the waiting room and admits from it now; what it cannot answer is `design/09` §4's third question — who is here, and are they around | Presence needs the ephemeral gossip of `design/01` §9, which nothing implements on either front end. Deliberately last: an interface that says "offline" when it means "I have not heard from them" is stating something it cannot know (§4.1), so the mechanism has to exist before the dot does | `design/09` §4.1, §7 |
| O7 | **No credentials and no backup.** Seeds are written to `<home>/seed` in the clear and never surfaced, so a member's only copy is a file, and anything with read access to the disk is that member | **Deliberately deferred, and it must land well before any 1.0.** The shape is decided (`design/02` §6.3): a local account whose password *wraps* a keyring of per-network seeds and never derives them, plus an export bundle of phrase, network id and relay address per network. Not needed to test between two machines you own; needed before anybody else's identity depends on it | `design/02` §6.3, `design/05` §5 |
| O8 | ~~**On Windows a seed is written with default ACLs.**~~ **Closed 2026-08-20.** `kols-node::secret` restricts a secret to this user on both platforms — a `chmod` on Unix, a *protected* DACL on Windows — and refuses rather than writing one it cannot protect | Confirmed by running it: a seed written by `kols.exe` on NTFS shows this account and nothing else in its Security tab, with no inherited `SYSTEM` or `Administrators` entry, which is what the protected flag is for. The refusal path was confirmed the same day, from a `\\wsl$\` path that has no permissions to set | `kols-node::secret`, `design/02` §6.3 |
| O9 | **A suspended node can lose its claim and not know.** `NODE_CLAIM_STALE` is wall-clock, so a laptop asleep past the window can have its claim taken over while it still believes it holds one | Making it impossible needs the holder to re-check ownership as it beats. Rare rather than impossible today, because taking over requires somebody to start a second node inside that window | `kols-node::store::NODE_CLAIM_STALE` |
| O10 | ~~**Answering a key request is not idempotent, so the one request a joiner sends can never be repeated.**~~ **Closed 2026-08-21.** `kols serve` now re-asks every 30 seconds while unkeyed | Landed as **E14**, Core §3.5.1 — but not in the shape it was written. Re-delivering keys would have restored read access and no group state, so the member would fall out at the next rotation; and re-adding is worse than "a lie in the log", since revocation resolves an identity to its *first* leaf and would remove the abandoned one while handing the new key to the member it excluded. Answering now **replaces** the leaf in one commit | `kols-node::serve`, `intranet-transport::answer_epoch_key` |
| O11 | **A relay may not be shared between networks, and nothing enforces it.** Decided 2026-08-21: reusing one relay across two of a member's networks breaks the unlinkability Core §1.2 exists to provide, so it is not permitted. Today it is merely *not done* — a founder can paste one address into two networks and nothing objects | A relay checks no membership by design (Core §5.5 — it replays no log and holds no capabilities), so it cannot refuse a peer for being in the wrong network. Underneath that, **the separation is not structural**: `kad::Behaviour::new` takes the default protocol name and `PROTOCOL_VERSION` is `/intranet/0.1.0` for every network, so two networks sharing a relay share a routing table and their members become mutually discoverable. Enforcing it means network-scoping the protocol names, which is a wire change and a protocol extension, not a client fix. Until then the client should at least refuse to designate a relay another of its own networks already uses — it holds the workspace, so it is the one party that can tell | `design/09` §1, §3; Core §5.5 |
| ~~O12~~ | ~~**The window cannot finish setting up a relay, and its own text says it can.**~~ **Closed 2026-08-21.** The window submits `SetBootstrapRelays` through a relay panel in the rail, shows the **full** network id with a copy button beside it — labelled as the `RELAY_NETWORK` a relay needs to boot — and the creation form no longer promises a "later" that did not exist. Designating one restarts the node onto it rather than telling you to reopen the network. Address validation moved to `kols_node::parse_relay` and is shared with the terminal, so `kols relay set` and `kols init --relay` refuse a relay naming no peer id too | Was: a command and gate that existed with no handler and no surface | `crates/kols-ui/*`, `crates/kols-app/src/main.rs` |
| ~~O13~~ | ~~**In the window a working relay is invisible; only a broken one reports.**~~ **Closed 2026-08-21.** New `Event::Relay { reserved, designated }`, emitted at startup in every case including success. It carries the count as well as the address so that "designates none" stays distinguishable from "designates some, none usable" — both leave a node reachable only on its own addresses, and only the second is a fault | The terminal keeps its `println!` and ignores the event, since it already said this where it happened | `crates/kols-api/src/event.rs`, `crates/kols-node/src/serve.rs` |
| ~~O14~~ | ~~**The client still lets a relayed circuit carry payload, which Core §5.2 now forbids.**~~ **Closed 2026-08-22** upstream (`45578b3`): a relayed connection no longer triggers a sync, a failed hole punch disconnects the peer, and the ceilings are sized for a negotiation (60s/256KB) rather than a session. Original entry: ** Corrected in the spec on 2026-08-21/22 (upstream `0b085e4`): there is no third tier, a circuit carries the DCUtR negotiation and is closed when the upgrade fails. Today a failed punch leaves the circuit open and everything keeps flowing over it | Three parts, in order of how much they buy. **Close the circuit on `HolePunchFailed`** — the direct expression of the rule, and it makes the failure visible instead of silent. **Refuse to send payload over a circuit**, so a circuit that exists for a negotiation cannot be used by anything else even transiently. **Lower DI-Relay's ceilings** from 120s/8MB toward the negotiation's own cost, so a relay enforces this itself rather than trusting every client — §5.3 now says exactly that. Not done at once because v0.6.0 is under test and changing transport behaviour underneath it would waste the run | Core §5.2, §5.3; `design/09` §3 |

**Closed since this register was written:** the executor, the two checks `authorize`
deliberately could not make, and the event half of the boundary. The first two were owed
*because* nothing on the command path held the store; the executor does, so an edit aimed at
somebody else's message and a record past the rate ceiling are now refused before anything is
signed. The third was owed because nothing emitted events — `kols serve` does now, and the
vocabulary was written from what it actually emits.

---

## 7. P0 Definition of Done

From `design/07-build-plan.md` §3. **All five are met** — see the per-row detail below.

| # | Criterion | State |
|---|---|---|
| 1 | 100 messages across sealed segments render in identical order on both nodes | **Met** — 40 permutations plus reversal at the merge layer; and 120 messages crossing two live nodes render identically on both |
| 2 | Appending one message transfers **only new tail chunks**, asserted on bytes moved | **Met** — 1,556 of 176,115 bytes locally; over the wire a reader holding the previous version refetches **1 chunk of 3** |
| 3 | Partition, both post, heal, converge byte-identically | **Met (merge layer)**; the wire path is exercised by criterion 1's live test, but a live *partition* test is P1 work |
| 4 | Records from an identity without `publish:chat-log` are refused by the *reader* | **Met** — non-members, forged signatures and wrong-channel records all refused at admission, with the reason surfaced |
| 5 | Pointer version collision resolves by lower record hash, loser re-publishes | **Met** — `AuthorLog::rebase` unions by record id and republishes at the next version, losing nothing. Winner's chunks recomputed locally, so a late record costs 15,901 of 164,956 bytes |

Criterion 2 is the one P0 exists for. If it fails, the segment model is wrong and the
design changes before anything else is built.

---

## 8. Where the Code Lives

- `ko-ls` — `origin` is `github.com/DriftingNarwhal/ko-ls`. Public, as are
  `distributed-intranet` and `DI-Relay`.
- `distributed-intranet` — `origin` is `github.com/DriftingNarwhal/distributed-intranet`,
  up to date. Nothing in the client builds against a *published* version of the protocol:
  the crates are path dependencies on the sibling checkout, deliberately, until the
  extensions stop moving.
- **Licensing, and one constraint that binds both repos.** This client is **AGPL-3.0-only**;
  the protocol crates are **MPL-2.0**; both are © DriftingNarwhal. The split is deliberate —
  the platform is meant to be built on by other implementations, the application is not meant
  to be enclosed. What ties them together: MPL files here are **not** marked "Incompatible
  With Secondary Licenses", and MPL §3.3 is the only reason an AGPL client may link them.
  Adding Exhibit B to a protocol source file would silently make this workspace
  undistributable. `DI-Relay` is AGPL for the same reason the client is: it is the component
  somebody would run as a service, and its Affero clause is the one that binds there — a relay
  is a service by definition, so a licence triggering only on distribution would never trigger.
  Note it does not reach the network served: a relay holds no state, so running one places no
  licence condition on anybody's client or content. **`specs/` in the protocol repo is CC BY
  4.0**, not MPL. All three repos require a DCO sign-off (`CONTRIBUTING.md`), and **anything
  depending on the protocol by tag must use `v1.0.1` or later** — `v1.0.0` predates the licence.

---

## 9. Log

Newest first. One line per change that moved the state above.

- **2026-08-22** — **Circuit lifetime raised to 60s, and §5.3 now says what the ceilings do not
  bound.** Asked whether the lowered ceilings limit a network to hundreds of thousands of nodes.
  They do not, and the arithmetic runs the other way: `max_circuit_duration` and
  `max_circuit_bytes` bound *one* circuit, so shorter and smaller means a relay performs **more**
  introductions per hour, not fewer — at 32 concurrent circuits, 120s allowed ~960/hour and 60s
  allows ~1,900. What bounds a single relay is `max_reservations` (128) and `max_circuits` (32),
  neither of which changed, and what scales past them is more relays and `relay_bootstrap_willing`
  members (§5.5) rather than a larger allowance on one host — a relay that could serve 100k
  members would be exactly the infrastructure §5.3 exists to prevent.

  The question did surface a coupling I had created: 30 seconds was chosen against the old
  behaviour, where a cut-short negotiation left the pair on a relayed connection. §5.2 made that
  fatal in the same commit, so the cost of being slightly tight became two members who cannot
  talk. Raised to 60 — still half the original, still nowhere near a session — with the bytes
  ceiling left at 256KB, where the anti-abuse work actually happens. Upstream `9951282`.

- **2026-08-22** — **O14 closed: a relayed circuit now carries the negotiation and nothing else.**
  Three parts, and the first was the one actually leaking.

  **A connection arriving relayed no longer triggers a sync.** `ConnectionEstablished` asked for
  the governance log, the ledger and every pointer over the circuit *before any upgrade had been
  attempted* — so payload crossed a relay on every connection, whether or not the punch later
  succeeded. The sync now happens when the peer becomes usable: on a direct connection, or on a
  successful punch, which performs it.

  **A failed hole punch disconnects the peer.** The circuit existed for that negotiation; leaving
  it open is how a relay quietly becomes the path. `dcutr` reports failure only after
  `MAX_NUMBER_OF_UPGRADE_ATTEMPTS`, so this is not closing on a first stumble.

  **The ceilings dropped from 120s/8MB to 60s/256KB**, in §5.3 and the implementation together.
  (First set to 30s, then raised on review: 30 was chosen against the *old* behaviour, where a
  circuit ending mid-negotiation left the pair relayed. §5.2 made that fatal in the same change,
  so a ceiling a few seconds too tight now costs two members who cannot talk. A DCUtR exchange
  runs ten to fifteen seconds on a lossy mobile link — which is exactly the pair that needs one.)
  The old figures predated the prohibition and were loose enough that a client relaying a whole
  conversation never met a limit — the rule held only by clients choosing to obey it, and one did
  not. `default_limits_match_the_spec_baselines` failed the moment the two disagreed, which is
  its job, and a new `a_circuit_cannot_carry_a_conversation` asserts the property rather than the
  constants.

  **Not covered:** behaviour under a real failed punch, which needs the NAT scenarios in harness
  spec §2.3 — including the new §2.3.6, where an IPv4-only CGNAT pair must *not* connect. Those
  scenarios do not exist yet, so this is verified by construction and by the suite, not by a
  failing punch. 655 upstream.

- **2026-08-22** — **A channel the founder created after the joiner arrived never showed up for
  them.** Suspected to be permissions; it was not. Reproduced first: `two_nodes.rs` gained
  `a_channel_created_after_a_member_joins_reaches_them`, and **it passes** — every other test
  here creates its channels before the joiner arrives, so that path had never been covered, and
  the entry does travel.

  So the sidebar was the fifth surface where a pushed event was the only path to a redraw. The
  channel list was drawn on `kols://governance` and nowhere else. The two-second tick now also
  re-reads what replay decides — this member's standing and the channel list — each drawn only
  when a signature over it changes, so a tick that finds nothing leaves the sidebar alone rather
  than fighting somebody using it.

  **The new test failed once and it was mine.** It passed alone and failed in the full suite,
  which in this project is the signal for a real race — but here it was a port collision with
  `a_founder_can_still_key_somebody_in_after_restarting`, so the second daemon bound nothing,
  reported nothing, and looked exactly like the feature failing. Moved to free ports; 196 green,
  and the ports now carry a comment saying why they are not shared.

- **2026-08-22** — **The two-machine test passes, including the part that matters most.** Two
  machines on separate networks, through a relay, admitted, keyed, messages both ways — and then
  **the relay was taken down and messages kept flowing**, which is the hole punch working and the
  relay out of the data path, confirmed on real networks rather than asserted. Edits, withdrawals
  and reactions all confirmed too.

  **Pinning "did nothing", and it was doing everything except showing.** Checked by running it:
  the record round-trips and `kols read` prints `[pinned]`, and `records.rs` has covered this
  since it was written. The window's entire signal for a pinned message was
  `box-shadow: inset 2px 0 0` on a row with 4px of padding — present, invisible, and exactly what
  a dead button looks like. It now carries a **`pinned` flag in words**, beside `edited`, plus a
  heavier bar and a tinted row.

  Two real defects came out of looking. The redraw signature ignored pin state, so a pin by
  *another member* on any message but the last would never have been drawn — the same
  only-when-you-act failure as before, one layer up. And `--remove` had no coverage, which
  matters now that a window offers pin and unpin as one button: a toggle whose second half is
  untested is half a feature. Both fixed; 195 green.

  A correction: I said pinning had no test at all. It did — an earlier grep matched "wra**ppin**g"
  and `head -5` hid the real hits.

- **2026-08-22** — **Core §5.2 corrected by its author: a bootstrap relay carries no payload, ever.**
  I had read "a correctness guarantee, not a usable path" as permitting a capped fallback, and
  argued that reading back. The author's intent is stricter and the spec now says it: **there is
  no third tier.** A relayed circuit carries the DCUtR negotiation; when the upgrade fails the
  circuit MUST be closed and MUST NOT carry protocol or application payload of any kind. A pair
  that cannot hole-punch reaches each other over IPv6 or not at all.

  Two consequences now stated rather than implied. Members with no mutual path are not
  partitioned from the network, only from each other — everything is pull-based and
  content-addressed, so they converge through any member both can reach, which is an ordinary
  sync partner rather than a route. And a two-member network where neither can reach the other
  has **no remedy**, which is a real limitation stated plainly instead of papered over.

  §5.3's ceilings are recast as defence in depth behind the rule, with 120s and 8MB noted as now
  generous by orders of magnitude. The harness spec asserted the opposite outcome in two
  scenarios and has been rewritten, plus a new one asserting that an IPv4-only CGNAT pair does
  **not** connect. Upstream `0b085e4`.

  **E13 loses its fallback**, and `design/09` §3 records why: the shared network's bootstrap
  relay was going to serve a DM's circuit on the reasoning that it "carries bytes and never
  inspects a join". It may carry a negotiation and nothing more. What survives is the rendezvous,
  which is the friction goal anyway — and the privacy flag that sat there goes with the fallback,
  since a relay that carries only a handshake observes that two identities met rather than
  watching a conversation.

  **Implementation is now behind the spec, deliberately and briefly.** The client still permits a
  relayed circuit to carry traffic — that is what v0.6.0 is being tested against right now, and
  changing transport behaviour underneath a running test would waste it. What is owed: close the
  circuit on a failed DCUtR upgrade, refuse to send payload over a circuit, and lower the
  relay's own ceilings so it enforces the rule rather than trusting clients. Recorded as **O14**.

- **2026-08-22** — **The client offered no IPv6, which is the path the spec designates when a hole
  punch fails.** Raised as "the relay should never carry messages", which sent me to Core §5.2 —
  and the sentence that matters is this one:

  > Two peers who can never hole-punch are expected to reach each other over IPv6, not over a
  > relay. … A deployment that expects CGNAT users to depend on relayed circuits for ordinary
  > traffic has misread this ordering.

  That is what had been built. Both front ends bound `/ip4/0.0.0.0/tcp/0` and nothing else: no
  IPv6, no QUIC — visible in a real log where every listening line was IPv4 TCP. §5.1 requires
  both families and both transports, `MemberNode::listen_default` had provided exactly that all
  along, and its own doc comment says binding all four "is what gives two peers behind CGNAT a
  path at all". Both front ends now default to it; `--listen` still overrides.

  **Where the stronger claim does not hold, recorded because it will come up again.** §5.2 keeps
  tier 3 deliberately: a relayed circuit is "a correctness guarantee, not a usable path", so that
  two peers are never partitioned. It is bounded by §5.3's ceilings, which "are the design, not a
  throttle to be raised when users complain". So content over a relay is permitted, expected to
  hit the caps, and never something to depend on — the remedy being IPv6 or a member volunteering
  as a relay, never a larger allowance on the hosted one.

  Dual-stack also multiplied what an invite carries: one machine had five interfaces before IPv6
  and QUIC doubled each. Loopback, unspecified and IPv6 link-local are now filtered out of the
  published set — legitimate to listen on, useless to hand somebody else, and previously shipped
  in every invite. 195 tests here.

- **2026-08-22** — **Taking the relay away silenced two nodes, and bringing it back did not
  reconnect them.** A deliberate test — is the hole punch working? — and it found three faults,
  one of which was hurting every conversation whether or not a relay was involved.

  **A closing connection was reported as a disconnect even when another remained.** A
  hole-punched peer has two connections: the relayed one it arrived on and the direct one that
  upgraded it. §5.2 exists so losing the first costs nothing, but `ConnectionClosed` reported
  `Disconnected` unconditionally, and the client removes a disconnected peer from the set it
  syncs with. **A circuit is capped at 120 seconds**, so the relay closes the relayed connection
  about two minutes into every conversation — meaning a *successful* hole punch was followed by
  the client dropping the peer and syncing with nobody. `num_established` is the answer libp2p
  already provides. Fixed upstream (`9e83bbc`).

  **Nothing re-dialled a peer that went away.** Addresses were dialled once at startup, so a peer
  lost after that stayed lost — a relay going down and coming back left two nodes that could both
  see it, had both re-reserved on it, and never spoke again. Re-dials every 15 seconds while
  nothing is connected, and only then. A reconnect is already a sync, so the connection is the
  whole recovery.

  **Nothing said whether the hole punch happened.** `HolePunchSucceeded` and `HolePunchFailed`
  were emitted by the transport and dropped by the client, so "is this still going through the
  relay?" had no answer short of taking the relay away. Both are now reported.

  On whether the relay should be out of the path after a punch: it is, and the 120-second cap is
  what enforces it — the relayed connection is closed by the relay itself shortly after the
  direct one exists. What was missing was the client surviving that closure, which is the first
  fix above rather than anything about the relay.

- **2026-08-22** — **A message from the other side did not render until the reader posted.** The
  records were in the store the whole time: the daemon absorbs them on its own — `two_nodes.rs`'s
  `a_reply_travels_back_and_both_sides_agree_on_the_order` asserts exactly that, with nobody
  posting — and the composer's own re-read after sending was what finally revealed them.

  **The fourth bug in three days where a pushed event was the only path to a redraw.** The
  others: the relay panel missing its one startup report, `kols://joins` emitted with no
  listener, and the near-miss before it. So this is fixed structurally rather than by another
  attempt to get an event right.

  The open channel is re-read every two seconds, matching the daemon's own sync tick, and drawn
  only when a signature over it changes — so an unchanged channel costs a local replay and no DOM
  work. `design/05` §3 already requires a consumer to merge rather than append, and the interface
  already redraws from the projection every time, so asking again can never show the wrong thing.
  O4's projection is what makes the replay itself cheap later.

  Also removed: `if (event.payload === state.current)` on the records listener. It could skip a
  redraw and could never cause one, which makes it a micro-optimisation whose only possible
  effect was the bug.

  And a consequence of drawing on a timer rather than on the reader's own action: scrolling to
  the end is now conditional on already being there, or a reader who scrolled up to read history
  would be dragged back every two seconds.

  **The two-machine test now passes its core**: two machines on separate networks, through a
  relay, admitted, keyed, messages both ways.

- **2026-08-22** — **A relay reservation was obtained once and never kept.** Reported as having to
  make sure the founder was still connected before the joiner could redeem an invite — which is
  the shape of a reservation that was lost and never re-obtained.

  **The 120 seconds is not the reservation.** It is `max_circuit_duration`, the cap on a single
  relayed connection (Core §5.3): a relay introduces, DCUtR hole-punches, and the direct
  connection carries the conversation. The reservation lease is an hour and libp2p renews it
  itself — verified in `libp2p-relay` 0.21.1, `reservation_duration` and the handler's renewal
  timer. So expiry was never the problem.

  **The connection ending was.** A relay holds reservation state in memory and is stateless
  across restarts (§5.5), so every redeploy drops every reservation — and there were many
  redeploys while the DNS fault was being chased. Nothing re-reserved: `reserve_via_relay` ran
  once at startup, and `circuit_listeners` only ever *grew*, so the node believed it held a
  circuit forever and advertised an address nothing answered on. Undetectable from inside and
  invisible from outside.

  Fixed in both repos. Upstream (`3335267`): listener loss prunes the sets and surfaces
  `NodeEvent::ListenAddrGone`, plus `has_circuit()` — a question that could not be asked before,
  because the answer was always yes. Here: a 20-second watchdog re-reserves through the same path
  startup uses, and a lost circuit says so on the terminal and in the relay panel.

  **Not done deliberately:** reserving on demand when an invite is redeemed. The reservation must
  exist *before* the joiner tries to reach the founder — they arrive through the circuit — so
  refreshing when needed is always too late. Maintaining it is the only ordering that works.

- **2026-08-22** — **A founder watching the door saw nobody at it.** B redeemed an invite, landed
  in the waiting room and stayed there; A's window never showed anyone waiting.

  The node did its half correctly: `record_waiting` writes the room to the store *before*
  `JoinAnswered` is emitted, so the answer was on disk the whole time. The shell emitted
  `kols://joins`. **Nothing listened for it.** The waiting list was only re-read inside `drawMe`,
  which runs on `kols://governance` and `kols://keys` — and a join produces neither. So the one
  event that means "somebody is at the door" was the one event with no listener.

  Audited the rest rather than fixing only the reported one: six events are emitted and
  `joins` was the only orphan. All six now have listeners.

  The doorway also re-reads every four seconds while a member who can admit is looking at it.
  `design/09` §4 already calls the waiting room stale by construction — it is live state in the
  node, written down for anything else to read — so refreshing is the model rather than a patch
  over a missed event. **This is the third bug in two days of the same shape**: an answer that
  existed, an event that carried it, and a consumer that was not listening at that moment.

  Also confirmed: v0.3.4 published a `.dmg`, so Tauri accepts `signingIdentity: "-"` and the
  ad-hoc signing landed.

- **2026-08-22** — **The macOS build said it was "damaged".** Not a corrupted download and not a
  bundler failure: **an arm64 binary must carry a valid signature to execute at all**, so an
  unsigned `.app` inside a quarantined `.dmg` fails Gatekeeper's check outright — and macOS
  reports that as *damaged and can't be opened*, which points at the Trash rather than at
  right-click → Open. The affordance everybody knows is the one for an app that is merely
  *unidentified*, which is a different state.

  `signingIdentity: "-"` in `tauri.conf.json` ad-hoc signs it, which turns the first message into
  the second. It is not notarisation and is not pretended to be — first launch still needs
  right-click → Open, and notarising needs a paid certificate this project does not have. The
  release notes and the runbook now say which message means what, since "damaged" is the one that
  makes somebody delete a working build.

  Two-machine test note: **the relay path works.** DNS, the relay, the circuit — all of it, on
  real machines across real networks. This is packaging, downstream of that.

- **2026-08-22** — **A node could not dial an address written as a name.** The root cause of a
  two-machine test that could not get a circuit, and the quietest failure this project has found.

  `SwarmBuilder` composes a transport from what it is asked for, and name resolution is one of
  those things. Without `.with_dns()` a `/dns4/…` address is unsupported — and nothing says so:
  `listen_on` for a circuit succeeds, because registering a listener resolves nothing; the dial
  that follows is refused inside the swarm; **no call returns an error**; the relay never sees a
  connection; and the node reports only that no reservation arrived. Every address in this
  project had been an `/ip4/` literal, so nothing had ever asked the transport to resolve a name
  — and a deployed relay is normally reachable *only* by hostname, since its operator does not
  control the IP.

  Fixed in `distributed-intranet` (`b66b16a`) for both node types, with
  `tests/dns_addresses.rs` dialling `/dns4/localhost` — no DNS server needed, so it cannot flake
  on one. Verified against the unfixed build first: it fails there, by timing out with nothing
  reported, which is the fault's own shape. 654 green upstream.

  **And the client's message asserted more than it knew.** It said the relay "answered and
  granted no usable circuit — it returned no address of its own", which is a claim about the far
  end. `reserve_via_relay` returning `Ok` means the reservation was *started*, not that anything
  replied. So a correctly configured relay was investigated for an evening on the strength of a
  sentence the code had no basis for. It now says a circuit did not arrive and names both
  possibilities, pointing at the relay's own log to tell them apart.

  Prior fixes in this chain were real and are kept — `RELAY_PUBLIC_ADDR` genuinely is required
  behind a proxy, and DI-Relay now reads Railway's proxy variables itself. They were not *this*
  fault.

- **2026-08-22** — **The relay panel showed a symptom with two opposite causes.** A first
  deployment could not get a circuit, tried a rebuilt relay from scratch, and got the same red
  line — because the line said "no circuit was granted" for both *nothing answered there* and
  *something answered and named no address*. Those need opposite fixes, and the node knew which
  had happened: the reason went out as `Degraded`, into a different part of the interface, while
  the status line kept its summary.

  `Event::Relay` now carries `failures`, the reason per relay in the order tried, and the panel
  prints them rather than a summary of them. The `RELAY_PUBLIC_ADDR` note is shown only for the
  case it explains — it says nothing useful about a relay nothing could reach.

  Verified rather than assumed, in `libp2p-relay` 0.21.1 itself: a reservation's address list is
  built from the relay's **external addresses only** (`behaviour.rs`), and the client turns each
  into a circuit listener (`priv_client/transport.rs`). An empty list is therefore granted and
  then unusable, which confirms the announce theory — and equally confirms it explains only one
  of the two failures.

  Also checked and ruled out: protocol drift between the relay's pinned `v1.0.1` and this
  checkout. The only changes are E14's, and none touch the relay path or `PROTOCOL_VERSION`.

- **2026-08-22** — **The relay panel could miss the answer it was waiting for.** Found by the
  first real use of the window: the panel sat on "waiting for this node to report" about a node
  that had already reported.

  The standing was **only ever pushed**, as `kols://relay`. The node starts in Tauri's `setup`,
  so it can settle its relay before the webview has finished registering listeners — and an
  event with nobody listening is gone, permanently, with no second chance because the node
  settles this once at startup. The window then reported a state that had no way of changing.

  Now the node *holds* its standing and `relays` returns it, so the event's only job is to say
  "ask again"; a missed one is harmless. The interface also polls every two seconds while
  unreported, stopping at about 30 — reservation is bounded near 20 — which covers a missed
  event without waiting on one.

  Two things that made this hard to see and are also fixed. **Nothing said the node was working
  on it**: pressing *designate* restarted the node and the panel changed almost nothing, so there
  was no way to tell the button had done anything. There is now an *asking the relay for a
  circuit…* state, visually distinct from the three verdicts. And **the refusal named no cause**,
  which for this failure is nearly always one thing: a relay behind a proxy announces its private
  container address, grants the reservation, reports healthy, and hands back a list clients
  reject. The panel now says that, with the `RELAY_PUBLIC_ADDR` to set — and the runbook adds it
  as step 4.5 rather than leaving it to be discovered.

  192 green, clippy clean.

- **2026-08-21** — **The window can now carry the whole test, and two gaps had to close before
  that was true.** Asked for a window-only test; checking rather than assuming found that it was
  not possible yet, for reasons neither of us had noticed.

  **The relay's identity needed a terminal.** `RELAY_PHRASE` came from `intranet-harness
  identity new` — a tool in the *protocol* repository — and DI-Relay refuses to start without
  one, on purpose. So "no step needs a terminal" was false for the one step a founder cannot
  skip. The relay panel generates one now: `MasterSeed::generate` and `to_backup_phrase`, from a
  crate `kols-app` already depended on. Shown once, stored nowhere, and marked as the relay's
  private key rather than the member's — the member's seed has no interface at all and must not
  start looking like it does.

  **React, revise, withdraw and pin did not exist in the window.** Seven of `kols-api`'s
  thirteen commands were wired; these four were not, and step 11 asks for exactly them, because
  each takes a different path through the merge. They are per-message controls revealed on hover.
  Two details worth keeping: `mine` on a reaction now comes from the core rather than being
  guessed, since a reaction is a toggle and a client that guessed would send an add for one the
  member already holds — a no-op that reads as a dead button; and withdrawal asks for
  confirmation in the words `design/01` §6 requires, *hidden, not unsent*.

  Pin is gated on the network-wide `moderate-content` and so misses a per-channel moderator. That
  error runs the safe way — it hides a control somebody may hold, rather than offering one that
  will be refused — and is noted where it is written.

  The runbook is now window-only end to end, cleanup included, and says plainly that it tests two
  unproven things at once. 192 green, clippy clean.

- **2026-08-21** — **`kols-cli` renamed to `kols-node`.** The name described 853 of its 5,578
  lines and misdescribed the rest. What is in it is the store, the node loop, the executor, the
  workspace, joining, inviting and secret-writing — the window's entire backend, and its only
  ko-ls dependency besides `kols-api` and `kols-core`. Calling that "the CLI" is what made "can
  we get rid of the CLI" sound like a small question. `cargo build -p kols-node` now; the binary
  is still `kols`. 192 green, clippy clean.

  **The binary is deliberately still here**, and this is the reasoning rather than an omission.
  Deleting it before the two-machine test costs three things and buys nothing: the runbook is
  written in `kols` commands and would be dead on arrival; CI's seed-permission check drives
  `kols init` and is the only test that inspects the built artifact rather than the source; and
  the window has still never had a flow run through it, so the reference path would be gone at
  exactly the moment a failure needs localising. D30 already makes it a development tool — a
  status, not a deletion — and its lines cost nothing while it waits.

  The remaining gate is that seed check. It *can* move into Rust: `windows-sys` is already a
  dependency with `Win32_Security_Authorization`, so a `#[cfg(windows)]` test can read a DACL
  back. It has not, because a Windows test cannot be run from this container and shipping an
  unverified one the day before a two-machine test is the wrong trade.

- **2026-08-21** — **D30: the terminal is a development tool, not a product surface.** Stated by
  the user and worth recording, because it changes what "finished" means rather than what the
  code does. Nobody is expected to use this application from a command line, so `kols` owes the
  window **no feature parity**, no discoverability and no end-user documentation. What it must
  keep is the one property that makes it worth having: it crosses the same `kols-api` boundary a
  window does, so "works in `kols`, not in the window" localises a fault to the interface.

  Consequences applied: the README's *Trying it* now opens with the release rather than
  `cargo build -p kols-cli`, and marks everything below it as the development path. The release
  notes said "`kols-desktop` is the window and `kols` the terminal client", which read as two
  products on the download page; `kols` is now described as what it is. §1's Runnable row is
  ordered product-first.

  **What this decision is not:** an instruction to delete the crate. `kols-cli` is 5,578 lines
  and 853 of them are the binary — the rest is `serve`, `store`, `executor`, `workspace`,
  `join`, `invite`, `network`, `chat` and `secret`, which is the window's entire backend and its
  only ko-ls dependency besides `kols-api` and `kols-core`. The binary is also what CI drives to
  check the seed's permissions on the built artifact, which is the one test that inspects the
  artifact rather than the source. When the window has carried the two-machine test end to end,
  `main.rs` and the `[[bin]]` target can go and the crate wants renaming to `kols-node`; the
  seed check moves to a library test on the same `secret::write_private` path.

- **2026-08-21** — **Runbook brought level with the window, and one asymmetry the restart
  created.** `kols relay set` now says what it cannot do: a node dials relays when it starts, and
  a terminal cannot restart a daemon in another process the way the window restarts its own. A
  running `kols serve` designating a relay it never dialled was previously silent. The runbook
  gains that caveat at step 4, the window's four relay states in the troubleshooting list, rows
  for steps 10 and 12 in the front-end mapping, and a note that the two machines need not use the
  same front end — A in the window and B in a terminal exercises both and leaves the detailed log
  on the side more likely to be waiting.

  Also: the relay panel no longer shows a permanent "…" to a member in the waiting room, who has
  no readable policy to draw and is exactly the person most likely to be looking at it.

- **2026-08-21** — **Designating a relay now restarts the node itself, and a latent restart race
  is gone.** Asked as "why would we not build it into the flow" — and the honest answer was that
  naming the next step is worse than taking it. `set_relays` restarts the node it just made
  policy for; a relay learned from *replay* does the same through `restart_node`, but only when
  there is no circuit, so a working node is never interrupted for a relay it does not need.

  **Underneath it was a real bug, not just a missing convenience.** `start_node` aborted the
  previous node and spawned the next immediately. `JoinHandle::abort()` does not drop the
  future — it asks the task to stop at its next await — and the store's claim is released *on
  drop*. So the replacement raced its predecessor's claim, and `hold_node` does not fail fast on
  a claim that looks fresh: it waits the staleness window out. The symptom would have been a
  window that hangs for half a minute and then works, on the ordinary path of reopening a
  network. The abort is now awaited, inside the new task so nothing blocks the interface thread.

  Also corrected: **the window has been launched.** Once, before anything was wired to it, and
  it rendered roughly correctly. Three documents said it never had. What is true is narrower and
  more useful — no *flow* has been run through it, so every path it has is unexercised. And §0's
  item 3 no longer says `kols-desktop` needs a Windows machine, which stopped being true when
  CI started building it.

- **2026-08-21** — **O12 and O13 closed: the window can set up a relay, and says when one
  works.** Found by a question worth recording — "why am I using the CLI at all if there is a
  window?" — which §1 had answered wrongly. It claimed no step needed a terminal while O12, three
  sections down, said relay setup was exactly the step that did.

  The window now submits `SetBootstrapRelays`, which had existed with a correct gate and a
  working executor and no handler on this side. Alongside it: the **full** network id with a
  copy button, labelled as the `RELAY_NETWORK` a relay will not boot without — it was rendered
  `slice(0, 16)`, which is worse than showing nothing because it looks copyable — and a
  creation form that no longer says "you can add one later" about a path that did not exist.

  O13 is `Event::Relay { reserved, designated }`, emitted at startup in **every** case. Relay
  failures already crossed the boundary as `Degraded` while success was a `println!`, so the
  window could report relay trouble and never relay health, which is the wrong half to have when
  the question is "is my relay working". The count rides along so that "designates none" and
  "designates some, none usable" stay distinguishable — both leave you reachable only on your
  own addresses, only the second is a fault.

  Address validation went to `kols_cli::parse_relay` rather than into the window, so the
  terminal gained it too: an address naming no peer id is refused by `kols relay set` and
  `kols init --relay` as well. That check earns its place — designating a relay writes a
  governance entry **every member replays**, so a bad address is carried by everybody and fails
  later on somebody else's machine. 3 tests; 192 green, clippy clean.

- **2026-08-21** — **Dropped the Intel macOS build leg, and bumped the actions off the
  deprecated Node 20 runtime.** The `macos-13` leg never produced a build that went through,
  and the Mac this is built for is Apple Silicon, so the matrix is now Windows and
  `macos-latest` only. Separately, every run warned that Node 20 is deprecated: `checkout` to
  v5, `upload-artifact` to **v6** and `download-artifact` to **v7**. Those are the *lowest*
  major of each that runs Node 24, deliberately — `upload-artifact@v5` is still Node 20, so
  the obvious one-major bump would have looked like a fix and changed nothing, and
  `download-artifact@v8` stops unzipping unconditionally, which would silently change what
  `find artifacts -type f` hands to `gh release create`. The reasoning is a header comment in
  the workflow, where the next person to see a deprecation warning will read it.

- **2026-08-21** — **Made every unlinkability claim say the same thing, and wrote the two-machine
  runbook down.**

  Core §1.2 has always been honest — two per-network public keys cannot be tied together, and
  IP-level and timing correlation are explicitly out of scope — and the documents restating that
  guarantee had been dropping the second half. A reader met the strong version in `design/00` §1,
  `design/03` §4.2, §4.4 and §4.6, and spec 07 §0, §1.5 and §8, and the honest version once, in
  `design/09` §1. **§1 is now named as the canonical statement and the others defer to it**, with
  Core §6 and spec 07's own summary carrying the limit too, since those are the sections
  consuming work is told to assume from.

  **Two of them were overclaims rather than omissions.** `design/03` §4.2 and spec 07 §1.5 both
  said a separate network reveals nothing about a conversation to the server. That is right about
  its log and its storage and wrong about E13's relay fallback, where the operator carrying the
  circuit sees one address acting as a member and as a party to a conversation and can infer that
  two members are talking. Both now say which of the two they mean.

  Nothing was weakened. Three documents stopped describing a narrower guarantee as a broader one,
  which is the failure this project objects to everywhere else.

  **`docs/two-machine-test.md` is new**, and is written to be thrown away: it exists because the
  test has not been run, and once the path is either routine or wrong it should be deleted or
  folded into the README rather than kept as a second thing to disagree with this file. It carries
  the three traps in the places they bite — never bind a relay to loopback, the Railway domain and
  TCP proxy are both required, and `relay reserved a circuit on …` is the line that means steps 3
  and 4 worked.

  Also refreshed the protocol repo's `CLAUDE.md`, which still said five amendments were
  implemented and E10 was the only one outstanding — stale since E14 landed and E13 and E15 were
  recorded.

- **2026-08-21** — **D29 recorded, and the direct-message bootstrap is not an exception to it.**
  O11's decision now lives in `design/00`'s register and its reasoning in `design/09` §3, rather
  than only in a status log where it would have been lost.

  **The distinction that resolves the apparent conflict.** D29 refuses a relay *designated by two
  networks*, carrying both as standing infrastructure. E13 is not that: it exchanges the DM
  network's addresses over a connection that already exists between two people who have already
  identified themselves to each other, then opens directly. **No relay is involved on the primary
  path at all**, and nothing is disclosed that either party did not already hold.

  **The fallback is bounded rather than free**, and the bound is a dependency nobody had noticed.
  When hole-punching fails, the shared network's bootstrap relay carries the circuit and does
  briefly serve two networks. It does not create D29's structural problem, because a
  conversation-profile network runs `Discovery::Off` and so has no routing table and joins nobody
  else's — **which turns O2 from a resource optimisation into a privacy requirement**, since with
  discovery on that fallback would put a DM node straight into the shared relay's routing table.
  O2's row says so now.

  What remains is IP correlation, which is real: the relay's operator sees one address as a member
  and as a party to a conversation, and can infer that two of its members are talking. That
  **weakens a claim `design/03` §4.2 makes** — that a separate network reveals nothing about the
  conversation to the server — against one party, in the case where hole-punching failed. Kept
  anyway, because the alternative is that symmetric-NAT users have no direct messages, and now
  stated in `09` §3 rather than left inside an earlier flag.

  **One correction worth recording, because it was nearly reasoned from.** A participant's identity
  in a DM network is *not* their identity in the shared network — it is a separate per-network
  derivation, which is precisely why `design/03` §4.3 step 4 carries a **voluntary identity link**
  binding the two. That the identities differ is the property making a conversation unlinkable from
  the server to everybody except the person a participant chose to prove it to. If they were the
  same, the DM network would be linked to the shared one by construction.

- **2026-08-21** — **Reviewed how a relay is actually reached, and one finding turned into a
  decision that changes what direct messages need.** No code moved; this is the review the
  two-machine test was waiting on, recorded as O11–O13.

  **What a relay is, checked against the code rather than the docs.** `RelayBehaviour` is circuit
  relay v2, identify, ping and kad — no governance replay, no ledger, no membership check, exactly
  as Core §5.5 says ("not a member of the network it serves"). It holds no state and circuits are
  capped at 120s and 8MB, so it establishes connections rather than carrying them. **The only
  network-specific thing about a relay is its identity**: `RelayNode::new` takes a
  `PerNetworkIdentity`, derives a keypair, and that is the whole role `RELAY_NETWORK` plays in
  DI-Relay.

  **Which raised the question, and the answer is no.** Because a relay checks nothing, one
  deployment *can* serve several networks — and it must not. Two networks sharing a relay share a
  routing table: `kad` runs under the default protocol name and `PROTOCOL_VERSION` is
  `/intranet/0.1.0` for every network, so nothing keeps their members from discovering each other,
  and two of one person's identities meeting on one relay is exactly the correlation Core §1.2
  exists to prevent. `design/09` §1 already listed "any relay that sees both" as an honest limit;
  reuse would have made an incidental limit into the normal case. **Decided: a relay is not shared
  between networks.** Recorded as **O11**, along with the uncomfortable half — nothing enforces it
  today, and enforcing it properly means network-scoping the protocol names, which is a wire change
  rather than a client fix.

  **The decision makes E13 load-bearing rather than convenient**, and that is the consequence worth
  not burying. Every direct message is its own network (D10). If a relay cannot be shared, then
  without cross-network bootstrap a conversation between two NATed people needs its own deployed
  relay — which nobody will do. E13 was scoped as removing friction from a flow that would
  otherwise work; it is now the only thing that makes DMs work across NAT at all. §4 says so now.

  **The window cannot finish the relay journey, which contradicts §0's own claim** that no step
  needs a terminal. It never shows the network id — the one string DI-Relay demands before it will
  boot — and its creation form says "You can add one later" when nothing adds one later, since
  `SetBootstrapRelays` reaches only `kols`. So a founder who skips the relay there is stuck at
  every exit and is not told why. **O12.** Alongside it, **O13**: `kols serve` reports reserved,
  granted-but-unusable, unreachable and none-designated, and says which — but success is a
  `println!` while the failures are `Event::Degraded`, so the window surfaces relay trouble and
  never relay health. "Is my relay working" is the question two machines will actually raise.

  **None of this blocks the two-machine test**, which is the useful conclusion. The CLI path is
  complete and signposted — `kols init` prints the network id and the exact ordering when no relay
  is given — and DI-Relay's own README covers the traps, including the `RELAY_PUBLIC_ADDR` case
  that looks like nothing is wrong. Deploy, and run the test on that path.

- **2026-08-21** — **E14 landed, and it was a security fix wearing a liveness fix's clothes.** It
  was written up as "a joiner can stall forever", which is true and is the smaller half.

  The stall is real: key delivery is a request and a response, the answer can be lost, and a
  request that is never answered strands a member permanently — in the log, served by honest
  nodes, holding no key that opens any of it. The client asked exactly once because retrying was
  known to be unsafe, so a lost answer was terminal.

  **What the fix had to be was not what `design/06` §14 specified.** That asked for key
  re-delivery: answer a member already in the group with the key material and append nothing.
  It reads correctly and restores half a member. A requester asking again has no group state *by
  construction* — it asks because it holds no key, and it holds no key because the Welcome that
  would have carried both never arrived — so keys alone leave it able to read what exists now and
  unable to apply any later commit. `apply_pending_rotations` returns immediately for a node with
  no group. It would have fallen out of the network at the next membership change, silently,
  having looked recovered.

  **And re-adding turned out to break revocation, which nobody had noticed.** §14's objection to
  it was honesty — a second leaf for one member is a lie in a log every member replays. The
  actual cost is worse: removal is expressed against an *identity* and applied to a *leaf*, so
  `leaf_index_for` finds the **first** leaf holding that credential. Revoking a doubled member
  removes the abandoned leaf, rotates, and hands the new epoch key to the member it was asked to
  exclude, still sitting on the leaf nobody removed. The removal reports success and Core §3.1's
  guarantee is gone. The new test fails against the old behaviour with the revoked member's key
  fingerprint **identical to the founder's**.

  So answering replaces the leaf — remove the stale one and add the requester's key package in one
  commit, producing one rotation and a Welcome they can open. Core §3.5.1, `GroupSession::
  replace_member`, which clears both proposals if either fails, since a dangling remove would be
  swept into whatever commit came next and drop a member nobody decided to drop.

  **A test was written, run against the old code, and thrown away**, which is the part worth
  keeping. It asserted the log stayed linear across two answers — and passed under both
  behaviours, because answering parents on the current tip, so a duplicate add never forked
  anything. The fork in the original report came from concurrent writers; the duplicate add's
  damage is the second leaf. A green test that asserts nothing is worse than no test, so it was
  replaced with the revocation one, and that one was checked against the old code before being
  believed.

  The client re-asks every 30 seconds while unkeyed — deliberately slow against a two-second tick,
  because every answer is a real rotation and a governance entry every member replays forever.

  Also: spec 07 §7's amendment table gains **E13, E14 and E15**, which were tracked only in the
  client, so the protocol repo's own record said one chat amendment was outstanding when four
  were.

  649 → 653 upstream, 189 here, clippy clean in both. `two_nodes` passes 10/10 at full width;
  **pinned to two cores it now fails one where it used to fail two**, which is recorded rather
  than claimed as a fix — the retry plausibly recovers one of the starved cases, and one run is
  not evidence.

- **2026-08-21** — **All three repos are licensed, and two of them were wrong in opposite
  directions.** `distributed-intranet` had no LICENSE and no licence metadata, which under
  default copyright is all rights reserved — public and legally unusable, for the repo whose
  specs exist to be implemented by other people. `ko-ls` declared `MIT OR Apache-2.0` in
  `Cargo.toml` with no LICENSE file behind it, which is both ambiguous and the most permissive
  reading available: it expressly allows closing this and selling it.

  The requirement was "free for everyone, and nobody can sell it", and those cannot both hold
  literally — every open-source licence, copyleft included, permits sale, and the Open Source
  Definition forbids discriminating against commercial use. What it resolves to is that nobody
  may **enclose** it, which is a copyleft question rather than a price one. So: **AGPL-3.0-only**
  for the applications, **MPL-2.0** for the protocol crates, © DriftingNarwhal. Selling stays
  legal and becomes pointless, since the buyer receives the source and may redistribute it.

  The Affero clause is the one doing work here: the obvious way to enclose a chat system is to
  host a modified copy rather than ship one, and plain GPL would permit exactly that. The MPL
  side is the reverse trade — other implementations may build over the protocol and licence
  their own work freely, while changes to these files stay open. §8 records the constraint that
  couples them, which is that the MPL files must never carry Exhibit B.

  Checked rather than assumed: the whole dependency tree is permissive (MIT, Apache-2.0,
  MPL-2.0, BSD, Zlib, Unicode) with no GPL or non-commercial terms, so nothing blocked the
  relicense; every crate in both workspaces now resolves a licence where nine resolved
  `UNDECLARED`; and both gates are green — 189 here, 649 upstream, clippy clean in both.

  **The specs are licensed separately, under CC BY 4.0.** A software licence is the wrong
  instrument for prose — MPL is written about Source Code Form and executables, and an
  implementer quoting a section into their own documentation should not have to work out
  whether that makes their document Covered Software. Attribution is the whole and correct ask
  for documents that exist to be implemented by other people.

  **DI-Relay needed a second fix that licensing it alone would not have delivered.** It pulled
  `intranet-transport` and `intranet-identity` at tag `v1.0.0`, which predates the protocol's
  licence — so anyone building it was building against code that was still all rights reserved.
  A licence on `main` does not reach a tag. `v1.0.1` is the first protocol tag carrying MPL-2.0,
  and DI-Relay is repointed at it; its README had also claimed MIT/Apache "matching the protocol
  repository", which was wrong in both halves, since the protocol repository had no licence to
  match.

  **All three repos now carry a `CONTRIBUTING.md` requiring a DCO sign-off.** Deliberately not a
  copyright-assignment CLA — contributors keep their copyright. A copyleft licence is only as
  good as the project's established right to ship the code under it, and with the repos public
  that right needs recording per commit rather than reconstructing later by asking every past
  contributor individually.

  **Honest limit:** `ko-ls` was public for a day declaring `MIT OR Apache-2.0`. A permissive
  grant already given cannot be withdrawn from copies already taken, so anyone who cloned it in
  that window may hold that snapshot under those terms. No LICENSE file existed and nothing was
  published to crates.io, which makes the grant weak and the practical exposure nil — but it is
  worth writing down rather than describing the change as clean.

- **2026-08-21** — **A second documentation pass, over the design set against the protocol it
  depends on rather than against itself.** The previous pass read the client's documents for
  internal contradictions and found three. Reading them against the landed specs is a different
  exercise and found five more, one of which had been true for weeks in a place nothing tracked
  and one of which was a gap in the normative document rather than in this repo.

  **`design/06` §2 described the opposite of what E2 landed.** It said all four channel entries
  are capability-gated and therefore *count* toward branch length. Core §2.7.2 excludes them,
  and is right — whether an application entry is cheap to mint depends on the tier of the
  capability it declares, which the branch-length metric deliberately cannot resolve. This
  file's own E2 log entry recorded the reversal on the day it landed; the design document that
  asked for it was never updated, so the request and the outcome disagreed in writing for two
  months. Now it carries both, because the reversal is the more useful half.

  **The same section overclaimed profile enforcement, and so did two others.** "A
  `ChannelDefinition` in a conversation network is rejected on replay" reads as the protocol
  enforcing it. It cannot: E2 landed generically, so the platform carries `chat` payloads
  without decoding them, and spec 07 §1.2 corrected the wording upstream. `01` §2.1, `03` §4.1
  and `06` §2 each still had the strong form. Fixing one and leaving two is how a correction
  becomes drift, so all three say the same weaker and true thing — every conformant reader
  refuses it, the platform enforces nothing here, and a modified client would see a channel
  where others see none.

  **`design/06` §16 still assumed a master seed.** D28 removed it in August; `03` §4.6 was
  updated for that and `06` was missed, so the one document that tracks what the protocol owes
  was describing an identity model the client had abandoned.

  **Which is how E15 was found, and it is the finding worth keeping.** The client does not
  implement Core §1.1 — one master seed per person, per-network keypairs derived from it — and
  does not intend to. D28 generates fresh entropy per network. That is a divergence from a
  normative specification, and `design/06` §0's own rule is that a needed protocol change is
  recorded there rather than assumed into existence. It never was: the decision lived in the
  implementation, then in a decision register, and in no list of what the protocol owes. It
  blocks nothing — the protocol's crates never see a seed, only the keypairs it produces, so
  the cost is conformance rather than function — which is exactly why it stayed invisible.
  Recorded now with what the amendment has to say, including the part the client cannot yet
  answer: what a backup *is* when there is no phrase to write down, which is O7.

  **The retention vocabulary split turned out to be hiding a real gap in the normative
  document.** `01` §8 named the default `Forever` in its prose and `Unbounded` in its table
  one screen later, and spec 07 §2.8 said `Unbounded` too. Aligning the word was the small
  half. The large half is that §2.8 described retention as one setting, named neither policy
  key, and never said how a network expresses "forever" in a key whose value is a day count —
  so the one genuinely normative question here was unanswered: **what does `0` mean?** Two
  clients answering that differently render different history from the same records, which is
  precisely the divergence §4.3 exists to prevent. The client has always read absent, zero,
  negative and overflow as `Forever`, fail-safe, because content allowed to go dark cannot be
  brought back. §2.8 now says so as a MUST, names both keys, carries the two-windows reasoning
  and the newest-record rule, and states that a reader must not report an unwrappable segment
  as deleted. §4.3's key list gains both keys. Spec text only — the protocol stores app-layer
  policy values without interpreting them (Core §2.6.2), so there is no protocol code to
  change and no protocol test to write; the behaviour being specified is `kols-core::policy`'s
  and was already tested here.

  Also: this file called `design/09` v0.2 in two places after it went to v0.3.

  **No code changed and no test moved**, and that was checked rather than assumed: 189 passing
  here and clippy clean, counted from a file rather than a pipe for the reason a previous entry
  records. Nothing upstream was touched, so its 649 stands unre-run.

- **2026-08-21** — **A documentation pass, and it found three stale claims rather than none.**
  The point of reading all of it after a run of landings is that some of it is wrong, and it was.

  **The README contradicted itself.** "The window" said *"It runs no node, so it will not show
  you another member's messages and will not update while you watch"* — three paragraphs above
  "What exists", which said the window runs a node and updates as records arrive. Both were
  written a day apart and only one was true. It now describes what the window does, including
  minting an invite and admitting from the waiting room, and says plainly what it still cannot
  do, which is presence.

  **`design/00`'s roadmap was two landings behind**, listing as *not yet* an executor behind the
  API boundary, its event half, edits and reactions as commands, invites, and "every part of the
  Tauri client". All but threads are built. A roadmap that describes finished work as pending is
  worse than none, because somebody plans around it.

  **`design/09` §5 still owed its presentation half**, which is built: the shell resolves each
  capability against replayed state and hands the interface a flag per control, so there is no
  second permission model in the front end to drift from the first. §7's invite question is
  mostly answered too, and what is left of it is now stated precisely — use-count and expiry are
  *defaulted rather than decided*, which is a smaller and more honest open question.

  **`design/05` §5 described a keychain nobody has built.** It read as a description and was a
  target: what exists is an unencrypted seed file restricted to its owner, refused where it
  cannot be restricted. Now says so, and points at O7.

  **E14 is new in `design/06`**, which is where a required protocol change belongs — O10 was
  living only in `STATUS`. It carries the mechanism, why re-delivery rather than re-add (a
  Welcome cannot be reissued, and re-adding rotates a group whose membership did not change,
  which is a lie in a log every member replays), and acceptance criteria.

  Also recorded in `design/05` §8: run the daemon tests starved as well as fast, because a wide
  machine wins every race a narrow one loses; and clean up after them, because this container's
  storage is the host's.

- **2026-08-21** — **The gate is gone: tests run here, GitHub builds.** A CI gate on every push
  was never asked for — builds were — and it turned three rounds of debugging into somebody
  copy-pasting log fragments for failures that one local `taskset -c 0,1` run reproduced with
  complete output. CI could say *that* tests failed; the container said *why*.

  What CI genuinely adds is the one thing this container cannot do: execute on Windows and
  macOS. So that is all it does now, and only when a build is wanted — a `v*` tag, or a manual
  dispatch. Each platform leg runs `cargo test -p kols-cli --lib`, which is the store and the
  seed, and then checks the **artifact** it just built: create a network, look at the seed's
  permissions, refuse the build if anybody else can read it.

  **That artifact check exists because the obvious thing does not work.** `secret::tests` is
  `#[cfg(all(test, unix))]`, so the Windows DACL path compiles out of the Rust tests entirely —
  a `cargo test` leg on Windows would have run the pure-function store tests and reported
  success while proving nothing about the one function written specifically for Windows. A test
  that looks complete and is not is worse than none.

  Also: **the test suite now cleans up after itself**, because this container's storage is the
  host's. `Home` already removed its directory on `Drop`; the `secret` tests added yesterday did
  not, and a day of Windows cross-compiles had taken `target/` to 11 GB. `Drop` rather than a
  line at the end of a test, because `Drop` runs on an unwind and a failing test is exactly when
  the scratch is left behind.

- **2026-08-21** — **The joiner never asked, and reading the code had said otherwise twice.** The
  round before this reported a stall after sixty seconds, which was useful and was not the
  finding. The finding came from **reproducing it** — the suite pinned to two cores, with the
  test harness changed to print *every* daemon's log rather than only the one being waited on.
  The founder's side is where the answer was, and no failure had ever shown it.

  What it showed: a joiner that learned its own admission and **never asked to be keyed in at
  all**. The request lived inside `Synced { accepted > 0 }`, nested under `learned > 0`, so it
  fired only on the sync that accepted new governance entries, and only if `ready` happened to
  succeed at that instant. Miss it once and there is no second chance — every later tick syncs,
  accepts nothing new, and skips the block. Starving the machine is what made that race lose:
  `ready` fails while the ledger advertisement has not landed, and on quiet loopback every entry
  arrives in one go, so the single opportunity was the one that failed.

  The ask now happens on the tick whenever this node is unkeyed, ready and has not asked yet.
  **Still exactly one request**, which is deliberate rather than timid: answering is not
  idempotent (O10), so this makes the single ask *reliable* without making asking *repeatable*.

  Pinned to two cores, `two_nodes` goes from six passing to eight; at four cores — the runner's
  width — all ten pass in 80s. **Two cores still fails two**, both now in the content path after
  keying rather than in keying, and that is recorded rather than tuned away.

  **The harness change is the durable part.** A two-node failure printed the log of the daemon
  that did not do the thing, and the reason is almost always in the other one — a founder
  refusing a request says so on *its* terminal, which the waiting side cannot see. That is how a
  stall reads as "nothing happened" from one side and cost two rounds of guessing.

- **2026-08-20** — **The Windows gate found a real bug, and it is not about Windows.** Four of
  six failures stop at the same line — `asked X to key us in`, then nothing for 135 seconds. On
  loopback that is not slow, it is stuck, and it is the one thing in the node loop that cannot
  recover on its own.

  **The request is sent once**, on the `Synced` event that first learns this node has been
  admitted, and that event need never recur. Everything else in the tick is re-asked every two
  seconds with a comment over it explaining why — a pull-based stack has no other way to learn
  that a peer changed its mind — and the key request is the one exception. A founder has
  ordinary reasons not to answer at that instant: answering appends a rotation, so it takes the
  store's append lock that every one-shot command also takes, and losing that race reports a
  degradation to *its* terminal and tells the asker nothing.

  **Retrying is the obvious fix and it is unsafe, which is worth knowing before somebody tries
  it again.** `answer_epoch_key` calls `add_member` unconditionally — there is no idempotence
  check — so a second request adds an existing member a second time and appends a second
  rotation, forking the log against the entry that admitted them. Implemented, and it voided a
  member's grant: a keyed member could no longer post, in a test that had passed for days. The
  retry is reverted and the finding kept.

  So the node **says it is stalled** rather than hiding it, after sixty seconds, naming why
  nothing will retry and what to do. The real fix is upstream, where the group is: answering a
  request from a member already in the group must re-deliver rather than re-add. Recorded as
  **O10**.

  This is why the Windows gate earns its cost even though the bug is not Windows-specific. A
  loaded runner loses a race a quiet development machine wins every time, and the two-machine
  test over a relay — the actual next milestone — is a far worse place to meet it than CI.

- **2026-08-20** — **Tripling the timeouts doubled the failures, which ruled out the diagnosis
  it was meant to fix.** Six tests failed where three had, and the suite took 335s where it took
  125. More time producing more failures is not what a machine that merely needs more time looks
  like.

  **The measurement that reframes it:** pinned to four cores, this suite passes on Linux in 83
  seconds; on a four-core Windows runner it failed six tests in 335. *Same width, different
  answer* — so parallelism was never the axis, and scaling patience by core count addressed the
  wrong variable. It stays, because a genuinely narrow machine does want it, but it was not the
  fix.

  What Windows charges that Linux does not is **per operation**. Defender inspects an executable
  on every launch and every file written beneath the tree; this suite launches a **76 MB debug
  binary about a hundred times** and its daemons write many small files per tick. Both workflows
  now exclude the workspace, the cargo directory and the temp directory, tolerating failure
  because a runner image with Defender already off should not fail the gate for refusing to
  turn it off twice.

  Ports and home directories were checked for collisions first and there are none — a previous
  entry records a test that did collide, so it was the obvious suspect and it is worth saying it
  was eliminated rather than never considered.

  **If this is not enough, the next lever is serialising the daemon-heavy suite on Windows
  rather than waiting longer**, and the one after that is the backfill test's thirty separate
  `kols post` launches, which exist to cross a seal threshold and could cross it with fewer,
  longer messages.

- **2026-08-20** — **The first Windows CI run failed three tests, and none of them was a
  Windows bug.** `two_nodes` timed out waiting for a joiner to backfill — on Windows only,
  which reads like a platform difference and is the wrong conclusion. **Pinning the same suite
  to two cores on Linux reproduces it exactly.**

  Every deadline in these tests is wall-clock and was tuned on a 24-core box, which makes them
  an assumption about the machine rather than about the software. The suite runs its tests in
  parallel and each spawns two or three daemons that sign, verify and encrypt, so a four-core
  runner does not run the same work slightly slower — it runs ten tests' worth against a sixth
  of the cores. The failing daemons were making steady progress and simply had less machine
  than the numbers assumed.

  So `tests/common::patience` scales a timeout by how much machine there is: at or above twelve
  cores nothing changes, below it the shortfall is the multiplier, bounded at eight because past
  that a hang should be reported as one rather than waited out. `KOLS_TEST_PATIENCE` overrides
  it, since `available_parallelism` reports cores rather than idleness and a loaded laptop looks
  nothing like an idle one. Applied inside `wait_for` rather than at each call site, and to the
  other five test files carrying the same assumption.

  **Verified at the width that failed**: all thirteen `two_nodes` tests pass pinned to four
  cores, in 83s. Two cores still fails one, and that is recorded rather than tuned away — it is
  half the narrowest runner anybody is proposing to use, and raising the bound far enough to
  cover it would trade a hang detector for a coffee break.

  Two notes for whoever touches this next. The helper's own tests live in `tests/patience.rs`
  rather than beside it, because `common` is compiled into every integration binary that
  declares it and unit tests there would run six times and report a count that lies. And
  `cargo fmt` reformats source files this repo has never run it over, so it was reverted off
  everything this change did not otherwise touch — a formatting sweep is its own commit, not a
  rider on a fix.

- **2026-08-20** — **All three repos are public, and CI needs no secret because of it.** The
  first workflow run failed at `actions/checkout` with a 404 on `distributed-intranet`, which
  reads as "that repository is gone" and means "this token cannot see it" — GitHub answers 404
  rather than 403 for a private repository so its existence is not disclosed. The fallback I
  wrote caused it: `secrets.PROTOCOL_REPO_TOKEN || github.token` is right for going public and,
  while private, quietly turned a missing secret into an unintelligible error two steps later.
  Same defect as the one fixed in `kols-cli::secret` the same morning, reintroduced in a
  workflow hours after writing about it.

  A preflight step now probes the API before checkout and fails with the cause named. **Two
  bugs came out of running that step rather than reading it**, which is the whole argument for
  the exercise. The heredoc terminator lands at column 0 only after YAML strips the block
  indentation — true, and worth confirming. And an **empty `Authorization` header is a bad
  credential rather than no credential**: GitHub answers 401 to it even for a public
  repository, so the check would have failed *after* the repos went public, which is the one
  scenario it existed to survive. The header is omitted when there is no token, and the
  post-public path is now the tested one: public repo, no secret, 200, exit 0.

  With the repos public the fallback is the live path and `PROTOCOL_REPO_TOKEN` is not needed.
  It stays supported, because visibility is a decision somebody may revisit and this keeps that
  a repository setting rather than an edit to every workflow.

  History was scanned for keys and credentials before this entry was written, since publishing
  a repository publishes every commit in it. Nothing found.

- **2026-08-20** — **Windows builds move to GitHub Actions, and the repo goes back to being
  OS-neutral.** Cross-compiling from the dev container was the fastest way to get an `.exe`
  into somebody's hands and the wrong place for it to live: it put mingw and a Windows Rust
  target into a Linux development image, and it made the only copy of a binary something that
  existed in one container.

  **The GNU build also cost something at the far end**, which is the argument for a Windows
  runner rather than a tidier cross-compile. An MSVC build links the WebView2 loader in; a
  mingw build *imports* `WebView2Loader.dll`, as a plain import rather than a delay import, so
  the window would not start at all unless that DLL travelled beside it. Building where the
  thing runs removes the whole problem instead of documenting it.

  `gate.yml` runs the tests and clippy on **Linux and Windows** for every push, and that half
  matters more than the release: `kols-cli::secret` restricts a seed differently per platform,
  and both bugs the first Windows run found were invisible from this container. A gate that
  only ever ran where development happens would ship them again.

  **The one thing that depends on the repos being private is written so it stops mattering.**
  CI checks out both repos as siblings, because the client depends on the protocol by path,
  and the default token reaches one repository only — so a PAT in `PROTOCOL_REPO_TOKEN` is
  needed while `distributed-intranet` is private. Both workflows resolve
  `secrets.PROTOCOL_REPO_TOKEN || github.token`, so going public means deleting a secret
  rather than editing a workflow.

  Also untracked `crates/kols-app/gen/schemas`, which Tauri regenerates on every build. Worth
  recording because it is the opposite of what the filenames suggest: `windows-schema.json` is
  regenerated by a **Linux** build, so these were never per-machine artefacts — just build
  output that produced a diff whenever a different machine built.

  **Written blind, and the first run is the test.** Nothing here has executed a workflow.
  Two mistakes were caught by reading rather than running — `cargo tauri` is a different
  distribution of the CLI from the `tauri` the npm package installs, and
  `--no-bundle-fail-on-warning` is a flag I invented — which is a reason to expect more.

- **2026-08-20** — **The window brings somebody in, so no step of the flow needs a terminal.**
  A founder could create a network in the window, run a node and never invite anybody, which
  is not a client you can hand to somebody else — it is a client plus an instruction to go and
  find one. `create_invite`, `waiting` and `admit` cross the boundary the same way everything
  else does, and the rail grows a doorway that is shown only to a holder of `approve-node`.

  **Seeing who is waiting needs the same capability as admitting them**, which is why it is one
  section and one flag rather than two. The waiting room is a local read rather than a command,
  for the reason `kols waiting` is: it is live state in the running node, which writes it down
  for anything else to read, so it is stale by construction and the interface says so where it
  shows it.

  `parse_identity` moved into the library rather than being copied into the shell. Two parsers
  for the same 32 bytes is how two front ends end up disagreeing about what is valid, and this
  is the third thing to make that trip — creating a network and serving one went first.

  **A first-launch bug, reported from Windows and fixed here.** The picker rendered correctly
  and the channel screen sat *underneath* it, reachable by scrolling. `hidden` is an attribute
  and the browser's `[hidden] { display: none }` is the weakest rule there is, so
  `.app { display: grid }` beat it. `[hidden] { display: none !important }` is the fix, and the
  `!important` is right exactly here: hiding is not a style choice a later rule may reasonably
  override, and a user theme (`design/09` §6) must not be able to reveal a screen this client
  decided you are not on. Every other `hidden` toggle in the interface had the same latent bug.

  Unguarded, and said rather than implied: nothing tests that a hidden element is invisible,
  because that needs a browser and this repo has no harness for one. The data path behind all
  three new commands is tested; what they look like is not.

- **2026-08-20** — **O8 closed on a real machine, and the window cross-compiles after all.**
  `kols.exe` created a network on NTFS and its seed's Security tab shows one account and
  nothing else — no inherited `SYSTEM`, no `Administrators`, which is exactly what the
  protected DACL is for and the one thing no build here could prove. Both halves of O8 are now
  observed rather than argued: the refusal from a filesystem that has no permissions to set,
  and the success on one that does.

  **`kols-desktop` was recorded as needing a Windows runner, and that was wrong.** Tauri on
  Windows is widely held to want MSVC, so the claim went in unexamined — twice, into `STATUS`
  and into the Dockerfile. What actually stopped the build was a **missing `icons/icon.ico`**,
  which `tauri-build` needs for a Windows resource file. An `.ico` generated from the 512×512
  PNG already in the repo, and the whole stack — tao, wry, webview2-com — links against mingw.
  A received opinion is not a finding, and this one cost nothing to check and was wrong.

  **What the GNU target changes about the product, which matters for handing it to somebody.**
  An MSVC build links the WebView2 loader statically; a GNU build **imports
  `WebView2Loader.dll`**, and it is a plain import rather than a delay import, so the process
  will not start at all without that DLL beside it. It is Microsoft's redistributable from the
  `Microsoft.Web.WebView2` NuGet package, it is not vendored in this repo, and packaging owes
  it a real answer — Tauri's own bundler does this on a Windows host, which nothing here is.

  Neither binary has been *run* as a window yet. Building is not running, which is the lesson
  of the entry above this one.

- **2026-08-20** — **The first Windows run failed, and both reasons were worth having.** It
  said `kols: Incorrect function. (os error 1)` — which names neither the file it was writing,
  nor what it was attempting, nor the one thing that would have fixed it.

  **The refusal was correct.** `SetNamedSecurityInfoW` returns `ERROR_INVALID_FUNCTION` on a
  filesystem that has no Windows permissions to set, and the run was from a `\\wsl$\` path,
  which is one. So the code did exactly what it was built to do — decline to write a seed it
  could not protect — and then failed at the only other thing it owed, which was saying so.
  A refusal nobody can act on costs more than the check that produced it saves. The message
  now names the file, the call, and the fix.

  **Underneath it was a real bug, and the reason it was silent is the interesting part.**
  `Store::default_root` read `$HOME`, which is normally unset on Windows — so it did not fail,
  it fell back to `"."` and put the store in the *current directory*. A client whose store
  follows the shell around is one that appears to lose a network whenever it is run from
  somewhere else, and nothing would ever have said why. Home is now `USERPROFILE` first on
  Windows and `HOME` first elsewhere, each falling back to the other, because Git Bash sets
  `HOME` on Windows and is common.

  **Set-but-empty is not an answer**, which `or_else` gets wrong: an empty variable would win
  the fallback and then be discarded, throwing away a good second candidate. That is a pure
  function with four tests rather than a line nobody can reach.

  Neither bug was reachable from this container — one needs a Windows filesystem and the other
  needs Windows environment variables — which is the argument for running the binary rather
  than admiring the build. 181 → 186 tests, clippy clean on both targets.

- **2026-08-20** — **`kols` builds for Windows, and the seed it writes there is restricted
  rather than inherited.** O8 was the gate on this and it is now written: `write_private`
  moved out of `store.rs` into `kols-cli::secret`, which restricts a secret to this user on
  every platform it supports and **refuses** on any it does not.

  **The ordering changed with it, and that is the half worth keeping.** It used to write the
  bytes and then chmod them, which leaves the secret on disk under the directory's
  permissions for a moment — and the moment is not the problem, a crash inside it is, because
  the file is still there afterwards. It now creates the file empty, restricts it, and only
  then writes. Restricting nothing costs the same as restricting a seed.

  On Windows the fix is a **protected** DACL, and the word is load-bearing: an unprotected one
  still inherits what its directory hands down, and inheritance is the whole defect — a seed
  in a profile directory picks up whatever that directory grants. Refusing when the DACL
  cannot be applied is the same call `design/00` §2's fail-closed principle makes everywhere
  else: a seed written where somebody else can read it is worse than a seed not written, since
  the second is an error a user sees and the first is one nobody ever does.

  **What is verified and what is not, stated because the gap is the whole risk.** The Windows
  path compiles for `x86_64-pc-windows-gnu` and `kols.exe` links — a real PE32+ binary. It has
  never run. Cross-compilation proves the calls exist with the shapes assumed here and proves
  nothing about the ACL that results, so O8 stays open with its remaining half named rather
  than being closed on a build.

  **Two things this shook out.** Clippy on the Windows target found a warning the Linux run
  cannot see, which means the gate is now two runs rather than one whenever `secret.rs`
  changes. And the confidence in a clean first compile of hand-written FFI was worth checking
  rather than enjoying: a deliberate type error inside the Windows branch proved it was being
  compiled and not quietly skipped by a `cfg`.

  The toolchain went into `.devcontainer/Dockerfile` rather than into this container by hand,
  which is `design/07` §2 S3's own lesson — the environment every claim depends on was once
  the one thing nothing recorded. 178 → 181 tests, clippy clean on both targets.

- **2026-08-20** — **The window creates a network, joins one by invite, and runs a node for
  it.** Three landings, one shape: everything the terminal could do that the window could not,
  because the code lived in a place only a terminal could call.

  **Creating** moved out of `kols init` into `kols_cli::workspace`, called by both front ends.
  That matters more than it sounds, because genesis has three requirements that are each silent
  when missed — `chat-log` on the content-type allowlist, the chat vocabulary registered, and
  `everyone` granted what a member needs — so a second copy in the shell would have looked right
  and failed at the first post. A workspace is a directory of networks, which is forced by the
  same thing that forces a node per network: `keypair_for` derives the libp2p keypair from the
  per-network identity, so networks cannot share a peer id without correlating identities Core
  §1.2 keeps unlinkable. It tolerates a `$KOLS_HOME` that is *itself* a store, because that is
  what `kols --home` means and still does.

  **The node** moved into the shell by giving `serve` a sink: a terminal prints its events, the
  window forwards them to the webview, and the loop knows about neither. The interface re-reads
  on every event rather than patching the screen — `design/05` §3's third property in its
  smallest form, since a record that arrived over gossip is also inside the segment that follows
  and a consumer that appended what it was handed would show every message twice.

  Only one process may run a node per network, and the store now enforces it: the MLS group is
  live state, and two nodes would each advance it without seeing the other, after which whichever
  saved last decides the network's key — with no symptom at the moment it happens. **The claim
  expires rather than only releasing on drop**, which turned out to be the load-bearing half: a
  window is closed by the window manager, which runs no destructors, so a claim released only on
  `Drop` would leak on the *normal* way this application ends. A pid check was the obvious
  alternative and is worse — liveness is a different answer on every platform and a reused pid
  looks alive while belonging to somebody else. Claiming also waits a stale claim out rather than
  refusing on sight, which two of the two-node tests found by restarting a daemon.

  **Joining** got the same treatment as serving, for the same reason. The picker offers it
  *before* creating, because those are not equally likely: somebody opening this client for the
  first time is usually holding an invite. Landing in a waiting room is reported as the success
  it is — under explicit intake an invite buys a connection and an identity and nothing else
  until a member admits you, so the window says so and shows the identity to be admitted rather
  than an empty network, which is what that state looks like when nothing explains it.

  172 → 178 tests. Neither the picker nor the join button can be pressed from a test, so the
  paths behind them are covered directly instead.

- **2026-08-20** — **Seeds are per network, and a password will wrap them rather than derive
  them.** `design/02` §6.3 said first run generates one master seed with a backup phrase. The
  implementation had always done something else — fresh entropy per network — and on reflection
  the implementation is right, so five documents changed instead of the code. The reasoning is
  the one per-network derivation already rests on: a master seed is the single object whose
  compromise correlates every membership at once, and the object a member would be told to write
  down. Recorded as **D28**.

  §6.3 now also says what restoring means, because "restore" promises more than it delivers. A
  seed derives an identity; it is not data and it is not a network. A phrase alone restores
  nothing, since a network's id cannot be derived from it — coming back needs the phrase, the
  network id and an address to reach the network at. Given those three, a member returns as *the
  same member*, and even their own messages come back, because their author log was published as
  content other members hold. What does not come back is named too, including the one that
  actually bites: the list of which networks they belonged to, which lives in the client's
  workspace and in no seed.

  And the shape of credentials is settled before anything builds them, because the tempting
  version is specifically wrong. Deriving a seed from a password and the network id needs no
  storage and is a **brainwallet with a verification oracle**: the network id is public,
  travelling in every invite, and member ids are in the governance log, so a guessed password
  produces a candidate identity checkable offline against a value the network publishes. A
  random seed *wrapped* under a password-derived key has nothing public to check against. Both
  credentials and backup are deferred deliberately and now sit in `design/00`'s roadmap as a
  release gate rather than a phase item.

- **2026-08-20** — **A network designates its relays, and §5.5 was optimistic about needing
  them.** Core §5.5 called bootstrap dependency "temporary per node" — a node caches peers on
  first join and reconnects without the bootstrap "as long as at least one previously-known peer
  is reachable". That clause assumes a reachable peer, and two members behind residential NAT
  are not reachable by each other. For an all-NAT network with no member relay, a bootstrap relay
  is a **standing dependency**, and the spec now says so.

  Underneath it was a structural gap. A hosted relay is **not a member** of the network it
  serves: `RelayNode` runs a restricted behaviour set and does not speak the ledger protocols, so
  it can never advertise itself the way a `relay_bootstrap_willing` member does. Nothing carried
  a newly deployed relay to members who had already joined — their invite is spent, their cache
  names a relay that may be gone. `NetworkPolicy.bootstrap_relays` is that carrier: replayed, and
  changed by `define-policy`. §5.5 now sets out all four carriers and why none replaces the
  others — invite, policy, local cache, ledger.

  Client side: `kols init --relay`, `kols relay list/set`, and `kols serve` reserving a circuit
  and recording it, so an invite carries the one address that works from another network.
  **`kols invite` refuses without a relay**, which is the honest ordering: a network needs one
  before it can invite anybody.

  **A wrong call of mine, corrected, and recorded because the misreading is easy to repeat.** A
  reservation the relay logged as *granted* left the member with no circuit listener, and I read
  that as a transport bug. It is not. A relay promotes listen addresses to external addresses
  only when they are **not loopback** — correct, since 127.0.0.1 is useless to another host — and
  libp2p builds a reservation's address list from external addresses alone. So a loopback relay
  grants every reservation, logs that it did, reports healthy, and hands back nothing. My test
  relay was on loopback.

  Two things worth keeping came out of it. `crates/kols-cli/tests/relay.rs` asserts what nothing
  asserted: that a reservation ends in a *usable* circuit. The protocol's own relay tests count
  grants on the relay side and its wildcard tests filter circuit addresses out to compare source
  ports, so "granted" was covered and "reachable" was not. And `kols serve` now names loopback as
  the likely cause when a circuit does not arrive, because every other vantage point says it
  worked.

  One real bug in the new code alongside it: `serve` appended this node's peer id to every listen
  address, and a circuit address already ends in one — producing `/p2p-circuit/p2p/<id>/p2p/<id>`,
  which nothing dials.

  171 → 172 tests here, 647 → 649 upstream.

- **2026-08-20** — **Invites: one string instead of three hex exchanges, and a protocol gap
  that nothing had hit.** Adding a second person used to mean `attach` with a 64-character
  network id, copying the joiner's 64-character identity back, `admit`, and then being told a
  multiaddr by hand. Now it is `kols invite` on one side and `kols join <that>` on the other.

  **The protocol could not serialize an invite.** Core §5.6 defines it as a credential carried
  out of band — pasted into a message, put behind a link — and the only place its bytes
  appeared was inside a `JoinRequest`, which is the *far end* of that journey. You could issue
  one and have no way to give it to anybody. Nothing had noticed because nothing had yet tried
  to invite a person. `encode_invite`/`decode_invite` landed upstream under their own domain
  tag, with the spec now saying an implementation owes this, since the omission was in the
  specification as much as in the code.

  **Decoding establishes framing and nothing else**, which is tested directly: a tampered
  invite still decodes and fails later at `validate`, where "does this issuer hold
  `approve-node` *now*" can actually be answered. A decoder that verified the signature would
  invite the reading that decoding had already settled something.

  **An invite needs an address and only a running node knows one.** So the daemon writes down
  what it is reachable on and `kols invite` reads that, refusing to mint rather than producing
  a credential that connects to nothing. In reverse, whoever redeems one keeps the addresses it
  carried — so `kols serve` needs no `--peer` afterwards, which was the last piece of manual
  address-passing in the flow.

  **The URI is a container, not a format.** The bytes are the protocol's; the scheme and the
  unpadded base32 are this client's, picked so an invite survives being pasted into a chat
  message and copied back out — tested against leading whitespace, a stripped scheme, wrapped
  lines and lower-casing, because an invite that only works when typed carefully has failed at
  its one job. A truncated one is refused where it was truncated rather than decoding into
  something that fails a signature check somewhere else.

  The waiting room needed the same treatment as the addresses: it is live state in the running
  node, so the daemon writes down who is in it and `kols waiting` reads that — stale by
  construction, and said so where it is shown.

  163 → 171 tests here, 644 → 647 upstream. Both gates green.

- **2026-08-20** — **Display names, and a design document that was wrong about where they
  live.** `design/02` §7 put a member's display name in the mutable pointer they own, next to
  their avatar and status. That works for everything except being unique — a pointer is
  single-writer by construction, so it says what I call myself and has no ordering relative to
  what anybody else published. Two members claiming one name produces two nodes that disagree
  about who is who, with nothing to settle it. **Uniqueness needs a total order, and the log is
  what has one.** Third time this project has reached that conclusion: App Hosting §4.3 for app
  names, D4 for channels, now this.

  So a claim is a `chat` application entry — E2's generic door, no platform change — and the
  avatar and status stay in the pointer, because neither needs ordering.

  **The payload carries no identity, which is the security property rather than an economy.**
  A claim binds whoever authored the entry, whom the protocol already verified, so claiming a
  name for somebody else is unsayable rather than refused. That is what lets `chat:set-name` be
  ordinary and sit on `everyone` at genesis without widening anything.

  **The interesting half is normalization**, spec 07 §3.9.1: NFKC, whitespace trimmed and
  collapsed, lower-cased, invisible code points refused outright rather than stripped — because
  stripping would let two claims that look identical produce one holder, quietly. Every step is
  pinned because two nodes normalizing differently would disagree about what is a duplicate,
  which is a consensus bug wearing the clothes of a display concern.

  **Confusables are deliberately not folded**, and the residual risk is written down rather
  than hidden: `alice` and a Cyrillic lookalike are distinct keys and both may be held. The
  tables are large, they collide names across scripts with every right to exist, and two nodes
  on different table versions would fork. So the obligation moves to interfaces — spec 07 §8
  now requires a name be rendered alongside enough of its holder's identity to tell two apart,
  and both the CLI and the window do.

  **A name is never released, including when its holder leaves.** History renders by author id
  with names resolved at display time, so an inherited name silently relabels somebody else's
  past messages: every line honestly attributed while the conversation becomes a lie.

  Two corrections to the spec came from implementing it. It asked for full case folding, which
  is the better tool and is not available identically everywhere — lowercasing is, and a rule
  everybody applies the same way beats a better rule applied two ways. And it claimed a
  determinism it cannot have: the character categories and the normalization are both defined
  against a Unicode version, so two nodes on different versions can disagree about a name built
  from newly assigned code points. Bounded, rare, resolved by upgrading — and the sharpest
  argument for refusing confusables, whose tables move far more.

  One conflict inside §3.9.1 surfaced only by running it: a tab is a control character, so step
  1 refuses it, while step 3 says whitespace is collapsed. Refusing is the consistent answer and
  the spec now says so — silently collapsing a character the claimant cannot see is exactly what
  step 1 exists to prevent.

  139 → 163 tests, clippy clean.

- **2026-08-20** — **A window, and a build that got smaller while gaining one.** The first
  interface slice: `kols-app` is a Tauri v2 shell holding one `Executor`, `kols-ui` is HTML,
  CSS and one script holding no keys, no sockets and no files. It lists channels, renders one,
  posts to it, and hides the controls this member cannot use — `design/09` §4's first two
  questions and §5's permission-gated chrome.

  **The shell converts rather than deriving, and that is the decision worth keeping.** The
  domain's records have exactly one serialization and it is normative: spec 07 §3's canonical
  encoding, hand-written because a record's id is the hash of those bytes. Putting `Serialize`
  on the same types would have created a second serialization beside the first, and what that
  invites is not hypothetical — somebody eventually sends the convenient one over a wire and
  finds ids no longer match. So `kols-app` owns view types, and `kols-api` has no `serde`
  dependency at all. The webview also never *builds* a command: it names an intent with plain
  arguments and the shell constructs it, which is one fewer place a front end can hand the core
  a shape it did not expect.

  **Adding Tauri made `target/` 7.0 GB, so the profile changed and it is now 2.4 GB.**
  Dependencies get no debug information at all — `[profile.dev.package."*"] debug = false` —
  which is the same cut this workspace already made once for its own crates, made again where
  it now costs the most: webkit, wry, tao, gtk and their bindings dwarf the code. Our crates
  keep line tables, so a backtrace still points at a file and line in this repo, which is what
  a failing test is read through. The build is *smaller than before Tauri arrived*, on a clean
  rebuild with everything green.

  **What could not be verified, stated rather than implied.** The data path is tested and the
  process starts, but the window was not looked at: the X display that S3 proved a Tauri window
  on was dead by the time this landed, and the Wayland path it does run on cannot be
  screenshotted from here. So the layout is unreviewed, and `design/09` §7's first open
  question — the navigation shape — is answered by a first guess rather than a decision.

  139 tests, clippy clean.

- **2026-08-20** — **The event half: the boundary now carries both directions, and the
  vocabulary was written from what the daemon already said.** `design/05` §3 sketches an
  `Event` enum with nine variants. What landed has six, and the difference is the method: the
  daemon has been reporting for weeks — records learned off a head segment, records recovered
  from behind it, a record arriving live, governance entries, an epoch rotation, a member
  keyed in, a handful of degradations it carries on through — and every one of those is now an
  event with something producing it. Nothing was added because a sketch listed it.

  **Two things are deliberately not events.** This node's transport — which addresses it
  listens on, which peers it is connected to — is a fact about the machine rather than about
  the network's content, and a sandboxed build would not be told it at all (App Hosting §3.2's
  "no ambient host access"). And the startup report, which is what this node *is* when it comes
  up rather than something that happened while it ran. Both keep printing; neither crosses.

  **Property 3 is the consumer's, and it comes down to one word.** Events are idempotent and
  re-deliverable not because the emitter is careful but because it cannot be: the live path may
  be lossy, and a record pushed over gossip is *also* inside the segment that follows it. So
  the obligation is to **merge, never append** — a records payload goes through `ChannelView`,
  which is a function of the record set and deduplicates by record id. Five tests hold a
  consumer to it, and the one worth naming is `a_record_that_arrives_live_and_again_in_a_segment_is_one_message`:
  that is the normal delivery pattern, not an edge case, so a consumer that appended would show
  every message twice, every time.

  `Arrival` carries how a record got here — live, off the head, or backfilled with how many
  sealed segments the walk reached. It exists for notification and progress and never for
  ordering, which is asserted directly, because it is the thing a client is most likely to get
  wrong: order is computed from the merged set (`design/01` §4), so the same record renders
  identically whichever way it came.

  The refactor is behaviour-preserving by construction: the two-node tests assert on the
  daemon's exact wording, so `render` reproducing it is what says the change moved code and not
  behaviour. 129 → 134 tests, clippy clean.

- **2026-08-20** — **The executor: one submit path, and a reader-side hole found by building
  it.** `authorize` returned an `Authorized` and each caller then took it apart and did the
  work inline — a gate with no dispatcher behind it, and a sequence a second caller would have
  copied slightly differently. `Executor::submit` is now the one way in: authorize, then run.
  The `Authorized` never leaves the module, because `run` is what requires one and nothing
  else can produce one, so the check is not something a future caller can be *asked* to
  remember. It returns typed `Outcome`s and prints nothing — an executor that printed is one
  no interface could reuse, which is the whole reason there is a boundary.

  **The finding is a pin.** `design/02` §2.2 puts pinning under `chat:moderate`, and the
  boundary requires it — but `ChannelView` admitted a `Pin` record under `chat:post`, like any
  ordinary record. So a modified client holding only posting rights could pin, and every
  conformant reader would have honoured it. **A check the writer makes and the reader does not
  is a check that does not exist**, and this one was decorative from the moment the boundary
  started requiring it. The reader now asks for moderation authority, through a new
  `Authority::may_moderate_now` — deliberately separate from `may_moderate_at`, because the two
  answer different questions: a redaction cites the governance head its author observed and
  keeps standing when that author is later demoted, while a pin cites nothing and should stop
  holding when its author's authority does. An existing test had to change with it, which is
  the honest signal that the behaviour did.

  **Two of §6's debts closed because the executor holds a store and the boundary deliberately
  does not.** An edit or withdrawal aimed at somebody else's message is refused before a record
  is signed, rather than after every reader has ignored it — structurally it was never possible
  to *succeed*, since nobody writes into another author's log, so what this adds is being told.
  And the rate ceiling is enforced over the author's own HLC readings, which is what makes it
  the same verdict on every node: a user typing too fast is told they are, and reader-side
  refusal stays the backstop against a modified client rather than the primary experience.

  **`kols-cli` is a library with a binary on top.** The executor was worth testing without
  spawning a process — a test that had to run `kols` to reach it would be testing argument
  parsing at the same time, and vague about which half failed. The binary is now argument
  parsing and rendering, which is also the shape the desktop client needs: a different front
  end over the same submit path rather than a second copy of it.

  **Every record kind the design describes now runs.** Edits, withdrawals, reactions and pins,
  plus channel rename, topic, slowmode and archive — with `read` printing message ids, because
  every command that acts on a message needs one and a user who cannot see it cannot act. Ten
  new tests drive them through the real binary. 117 → 129, clippy clean.

- **2026-08-19** — **A documentation pass, and it found three stale claims rather than
  none.** The point of reading all of it after a landing is that some of it is wrong, and it
  was: `design/02` announced E11 as outstanding and told a reader the capability registry
  matches names exactly, so a parametrized capability needs an entry per scope — the exact
  problem E11 landed to remove, described in the present tense two rounds after it was gone.
  `design/01` §3.2 still flagged derived pointer ids as a protocol change to make, when E3
  was withdrawn because `PointerId::from_bytes` was already public. And §7 still called
  gossipsub "the recommendation" after E4 shipped it. A design document that describes a
  solved problem as open is worse than one that says nothing, because somebody will plan
  around it.

  **§6 is new: what this client owes, and why each thing is outstanding.** Seven items — the
  executor behind `kols-api`'s gate, the event half and the idempotence property with it, the
  two checks `authorize` deliberately does not make, the commands with no code behind them,
  E12's client half, `may_moderate_at` ignoring the head it is given, and `kols-store` not
  existing. None blocks anything else, which is exactly why they need writing down: a debt
  nobody records is one somebody rediscovers as a surprise. Every entry names where it was
  incurred, so the reason survives without the person who had it.

  Two decisions from the boundary work joined the register in `design/00` §3, since both are
  architectural rather than incidental: **D25**, that authorization is a type rather than a
  call, so an executor cannot receive a command nobody checked; and **D26**, that a command's
  consent class follows the tier of the capability it needs rather than how consequential the
  action looks.

  Also refreshed: `design/00`'s roadmap, which listed P1 as future work while it was under
  way; `design/07`'s status, which said P1 had not started and is now marked complete in its
  own terms, with a line saying its job is finished and `STATUS.md` carries P1 — a build plan
  kept as a running status is a second one to disagree with this file; `design/05` §3 and its
  test table; `design/09` §5, which claimed the enforcement half as future when it is built;
  and the README, which described three crates and 98 tests.

- **2026-08-19** — **`kols-api`: the boundary exists, and the CLI is the first thing to
  cross it.** `design/05` §3 fixes three properties and says retrofitting any of them is
  expensive. Two are now held; the third has nothing to hold yet.

  **`Authorized` is the shape that makes the check unavoidable.** It wraps a `Command`, has
  no public constructor and no public field, so the only way to hold one is to have passed
  `authorize` — an executor takes an `Authorized` rather than a `Command`, and skipping the
  gate stops being a thing a reviewer has to notice and starts being a thing that does not
  compile. That claim is a `compile_fail` doctest rather than a sentence. It is the same
  structural move the protocol repo made with the media relay's guard, where `authorize` is
  the only way to learn a frame's recipients, and for the same reason: a check a caller can
  route around eventually is.

  **`Sensitivity` is derived from the capability vocabulary, not from judgement.** App
  Hosting §3.3 requires a platform prompt before any signed action on the user's behalf, so
  the line that matters is whether a command signs. Above that line the class follows the
  *tier* `design/02` §2.2 assigns — which is why `Pin` is `Governs` and `SendMessage` is not,
  despite pinning being the smaller act: pinning needs `chat:moderate`, which is
  governance-tier. A drift test resolves every command's verb against
  `capabilities::VERBS`, so re-tiering a verb and forgetting this classification fails there
  rather than in a consent prompt that silently stopped appearing.

  **One small thing was missing from `kols-core` and is now there.** Creating a channel can
  only ever be authorized at category or network scope, because the channel's id is minted by
  the entry that creates it and no grant could name it beforehand. `holds_in_scope` is
  `holds` with its first step removed — separate rather than reached by passing a placeholder
  channel id, so a caller cannot invent an id to ask about and have the answer quietly depend
  on it.

  **The CLI crosses the boundary for `channel create`, `post` and `read`**, and the checks
  those used to open-code are gone rather than duplicated: `may_post`, the message ceiling
  and the network-profile test all live in one place now, and `network::require_server` was
  deleted because the boundary answers it. The binary-driving tests pass unchanged, which is
  the only evidence worth having that the seam holds — it is the same argument `kols` itself
  was written for.

  **What is deliberately not in this crate.** No `Event`: the sync engine that would emit one
  is `kols serve`, whose records do not cross this boundary yet, and an event vocabulary
  written before anything emits one is a contract with no implementation to keep it honest.
  No DM, search, voice or stage commands — each has a line in `design/05` §3 and no code
  behind it, and adding the variants now would put a claim in a type nothing could serve.
  Two checks are named in `authorize`'s own docs as *not* done there, with where they are
  done instead: that an edit targets a message you wrote (a fact about the record set, caught
  on read by `ChannelView`), and the message rate ceiling (computed over the author's own
  HLCs, so also the store). A check that looks complete and is not is worse than one that
  says what it does not cover.

  98 → 117 tests, clippy clean.

- **2026-08-19** — **S3 done: the environment builds and runs a Tauri app, and the list of
  what it needed was wrong in two places.** Installing the packages is not the interesting
  part; finding out what the packages actually are is. `design/07` §2 named
  `libappindicator3-dev`, which **does not exist in Debian 12** — the tray library it ships is
  the Ayatana fork — so the environment as specified could not have been built by anyone
  following it. And the display arrives by a different route than assumed: there is no
  `/mnt/wslg` inside the container, because WSLg runs on the *host*; what reaches the
  container is VS Code's own forwarding of both an X server and a Wayland socket, and both are
  live.

  **Confirmation had to be a running window, because nothing weaker distinguishes "the
  packages are installed" from "a GUI works here".** A scaffolded Tauri v2 app compiled and
  linked against the system webview in 44 seconds, and an 800×600 window mapped on `$DISPLAY`
  within a second with its WebKit child process alongside. Built in a scratch directory, not
  in this repo — `kols-app` gets created when it has code, not to hold a smoke test.

  One consequence worth keeping for whoever writes a test that looks for a window: GTK takes
  the Wayland socket when `WAYLAND_DISPLAY` is set, and a Wayland window is invisible to
  `xwininfo`. The app is fine either way; a *script* asserting on a window needs
  `GDK_BACKEND=x11`, or it will conclude nothing opened.

  **A papercut found on the way, worth having now rather than at P1**: the image carried only
  the C locale while `LANG` arrived set to `en_US.UTF-8`, so every GTK process started with
  "Locale not supported by C library" and fell back. That is a poor footing for a client whose
  entire payload is other people's text, and it was one generated locale away from fixed.

  All of it landed in `.devcontainer/` rather than in this shell's history: the Dockerfile
  carries the packages, Node 24 LTS from NodeSource (Debian ships 18, past end of life) and
  the locale; `devcontainer.json` gains `ko-ls/Cargo.toml` in `rust-analyzer.linkedProjects`,
  which had named only the protocol workspace and left half the tree unanalysed, and its
  `postCreateCommand` now checks node, npm and `pkg-config --modversion webkit2gtk-4.1` —
  the dependency whose absence otherwise surfaces as a Rust link error.

  **That config now lives in this repo, at `.devcontainer/`, and is tracked.** It sat at the
  workspace root, which is in neither git repo — so the environment every claim above depends
  on was the one thing nothing recorded. It is the client that needs Tauri and this repo's
  `design/07` that owns S3, so this is where it belongs. Because the client builds against
  its sibling by path dependency, the container still has to see both: `workspaceMount` binds
  the *parent* of the two repos and `workspaceFolder` lands at `/workspaces/ko-ls`, so the
  folder you open is now the `ko-ls` repo and the tree you land in is unchanged.

  **Two build caches, not one.** `ko-ls/target` had been sitting on the bind mount while the
  protocol's was in a named volume — the exact case the volumes exist to avoid — so
  `ko-ls-client-target` joins them. The first build after this recompiles from scratch, and
  the 2.1 GB already in `ko-ls/target` on the host is shadowed rather than removed; it wants
  deleting from outside the container.

  **`distributed-intranet/.devcontainer/` is deleted.** It called itself `dclone-dev`,
  predated the NAT harness and the clippy half of the gate, mounted no docker socket, and was
  referenced by nothing. One config, in one place, that is actually the one being used.

  **The Dockerfile was then built from scratch rather than trusted**, because everything
  above had been proven against a container patched by hand, and "a rebuild has these" is a
  different claim from "this machine has these". A clean `docker build` of it comes up with
  Node 24.19.0, webkit2gtk 4.1, JavaScriptCore, libsoup 3, Ayatana appindicator, patchelf and
  a resolving `en_US.UTF-8` — so the image reproduces the environment rather than recording
  what was done to one instance of it.

  No client code changed: 98 tests here and clippy silent, both unchanged.

- **2026-08-19** — **E12 landed, and landed narrower than it was written.** `design/06` §12
  asked for tiered node liveness — hot, warm and cold, with a warm node holding a relay
  reservation without a full behaviour set. What went into the protocol is **Core §5.1.1:
  peer discovery is optional**, and nothing else. `MemberBehaviour`'s `kad` and `mdns` are
  `Toggle`d and `MemberNode::with_discovery(.., Discovery::Off)` builds a node without them,
  keeping everything else — it still listens, dials, is dialable, relays, hole-punches,
  gossips and serves every request-response protocol.

  **The split is the finding, not an omission.** The behaviour set is the platform's because
  a consuming client cannot assemble a partial one. The tiering is not: whether a node exists
  at this moment, and whether it holds a reservation while nothing is happening, is a decision
  made over time by whoever is holding the nodes, and a specification has no view on it.
  Asking for it in a spec would have put client policy in the platform. Hot, warm and cold
  stay in `design/09` §2, built on reservations and dialability, both of which already existed.

  **How absence is reported is the part that needed care.** `find_providers` and
  `enumerate_collection` return `Option` now, and `None` means *there was no query to run* —
  returning a query id that never resolves would be indistinguishable from content that
  genuinely has no holders, which is the confusion `set_dht_server_mode` already exists to
  prevent. Announcing is a no-op rather than an error.

  **One consequence surfaced in review and is now specified.** The routing table is also the
  address book, so a node without discovery dials by address and never by peer id alone, and
  caching an address against a peer is a no-op there. A pairwise network pays nothing for it —
  addresses arrive with membership — but it constrains any other use.

  Protocol repo: 644 tests, clippy clean, `tests/discovery_off.rs` new. Client: 98 tests,
  unchanged and still green against it. The client half — asking for `Discovery::Off` on a
  conversation-profile network — is owed and blocks nothing.

- **2026-08-19** — **`design/09` written: the interface, before any interface code.** `05`
  fixed the client's architecture and stopped before anything about what the interface looks
  like; eight design documents held two UX commitments between them, both incidental to
  sections about something else. That gap is now a document.

  **Three findings came out of writing it, and two became protocol extensions.**

  *One node per network is forced, not chosen.* `keypair_for` derives the libp2p keypair from
  the per-network identity, so the tempting optimisation — one swarm across several networks —
  would mean one peer id across them and would correlate identities Core §1.2 keeps
  unlinkable. The resource-saving version of the switcher is the one that breaks the security
  model, so it is written down as rejected rather than left to be rediscovered as a
  performance idea. Since a DM *is* a network (`03` §4.3), the node count is
  `servers + conversations` — hence **E12**, tiered liveness, which is P1 and blocks nothing
  else.

  *The wake-up ping does not need to exist.* Keeping a connection open for a quiet DM is
  impossible anyway — Core §5.3 caps a relayed circuit at 120 seconds deliberately, because a
  relay assists connection establishment rather than carrying traffic. But a *reservation* is
  metered separately and is long-lived, so being dialable is the primitive, and **the dial is
  the wake signal**: an inbound stream wakes the handler, with no extra round trip and no new
  message type. This was very nearly specified as its own mechanism.

  *Two people starting a DM must never provision a relay.* The shared network is already the
  rendezvous — `03` §4.3 carries the invite over a stream inside it, with a common-ownership
  proof — so the DM connection can be bootstrapped over the connection that already exists
  (**E13**). Relaying DM traffic through a shared-network node also works and is worse: it
  tells a third party who is talking to whom, where address exchange tells nobody anything
  they did not know. It stays as the fallback, and Core §5.3's correction says a stateless
  bootstrap relay "carries bytes and never inspects a join at all", so that fallback needs no
  protocol change. E13 carries one hard gate: addresses for network X are disclosed only to a
  member of X, or it becomes an oracle for enumerating a user's other identities.

  On theming, the security question resolved cleanly rather than being traded away. CSS can
  exfiltrate — attribute selectors plus any URL-loading property — but it has exactly one way
  to do it, a network request, and `url()`, `@import` and `@font-face src` are the complete
  set. Under a CSP permitting no remote origins, arbitrary user CSS *cannot* phone home. What
  CSP does not solve is spoofing, so security-critical surfaces render as native dialogs
  outside the themeable DOM: a theme may make the client unrecognisable and can never fake a
  signature prompt.

- **2026-08-19** — **E11 landed: a registry entry can cover a namespace.** An extension
  capability's tier came from a registry matching names *exactly*, and chat capabilities are
  parametrized by scope — so `chat:post:<channel>` needed an entry per channel, added by a
  policy change. Creating a channel with a permission override meant amending network
  policy, and the registry grew with the channel count forever. A registration ending in
  `:` now covers the namespace beneath it, longest match winning (Core §2.2.1).

  **The separator requirement is the part `design/06` §11 left open, and it mattered.**
  Plain prefix matching would let a registration for `chat:post` also cover
  `chat:postmortem` — a different capability that merely starts with the same letters,
  silently inheriting a tier nobody chose for it. Requiring a namespace to end at a
  separator means it can only cover names genuinely within it, and it is also what keeps
  every existing exact registration exact.

  **Removing the workaround showed it had never actually worked.** `design/06` §11 described
  it as registering "a scope's names when that scope is created", and nothing in the client
  ever did that: `for_category` and `for_channel` existed and were called only by tests. So
  `chat:<verb>:<channel>` was in no registry anybody wrote, a per-channel grant was refused
  at replay, and the per-channel override `design/02` §4 describes could not be used at all.
  That is now a test rather than a discovery waiting to happen.

  The drift test between what the client registers and what it declares got stronger on the
  way through. It could previously only check the network-wide `:*` name, because scoped
  names were supposedly registered elsewhere; it now resolves *every* form a declaration can
  take against the real registry, including a scope invented after genesis.

- **2026-08-19** — **A key per segment, and a freshness bound on the live path.** The two
  gaps the sealing work left open, closed.

  **Per-segment keys were forced, not chosen.** `MutablePointer::update` carries
  `dek_commitment` forward unchanged — deliberately, since Storage §1.2 fixes a DEK for its
  object's lifetime — so a pointer commits to one key for its entire life, and every
  segment sharing a pointer shares a key. A key that opens the newest message then opens
  the oldest, and retention can only ever forget a whole log. There is also no way to smuggle
  a key in beside it: `PointerRequest::Fetch` returns wrappings only alongside a pointer
  record that exists, so a wrapping for a pointer nobody published never syncs. A
  separately-forgettable key therefore needs a pointer of its own, and that settles the
  design: `author_segment_pointer(channel, author, sequence)`, one per segment.

  Sealing now starts a new segment **and** a new key, and the sealed segment keeps the key
  it was written under — nothing is re-encrypted. Re-keying it would move every CID in it,
  forcing every reader holding it to refetch the whole object, and would leave the
  superseded ciphertext readable under a key nothing retires, which is the exact thing this
  makes possible to avoid.

  The cost is one indirection on the read side. `author_log_pointer` — the derivation a
  reader computes from public information alone — no longer names the messages; it names a
  head index, an otherwise empty segment whose `sequence` says which segment is current.
  Reusing `Segment` for it meant no new type, encoding or content type. Its pointer version
  **is** the sequence, and that is not cosmetic: same-version pointer records are settled by
  lower record hash, so an index republished at version zero would lose that coin-flip
  against the copy peers hold about half the time, and newer history would never be found.

  Two things fell out of building it. Reading past a retention boundary is deliberately
  indistinguishable from history that has not arrived — a missing wrapping, a wrapping under
  an epoch this node has not caught up to, and a segment retired last year all look the
  same, and a client claiming "this was deleted" would assert what it cannot know. And
  because a retired boundary never becomes readable, a walk would have re-fetched,
  re-decrypted and re-verified every signature behind it on every tick forever; the store
  now keeps each segment's chain link, so a re-walk costs file reads and no crypto.

  **The live path's retry is now bounded.** It exists to shave latency off records being
  written now, but an unbounded retry made its retry set everything the node ever wrote, so
  an author's whole history went out over gossipsub the instant anybody subscribed. §6.1
  says nothing may *depend* on that path; it does not license the path to substitute for the
  durable one. Records outside the window are retired from the set rather than skipped, so a
  tick stops rescanning history to decide against it again.

  One test bug worth recording: the new live-path test reused ports another test already
  had, so two daemons shared a log file and one could not bind — which surfaced as a daemon
  whose log went *empty* after previously containing output, not as a bind error.

- **2026-08-19** — **Segments seal, and readers walk back through them.** Backfill was
  supposed to be the reader half of a model already in place. It was not: `AuthorLog::seal`
  existed and the daemon never called it, so every author log was a single ever-growing
  segment. That has a consequence worth stating plainly, because it inverts the reason for
  doing the work — **a reader already saw all of an author's history**, since the one
  segment held it. The gap was not scrollback. It was that opening a channel cost the whole
  conversation rather than a screenful, which is exactly what `design/01` §5's
  "head segment only" exists to prevent.

  So sealing landed first, on §3.1's two thresholds, and the walk after it. Sealing needs
  **no persisted state**, which is the part worth keeping: boundaries are a pure function
  of the record sequence, so a node that restarts and replays its store re-derives the same
  seals and republishes the identical chain. That only holds because age is measured across
  a segment's own records rather than against the clock — "older than a day *now*" would
  seal somewhere new on every restart and publish a chain competing with the one readers
  already hold.

  Two bugs, both found by the test rather than by reading. The walk marked a segment
  absorbed as soon as its records were stored, and stopped at the first marked segment — so
  a reader took exactly one hop of history and then stopped, permanently, while *reporting
  a successful backfill*. A mark now means "this and everything behind it", which can only
  be set once the chain bottoms out. And the test itself was passing for the wrong reason
  until `--no-live` existed: an author retries a record that failed to publish, so its
  entire backlog goes out over gossipsub the moment a peer subscribes, and Bob was learning
  all thirty messages live. That retry is still unbounded — spec 07 §6.1 says nothing may
  *depend* on the live path, not that it may substitute for the durable one — and it is
  logged in §1 rather than fixed here.

  `kols serve --no-live` also closes a MUST: §6.1 requires conformance be testable with
  gossip disabled, and the test that claimed to do that had been arranging for the two
  daemons never to overlap, which held only while there was a single record to race on.

- **2026-08-19** — **E4 landed: records arrive live as well as durably.** Gossipsub joins
  `MemberBehaviour` (Core §5.1) as the one broadcast primitive in a stack that is otherwise
  pull-based — and the exception proves the rule, since everything else carries state a
  partitioned node must be able to obtain *late*, which a broadcast cannot provide. A live
  payload is the opposite case: one nobody needs to receive at all.

  Three configuration choices are load-bearing rather than incidental. **Signing is off** —
  a record already carries its own signature over its own canonical bytes, so signing them
  again with the transport keypair would leave a receiver with two authorities for "who
  wrote this" and no rule for choosing; `gossip_behaviour` takes no keypair, so the absence
  is visible in the signature. **Message ids are content hashes**, not the default sender
  and sequence, because the same record legitimately arrives twice — once live, once in a
  segment — and deduplication has to agree with the consumer's own content addressing.
  **The transport validates nothing**: it does not know what a payload means, and half a
  check would be worse than none, since a caller would read it as done.

  The payload is sealed under spec 07 §5.2's channel content key, derived from the epoch
  and bound to both channel and rotation — so it cannot be read by a non-member sharing the
  mesh, cannot be relayed into another channel, and survives publication either side of a
  rotation because it carries the `rotation_ref` its sender used.

  **Three things this shook out, and the third was my own test being wrong.** A record was
  marked broadcast even when the publish failed for want of a subscriber, so a message
  posted moments before a peer subscribed never went live at all — the one case the path
  exists for. Landing it then broke two passing tests, which was the useful finding: they
  waited on the durable path's "learned 1 record", and a record arriving live is *already
  stored* by the time the durable absorb runs, so that line correctly never printed. Both
  paths now report in the same words with the path named, because two vocabularies for one
  event make "did this arrive" depend on which way it came.

  The third: the new test waited for the record to reach the peer *live* and kept failing —
  and the path was fine. On loopback the durable path is milliseconds too, so which arrives
  first is a race, and §6.1 promises only that the record arrives. The test was demanding
  something the design explicitly declines to offer. It now asserts what is actually
  promised: the record demonstrably goes out live (a publish only succeeds once somebody is
  subscribed, so the daemon reports that separately), it arrives, and it lands **exactly
  once** however many paths carried it.

  The gossip-disabled case §6.1 requires be testable is covered by never overlapping the
  two nodes: Alice writes and stops before Bob runs, so nothing can reach him live and
  everything he ends up with came through the durable path alone.

  **A third finding, and the worst of the three: the daemon was starving its own CLI.**
  `exclude_removed_members` took the append lock on *every tick* — thirty acquisitions a
  minute, each held across a full log read — while one-shot commands need the same lock to
  append at all. `kols admit` timed out waiting for it. The check now runs *before* locking
  and only locks when there is genuinely a member to exclude, which is almost never. The
  lock guards appends; reading never needed permission, and treating it as though it did
  made the daemon and its own commands compete for a resource neither of them was really
  using.
- **2026-08-19** — **Retention landed as two windows, and a catch-up bug was found asking
  what the default should be.** The question was what to set for retiring superseded epoch
  keys, with the worry that somebody absent for thirty days would come back locked out.
  Checking that premise found it was not the failure mode — every rotation carries an MLS
  commit, so an absent member derives the keys it missed by replaying them (Core §3.3), and
  the log never shrinks. **But the daemon never called `apply_pending_rotations`**, so
  nobody caught up on anything. It stayed invisible because an object keeps its DEK for
  life: an absent node could still read appends to logs it already knew, and only a *new*
  object under an epoch it never derived would fail — as content that fetched perfectly and
  would not open. Fixed, with a three-node test where a member is offline while somebody
  joins and posts.

  That also settled the shape of the setting. Retiring keys is not a knob: a key is
  droppable once nothing inside the retention window is still wrapped under it, which the
  re-wrap-on-read path already arranges. A separate key-lifetime setting could contradict
  the retention window — keep content a year, drop its key at six months — and make
  retained content silently unreadable.

  So it is **two content windows, not one**: `chat:retain-messages-days` and
  `chat:retain-attachments-days`. A message is capped at 8 KiB with a capped rate, so a
  million of them is a few gigabytes network-wide; one attachment may be 25 MiB, ten to a
  message. A network bounding what it spends on other members' disks means the attachments,
  and a single window would charge it the scrollback too.

  **Both default to `Forever`, which revises `design/01` §8's earlier rolling-window
  default.** The argument is asymmetry rather than taste: retention can be switched on
  whenever a network wants it, and content already allowed to go dark cannot come back — so
  a network that never thinks about the setting should keep its history. Zero, negative and
  absurd values all read as `Forever` for the same reason. A log is judged on its *newest*
  record: one somebody is still writing to is live however far back it reaches.

  Attachments have a window and nothing yet to apply it to, since the CLI does not carry
  attachments — the policy is readable and honest about that rather than pretending. 78 → 83.
- **2026-08-19** — **Made "try every epoch key" stop growing without bound.** Raised as a
  scaling worry and it was a fair one, though the proposed fix — re-encrypting old content
  under a new key — is the one thing this design must never do: a per-object DEK is fixed
  for the object's lifetime (Storage §1.2), which is what makes chunk encryption
  deterministic, which is what makes delta-fetch work. Re-encrypting re-chunks everything
  and destroys the 1,556-of-176,115 property the segment model exists for.

  What gets refreshed is the **wrapping** — a 48-byte record, one AEAD seal — which Storage
  §5.3 already specifies and `design/05` §4 already lists as daemon maintenance nobody had
  written. `design/01` §8's retention is the same lever inverted: content that stops being
  re-wrapped goes dark on its own.

  **Measured before changing anything**, at ~720ns per unwrap attempt: 0.72ms to scan a
  thousand keys, 3.6ms for five thousand. Survivable per unwrap; the real cost was that
  `absorb_segments` called `epoch_keys()` *inside* a channels × members loop, and that reads
  and AEAD-opens every stored key from disk — a thousand directory scans per two-second
  tick on a network that had rotated a thousand times, dwarfing the crypto it was there to
  serve. A wrapping that opened under no held key paid the full scan forever, every tick,
  never converging.

  Three changes, all correctness-preserving: keys come back **current-first**, so a
  refreshed wrapping opens on the first attempt; a wrapping opened under an older key is
  **re-wrapped under the current one**; and a DEK learned from somebody else's wrapping is
  **remembered**, so the scan happens once per object rather than every tick. Foreign DEKs
  are checked against the pointer's own commitment, so a stale cache — the author sealed
  that object and started another — is discarded rather than used to fail at decryption.

  **Retiring old keys is deliberately not done here.** Dropping a key makes anything still
  wrapped under it unreadable forever, so it is `design/01` §8's retention decision rather
  than a cleanup to do quietly. Next. 77 → 78 tests.
- **2026-08-19** — **`kols revoke` closes the revocation path, and found two bugs doing it.**
  The command writes the membership removal; the daemon rotates the epoch to exclude them.
  The split is not convenience — rotating needs the live MLS group only the daemon holds, and
  Core §3.3 requires the removal to be logged *first* anyway, because a rotation minted while
  somebody is still a member produces a key they remain entitled to and §3.1 says a key
  cannot be un-known afterwards. `revoke` says so in its own output rather than implying the
  job is finished when it returns.

  **The first working revocation deleted a channel**, and the cause is worth keeping. The
  store has two writers — one-shot commands and the daemon — and each parented its entry on
  the head *it* last saw. `channel create` and the daemon's admission rotation landed on the
  same parent, forking the log; fork-choice picked the rotation branch and voided the
  channel. That is the protocol working exactly as specified, and a disaster in a client that
  never noticed. There is now an append lock (an atomic `create_dir`), held across
  read-head-then-write by every writer, and the daemon adopts whatever the store gained
  *inside* the lock before appending so its parent is the real head.

  **The second bug only a rotation could reveal**: reading a DEK unwrapped it with the
  *current* epoch key, but a wrapping is made under whichever epoch was current when it was
  written. The moment anything rotated, a node could not open its own content — Alice could
  not post to her own channel after removing somebody. It now tries every held key and
  re-wraps under the current one, which is what Storage §5.3 means by any current member
  re-wrapping on rotation.

  Both were reachable only once rotation existed, which is why they surfaced today rather
  than when the daemon was written. 75 → 77 tests.
- **2026-08-19** — **MLS group state survives a restart, and the spec says it must.** The
  last confidentiality gap, and it was a gap in the *specification* as much as in the code:
  §3 asked members to rotate, welcome and revoke without ever saying that the state those
  need has to outlive the process holding it. An implementation reading §3 alone keeps it in
  memory, which is what every MLS library makes easy, and the result is survivable in a test
  and not in a deployment. **Core §3.3.1** now states the obligation and its three
  properties: a restored member derives the same key, can still advance the epoch, and fails
  rather than half-loading.

  `GroupSession::save`/`restore` in `intranet-epoch`, `save_epoch_group`/`restore_epoch_group`
  on `MemberNode`, and `kols serve` restoring its group on startup and re-saving whenever the
  group advances. A founder who restarts can now key somebody in — before this, they kept
  their epoch keys, could still read, and could never welcome anybody again, with no symptom
  until the next person tried to join.

  Two implementation notes worth keeping. openmls gates `MemoryStorage::serialize` behind its
  `test-utils` feature and says so in a comment, so the save path reads the storage's public
  `values` map directly rather than turning a test-only feature on in a shipping build. And
  the signature key pair goes *into* that storage via `SignatureKeyPair::store` rather than
  travelling as its own field, because openmls's accessor for the private half is likewise
  test-only — `restore` reads it back out with `read`.

  **The blob is secret** — the group's secret tree and the member's signature private key,
  jointly enough to impersonate them and read the network — so it is sealed at rest under the
  same seed-derived key as the epoch keys, and the spec section says plainly that a client
  writing it in the clear has given away the network. Six conformance tests upstream, and one
  in the client that restarts a founder and has them key in somebody new who then reads
  pre-restart content. Protocol 626 → 632.
- **2026-08-19** — **Two nodes hold a conversation.** `kols serve` is a real node: it
  listens, syncs the governance log, advertises capacity, publishes this member's segments,
  pulls other members' pointers, fetches their segments and stores the records. `attach`
  bootstraps a store from nothing but a network id, and `admit` is the explicit-intake half
  of `design/02` §6.2. A joiner now goes from knowing one hex string to reading messages
  written before they existed, with nothing shared out of band.

  **The daemon owns the MLS group, and that is why it exists.** `GroupSession` keeps live
  state in an in-memory provider, so keying somebody in needs a process that has not exited
  since the group was made — which a one-shot command can never be. `init` therefore no
  longer creates the group; the founder's first `serve` does. The cost is stated in `init`'s
  own output: serve before posting.

  **Four bugs, each invisible to every existing test**, which is the argument for testing
  the binaries rather than the libraries:

  - A joiner could not advertise capacity before syncing, and could not sync without
    advertising. Startup work is best-effort now and retried after every governance sync,
    because "not a member yet" is a state to sync out of rather than fail on.
  - A fetch was requested once. It needs **two rounds** — the manifest, then the chunks it
    names — so remembering "already asked" left every segment permanently half-fetched.
  - Only the newest epoch key was kept. Adding a member *rotates* the epoch, so content
    written before a joiner arrived is wrapped under an earlier key: it fetched perfectly
    and decrypted never. The store now holds the whole keyring, and unwrapping tries each.
  - `read` on a fetched segment must take the DEK from the pointer's **wrapping**, not from
    the local store — the local path mints a DEK for objects this node owns and would have
    produced a key that opens nothing.

  **One design consequence worth knowing before it surprises somebody:** `post` now requires
  `serve` to have run, because only the daemon can mint the first epoch key. Every test that
  posts starts a node first, which is what a user does anyway.

  **A fifth bug, found by fixing the fourth's leftovers**, and it is the one worth
  remembering: the daemon re-asked for the governance log and for pointers on every tick,
  but never for the **capability ledger**. Source selection drops a holder that has not
  advertised capacity, as not having volunteered — and a joiner advertises only once
  admitted, which is *after* the ledger exchange that ran when it connected. So a joiner
  stayed permanently unrankable as a source: its pointer arrived, its DEK wrapping arrived,
  and the chunk itself never did. The crate's own guidance says a fetch that mysteriously
  finds nothing is usually exactly this, and it was. The tick now re-asks for all three.

  With that, a reply travels between two live daemons with nothing restarted, which the
  second two-node test now asserts. 72 → 74 tests.
- **2026-08-19** — **Content keying moved onto the real path.** Raised as a concern, and it
  was a fair one: the CLI's first cut derived its DEK from the network id, which is public
  in every meaningful sense — it is in every invite, every address and every log entry — so
  the only thing keeping a non-member from reading a segment they had obtained was honest
  nodes declining to serve them. A serving policy standing in for cryptography.

  It now does what Storage §5 specifies. Every author log gets a **random** DEK; what
  persists is the DEK **wrapped under the network's epoch key**, which is exported from a
  real `GroupSession` created at `init` rather than derived from anything public. The epoch
  key is sealed at rest under a key derived from the master seed, because
  `EpochKey::expose_for_delivery` states outright that storing it unsealed defeats the
  guarantee the module exists to provide. Secrets are written `0600` and a test asserts it.

  **What is still missing is rotation, and it is a real gap rather than a rounding error.**
  Core §3.3 advances the epoch on every membership change, and that is what stops a removed
  member reading anything published afterwards. Advancing it needs live MLS state, and
  `GroupSession` holds an in-memory openmls provider with no persistence — so a process that
  exits cannot rotate. A removed member keeps the key, which is precisely the naive scheme
  Core §3.2 rejects. Two ways out, both real: `intranet-epoch` growing a persistent storage
  provider, or a long-running node here holding the group in memory. The second is the same
  daemon the wire half needs, so they likely land together.

  Three tests pin what changed: two networks share no key material and no DEK; secrets are
  unreadable to other users; and a store whose epoch key has gone refuses rather than
  minting a fresh one, since silently re-keying would produce a node writing content nobody
  else can read while looking like it works. 69 → 72.
- **2026-08-19** — **`kols` exists, and the project is runnable for the first time.**
  Chosen over E4 deliberately: gossip is an optimization of a path that already works
  (`design/01` §7 — "a client with gossip entirely disabled is slower and completely
  correct"), while nothing had ever composed genesis, permission resolution, channel
  entries, author logs and rendering into one path. That seam had a bug in it the same day
  it was written, which is the argument in miniature.

  `init`, `whoami`, `channel create`, `channel list`, `post`, `read` — persisted between
  invocations, replaying a real governance log each time rather than caching state.
  Seven tests drive the actual binary rather than the library, because what is under test
  is that separate processes agree through the store, which an in-process test would
  share its way past.

  Three things worth carrying:

  - **Genesis has three requirements and each is silent when missed.** `chat-log` must be
    on the content-type allowlist, the chat vocabulary must be registered, and `everyone`
    needs `publish:chat-log` alongside `chat:post`. Miss any one and the network looks fine
    until the first post is refused by the author's own node. `kols-cli::network::genesis`
    is now the one place that gets it right.
  - **The CLI's DEK started as a stand-in and was replaced the same day.** The first cut
    derived it from the network id — which travels in every invite, address and log entry —
    so anyone who ever saw the id could decrypt any segment they obtained. It now does what
    Storage §5 actually specifies: a random DEK per author log, **wrapped under an epoch key
    exported from a real MLS group**, with only the wrapping persisted and the epoch key
    itself sealed at rest under a seed-derived key. `EpochKey::expose_for_delivery` says
    plainly that storing it unsealed defeats the guarantee, so it is not stored unsealed.
  - **A network name is not a policy value.** Spec 07 defines no key for one, so the CLI
    keeps a local label rather than inventing vocabulary the normative document lacks —
    which is how two clients end up disagreeing about what a network is called.

  **What it cannot do: reach another node.** `kols-net` publishes and fetches between two
  live nodes in tests, and nothing in the binary drives it. That is the next task, not a
  gap being glossed. 62 → 69 tests.
- **2026-08-19** — **Chat channel entries landed, and spec 07 gained the bytes they
  needed.** E2 put channel structure in one generic application entry rather than four
  chat-shaped variants; `kols-core::channel` is the chat side of that. All four kinds —
  definition, update, membership, rotation — encode as `chat`-namespace payloads under a
  new `intranet.chat-channel-entry.v1` tag, with the header mirroring a record's so the
  reasoning transfers: no `network_id`, because the channel id already derives from it.

  **A contradiction in the normative spec had to be settled first, and the user chose.**
  §1.3 said a channel definition must declare `chat:manage-channel`, while §4.1 listed
  `chat:create-channel` as Ordinary and then never used it anywhere. Resolved the way
  `design/02` §2.2 always had it: **creating is ordinary, changing is governance**, because
  a definition grants nobody access to anything — a new private channel has an empty roster
  until a membership entry adds someone, and that entry is the governance-tier one. The tier
  follows what an action can widen, not how consequential it sounds. Spec 07 §1.3 corrected,
  §3.8 written to carry the mapping normatively.

  **Both obligations E2 moved onto readers are now unavoidable rather than optional.**
  `ChannelEntry::read` is the only way to get an entry out of a log body, and it runs the
  capability check and the profile check on the way through; decoding bytes directly is
  still possible but is named `decode_payload` and yields only a value. The writing side
  declares its capability *from the entry's own kind*, so a client cannot publish one
  declaring something else — the same structural move as `authorize` in the protocol's
  media limiter, and for the same reason: a check a caller can route around eventually is.

  One decision worth keeping: an unallocated discriminant in a channel entry is **refused**,
  the opposite of the rule for record kinds. An unknown record is retained, counted and not
  rendered (`design/08` §9); an unknown channel entry carries structure, and a reader that
  skipped it would hold different channel state from one that understood it.

  Frozen vectors included, since §3.8 is normative now and a change to one is a wire break
  rather than a value to re-bless.

  **A bug found immediately afterwards, by wiring it to a real governance log.** The entry
  declared a *fixed* capability per kind, which made channel creation work for Founders and
  nobody else: an extension capability resolves by exact name, so a member holding
  `chat:create-channel:*` — the grant `capabilities::network_scoped()` exists to register,
  and the whole reason the verb is Ordinary — was refused, because the entry declared a
  channel-scoped name they did not hold and which nobody *could* have registered in advance,
  the channel id not existing until the entry creating it does. The declaration is now
  chosen from what the author actually holds, narrowest first, and refuses up front when
  they hold nothing rather than producing a log entry every node rejects.

  The lesson is worth more than the fix: **what an entry declares has to be a name its
  author was really granted, not the one that best describes the action.** The encoding
  tests could not have caught it — they build values by hand, which is right for a wire
  contract and blind to whether the protocol accepts what this client produces.
  `channel_governance.rs` closes that, replaying a real log end to end. 37 → 62 in this
  repo.
- **2026-08-19** — **A relay can now refuse, and its advertisement binds.** Found by asking
  what actually stops a volunteer being asked for more than it offered: nothing did. A
  node's `bandwidth_cap` was read by every node *except* the one that declared it — it
  steered other members' relay and source selection while the volunteer enforced nothing,
  so a user could set a limit, watch the client ignore it, and have no way to tell. That was
  survivable while a relay forwarded one envelope per envelope received. Fan-out made it a
  multiplier, which is why this follows E5 rather than standing alone.

  Specified in Real-Time §2.2.2 as a requirement to *have* ceilings without prescribing
  values, and implemented as `intranet-transport::media_limits`: concurrent calls,
  participants per call, and sustained bytes forwarded — charged for what **leaves** the
  node, since charging the inbound size under-meters by exactly the fan-out factor the
  ceiling exists to bound. Kept out of network policy deliberately: a ceiling describes one
  node's hardware, so a network able to set it could compel a member to spend bandwidth it
  never offered, which inverts Core §4.3's opt-in.

  Two things the shape had to get right. `MediaRelayGuard` owns the participant sets, so
  `authorize` is the only way to learn a frame's recipients and it charges in the same call
  — the same structural answer `relay_limits` uses against a limiter that computes a verdict
  and never enforces it. And a fan-out that does not fit is refused **whole**, because
  serving some participants and not others turns a bandwidth ceiling into silent one-sided
  call degradation, which is worse to experience and harder to diagnose than a refusal.

  Thirteen tests: eleven unit over the charging arithmetic, refill curve, a backwards clock
  and a carrier that is also a participant, plus two over live nodes. `relay_call` now
  returns `Result`, and refusing is ordinary — the call renegotiates onto another relay
  exactly as it would if this one went offline, so the client shows nothing.

  One bug caught in this change's own code, by writing the test to exercise the real path
  rather than the convenient one: changing a node's limits replaced the guard wholesale and
  silently dropped every call it was carrying, which would have hung up on everyone with no
  explanation at the far end. Limits now change in place — a lowered ceiling stops this node
  taking new work without retracting agreement already given, and a raised one grants no
  allowance, since allowance is earned from elapsed time and a config change that minted it
  would be a rate limit anyone could step around by toggling a setting.
- **2026-08-19** — **E5 landed, pulled forward from P3.** The relay reduced nobody's upload:
  `MediaEnvelope` carried one `to`, so a sender in an n-party relayed call emitted n−1
  envelopes and the relay added a hop to each. It now carries `Recipient::{One,
  Participants}` — one envelope in, n−1 out — specified in Real-Time §2.2.1 with the
  implementation, ten live-node tests and four encoding tests. Three things the proposal
  had not worked out, each found while building it: the fan-out form deliberately carries
  **no recipient list**, so it is stricter than the form it replaces rather than looser;
  the relay must **readdress every forwarded copy**, or a participant that also relays
  would fan the same envelope out again; and the relay must **bind the claimed sender to
  the connection**, a check that never existed on this path and that fan-out turns from one
  stray frame into n−1 sends at the relay's expense. The wire break is versioned rather
  than smuggled — the envelope's domain tag is now `intranet.wire.call-media.v2`, so a v1
  envelope fails to decode instead of parsing its recipient out of the wrong bytes.

  The gotcha `design/05` §4 carries about `next_swarm_event` draining its buffer on entry
  turned out to be live rather than theoretical, and caught this change on the way through:
  a relay that is itself a participant buffers its own copy of a frame, and in a call whose
  only other participant is the sender there is nothing to forward alongside it — so the
  buffered event waited for unrelated traffic that on a quiet call never comes. It is
  returned directly in that case, with a test whose deadline is deliberately tight, since a
  generous one passes under both behaviours and pins nothing.

  The client's advice to stay in mesh (`design/04` §3.1) is withdrawn.
- **2026-08-19** — **Protocol repo test count refreshed.** Its README claimed 591 in two
  places, which was true when written and went stale when E9 and E2 landed — each added
  conformance tests and neither updated the figure. Measured rather than inferred this
  time — **613 passing, clippy clean** — and worth recording how, because the first two
  attempts disagreed: `cargo test --workspace` piped into `grep` **undercounts**,
  because cargo interleaves the output of concurrently running suites and a result line
  that lands mid-line no longer matches `^test result` — two piped runs of identical source
  reported 571 and 612. Redirect to a file and count there. The tree at `HEAD` was 604, so
  591 was thirteen tests stale, which is about what E9 and E2 added between them.
- **2026-08-19** — **E2 landed, but generically.** Four chat-shaped entry variants would have
  repeated the mistake E9 avoided, so the log gained one application entry instead:
  namespace, kind, declared capability, opaque payload. Two claims weakened honestly as a
  result — the protocol enforces the capability an entry *declares* rather than the right
  one, and rejecting channel entries in a conversation network is now every conformant
  reader's job rather than the protocol's. Application entries also do **not** count toward
  branch length, reversing this design's earlier reasoning, because the metric cannot
  resolve a capability's tier. A variant-discriminant collision found on the way is now
  guarded by a round-trip test over every entry body, including distinct action hashes.
- **2026-08-19** — **E9 landed in the protocol repo.** `NetworkPolicy` now carries namespaced
  application-layer values it stores, orders, encodes and hash-covers without interpreting —
  the same division `extension_capabilities` already used. Specified in Core §2.6.2, six
  conformance tests, both repos' gates green. `kols-core::policy` reads the chat settings
  out of it, including the network profile, with defaults applying when a key is absent.
- **2026-08-19** — **Conformance obligation 6 met.** The encoding now runs on a big-endian
  target: `scripts/cross-check.sh` cross-compiles `kols-core` to s390x and runs the suite
  under qemu, where all 30 tests pass including the frozen vectors. Byte order was correct
  by construction before this — no native-endian conversion, no transmute, no unsafe
  anywhere in the path — but that was an argument, and this replaces it with a run.
- **2026-08-19** — **F3 done: design set reviewed and bumped to v1.0.** The pass earned its
  keep: `08` stopped being normative, since its content moved upstream in F2 and two
  normative descriptions of one wire format is the drift this project criticises elsewhere.
  It also caught the set claiming no implementation existed, a roadmap listing P0 as future
  work, a scale section calling two now-measured numbers guesses, and a test plan with no
  column for what had actually been written. Four new decisions recorded (D22–D24 plus the
  precedence rule).
- **2026-08-19** — **F2 done: spec 07 written and committed upstream.** The first
  application-layer spec, carrying the channel and record model, the normative encoding,
  the capability vocabulary, the keying tiers, and §7's five platform amendments. Written
  from a working implementation rather than ahead of one, so its two most consequential
  rules — the count-free record list and per-device HLC strictness — arrive with the
  measurements that produced them. Protocol repo stays green: 591 tests, clippy clean.
- **2026-08-19** — **P0 closed: the wire half works.** `kols-net` publishes a segment
  (store, announce per chunk, accept pointer) and reassembles one from fetched chunks. Two
  live `MemberNode`s: 120 messages cross the wire and render identically on both sides, and
  a reader holding the previous version refetches **1 chunk of 3** after an append. Three
  documented gotchas were all real — Kademlia client mode had to be turned off, ledger
  advertisement had to precede the fetch, and the manifest needs its own round before the
  chunks it names.
- **2026-08-19** — **P0 criterion 5 met.** `AuthorLog::rebase` implements the content-merge
  semantics Storage §2.2 deliberately left to the application layer: union by record id,
  republish at the next version, nothing dropped. Two findings recorded in the design —
  HLC strictness is per **(author, device)** rather than per author, since two devices
  cannot coordinate a shared counter without a lock; and rebase cost depends on where the
  loser's records sort, cheap when they follow the winner's and expensive when they
  interleave.
- **2026-08-19** — **P0 criteria 1, 3 and 4 met at the merge layer.** `ChannelView` renders
  as a pure function of the admitted record set: 40 permutations, reversal, duplicate
  delivery and a two-sided partition heal all converge on identical output. Reader-side
  refusal covers non-members, forged signatures and wrong-channel records. Honest scope:
  this proves the *merge*, not the wire — `kols-net` still owes the byte-transfer half.
- **2026-08-19** — **E11 found:** the extension-capability registry matches names exactly,
  but every chat permission is parametrized by scope, so each scope would need its own
  policy entry. Namespace registration proposed in `design/06` §11; `kols-core::capabilities`
  carries the workaround meanwhile. `AuthorLog` publishes
  segments through `intranet-storage`; the byte-level assertion showed 51,405 of 176,123
  bytes moving per appended message. Cause: `Enc::seq`'s count prefix sits at the head of
  the encoding, so every append changed the first chunk. Removing the count — the record
  list runs to end of input, each record already length-prefixed — brings it to **1,556 of
  176,115, one new chunk of eight**. `design/08` §6 and `design/01` §3.1 updated.

- **2026-08-19** — `kols-core` encoding implemented against `design/08`: records, segments,
  HLC, derived identifiers. 13 tests green (round-trip, injectivity, domain separation, id
  stability, rate-class, bounds), clippy clean. Test vectors frozen — a change to one is a
  wire break, not a value to re-bless.
- **2026-08-19** — E3 withdrawn: deriving pointer ids needs no protocol change, so **P0
  requires no change to `distributed-intranet` at all**. That repo remains untouched.
- **2026-08-19** — S1: client repo created, design docs moved in, `kols-core` scaffolded.
- **2026-08-19** — F1 done: canonical record encoding pinned in `design/08-record-encoding.md`.
- **2026-08-19** — Design set `00`–`07` drafted and reviewed across four passes; 21 decisions recorded.
