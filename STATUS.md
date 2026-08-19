# ko-ls — Implementation Status

**Updated:** 2026-08-19 (`kols-api` — the boundary exists and the CLI crosses it)
**Phase:** P1 — two nodes talk live and durably, a joiner reads back through sealed
history, and every command the client runs now crosses the API boundary
**Design:** [`design/`](design/) — `00`–`08` at v1.0, `09` at v0.2. **`distributed-intranet/specs/07` is normative** where it and the design set overlap.

This file is the answer to "where are we?". It is updated in the same change that moves
work, never afterwards from memory — a status file that lags is worse than none, because
it is believed.

---

## 0. Resuming After a Break

**Read this section, then §1. Everything else is reference.**

Two repositories on this machine, **both pushed and current**:

| Repo | Remote |
|---|---|
| `ko-ls` (this one) | `DriftingNarwhal/ko-ls` (private), branch `main` |
| `../distributed-intranet` | `DriftingNarwhal/distributed-intranet`, branch `main` — carries spec 07, E9 (Core §2.6.2), E2 (Core §2.7.2), E5 (Real-Time §2.2.1), MLS persistence (Core §3.3.1), E4 (gossipsub, Core §5.1), E11 (Core §2.2.1) and **E12 (optional peer discovery, Core §5.1.1)** |

The client builds against the sibling checkout by **path dependency**, not a published
version, and deliberately so while the extensions are still moving. A fresh machine needs
both repos cloned side by side.

**Next task:** **the executor, and then the event half of `kols-api`.** The boundary now
exists and refuses, but each caller still takes its `Authorized` apart and does the work
inline — so there is a gate and no dispatcher behind it. `Event` is deliberately absent until
there is something emitting events: the sync engine (`design/05` §4) is `kols serve`, whose
records do not yet cross this boundary, and an event vocabulary written before anything
emits one is a contract with no implementation to keep it honest.

**`kols-api` is the boundary `design/05` §3 describes, and two of its three properties are
now held rather than intended.** *No ambient authority*: every command names its target, the
gate resolves permission by replaying governance state, and `Authorized` has no public
constructor — so an executor takes one and "somebody forgot to check" stops being something
a reviewer has to notice. The compiler runs that claim as a `compile_fail` doctest. *Consent
is a decorator*: every command carries a `Sensitivity`, derived from the tier the capability
vocabulary assigns rather than from how consequential an action feels, with a drift test
against `capabilities::VERBS` so re-tiering a verb and forgetting the classification fails
loudly instead of in a prompt that quietly stopped appearing.

**The CLI crosses it, which is the only reason to believe it works.** `channel create`, `post`
and `read` all go through `authorize` now, and the checks they used to open-code — may-post,
the message ceiling, the network profile — are gone from `kols-cli` rather than duplicated
beside it. The binary-driving tests still pass unchanged, which is what says the seam holds.

**S3 is done and was confirmed by running, not by installing.** A scaffolded Tauri v2 app
compiled and linked against the system webview in 44 seconds and mapped an 800×600 window on
the X display within a second, WebKit child process alongside. It was built in a scratch
directory rather than this repo, because a crate here is a claim that something exists. Two
corrections came out of it, both in `design/07` §2 now: `libappindicator3-dev` does not exist
in Debian 12 (the Ayatana fork is what Tauri builds against), so the package list as written
could not have been installed; and there is no `/mnt/wslg` in the container — WSLg is on the
host, and what arrives is VS Code's forwarding of both an X server and a Wayland socket. GTK
prefers Wayland when `WAYLAND_DISPLAY` is set, which leaves `xwininfo` nothing to observe, so
`GDK_BACKEND=x11` is what a *script* checks a window with.

**One small client item is owed and is not blocking:** `kols` still builds every node with
`MemberNode::new`, so nothing yet asks for `Discovery::Off` on a conversation-profile network.
That is the client half of E12 (`design/06` §12) and wants a profile to read: `kols init`
writes no `chat:network-profile` key, so every network the CLI creates is a `server` by
default and there is not yet a conversation network to build the leaner node for.
`kols_core::policy::conversation_genesis_values` already exists for one; only a test calls it.

**`design/09` is the interface design**, written before any interface code. It settles the
navigation model, the hot/warm/cold liveness tiers, presence honesty, permission-gated
chrome and the theming system with its CSP contract. Two protocol extensions came out of
writing it — **E12** and **E13** (cross-network connection bootstrap for DMs, P2).

**E12 landed narrower than it was asked for, and the narrowing is the finding.** It was
raised as *tiered node liveness*: hot, warm and cold, a warm node holding a relay reservation
without a full behaviour set. Only the behaviour set was ever the protocol's — a consuming
client cannot assemble a partial one — so Core §5.1.1 now makes Kademlia and mDNS optional
and stops there. Whether a node exists at this moment, and whether it holds a reservation
while nothing is happening, is policy over time and belongs to whoever holds the nodes; a
specification has no view on it. The tiers therefore stay in `design/09` §2 as client
behaviour, built on reservations and dialability, which already existed.

**Sealing, backfill, per-segment keys and the live-path bound are all in.** Both gaps the
backfill work left open are now closed:

