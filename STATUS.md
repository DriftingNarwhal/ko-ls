# ko-ls — Status

**Updated:** 2026-08-23
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

Two repositories, side by side, both on `main` and pushed:

| Repo | Remote | What it is |
|---|---|---|
| `ko-ls` (this one) | `DriftingNarwhal/ko-ls` | The client. AGPL-3.0-only |
| `../distributed-intranet` | `DriftingNarwhal/distributed-intranet` | Protocol and specs. MPL-2.0, specs CC BY 4.0 |

The client builds against the sibling checkout by **path dependency** while the extensions
still move, so a fresh machine needs both cloned as siblings. `.devcontainer/` lives in *this*
repo and builds both, plus the Tauri toolchain — open the `ko-ls` folder in it, not the parent.

### Which document owns what

Read this table before reaching for a file. **`distributed-intranet/specs/07` is normative**
where it and the design set overlap; the design set owns client design, rationale and
sequencing.

| If you need | Read |
|---|---|
| Why anything is the way it is; the decision register D1–D37; the roadmap | [`design/00`](design/00-overview.md) |
| Channels, records, ordering, edits, retention, the live path, abuse limits | [`design/01`](design/01-messaging-model.md) |
| Roles, capabilities, permission resolution, onboarding, seeds and backup | [`design/02`](design/02-membership-and-permissions.md) |
| Keying tiers, private channels, direct messages, search leakage | [`design/03`](design/03-confidentiality.md) |
| Voice, video, stage, media transport | [`design/04`](design/04-realtime.md) |
| Crate layout, the `kols-api` boundary, the node loop, local state, testing | [`design/05`](design/05-client-architecture.md) |
| Every change the protocol still owes, with acceptance criteria — E1–E15 | [`design/06`](design/06-protocol-extensions.md) |
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
| Windows and macOS builds | **done**, in CI. `kols.exe` runs; `kols-desktop.exe` is built and unrun |
| Two nodes meeting through the deployed relay | **Done, on one LAN.** Both ends ran the window, connected and reconnected several times, messages crossed both ways, and an established connection survived the relay going down |
| **Two nodes on separate networks** | **Unproven, and it is this milestone's own case.** A hole punch across one LAN is the easy version of the problem Core §5.5 exists for |

**Runnable.** `kols-desktop` is the product (`design/00` D30); `kols` is a development tool
over the same `kols-api` boundary, owed no feature parity and no end-user documentation.

- **`kols-desktop`** — creates or joins a network, runs a node for it, generates a relay
  identity and designates relays, lists and renders channels in the order the network agrees
  on, posts, reacts, revises, withdraws and pins, manages channels and folders, mints an
  invite and admits from the waiting room, and says so when a healed fork undid something.
- **`kols`** — init, relay list/set, invite, join, waiting, attach, admit, revoke, name,
  serve, post, read, edit, delete, react, pin, and channel
  create/list/rename/topic/slowmode/archive.

**Gates green as of this date:** 247 tests here, 655 in `../distributed-intranet`, clippy
clean in both.

---

## 2. What Is Owed

Debts this client took on deliberately. **Each is specified where it was incurred** — this
table is an index, and the right-hand column is where the substance lives.

Nothing here blocks anything else. Where an item exists because something it depends on does
not, the dependency is named in the owning document.

| # | Owed | Specified in |
|---|---|---|
| O1 | Commands for direct messages, search, voice and stage — each has a line in the boundary and no code behind it | `design/05` §3, `design/00` §5 |
| O2 | `Discovery::Off` for conversation-profile networks. **Load-bearing for privacy rather than merely leaner**: with discovery on, a DM node meeting a peer at the shared network's relay lands in its routing table, which is the correlation D29 forbids | `design/06` §12, `design/09` §3 |
| O3 | `may_moderate_at` answers from current state, ignoring the head it is given — so demoting a moderator retroactively invalidates redactions that should stand | `specs/07` §9 Q1, `design/01` §6 |
| O4 | `kols-store` does not exist; `kols-node` carries a file-backed store instead of the SQLite projection | `design/05` §2, §5 |
| O5 | The executor rebuilds an author's whole log to append one record, and replay walks the log once per question | `design/05` §5 |
| O6 | The window has no presence — `design/09` §4's third question has no answer, and is last deliberately | `design/09` §4.1 |
| O7 | **No credentials and no backup.** Seeds are written to `<home>/seed` in the clear, so anything with read access to that disk is that member. **A release gate, not a feature** | `design/02` §6.3, `design/00` §5 |
| O9 | A suspended node can lose its claim without knowing — the staleness check is wall-clock, so a sleeping laptop is indistinguishable from a dead one | `design/05` §4 |
| O11 | A relay may not be shared between two of a member's networks, and **nothing enforces it**. Enforcing it means network-scoping the protocol names, which is a wire change rather than a client fix | `design/00` D29, `design/09` §3 |
| O15 | Content routing has never been observed working. Two nodes cannot demonstrate it: with nobody to route *through*, a one-hop table and a working DHT behave identically | `design/05` §8 |
| O16 | Two members on one network still cannot find each other without the relay. mDNS runs and the transport caches what it finds, but never auto-dials, and nothing here handles the event it emits | `design/00` §6 |

O8, O10, O12, O13, O14, O17 and O18 are closed. What each was, and what closing it turned up,
is in [`docs/log.md`](docs/log.md). The numbers are retired rather than reused, so the log
stays readable.

---

## 3. What Exists

**Crates.** `design/05` §2 owns the layout and what each crate deliberately does not own.

| Crate | State |
|---|---|
| `kols-core` | Encoding, author logs, merge, collision recovery, chat policy, channel structure, `sidebar_order`, reader-side limits. 126 tests |
| `kols-net` | Publish and fetch over a running node. Two live two-node tests |
| `kols-api` | The whole boundary — all three of `design/05` §3's properties held. 33 tests |
| `kols-node` | `kols`, its node daemon, the executor, the store and the workspace — the window's entire backend. Ten tests over a live wire between two processes |
| `kols-app` | The Tauri shell, holding a workspace and an executor for whichever network is open. Builds `kols-desktop`. 7 tests |
| `kols-ui` | The interface: HTML, CSS and one script, holding no keys, no sockets and no files |
| `kols-store`, `kols-media` | Not created. A crate is made when there is code for it — an empty one is a claim that something exists |

**Protocol extensions.** [`design/06`](design/06-protocol-extensions.md) §0 carries the table
and is the one place their state is kept. In summary: E1 and E3 withdrawn as unnecessary;
**E2, E4, E5, E9, E11, E12 and E14 landed**; E7, E10 and E13 are P2, E6 is P3, E8 is P4, and
E15 is spec text that blocks nothing.

**P0 is closed** — all five criteria met, recorded in `design/07` §3. The measurements it
produced, which the whole segment model rests on, are in `design/08` §4.

---

## 4. Log

Moved to [`docs/log.md`](docs/log.md) — 96 entries, newest first.

What happened *lately* is §1. The log is why things are the way they are: the reasoning behind
a change, the thing tried and abandoned, the bug that turned out to be a different bug. It
lives outside this file because it is history rather than state, and 187 KB of history at the
bottom of a status file stops anybody reading the status.
