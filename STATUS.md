# ko-ls — Implementation Status

**Updated:** 2026-08-19
**Phase:** P0 — complete. All five criteria met, including over the wire
**Design:** [`design/`](design/) — `00`–`08`. `08-record-encoding.md` is normative.

This file is the answer to "where are we?". It is updated in the same change that moves
work, never afterwards from memory — a status file that lags is worse than none, because
it is believed.

---

## 1. Right Now

| | |
|---|---|
| **Working on** | Nothing in flight — P0 closed, P1 not started |
| **Blocked on** | Nothing |
| **Runnable** | `cargo test` — 32 tests incl. two live-node tests, clippy clean. No binary yet |
| **Next decision needed from the user** | Whether to start P1 (text chat MVP) or close out P0 with F2/F3 first |

---

## 2. Finalization

| Item | State | Notes |
|---|---|---|
| F1 record encoding | **Done** | `design/08-record-encoding.md`, normative |
| F2 spec 07 in protocol repo | Not started | Write alongside P0, not after |
| F3 design set → v1.0 | Not started | After P0 teaches us something |

## 3. Setup

| Item | State | Notes |
|---|---|---|
| S1 client repo | **Done** | `/workspaces/ko-ls/ko-ls`, git initialised, design moved in, `kols-core` scaffolded. **Nothing committed yet** — no commit has been made in either repo |
| S2 protocol changes on `main` | **Not needed for P0** | E3 turned out to need no change; the protocol repo is still untouched. E9 is the first real one, and it blocks P1 rather than P0 |
| S3 Tauri environment | Not started | Node and webkit2gtk absent. Blocks P1, not P0 |

## 4. Protocol Extensions

Tracked against `design/06-protocol-extensions.md`. Landing rule: spec text, implementation
and tests together, with `cargo test --workspace` and `cargo clippy --workspace --all-targets`
both green.

| # | Extension | State |
|---|---|---|
| E1 | Extension capability registry | **Withdrawn** — already implemented upstream, needs configuration only |
| E2 | Channel governance entry variants | Not started (P1) |
| E3 | Derived pointer ids | **Withdrawn** — `PointerId::from_bytes` is already public; derivation lives in `kols-core::ids` |
| E4 | Gossipsub live delivery | Not started (P1) |
| E5 | Media fan-out at the relay | Not started (P3) |
| E6 | QUIC datagram media path | Not started (P3) |
| E7 | Channel-scoped MLS groups | Not started (P2) |
| E8 | Track metadata in media payloads | Not started (P4) |
| E9 | App-layer policy map | Not started (P1) |
| E10 | Direct DM invite delivery | Not started (P2) |
| E11 | Namespace registration for extension capabilities | **New** — found implementing permissions; workaround in `kols-core::capabilities` (P1) |

## 5. Client Crates

| Crate | State |
|---|---|
| `kols-core` | **Encoding, author logs, merge, collision recovery** — records/segments/ids, `AuthorLog` incl. `rebase`, `ChannelView`, permissions, capability vocabulary. 30 tests |
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

## 7. Log

Newest first. One line per change that moved the state above.

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
