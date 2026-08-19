# ko-ls — Implementation Status

**Updated:** 2026-08-19
**Phase:** P0 (spike) — encoding landed, storage round-trip next
**Design:** [`design/`](design/) — `00`–`08`. `08-record-encoding.md` is normative.

This file is the answer to "where are we?". It is updated in the same change that moves
work, never afterwards from memory — a status file that lags is worse than none, because
it is believed.

---

## 1. Right Now

| | |
|---|---|
| **Working on** | `kols-core` — encoding done; next is the storage round-trip P0 needs |
| **Blocked on** | Nothing |
| **Runnable** | `cargo test` in this repo — 13 tests, clippy clean. No binary yet |
| **Next decision needed from the user** | None outstanding |

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

## 5. Client Crates

| Crate | State |
|---|---|
| `kols-core` | **Encoding complete** — `Hlc`, `ChannelId`/`MessageId`, all six record kinds, `Segment`, every derived id from `design/08` §7. 13 conformance tests incl. frozen vectors |
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
| 1 | 100 messages across sealed segments render in identical order on both nodes | Not started |
| 2 | Appending one message transfers **only new tail chunks**, asserted on bytes moved | Not started |
| 3 | Partition, both post, heal, converge byte-identically | Not started |
| 4 | Records from an identity without `publish:chat-log` are refused by the *reader* | Not started |
| 5 | Pointer version collision resolves by lower record hash, loser re-publishes | Not started |

Criterion 2 is the one P0 exists for. If it fails, the segment model is wrong and the
design changes before anything else is built.

---

## 7. Log

Newest first. One line per change that moved the state above.

- **2026-08-19** — `kols-core` encoding implemented against `design/08`: records, segments,
  HLC, derived identifiers. 13 tests green (round-trip, injectivity, domain separation, id
  stability, rate-class, bounds), clippy clean. Test vectors frozen — a change to one is a
  wire break, not a value to re-bless.
- **2026-08-19** — E3 withdrawn: deriving pointer ids needs no protocol change, so **P0
  requires no change to `distributed-intranet` at all**. That repo remains untouched.
- **2026-08-19** — S1: client repo created, design docs moved in, `kols-core` scaffolded.
- **2026-08-19** — F1 done: canonical record encoding pinned in `design/08-record-encoding.md`.
- **2026-08-19** — Design set `00`–`07` drafted and reviewed across four passes; 21 decisions recorded.
