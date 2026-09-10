# ko-ls — Status

**Updated:** 2026-09-10
**Phase:** P2 has started. P1's register was cleared, both working branches merged on 2026-09-09,
and the client now **runs a node per network** rather than one for whichever is in view — which is
what direct messages were actually waiting on, and what `v0.13.1` carries. What remains before
they work is the flow itself, and §1 says where it starts.

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

Three repositories, side by side:

**All three are on `main`, and this line is a description again rather than a claim about
somewhere the work is not.** `ko-ls` and `distributed-intranet` were on
`replica-duty-and-storage-ceilings` and `close-the-moderation-head-question` respectively until
2026-09-09, deliberately, until the owed register was clear. It is, so both merged — fast-forward
in each repo, since `main` had no commit either branch lacked — and both are pushed.

**There are no branches now, in any of the three.** The merged ones were deleted locally and on
their remotes on 2026-09-10, `settings-and-permissions` included — it had been an ancestor of
`main` for weeks and was carrying nothing. Each was checked for commits `main` lacked before it
went, which for all three was zero.

Kept as a caution rather than deleted, because the state it describes was reached twice. A branch
held until a register clears is fine; a `main` that quietly falls sixteen commits behind while
this file says otherwise is how somebody clones a client whose seeds are written to disk in the
clear. What the branches were carrying at the end was the whole credentials gate, which is
exactly the kind of thing that should not sit off `main`.

| Repo | Remote | What it is |
|---|---|---|
| `ko-ls` (this one) | `DriftingNarwhal/ko-ls` | The client. AGPL-3.0-only |
| `../distributed-intranet` | `DriftingNarwhal/distributed-intranet` | Protocol and specs. MPL-2.0, specs CC BY 4.0 |
| `../DI-Relay` | `DriftingNarwhal/DI-Relay` | The bootstrap relay of Core §5.5, deployed and operational. AGPL-3.0-only |

The client builds against `../distributed-intranet` by **path dependency** while the
extensions still move, so a fresh machine needs those two cloned as siblings.
`.devcontainer/` lives in *this* repo and builds both, plus the Tauri toolchain — open the
`ko-ls` folder in it, not the parent.

**The protocol is tagged `v1.2.0`** as of 2026-09-10, its first tag since `v1.0.2` on 08-22. The
major and minor track the Core spec, as the `v1.0.x` run did while Core sat at v1.0; Core is now
v1.2, and `v1.1.0` is deliberately absent because Core v1.1 existed only between commits and was
never tagged. `DI-Relay` still pins `v1.0.2` and can move whenever its operator wants a redeploy —
nothing obliges it to.

**This client stays on the path dependency**, and `design/07` S1's instruction to switch to a tag
"once the protocol changes have landed and stabilised" is not yet met: E7, E10 and E13 are P2 and
unbuilt, so the extensions are still moving. The tag is for consumers who are not sitting in this
workspace.

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
| **Next decision needed** | Nothing blocking, but **O26 comes before the two-machine test rather than after it**: a joiner being admitted and keyed fails deterministically in the suite, which is the path that test walks. It predates this work — bisected to the `v0.12.0` state of both repos — and it is isolated, so it may well be the harness rather than the client. Finding out which is cheaper than losing a two-machine session to it |

**What P2 starts with** — and the order changed on 2026-09-10, because the thing blocking direct
messages turned out not to be on the list at all.

~~**E10**~~ **landed** → ~~multi-node hosting~~ **landed** → **O1's direct-message commands**
(next) → **E13**, much reduced → **E7** (channel-scoped MLS, large) → O1's `Search`.

**The blocker was that the client could only run one node**, and nothing had written that down as
blocking anything: `design/05` §4 described a node per network as the architecture while `09` §2
called the liveness tiers unwritten policy, and no document joined those two facts to direct
messages. It bit a step earlier than messaging, too — minting a conversation's invite needs an
address, and only a running node knows one (`02` §6.1).

**E13 is smaller than it was.** `design/06` §13 asks for a hand-rolled simultaneous open with the
shared network's connection as the signalling channel, and libp2p's `dcutr` cannot be driven that
way. It is not needed: a pair who can dial each other directly need no punch, and a pair who
cannot may **borrow** the shared network's relay under D39 and let ordinary tier 2 do the work.
What is left of E13 is the address exchange over E10's carrier and the disclosure gate — a
member's addresses for network X go only to a member of X, verified by replay.

