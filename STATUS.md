# ko-ls — Status

**Updated:** 2026-09-08
**Phase:** P1 — two nodes talk live and durably, a joiner reads back through sealed history,
and the boundary carries commands in and events out.

**This file is a map, and holds nothing that lives anywhere else.** It says where the work
stands and which document owns each part of it. Anything long-lived — how a mechanism works,
why it was chosen, what a limit costs — belongs in a design document or a specification, and
this file points at it rather than restating it. Two copies of a fact drift in one of them,
and this file is the one that gets believed.

It is updated in the same change that moves work, never afterwards from memory.

---

## 0. Start Here

A Discord-shaped chat client on the Distributed Intranet protocol. A "server" is a network:
its own governance log, membership, epoch key chain and DHT namespace.

Three repositories, side by side, all on `main` and pushed:

| Repo | Remote | What it is |
|---|---|---|
| `ko-ls` (this one) | `DriftingNarwhal/ko-ls` | The client. AGPL-3.0-only |
| `../distributed-intranet` | `DriftingNarwhal/distributed-intranet` | Protocol and specs. MPL-2.0, specs CC BY 4.0 |
| `../DI-Relay` | `DriftingNarwhal/DI-Relay` | The bootstrap relay of Core §5.5, deployed and operational. AGPL-3.0-only |

The client builds against `../distributed-intranet` by **path dependency** while the
extensions still move, so a fresh machine needs those two cloned as siblings.
`.devcontainer/` lives in *this* repo and builds both, plus the Tauri toolchain — open the
`ko-ls` folder in it, not the parent.

### Which document owns what

Read this table before reaching for a file. **`distributed-intranet/specs/07` is normative**
where it and the design set overlap; the design set owns client design, rationale and
sequencing.

| If you need | Read |
|---|---|
| Why anything is the way it is; the decision register D1–D38; the roadmap | [`design/00`](design/00-overview.md) |
| Channels, records, ordering, edits, retention, the live path, abuse limits | [`design/01`](design/01-messaging-model.md) |
| Roles, capabilities, permission resolution, onboarding, seeds and backup | [`design/02`](design/02-membership-and-permissions.md) |
| Keying tiers, private channels, direct messages, search leakage | [`design/03`](design/03-confidentiality.md) |
| Voice, video, stage, media transport | [`design/04`](design/04-realtime.md) |
| Crate layout, the `kols-api` boundary, the node loop, local state, testing | [`design/05`](design/05-client-architecture.md) |
| Every change the protocol still owes, with acceptance criteria — E1–E16 | [`design/06`](design/06-protocol-extensions.md) |
| How the project got from a finished design to first code — F/S items, P0 | [`design/07`](design/07-build-plan.md) |
| Encoding conformance obligations and the `kols-core` module map | [`design/08`](design/08-record-encoding.md) |
| Navigation, liveness tiers, presence honesty, settings, theming | [`design/09`](design/09-interface.md) |
| The normative wire contract — encoding, entry payloads, capabilities, limits | [`specs/07`](../distributed-intranet/specs/07-chat-application-spec.md) |
| How to build and test, and the traps that make a red suite lie | [`CONTRIBUTING.md`](CONTRIBUTING.md) |
| Running it across two machines | [`docs/two-machine-test.md`](docs/two-machine-test.md) |
| Why a thing was done the way it was — the reasoning archive | [`docs/log.md`](docs/log.md) |

---

## 1. Right Now

| | |
|---|---|
| **Milestone** | A client that can be handed to somebody else, so two people **on entirely separate networks** can talk, using a bootstrap relay and no VPS. The first test is two of the user's own laptops, one on a mobile hotspot |
| **Blocked on** | Nothing |
| **Next decision needed** | Nothing blocking |

Where that milestone stands:

