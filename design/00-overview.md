# ko-ls — Design Overview

**Project:** ko-ls (working name, may be replaced; nothing in the design depends on it)
**Document status:** v1.0 — reviewed against a working P0 implementation.
**Precedence:** `distributed-intranet/specs/07` is normative where it and this set overlap; this set owns client design, rationale and sequencing.
**Depends on:** Distributed Intranet Protocol v1.0 (specs `01`–`06`) and the Chat Application Spec (`07`)
**Consumed by:** every other document in this set

---

## 0. Purpose and Scope

This document set specifies a Discord-style chat application built on the Distributed
Intranet protocol as its networking, identity, storage and real-time backend. It covers
text chat, voice chat, and — later — video and screen share, delivered as a standalone
desktop application with a deliberate path toward running as an in-network published
app-bundle once that execution environment exists.

**The protocol specs are authoritative and this set is subordinate to them.** Where this
design needs something the protocol does not provide, it says so explicitly and the
required change is recorded in [`06-protocol-extensions.md`](06-protocol-extensions.md)
rather than assumed into existence. Where a protocol guarantee is weaker than a user
would expect from a mainstream chat product, this design states the weaker guarantee
rather than papering over it — the same posture the protocol specs take with revocation
(Core §3.1), VOD opt-out (Real-Time §4.2) and swarm-serving convergence (Storage §5.4).

### Document map

| Document | Owns |
|---|---|
| `00-overview` | Concept mapping, principles, decision register, roadmap |
| `01-messaging-model` | Channels, messages, ordering, edits, history, live delivery |
| `02-membership-and-permissions` | Roles, capabilities, channel permissions, moderation, onboarding |
| `03-confidentiality` | Keying tiers, private channels, direct messages, search implications |
| `04-realtime` | Voice, video, stage broadcast, media transport |
| `05-client-architecture` | Desktop client structure, the API boundary, local state |
| `06-protocol-extensions` | Every change required in `distributed-intranet`, with acceptance criteria |
| `07-build-plan` | What remains before code: spec text, repo and environment setup, P0 |
| `08-record-encoding` | Conformance obligations and module map. The encoding itself moved to spec 07 §3 |

---

## 1. Concept Mapping

The single most consequential design decision is what a Discord "server" maps to. It maps
to a **network**, and almost everything else follows from that.

| Chat concept | Protocol concept | Notes |
|---|---|---|
| Server / guild | One network | One governance log, one membership, one epoch key chain, one DHT namespace, one search index |
| Member | Per-network identity (Core §1.2) | Distinct keypair and PeerId per server, by construction |
| Role | Governance group (Core §2.1) | Flat, no nesting — matches Discord's flat role model exactly |
| Permission | Capability (Core §2.2), mostly extension capabilities | See `02` |
| Channel | App-layer object, defined in the governance log | See `01` §2 |
| Message | A signed record inside an author's per-channel log | See `01` §3 |
| Attachment | Ordinary content object (Storage §1) | Fetched by swarm serving |
| DM | **Its own two-person network**, `conversation` profile — no channels, no roles | See `03` §4 |
| Group DM | **Its own network too**, distinct from every pairwise conversation among the same people | See `03` §4.4 |
| Friends list | The set of `conversation`-profile networks this client belongs to | See `03` §4.6 |
| Voice channel | A long-lived call session bound to a channel id | See `04` |
| Membership screening | Explicit-intake waiting room (Core §2.4) | Already implemented |
| Ban | `revoke-node` + epoch rotation (Core §3.3) | Already implemented |
| Message delete by moderator | Redaction record, validated on read | See `01` §6 |

### What this mapping costs, stated up front

- **There is no cross-server identity, and there cannot be.** Per-network derivation
  (Core §1.2) exists specifically so two of your memberships cannot be correlated. A
  user has one master seed and one account *from their own point of view*, but no other
  member can tell that your identity here and your identity there are the same person
  unless you deliberately prove it. There is no global username and no global profile.
  Messaging someone you met in a server works by *deliberately* proving it, once, to that
  one person (`03` §4.3) — which is the mechanism Core §1.2 provides for exactly this.
  This is the protocol working as designed, not a gap to close.
- **There is no server list to browse.** No cross-network discovery exists (Core §0).
  Joining is invite-driven, always.
- **There is no operator.** Every moderation and administration action is a governance
  action taken by a capability holder and recorded in a log every member can replay.
  There is no support channel to appeal to above the network's own members.

---

## 2. Design Principles

