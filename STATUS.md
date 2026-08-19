# ko-ls — Implementation Status

**Updated:** 2026-08-19 (end of session; both repos pushed)
**Phase:** P1 — E9 and E2 landed; chat entry payloads next
**Design:** [`design/`](design/) — `00`–`08`, all v1.0. **`distributed-intranet/specs/07` is normative** where it and the design set overlap.

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
| `../distributed-intranet` | `DriftingNarwhal/distributed-intranet`, branch `main` — carries spec 07, E9 (Core §2.6.2) and E2 (Core §2.7.2) |

The client builds against the sibling checkout by **path dependency**, not a published
version, and deliberately so while the extensions are still moving. A fresh machine needs
both repos cloned side by side.

**Next task:** the chat side of E2 — define `ChannelDefinition`, `ChannelUpdate`,
`ChannelMembership` and `ChannelRotation` as payloads in the `chat` namespace of the new
generic `EntryBody::AppEntry` (Core §2.7.2), in `kols-core`. Two checks belong to the
client and did not exist before E2 landed generically:

1. **Verify the declared capability is the one this design requires** for that kind — a
   `chat:channel-definition` must demand `chat:manage-channel`, not merely *some*
   capability the author happens to hold. The protocol enforces what was declared and
   cannot know what should have been.
2. **Reject channel entries in a `conversation`-profile network.** `ChatPolicy::profile()`
   reads the profile; the protocol carries `chat` payloads without decoding them, so this
   rejection is every conformant reader's job now.

**To pick up:** `cargo test` in this repo should show 37 passing and clippy silent. If it
does not, fix that before anything else — the tree was left green.

---

## 1. Right Now

| | |
|---|---|
| **Working on** | Chat channel records as `chat`-namespace payloads in `kols-core` |
| **Blocked on** | Nothing |
| **Runnable** | `cargo test` — 37 tests, clippy clean; `scripts/cross-check.sh` for big-endian. No binary yet |
| **Next decision needed from the user** | Whether to start P1 with E9/E2 in the protocol repo, or build a runnable CLI first |

---

## 2. Finalization

| Item | State | Notes |
|---|---|---|
| F1 record encoding | **Done** | `design/08-record-encoding.md`, normative |
| F2 spec 07 in protocol repo | **Done** | `distributed-intranet/specs/07-chat-application-spec.md`, committed there. README and CLAUDE.md updated — the repo no longer claims six specs |
| F3 design set → v1.0 | **Done** | All nine documents at v1.0. The review pass demoted `08` from normative (its content is upstream now), refreshed the roadmap and scale claims, and turned `05` §8's test plan into a table with real state per row |

## 3. Setup

| Item | State | Notes |
|---|---|---|
| S1 client repo | **Done** | `/workspaces/ko-ls/ko-ls`, git initialised, design moved in, `kols-core` scaffolded. **Nothing committed yet** — no commit has been made in either repo |
| S2 protocol changes on `main` | In progress | E9 (Core §2.6.2) and E2 (Core §2.7.2) landed, each with spec text, implementation and tests together. E11 remains for P1 |
| S3 Tauri environment | Not started | Node and webkit2gtk absent. Blocks P1, not P0 |

## 4. Protocol Extensions

Tracked against `design/06-protocol-extensions.md`. Landing rule: spec text, implementation
and tests together, with `cargo test --workspace` and `cargo clippy --workspace --all-targets`
both green.

| # | Extension | State |
|---|---|---|
| E1 | Extension capability registry | **Withdrawn** — already implemented upstream, needs configuration only |
| E2 | Channel governance entries | **Landed, generalised** — one `AppEntry` variant (Core §2.7.2) rather than four chat-shaped ones; chat records become payloads |
| E3 | Derived pointer ids | **Withdrawn** — `PointerId::from_bytes` is already public; derivation lives in `kols-core::ids` |
| E4 | Gossipsub live delivery | Not started (P1) |
| E5 | Media fan-out at the relay | Not started (P3) |
| E6 | QUIC datagram media path | Not started (P3) |
| E7 | Channel-scoped MLS groups | Not started (P2) |
| E8 | Track metadata in media payloads | Not started (P4) |
| E9 | App-layer policy map | **Landed** — `PolicyValue`, namespaced keys, Core §2.6.2; client accessors in `kols-core::policy` |
| E10 | Direct DM invite delivery | Not started (P2) |
| E11 | Namespace registration for extension capabilities | **New** — found implementing permissions; workaround in `kols-core::capabilities` (P1) |

## 5. Client Crates

| Crate | State |
|---|---|
| `kols-core` | **Encoding, author logs, merge, collision recovery, chat policy** — records/segments/ids, `AuthorLog` incl. `rebase`, `ChannelView`, permissions, capability vocabulary, `ChatPolicy`. 35 tests |
| `kols-net` | **Publish and fetch** — stores/announces chunks, accepts pointers, reassembles segments. Two live two-node tests |
| `kols-store` | Not created |
| `kols-net` | Not created |
| `kols-media` | Not created |
| `kols-api` | Not created |
| `kols-app` | Not created |
| `kols-ui` | Not created |

Crates are created when there is code for them, not in advance — an empty crate is a
claim that something exists.

## 6. P0 Definition of Done

From `design/07-build-plan.md` §3. None of these pass yet.

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