| | |
|---|---|
| Relay reachability — a network designates relays, a node reserves a circuit, invites carry it | **done** |
| One string to join | **done** |
| A window that creates, joins, opens and runs a node for a network | **done** |
| Minting an invite from the window, with the waiting room and admitting beside it | **done** — no step of the flow needs a terminal |
| Windows and macOS builds | **done**, in CI, and both have now been run in the field across several rounds of testing. `kols-desktop.exe` opens no console behind the window — the attribute that does that had to land with `kols_node::Report`, since a GUI-subsystem process has no stdout and Rust *panics* on the write rather than dropping it |
| Two nodes meeting through the deployed relay | **Done, on one LAN.** Both ends ran the window, connected and reconnected several times, messages crossed both ways, and an established connection survived the relay going down |
| Content outliving the node that wrote it | **done** — a member reads what an offline member wrote, because a third kept it, and a restart no longer discards what a node kept (`three_nodes.rs`). Before this a node was a member of the storage swarm only until its process ended |
| **Two nodes on separate networks** | **Done — this milestone's stated first test passes.** Two of the user's own laptops, one on a mobile hotspot, and then a third person on a third network: connection worked across all three, survived close and reopen, reconnected, and roles, permissions and every chat function (posting, voting, withdraw, edit, channel creation by an invited member) worked. The one defect that test found — a node losing its whole servable contribution on restart — is the row above |
| Invites short enough to send somebody | **done** — one real machine's invite went from ~4,750 characters to ~1,324, about half from carrying only the addresses a recipient could dial and about half from an encoding that stops repeating the peer id once per address (`design/02` §6.1, Core §5.6) |
| The window hearing what the node tells it | **done, and it never had.** The application declared no Tauri capabilities, so its ACL was empty and every `plugin:` command was refused — `listen` included. No node event had ever reached the window for the life of the client; three polls had been written as fixes for what was one denial, and the features with no poll behind them read as unbuilt (`design/05` §1) |
| Dragging a channel where you meant | **done** — a drag needs a target for every destination rather than for every thing, and two of four had none: the end of a list, and the top level once every channel was in a folder. Rows are two targets split at the midpoint, and the sidebar's empty space is the way back out |
| Reordering channels at all | **done, and it never had been.** Drag was the only route and never once started: Tauri installs a native drag handler on the webview by default and it takes the drag before the page sees it, so for as long as folders have existed no channel could be reordered. `dragDropEnabled: false` is the fix, asserted by a test; the menu keeps move up and move down beside it |
| Closing the window without losing anything | **done** — there was no shutdown path at all, and the risk was not the one expected: every durable write went straight at its destination, so a process ending mid-write could leave a governance log that no longer decoded and a network that would not open again. Writes are atomic now, and the node is stopped on exit so its claim and its relay slot are released rather than left to expire (`design/05` §1.1) |
| Coming back after a laptop sleeps | **done** — the re-dial loop asked whether *anything* was connected, and a relay is a connection, so a node holding a reservation never re-dialled a lost peer. Sleep, wake, and the only way back was restarting the application |
| A network's name reaching the people in it | **done** — `genesis` never wrote `chat:network-name`, so a founder's name lived on their machine alone and every joiner saw an id. Networks created before this need it set once under settings → network |
| Leaving a network, and the network hearing about it | **done** — and it had never been possible in either half. The protocol had no entry a departing member was allowed to write (Core §2.5.1 now does), and the client refused every self-removal outright on a concern that is true only of the last `revoke-node` holder. `forget` announces before it deletes, and reports how many members were connected when it went out rather than claiming they received it (`design/02` §6.5) |
| A node that holds content for the network rather than only for itself | **done, and it never had.** Placement was computed nowhere and `replication_factor` read nowhere: every byte on disk was what this node had fetched to read, so content outlived its author because somebody happened to have opened that channel. Durability was a happy accident rather than a property (`design/05` §5.1) |
| A disk that does not fill up | **done** — an installation-wide ceiling, with the fetch bounded by it, cached copies shed before replicas, replicas given back only when two others demonstrably hold them, and a last known copy held past the ceiling for a week with a warning before it goes. What is given up is recorded |
| Saying what a contribution is, and is not | **done** — storage, upload, download and relay willingness are all settable per network, relaying offered only where this node has been seen from outside. Contributing nothing is an ordinary configuration and the interface says so, because a phone, a metered link and a full disk are all reasons and none makes somebody a lesser member |
| A member who offers a lot actually catching what falls | **done** — repair, the other half of Storage §3.4. A node ranked past the replica set takes on content the network is short of, and how deep that reaches follows the size of the hole rather than who noticed: one missing copy wakes one standby, three wake three. Before this, offering a great deal of disk caught falling content only where placement had already ranked you — a backstop by coincidence. The offer buys **ranking rather than a crawl** (D38), so it is a ceiling on willingness and not a target: a 50 GB offer on a network holding 2 GB holds 2 GB |
| Knowing who is around | **done** — presence, `design/01` §9's ephemeral gossip: a signed beat every thirty seconds under the network's epoch key, and a roster carrying two marks that may never stand in for each other — a ring for *this node has a connection to them*, a word for *what they said*. **No word is silence rather than "offline"**, because a stale beat, a member who chose to be invisible and one never heard from are three different things and nothing here tells them apart. Invisible publishes nothing at all, not the word "invisible" |
| An interface that survives being used | **done** — the first field test's list, worked through: first-sight marks on messages that land mid-timeline, the roster as a counted dropdown at the top right, the door as a sheet behind a counted button, and settings as a screen rather than a sheet over a dimmed channel (`design/09` §4.1–§4.3) |