- **Retention is per segment** (`design/01` §3.1.0). Each segment lives under its own
  derived pointer and therefore its own DEK, so an author stops republishing and
  re-wrapping what has aged out and keeps the rest — Storage §5.2 does the rest. This was
  forced rather than chosen: a pointer commits to one DEK for its whole life, and a
  wrapping only travels alongside a pointer that exists, so a separately-forgettable key
  needs a pointer of its own. The cost is one indirection — `author_log_pointer` now names
  a *head index* saying which segment is current, and everything else is derivable again.
- **The live path no longer carries backlog.** A failed publish is retried only while the
  record is inside a freshness window (`--live-window-millis`, default one minute), so a
  record written a moment before a peer subscribed still goes out and last week's history
  does not.

The keying gaps are closed end to end: `GroupSession::save`/`restore` (Core §3.3.1) mean a
founder survives a restart and can still key people in, and `kols revoke` now drives a real
removal — membership entry from the command, epoch rotation from the daemon, in that order
because Core §3.3 requires it.

**Just done:** the chat side of E2. `ChannelEntry` encodes all four kinds as `chat`-namespace
payloads (spec 07 §3.8, written for this and normative), and `ChannelEntry::read` is the
only way to get one out of a log entry — so both checks E2's generalisation moved onto
readers are unavoidable rather than optional:

1. **The declared capability must be the one the kind requires.** The protocol verified the
   author holds what the entry *declared*; only a reader that understands `chat` knows what
   it *should* have declared. Without this, an author holding `chat:post:*` — the most
   ordinary grant a network issues — could mint channel structure.
2. **A channel entry is invalid in a `conversation`-profile network.** The protocol carries
   `chat` payloads without decoding them, so it cannot reach this verdict.

**To pick up:** `cargo test` in this repo should show 98 passing and clippy silent, and 644
in `../distributed-intranet`. If it does not, fix that before anything else — both trees were
left green.

---

## 1. Right Now

| | |
|---|---|
| **Working on** | P1 — `kols-api` landed; the executor and the event half are next |
| **Blocked on** | Nothing |
| **Runnable** | **`kols`** — init, attach, admit, revoke, serve, channel create/list, post, read. Two nodes hold a conversation. `cargo test` — 117 tests here and 644 in `../distributed-intranet`, clippy clean in both; `scripts/cross-check.sh` for big-endian |
| **Next decision needed from the user** | Nothing blocking |

---

## 2. Finalization

| Item | State | Notes |
|---|---|---|
| F1 record encoding | **Done** | `design/08-record-encoding.md`, normative |
| F2 spec 07 in protocol repo | **Done** | `distributed-intranet/specs/07-chat-application-spec.md`, committed there. README and CLAUDE.md updated — the repo no longer claims six specs |
| F3 design set → v1.0 | **Done** | `00`–`08` at v1.0. The review pass demoted `08` from normative (its content is upstream now), refreshed the roadmap and scale claims, and turned `05` §8's test plan into a table with real state per row. `09` was written after that pass and is at v0.2 — it describes an interface nothing has built yet, so v1.0 would be a claim about settled work |

## 3. Setup

| Item | State | Notes |
|---|---|---|
| S1 client repo | **Done** | `/workspaces/ko-ls/ko-ls`, design moved in, crates building. Committed and pushed to `DriftingNarwhal/ko-ls` on `main`, in step with `origin` — §0's table is the current statement of that |
| S2 protocol changes on `main` | In progress | E9 (Core §2.6.2), E2 (Core §2.7.2), E5, E4, E11 (Core §2.2.1) and E12 (Core §5.1.1) landed, each with spec text, implementation and tests together. P1's protocol work is finished again; E7, E10 and E13 are P2 |
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
| E13 | Cross-network connection bootstrap | **New** — from `design/09` §3; so two people starting a DM never provision a relay (P2) |

## 5. Client Crates

| Crate | State |
|---|---|
| `kols-core` | **Encoding, author logs, merge, collision recovery, chat policy, channel structure** — records/segments/ids, `AuthorLog` incl. `rebase`, `ChannelView`, permissions, capability vocabulary, `ChatPolicy`, `ChannelEntry`. 60 tests |
| `kols-net` | **Publish and fetch** — stores/announces chunks, accepts pointers, reassembles segments. Two live two-node tests |
| `kols-cli` | **`kols`, and its node daemon** — creates a network, admits and keys in joiners, serves and fetches content, renders a merged view across authors. 119 tests that drive the real binaries, eight of them over a live wire between two processes |
| `kols-store` | Not created |
| `kols-media` | Not created |
| `kols-api` | **The command surface and its gate** — `Command`, `Sensitivity`, `Refusal`, and `authorize` returning an `Authorized` nothing else can construct. `kols-cli` crosses it for create, post and read. No `Event` yet, deliberately — §1. 18 tests |
| `kols-app` | Not created |
| `kols-ui` | Not created |

Crates are created when there is code for them, not in advance — an empty crate is a
claim that something exists.

## 6. P0 Definition of Done

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

## 7. Where the Code Lives

- `ko-ls` — `origin` is `github.com/DriftingNarwhal/ko-ls` (private).
- `distributed-intranet` — `origin` is `github.com/DriftingNarwhal/distributed-intranet`,
  up to date. Nothing in the client builds against a *published* version of the protocol:
  the crates are path dependencies on the sibling checkout, deliberately, until the
  extensions stop moving.

---

## 8. Log

Newest first. One line per change that moved the state above.

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