These are inherited from the protocol and restated because they decide arguments later.

1. **The durable path is the source of truth; the live path is an optimization.** Every
   message reaches every member through content-addressed storage. Gossip delivery makes
   it arrive in milliseconds instead of seconds. A client that ignored the live path
   entirely would see identical history. Nothing about correctness may depend on gossip.
2. **Fail closed.** Unknown record types, unverifiable signatures, unresolvable
   permissions and missing keys produce refusal, never a degraded render.
3. **Authorization is a computation.** "May they post here?" is answered by replaying the
   governance log, never by asking a peer or trusting a cached claim.
4. **Determinism where it is load-bearing.** Message ordering, channel id derivation,
   pointer id derivation and permission resolution must produce identical results on
   every node. Where they cannot (clock skew), the design says so.
5. **Honest guarantees.** Deletion hides a message from conformant clients; it does not
   un-send bytes. Retention stops re-wrapping keys; it does not erase copies. Private
   channels are private from non-members, not from their own members.
6. **The UI never holds a key.** Every privileged action crosses a narrow API boundary
   (`05` §3). This is what makes the eventual sandboxed variant a re-host rather than a
   rewrite.

---

## 3. Decision Register

Decisions taken in this design pass, with the reasoning compressed. Each is expanded in
the document named.

| # | Decision | Where |
|---|---|---|
| D1 | One network per server; channels are app-layer, not networks | `00` §1 |
| D2 | Messages live in **per-(channel, author) segment logs** — one mutable pointer per author per channel, chained immutable segments behind it | `01` §3 |
| D3 | Pointer ids are **derived**, not random: `H("chat-log" ‖ channel_id ‖ author_id)`, so any member can locate any author's log without a directory | `01` §3.2 |
| D4 | Channel definitions are **governance-log anchored** (durable, ordered, non-lapsing), with an append-set as a best-effort browse index — the App Hosting §4.3 pattern | `01` §2 |
| D5 | Ordering is **hybrid logical clock + lower-record-hash tie-break**; per-author order is exact, cross-author order is skew-bounded | `01` §4 |
| D6 | Live delivery uses **gossipsub, one topic per channel**, subscribed on demand | `01` §7 |
| D7 | Every record is **individually signed**, so live and durable paths carry identical, independently verifiable records | `01` §3.3 |
| D8 | Retention and joiner-history are **two orthogonal network policies**, which is what lets all three of the requested history models exist | `01` §8 |
| D9 | Private channels get a **per-channel MLS subgroup**, not a key-wrapped-per-member blob — same mechanism the network already uses for itself | `03` §3 |
| D10 | A **DM is its own two-person network**, not a channel — so its content only ever lands on the two nodes having the conversation, and nothing about it burdens a shared log | `03` §4 |
| D11 | Private-channel content is **never announced to the network search index** by default | `03` §6 |
| D12 | Permissions bind at **category** scope by default, per-channel only as an override, to keep capability sets small at scale | `02` §4 |
| D13 | Moderator deletion is a **redaction record in the moderator's own log**, validated on read — not a governance log entry per deleted message | `01` §6 |
| D14 | Voice media is designed against an **unreliable datagram interface**, shipped initially over the existing reliable fallback | `04` §5 |
| D15 | Large voice/stage is the **live-stream** path (HRW tier), not the call path | `04` §4 |
| D16 | Client is **Tauri v2 + Rust core**, with a capability-shaped API boundary between them | `05` |
| D17 | Multi-device is **designed now, built later** — records are device-attributed from day one | `05` §6 |
| D18 | Abuse limits are **network policy with shipped defaults**, changed by `define-policy` holders; per-channel slowmode is a separate, delegable knob on the channel definition | `01` §10 |
| D19 | DM delivery is **asynchronous with overlap** — send to an offline contact, delivered at the next mutual online window; three honest states (sent / delivered / read) | `03` §4.5 |
| D20 | Two **network profiles**, declared at genesis and enforced on replay: `server` (channels, roles, categories) and `conversation` (one implied channel, nothing else) | `03` §4.1 |
| D21 | A **group conversation is its own network**, distinct from every pairwise one among the same people, with **every participant a Founder** | `03` §4.4 |
| D22 | The segment record list carries **no count prefix** — the one departure from the project's framing rule, because a count at the head re-chunks the whole object on every append. *Found by measurement in P0* | spec 07 §3.5 |
| D23 | HLC strictness is per **(author, device)**, not per author — two devices cannot share a counter without a lock across machines, and a merged segment interleaves them | spec 07 §2.6 |
| D24 | A **losing pointer version republishes the union** of both record sets, discarding nothing — the content-merge semantics Storage §2.2 defers to the application layer | `01` §3.1.1 |