**E10 landed as a carrier rather than as a chat protocol** — Core §5.1's
`/intranet/direct/1.0.0`, taking a namespace, a kind and an opaque payload, with chat's
`dm-invite` as its first tenant. Third time a chat-shaped request became a platform-shaped one
after E2 and E9, and `design/06` §10 now records that as a pattern to expect. The carrier owes
the signature, the connection binding and the per-identity metering; `kols-core::DmInvite` owes
the identity-link check, which is the half no platform can hold — a proof with two genuine
signatures over a true statement about **somebody else** verifies perfectly, so the pair it
names has to be compared against the pair expected. Building it also found that
`CommonOwnershipProof` had no serialized form: Core §1.2 called it *shared voluntarily* while
providing no way to share it, the same gap §5.6 records for invites.

**So E13 is now the only thing between the platform and direct messages**, and what remains on
this side is the flow rather than the mechanism — O1's commands to create a conversation, deliver
a request and accept one. E7 is where `design/03` §3.5's
obligation comes due: placement must rank over a private channel's **roster** rather than over the
capability ledger, or its effective replica set becomes *roster ∩ top-k* and can be empty. That is
written down as a requirement of the work rather than left to be found during it, which is the
mistake this project has paid for before.

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
| A stolen laptop that is not an identity | **done** — the release gate `design/00` §5 has carried since before there was code. An account password derives an Argon2id key that *wraps* each per-network seed and never derives one, because a derived identity is checkable offline against ids the network publishes. There is no unwrapped path, including for tests. The window gates on login and starts no node until somebody has logged in, which is the half that decides what the password protects; locking hides the window and leaves the node answering. The export is **portability rather than recovery** — the seed is the member and lives on one disk (`design/02` §6.3) |
| Belonging to more than one network at a time | **done, and it never had.** The shell ran a single node — for whichever network was in view — and stopped it the moment the member looked elsewhere, so somebody in a dozen networks received in one of them and the other eleven **served nothing**, quietly dropping replica duty they had taken. `design/05` §4 had described a node per network since before there was code; nothing recorded that the shell did not do it. Everything joined is warm unless the member sets it aside, and a set-aside network is *polled* rather than switched off (`design/09` §2) |
| Knowing which network an event came from | **done** — `Event` says what happened and not where, which was unambiguous while one node reported and is not now. The supervisor tags each batch with the network whose node produced it, and the interface ignores what is not about the network it is showing. Without it a message in a conversation would count against a server's unread and reopen its channel |
| Scrolling back without reading the whole channel | **done** — and bounded rather than merely fast, which is the stronger claim. A settled page costs one file read per record drawn and **no directory listings at all**: 52 reads for a page of fifty, identical at five hundred records and at eight thousand. The guarantee is a count of work asserted equal across a fifteen-fold growth, not a duration that looked flat — `design/09` §4.4 forbids the two-second tick growing at all, which is the storage ceiling's argument about the other resource |

**Runnable.** `kols-desktop` is the product (`design/00` D30); `kols` is a development tool
over the same `kols-api` boundary, owed no feature parity and no end-user documentation.

- **`kols-desktop`** — opens on a lock, and makes an account on first launch rather than
  offering one. Behind it: creates or joins a network, runs a node for it, generates a relay
  identity and designates relays, lists and renders channels in the order the network agrees
  on, posts, reacts, revises, withdraws and pins, pages back through history and asks the
  network for what this disk stopped holding, manages channels and folders, mints an
  invite and admits from the waiting room, says who is around without claiming to know who is
  not, and says so when a healed fork undid something.
  Settings is a screen of five sections split by what a click costs (`design/09` §4.2), and carries the
  network's name (D32), a **role-first permissions surface** — roles, what each holds and at
  which scope, and who is in them — the network's own policy: admission mode, the abuse
  limits of spec 07 §4.3 and the two retention windows of §2.8 — and what this machine gives
  other members, with a ceiling on it and an export of the identities behind it.