**Runnable.** `kols-desktop` is the product (`design/00` D30); `kols` is a development tool
over the same `kols-api` boundary, owed no feature parity and no end-user documentation.

- **`kols-desktop`** — creates or joins a network, runs a node for it, generates a relay
  identity and designates relays, lists and renders channels in the order the network agrees
  on, posts, reacts, revises, withdraws and pins, manages channels and folders, mints an
  invite and admits from the waiting room, and says so when a healed fork undid something.
  Settings is a screen of five sections split by what a click costs (`design/09` §4.2), and carries the
  network's name (D32), a **role-first permissions surface** — roles, what each holds and at
  which scope, and who is in them — and the network's own policy: admission mode, the abuse
  limits of spec 07 §4.3 and the two retention windows of §2.8.
- **`kols`** — **the multi-process test harness, and not a product surface** (D30). Six test
  files drive `CARGO_BIN_EXE_kols`, and separate processes are the whole point: separate stores
  and swarms, a node that can be killed and restarted, and the single-node claim exercised for
  real. **Nothing in the shipped application invokes it** — the window runs
  `kols_node::serve::serve` in-process. Its output is therefore a test contract rather than an
  interface, and carries no end-user prose. It covers init, relay list/set, invite, join,
  waiting, attach, admit, revoke, leave, name, serve, post, read, edit, delete, react, pin,
  contribute, presence, storage, history, and channel
  create/list/rename/topic/slowmode/archive.

**Gates green as of this date:** 332 tests here, 672 in `../distributed-intranet`, clippy
clean in both, and `crates/kols-ui/drive.mjs`'s 78 checks green by hand. O20 reproduced once
across four full-width runs on 2026-08-31 and 2026-09-01, which makes it **intermittent rather
than deterministic** — `CONTRIBUTING.md` said it failed on every full-workspace run, and that
was a run of bad luck rather than a property. The one failure arrived directly after a
governance change and was attributed by measurement rather than by reading; how, and why the
comparison is worth running even when you are sure, is in `CONTRIBUTING.md`.

**`v0.11.1` is the release the rows above describe**, cut 2026-09-01. It carries everything
from 2026-08-30 — the event path, drag, shutdown, re-dial after sleep, the network's name —
none of which was in a published release until now: `v0.11.0` was eight days and forty
commits behind, while `README.md` and `docs/two-machine-test.md` both send a new user to
Releases. That gap is the thing to watch rather than the version number; a distributable that
trails `main` by a week of fixes is indistinguishable, from the far end, from fixes that were
never made.

---

## 2. What Is Owed

Debts this client took on deliberately. **Each is specified where it was incurred** — this
table is an index, and the right-hand column is where the substance lives.

Nothing here blocks anything else. Where an item exists because something it depends on does
not, the dependency is named in the owning document.

| # | Owed | Specified in |
|---|---|---|
| O1 | Commands for direct messages, search, voice and stage — each has a line in `design/05` §3's boundary *grammar* and nothing in `kols-api`. **`SetContribution` and `SetPresence` are built** (25 commands now); the rest wait on E10/E13, `03` §6's indexes, and `kols-media` | `design/05` §3, `design/00` §5 |
| O2 | `Discovery::Off` for conversation-profile networks. **Load-bearing for privacy rather than merely leaner**: with discovery on, a DM node meeting a peer at the shared network's relay lands in its routing table, which is the correlation D29 forbids | `design/06` §12, `design/09` §3 |
| O4 | `kols-store` does not exist; `kols-node` carries a file-backed store instead of the SQLite projection | `design/05` §2, §5 |
| O5 | The executor rebuilds an author's whole log to append one record, and replay walks the log once per question | `design/05` §5 |
| O7 | **No credentials and no backup.** Seeds are written to `<home>/seed` in the clear, so anything with read access to that disk is that member. **A release gate, not a feature** | `design/02` §6.3, `design/00` §5 |
| O11 | A relay may not be shared between two of a member's networks, and **nothing enforces it**. Enforcing it means network-scoping the protocol names, which is a wire change rather than a client fix | `design/00` D29, `design/09` §3 |
| O15 | **Provider discovery through a peer that is not the holder has never been observed.** Narrower than this entry used to claim: `three_nodes.rs` does prove a node serves an object it did not author, with the fetcher pointed at one peer and the author offline. But the fetcher is *connected* to the holder there, so a one-hop table and a working DHT behave identically — forcing the two apart is the remaining test, and needs the Docker NAT matrix rather than local daemons | `design/05` §8 |
| O20 | **The daemon suite run starved is unreliable**, and `CONTRIBUTING.md` asks for exactly that run. One or two of eleven time out in `wait_for` under `taskset -c 0,1`; each passes alone. Measured at `main` on 2026-08-29, so it is the suite rather than any change — but it makes the starved run a signal to isolate rather than a gate, which is weaker than what it was added for | `CONTRIBUTING.md`, `tests/common::patience` |