---

## 4. What Scales, and What Breaks First

Initial use is 5–50 members per network; the design must reach thousands. Rather than
claim it scales, this records what is expected to break first and where the mitigation
already sits in the design.

| Pressure at scale | First failure | Mitigation in this design |
|---|---|---|
| Many authors per channel | Fan-in: resolving N author pointers to render one channel | Live path carries recent messages; history backfills lazily and bounded (`01` §5) |
| Many channels | Capability set size per group grows with channel count | Category-scoped permissions (`02` §4); design target is hundreds of channels, not thousands (§6) |
| Long-lived network | Governance log replay cost for joiners | Only *structure* enters the log — never messages, never threads, never DMs. Checkpointed replay is the valve if it is ever needed |
| Presence | Network-wide presence gossip is O(members) per heartbeat | Per-channel presence, opt-out, coarse states (`01` §9) |
| Private channels | One MLS group per private channel | Bounded by channel size, not network size — this is why D9 beats per-member key wrapping |
| Many conversations | Each DM is a whole network with its own log and keying | Paid by the two participants only; no shared network carries any of it (`03` §4) |
| Large voice | Relay bandwidth, O(n) key envelopes | Stage mode switches to live-stream distribution (`04` §4) |

**Two of these are now measured; the rest are not, and the design does not pretend
otherwise.** An append moves 1,556 bytes of 176,115 locally and one chunk of three across
the wire (`08` §4), which is what makes per-author fan-in affordable. Segment sealing
thresholds, gossip fanout, backfill depth and presence cadence remain unmeasured beyond a
single spike, and want data from a real deployment rather than another guess.

---

## 5. Roadmap

Phases are ordered so that each one is independently usable and each one de-risks the
next. Estimates are deliberately absent — sequence is the useful part.

- **P0 — Spike. ✅ Complete.** Proved the author-log model against the real storage layer
  and across two live nodes: delta-fetch, merge convergence under permutation and
  partition, reader-side refusal, and pointer-collision recovery. It also falsified two
  things the design had wrong (D22, D23), which is what it was for.
- **P1 — Text chat MVP.** Channels and categories, roles and permissions, live gossip
  path, history backfill, edits/deletes/reactions/threads, invites and onboarding, the
  Tauri desktop client. Public channels only.
- **P2 — Private channels and direct messages.** Per-channel MLS subgroups and channel
  membership entries; the DM flow (network creation, direct invite delivery, voluntary
  identity link) and a DM inbox spanning networks; moderation redactions; search — network
  index for public channels, local index for private.
- **P3 — Voice.** Call sessions bound to voice channels, mesh and relay topologies,
  Opus, jitter buffer, the datagram-shaped media interface over the reliable fallback.
- **P4 — Video and stage.** Video and screen-share tracks; stage/broadcast mode on the
  live-stream path; VOD of recorded stages falls out of Real-Time §4 for free.
- **P5 — Multi-device and sandbox packaging.** Device enrollment UX, cross-device read
  state, and the app-bundle build of the same UI against a consent-decorated API.

---

## 6. Open Questions

No architectural questions remain open. What is left is implementation-level tuning, and
it is listed where it belongs: media rate control, stage mixing limits and echo
cancellation in `04` §8, and the measurement list in §4 above.

**Resolved since the first draft:**

- *Governance log growth from channel definitions* — the design target is **hundreds of
  channels, comfortably**, not thousands. A few thousand structural entries is under a
  megabyte and a first-join replay measured in milliseconds; discomfort starts around
  100,000 entries, where signature verification alone adds seconds. Channels never got
  close to that. What would have was DMs, and D10's revision removes them from the log
  entirely. Threads never write to it either, being derived (`01` §2.2). The remaining
  pressure valve, if a real network ever needs it, is checkpointed replay, which Core
  §2.7 already permits — worth measuring before building.
- *Naming* — **ko-ls**, working name, replaceable.

- *Message rate ceiling* and *attachment limits* — both are network policy with shipped
  defaults, set and changeable by the network's founder via `define-policy`, with
  per-channel slowmode delegable separately to channel managers (D18, `01` §10). The
  defaults are starting values expected to be revised from real usage, in the same spirit
  as the protocol's own tunables — but they are concrete now, so implementation has
  something to enforce rather than a gap to invent around.
