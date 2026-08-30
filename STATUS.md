# ko-ls — Status

**Updated:** 2026-08-30
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
| Why anything is the way it is; the decision register D1–D37; the roadmap | [`design/00`](design/00-overview.md) |
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
| Windows and macOS builds | **done**, in CI. `kols.exe` runs; `kols-desktop.exe` is built and unrun. It no longer opens a console behind the window — and the attribute that does that had to land with `kols_node::Report`, since a GUI-subsystem process has no stdout and Rust *panics* on the write rather than dropping it |
| Two nodes meeting through the deployed relay | **Done, on one LAN.** Both ends ran the window, connected and reconnected several times, messages crossed both ways, and an established connection survived the relay going down |
| Content outliving the node that wrote it | **done** — a member reads what an offline member wrote, because a third kept it, and a restart no longer discards what a node kept (`three_nodes.rs`). Before this a node was a member of the storage swarm only until its process ended |
| **Two nodes on separate networks** | **Done — this milestone's stated first test passes.** Two of the user's own laptops, one on a mobile hotspot, and then a third person on a third network: connection worked across all three, survived close and reopen, reconnected, and roles, permissions and every chat function (posting, voting, withdraw, edit, channel creation by an invited member) worked. The one defect that test found — a node losing its whole servable contribution on restart — is the row above |
| Invites short enough to send somebody | **done** — one real machine's invite went from ~4,750 characters to ~1,324, about half from carrying only the addresses a recipient could dial and about half from an encoding that stops repeating the peer id once per address (`design/02` §6.1, Core §5.6) |
| The window hearing what the node tells it | **done, and it never had.** The application declared no Tauri capabilities, so its ACL was empty and every `plugin:` command was refused — `listen` included. No node event had ever reached the window for the life of the client; three polls had been written as fixes for what was one denial, and the features with no poll behind them read as unbuilt (`design/05` §1) |
| An interface that survives being used | **done** — the first field test's list, worked through: a visible handle on the channel controls that were right-click-only, first-sight marks on messages that land mid-timeline, the roster as a counted dropdown at the top right, the door as a sheet behind a counted button, and settings as a screen rather than a sheet over a dimmed channel (`design/09` §4.1–§4.3) |

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
- **`kols`** — init, relay list/set, invite, join, waiting, attach, admit, revoke, name,
  serve, post, read, edit, delete, react, pin, and channel
  create/list/rename/topic/slowmode/archive.

**Gates green as of this date:** 299 tests here, 666 in `../distributed-intranet`, clippy
clean in both. O20's starved-run flake did not reproduce on this full-width run; it is not
fixed, and one green run is not evidence that it is.

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
| O6 | The window has no presence — `design/09` §4's third question has no answer, and is last deliberately. What it shows instead now answers a narrower question in the frame rather than behind a click: how many members this node is connected to, and whether that is more than none | `design/09` §4.1 |
| O7 | **No credentials and no backup.** Seeds are written to `<home>/seed` in the clear, so anything with read access to that disk is that member. **A release gate, not a feature** | `design/02` §6.3, `design/00` §5 |
| O9 | A suspended node can lose its claim without knowing — the staleness check is wall-clock, so a sleeping laptop is indistinguishable from a dead one | `design/05` §4 |
| O11 | A relay may not be shared between two of a member's networks, and **nothing enforces it**. Enforcing it means network-scoping the protocol names, which is a wire change rather than a client fix | `design/00` D29, `design/09` §3 |
| O15 | Content routing has never been observed working. Two nodes cannot demonstrate it: with nobody to route *through*, a one-hop table and a working DHT behave identically | `design/05` §8 |
| O21 | A joiner admitted under auto-admit whose response never arrives is a member who believes they are waiting. Their client does not ask again, so nothing on either side surfaces the disagreement | `design/09` §4 |
| O16 | Two members on one network still cannot find each other without the relay. mDNS runs and the transport caches what it finds, but never auto-dials, and nothing here handles the event it emits | `design/00` §6 |
| O19 | **A role cannot be deleted.** `EntryBody` expresses no group removal, so what a role holds can be emptied and its members taken out, and the name stays in replayed history forever. The interface says so rather than offering a control that cannot work. Not a protocol change being asked for — nobody has needed one — but a limit somebody will meet and should not have to discover | `design/05` §3 |
| O22 | **Forgetting a network does not tell it.** `forget` is local — it drops this installation's store and seed, and to every other member nothing has happened: still in `everyone`, still in every role, still a leaf the epoch rotates key material for, forever. Needs E16; the client half is an announcement published *before* the seed is destroyed, plus an executor guard that refuses every self-removal and means to refuse only the last `revoke-node` holder. `forget` stays all-or-nothing by decision — a seed-preserving leave is a property nobody wanted | `design/06` §16, `design/02` §6.5 |
| O20 | **The daemon suite run starved is unreliable**, and `CONTRIBUTING.md` asks for exactly that run. One or two of eleven time out in `wait_for` under `taskset -c 0,1`; each passes alone. Measured at `main` on 2026-08-29, so it is the suite rather than any change — but it makes the starved run a signal to isolate rather than a gate, which is weaker than what it was added for | `CONTRIBUTING.md`, `tests/common::patience` |

O8, O10, O12, O13, O14, O17 and O18 are closed. What each was, and what closing it turned up,
is in [`docs/log.md`](docs/log.md). The numbers are retired rather than reused, so the log
stays readable.

---

## 3. What Exists

**Crates.** `design/05` §2 owns the layout and what each crate deliberately does not own.

| Crate | State |
|---|---|
| `kols-core` | Encoding, author logs, merge, collision recovery, chat policy, channel structure, `sidebar_order`, reader-side limits, and `Scope` — the one construction of a capability's name, used by the writer and the resolver alike. 126 tests |
| `kols-net` | Publish and fetch over a running node. Two live two-node tests |
| `kols-api` | The whole boundary — all three of `design/05` §3's properties held. 39 tests, and the consent drift test is now guarded at both ends: a new command stops the suite compiling until it is sampled |
| `kols-node` | `kols`, its node daemon, the executor, the store and the workspace — the window's entire backend. Ten tests over a live wire between two processes, and eight in-process over roles and grants |
| `kols-app` | The Tauri shell, holding a workspace and an executor for whichever network is open. Builds `kols-desktop`. 8 tests, one of which resolves the webview's ACL against the real configuration — the boundary whose failure produces no output |
| `kols-ui` | The interface: HTML, CSS and one script, holding no keys, no sockets and no files |
| `kols-store`, `kols-media` | Not created. A crate is made when there is code for it — an empty one is a claim that something exists |

**Protocol extensions.** [`design/06`](design/06-protocol-extensions.md) §0 carries the table
and is the one place their state is kept. In summary: E1 and E3 withdrawn as unnecessary;
**E2, E4, E5, E9, E11, E12 and E14 landed**; E7, E10 and E13 are P2, E6 is P3, E8 is P4,
E15 is spec text that blocks nothing, and E16 is small and blocks leaving a network at all.

**P0 is closed** — all five criteria met, recorded in `design/07` §3. The measurements it
produced, which the whole segment model rests on, are in `design/08` §4.

---

## 4. Log

Moved to [`docs/log.md`](docs/log.md) — 103 entries, newest first.

What happened *lately* is §1. The log is why things are the way they are: the reasoning behind
a change, the thing tried and abandoned, the bug that turned out to be a different bug. It
lives outside this file because it is history rather than state, and 187 KB of history at the
bottom of a status file stops anybody reading the status.