- **`kols`** — **the multi-process test harness, and not a product surface** (D30). Six test
  files drive `CARGO_BIN_EXE_kols`, and separate processes are the whole point: separate stores
  and swarms, a node that can be killed and restarted, and the single-node claim exercised for
  real. **Nothing in the shipped application invokes it** — the window runs
  `kols_node::serve::serve` in-process. Its output is therefore a test contract rather than an
  interface, and carries no end-user prose. It covers init, relay list/set, invite, join,
  waiting, attach, admit, revoke, leave, name, serve, post, read, edit, delete, react, pin,
  contribute, presence, storage, history, and channel
  create/list/rename/topic/slowmode/archive.

**Gates green, re-run 2026-09-10:** 417 passed and 0 failed here (4 ignored, all measurements),
703 passed and 0 failed in `../distributed-intranet`, clippy clean in both, and
`crates/kols-ui/drive.mjs`'s 123 checks green by hand with no uncaught errors. The daemon suites
were clean on this run, full width. O20 reproduced once
across four full-width runs on 2026-08-31 and 2026-09-01, which makes it **intermittent rather
than deterministic** — `CONTRIBUTING.md` said it failed on every full-workspace run, and that
was a run of bad luck rather than a property. The one failure arrived directly after a
governance change and was attributed by measurement rather than by reading; how, and why the
comparison is worth running even when you are sure, is in `CONTRIBUTING.md`.

**`v0.13.1` is the release the rows above describe**, cut 2026-09-10, and it exists because
`v0.13.0` crashed on selecting a network — `tokio::spawn` panics outside a runtime, and a Tauri
command is a synchronous caller with none entered. `docs/log.md` has why no test saw it, and why
the smoke test before that tag could not have: it checked that the window *opened*, which is a
different claim from the window *working*.

**What `v0.13.1` has actually been exercised for, stated at the strength it holds.** On one
machine: it starts, unlocks, opens a network, sends messages, and switches between networks
without stopping the one being left. **Not tested: anything between two clients.** That is the
gap worth naming rather than leaving implied — the change this release is *about* is running
several nodes at once, and the failure modes that matters for are contention, delivery to a
network nobody is looking at, and claims released on close, none of which one machine can show.
`docs/two-machine-test.md` is the procedure.

**`v0.13.0`**, cut 2026-09-10. What it adds is a change in
how the client *runs* rather than a feature beside the others: **a node per network**. Until it,
the shell ran one — for whichever network was in view — so a member in a dozen networks was
receiving in one of them, and every other network's node was stopped the moment they looked
away. `design/05` §4 had described a node per network as the architecture since before there was
code; nothing had recorded that the shell did not do it, or that it was what direct messages were
waiting on.

Also in it: E10's direct-delivery carrier and the request payload that rides it, and D39's
borrowed rendezvous. **Not** the direct-message flow itself, which is next.

**One thing this release is thin on evidence for, said plainly.** The supervisor is covered by
tests and the window's own startup was checked by launching it, but no manual session has yet
opened two networks and switched between them. The failure modes there — a dozen nodes competing
for one runtime, claims not released on close — are the kind this suite is weakest at.

**`v0.12.0`**, cut 2026-09-09 from `main` after both
branches merged. It carries the nineteen commits `v0.11.1` was missing, and **what it adds
differs in kind from an ordinary release**: the storage ceilings and repair, the paged read and
the bounded tick, and the whole of O7 — an account password wrapping every seed, a lock in front
of the window, and an export. `design/00` §5 has carried that last one as a *release gate* since
before there was code, so `v0.11.1` was the last published client that writes its seeds to disk
in the clear, and it stopped being the one Releases serves.

**The gap is the thing to watch rather than the version number**, which is why this paragraph
keeps its history. `v0.11.0` sat eight days and forty commits behind before `v0.11.1`; `v0.11.1`
then sat eight days and nineteen commits behind before this. `README.md` and
`docs/two-machine-test.md` both send a new user to Releases, so a distributable trailing `main`
by a week of fixes is indistinguishable, from the far end, from fixes that were never made — and
twice now it has been trailing something more serious than fixes.

---

## 2. What Is Owed

Debts this client took on deliberately. **Each is specified where it was incurred** — this
table is an index, and the right-hand column is where the substance lives.

Nothing here blocks anything else. Where an item exists because something it depends on does
not, the dependency is named in the owning document.