O3, O6, O8, O9, O10, O12, O13, O14, O17, O18, O21, O22, O23 and O24 are closed. What each was, and what closing it turned up,
is in [`docs/log.md`](docs/log.md). The numbers are retired rather than reused, so the log
stays readable.

**O16, O19 and O25 are accepted rather than closed, which is a different thing and is why they
are named separately.** None was fixed; all were decided against — O16 and O19 on 2026-09-07,
O25 on 2026-09-08 — and an accepted limit left in the table above would read as a fix nobody
had got round to.

| # | Accepted limit | Decided in |
|---|---|---|
| O16 | **This client does not dial a LAN peer that mDNS finds.** A node that did would make two of a member's networks correlatable by anyone watching that LAN — D29 one layer down, reached with no relay involved. The cost is that two members in one room still need a routable third party to meet, which is narrow and is the price of the property | `design/00` §6 |
| O19 | **A role cannot be deleted.** `EntryBody` expresses no group removal, so a role can be emptied of capabilities and members and its name stays in replayed history. A role holding nothing grants nothing, and no protocol change is being asked for — the interface explains the limit instead | `design/05` §3 |
| O25 | **A node holds only what it can read**, so a storage offer funds durability for the channels its owner is keyed for and no others. Kept deliberately: the alternative is nodes hoarding ciphertext against keys they do not have, which gives up the forward secrecy `03` §3.1 chose MLS for — an epoch key compromised later cannot open bytes a node never kept. **It costs nothing today**, because roster keying is unbuilt and every member can decrypt every channel; `channel_dek` derives from the network epoch regardless of a channel's privacy flag. What it will cost when private channels land is written down as a requirement of that work rather than left to be discovered during it | `design/03` §3.5, `design/06` E2 |

---

## 3. What Exists

**Crates.** `design/05` §2 owns the layout and what each crate deliberately does not own.

| Crate | State |
|---|---|
| `kols-core` | Encoding, author logs, merge, collision recovery, chat policy, channel structure, `sidebar_order`, reader-side limits, and `Scope` — the one construction of a capability's name, used by the writer and the resolver alike. 126 tests |
| `kols-net` | Publish and fetch over a running node. Two live two-node tests |
| `kols-api` | The whole boundary — all three of `design/05` §3's properties held. 24 commands, 50 tests, and the consent drift test is guarded at both ends: a new command stops the suite compiling until it is sampled, which is how `LeaveNetwork` was caught unsampled the moment it existed |
| `kols-node` | `kols`, its node daemon, the executor, the store and the workspace — the window's entire backend, and the largest crate here at 121 tests. Fifteen of them run over a live wire between separate processes (`two_nodes`, `three_nodes`, `relay`); thirteen are in-process over roles and grants; the rest cover the workspace, the store, invites, names and records |
| `kols-app` | The Tauri shell, holding a workspace and an executor for whichever network is open. Builds `kols-desktop`. 8 tests, one of which resolves the webview's ACL against the real configuration — the boundary whose failure produces no output |
| `kols-ui` | The interface: HTML, CSS and one script, holding no keys, no sockets and no files |
| `kols-store`, `kols-media` | Not created. A crate is made when there is code for it — an empty one is a claim that something exists |

**Protocol extensions.** [`design/06`](design/06-protocol-extensions.md) §0 carries the table
and is the one place their state is kept. In summary: E1 and E3 withdrawn as unnecessary;
**E2, E4, E5, E9, E11, E12, E14 and E16 landed**; E7, E10 and E13 are P2, E6 is P3, E8 is P4,
and E15 is spec text that blocks nothing, deliberately sequenced to land beside the credentials
work it describes rather than on its own.

**P0 is closed** — all five criteria met, recorded in `design/07` §3. The measurements it
produced, which the whole segment model rests on, are in `design/08` §4.

---

## 4. Log

Moved to [`docs/log.md`](docs/log.md) — 121 entries, newest first.

What happened *lately* is §1. The log is why things are the way they are: the reasoning behind
a change, the thing tried and abandoned, the bug that turned out to be a different bug. It
lives outside this file because it is history rather than state, and 187 KB of history at the
bottom of a status file stops anybody reading the status.