| # | Owed | Specified in |
|---|---|---|
| O1 | Commands for direct messages, search, voice and stage — each has a line in `design/05` §3's boundary *grammar* and nothing in `kols-api`. **Not work that can start**: the DM commands wait on E10 and E13, `Search` on `03` §6's two indexes, and the voice and stage set on `kols-media`, which does not exist because nothing has written code for it yet. This is P2's surface rather than a debt before it | `design/05` §3, `design/00` §5 |
| O26 | **`a_joiner_is_admitted_keyed_and_reads_what_was_written_before_they_arrived` fails deterministically, and it is not from this work.** The joiner dials the founder, the founder writes and picks up the membership entry, and the joiner never leaves *not a member of this network yet* — so it is never keyed and the 45-second wait expires. **Attributed by removing things, not by reading**: it fails at `main`, at the commit before the supervisor, at the commit before E10's client half, and at the exact `v0.12.0` state of *both* repositories — the protocol being a path dependency means an old client commit still builds against today's protocol, so both had to be moved back. It also **passed on this machine an hour earlier at identical code**, which makes it state-dependent rather than a plain code fault. It is isolated: 11 of 12 in that file pass, including `a_joiner_walks_back_through_sealed_segments_to_read_the_start` and `a_founder_can_still_key_somebody_in_after_restarting`, which exercise joining and keying respectively. **Worth treating as urgent despite that**, because it is the admit-and-key path the two-machine test depends on | `tests/two_nodes.rs`, `CONTRIBUTING.md` |
| O20 | **The daemon suite run starved is unreliable**, and `CONTRIBUTING.md` asks for exactly that run. One or two of eleven time out in `wait_for` under `taskset -c 0,1`; each passes alone. Measured at `main` on 2026-08-29, so it is the suite rather than any change — but it makes the starved run a signal to isolate rather than a gate, which is weaker than what it was added for | `CONTRIBUTING.md`, `tests/common::patience` |

O2, O3, O4, O5, O6, O7, O8, O9, O10, O12, O13, O14, O15, O17, O18, O21, O22, O23 and O24 are
closed. What each was, and what closing it turned up, is in [`docs/log.md`](docs/log.md). The
numbers are retired rather than reused, so the log stays readable.

**O4, O5 and O7 left this table on 2026-09-09 rather than being deleted from it**, and the
distinction is the same one the accepted limits below are separated for: a row reading
*closed* in a table headed *what is owed* is a debt to anybody skimming it. The three that
mattered most are worth one line each here, because they are what the phase line above rests
on — **O7** put an account password in front of the seeds and a lock in front of the window,
which is `design/00` §5's release gate; **O4** made reading a channel cost a page rather than
a history, and made the tick that re-reads it bounded rather than merely fast; **O5** made a
send cost the same at six thousand of an author's records as at twenty-five.

**O11, O16, O19 and O25 are accepted rather than closed, which is a different thing and is why
they are named separately.** None was fixed; all were decided against — O16 and O19 on
2026-09-07, O25 and O11 on 2026-09-08 — and an accepted limit left in the table above would read
as a fix nobody had got round to.

| # | Accepted limit | Decided in |
|---|---|---|
| O11 | **A relay shared between two of a member's networks is warned about and never refused.** Enforcing it means network-scoping the protocol names, which is a wire change and not a client fix — and refusing would stop the honest case while the determined one designates the address anyway, as well as blocking a member legitimately relaying on their own LAN for two of their own networks. What the client owed was the notice, since it holds the workspace and is the only party that can see this at all, and **that is built**: both designations warn, comparing by peer id rather than by address because one relay answers at several | `design/00` D29, `design/09` §3 |
| O16 | **This client does not dial a LAN peer that mDNS finds.** A node that did would make two of a member's networks correlatable by anyone watching that LAN — D29 one layer down, reached with no relay involved. The cost is that two members in one room still need a routable third party to meet, which is narrow and is the price of the property | `design/00` §6, `design/09` §3 |
| O19 | **A role cannot be deleted.** `EntryBody` expresses no group removal, so a role can be emptied of capabilities and members and its name stays in replayed history. A role holding nothing grants nothing, and no protocol change is being asked for — the interface explains the limit instead | `design/05` §3 |
| O25 | **A node holds only what it can read**, so a storage offer funds durability for the channels its owner is keyed for and no others. Kept deliberately: the alternative is nodes hoarding ciphertext against keys they do not have, which gives up the forward secrecy `03` §3.1 chose MLS for — an epoch key compromised later cannot open bytes a node never kept. **It costs nothing today**, because roster keying is unbuilt and every member can decrypt every channel; `channel_dek` derives from the network epoch regardless of a channel's privacy flag. What it will cost when private channels land is written down as a requirement of that work rather than left to be discovered during it | `design/03` §3.5, `design/06` E2 |

---

## 3. What Exists

**Crates.** `design/05` §2 owns the layout and what each crate deliberately does not own.

| Crate | State |
|---|---|
| `kols-core` | Encoding, author logs, merge, collision recovery, chat policy, channel structure, `sidebar_order`, reader-side limits, and `Scope` — the one construction of a capability's name, used by the writer and the resolver alike, and the direct-message request payload with the identity-link check no platform can make for it. 150 tests |
| `kols-net` | Publish and fetch over a running node. Two live two-node tests |
| `kols-api` | The whole boundary — all three of `design/05` §3's properties held. 25 commands, 10 events, 50 tests, and the consent drift test is guarded at both ends: a new command stops the suite compiling until it is sampled, which is how `LeaveNetwork` was caught unsampled the moment it existed |
| `kols-node` | `kols`, its node daemon, the executor, the store and the workspace — the window's entire backend, and the largest crate here at 193 tests. Sixteen of them run over a live wire between separate processes (`two_nodes`, `three_nodes`, `relay`); the rest cover the workspace, the store, roles and grants, invites, names, records and the paged read |
| `kols-app` | The Tauri shell, holding a workspace, an executor for whichever network is open, and **the supervisor running a node for every network** (`design/09` §2). Builds `kols-desktop`. 8 tests, one of which resolves the webview's ACL against the real configuration — the boundary whose failure produces no output |
| `kols-ui` | The interface: HTML, CSS and one script, holding no keys, no sockets and no files |
| `kols-store` | The read-side projection, **built and switched on**: the schema, the range and target queries, the stored rate verdict and its fingerprint. 14 tests — six compare its verdicts against `kols_core::withheld` itself rather than a second implementation of the fold, and eight walk a channel whose records share a reading, which is the boundary a bare clock reading cannot cut. This row said 19 until 2026-09-09, while its own breakdown summed to 14 |
| `kols-media` | Not created. A crate is made when there is code for it — an empty one is a claim that something exists |

**Protocol extensions.** [`design/06`](design/06-protocol-extensions.md) §0 carries the table
and is the one place their state is kept. In summary: E1 and E3 withdrawn as unnecessary;
**E2, E4, E5, E9, E10, E11, E12, E14, E15 and E16 landed**; E7 and E13 are P2, E6 is P3 and E8
is P4. E10 landed on 2026-09-10 as a generic carrier rather than the chat-named protocol it was
asked as, which is the third time that has happened and is now recorded in `design/06` §10 as a
pattern to expect.

**E13 is no longer what stands between here and direct messages, and is smaller than it was.**
What stood there was the client running one node, which is built. And §13's hand-rolled
simultaneous open is not needed: a pair who can dial each other directly need no punch, and a
pair who cannot may borrow the shared network's relay (D39) and let ordinary tier 2 do it. What
remains of E13 is address exchange over E10's carrier plus the disclosure gate. E15 landed on 2026-09-09 beside the credentials work it describes, as it was sequenced
to — and took one amendment its proposal had not asked for: the harness spec tested the derived
*mechanism* rather than the properties Core §1.2 requires, so the conformance suite would have
failed a client the amended §1.1 calls conformant.

**P0 is closed** — all five criteria met, recorded in `design/07` §3. The measurements it
produced, which the whole segment model rests on, are in `design/08` §4.

---

## 4. Log

Moved to [`docs/log.md`](docs/log.md) — 140 entries, newest first.

What happened *lately* is §1. The log is why things are the way they are: the reasoning behind
a change, the thing tried and abandoned, the bug that turned out to be a different bug. It
lives outside this file because it is history rather than state, and 292 KB of history at the
bottom of a status file stops anybody reading the status.

**`WORKING.md` was retired on 2026-09-09 and is not coming back.** It was a temporary tracking
file for clearing the owed register before P2, untracked on purpose, and it said from its first
paragraph what would let it go: every decision in it had to reach `design/` or this file first.
Retiring it was therefore an audit rather than a deletion, and the audit found one decision that
had never landed — O16's statement was owed to `design/09` §3 and only ever reached `design/00`
§6. That, and three drifts in `design/05` §3 found the same way, are the newest entry in the log.
