# Client Architecture

**Document status:** v1.1 — `kols-core`, `kols-net`, §3's boundary in both directions, an executor behind it and a first `kols-app`/`kols-ui` slice exist; the store and media do not
**Depends on:** all preceding documents; App Hosting Spec §1–§3 for the sandbox path
**Consumed by:** implementation; `09` for the interface built on §3's boundary

This document is the client's *architecture*. What the interface looks like and how it
behaves is `09` — including the one thing §1's diagram leaves open, that `kols-net` owns one
`MemberNode` **per network**, and a user belongs to several.

---

## 1. Shape

A standalone desktop application: a Rust core linking the `intranet-*` crates directly,
and an HTML/CSS/JS interface rendered in a webview, with **Tauri v2** as the shell.

```
┌─────────────────────────────────────────────────────────┐
│  kols-ui        HTML/CSS/JS in a webview                │
│                 holds no keys, no sockets, no files     │
└───────────────────────────┬─────────────────────────────┘
                            │  kols-api  (§3) — the only crossing
┌───────────────────────────┴─────────────────────────────┐
│  kols-core      domain: channels, records, merge order, │
│                 permissions, keys, retention, search    │
│  kols-store     SQLite projection + blob cache          │
│  kols-net       owns the MemberNode event loop          │
└───────────────────────────┬─────────────────────────────┘
                            │
              intranet-* crates (protocol v1.0)
```

**Why Tauri rather than a native Rust GUI.** The stated goal is to eventually run this
inside the app-bundle sandbox, and App Hosting §1.2 fixes that sandbox as a webview
running HTML/CSS/JS. A native GUI would mean writing the interface twice. Building on a
webview now means the sandboxed variant is a re-host of the same interface against a
narrower API (§7), not a second product.

**Why not Electron.** Same webview model, considerably heavier install, and an IPC
boundary to a separate Rust process instead of an in-process call. Tauri's boundary is
the one we want to define carefully anyway (§3), so getting it for free is worth more
than Electron's ecosystem advantage.

---

## 2. Crate Layout

| Crate | Owns | Deliberately does not own |
|---|---|---|
| `kols-core` | Record types and canonical encoding, merge ordering, permission resolution, channel/session key management, retention policy, local search | Any I/O, any libp2p type |
| `kols-store` | SQLite schema and queries, blob cache, migrations | Domain rules |
| `kols-net` | The `MemberNode` event loop, gossip subscriptions, publish/fetch scheduling, sync back-off | Domain rules, UI state |
| `kols-media` | Capture, encode, jitter buffer, playback, `MediaTransport` impls (`04` §5) | Signalling policy |
| `kols-api` | The command/event surface (§3) and its consent decorators | Anything else |
| `kols-app` | Tauri shell, window/tray/notifications, OS keychain, and the view shapes the webview receives | Domain rules |
| `kols-ui` | The interface | Everything above |

**`kols-app` converts rather than deriving.** The domain's records have exactly one
serialization and it is normative — spec 07 §3's canonical encoding, hand-written because a
record's id is the hash of those bytes. Putting `Serialize` on the same types would create a
second serialization living beside the first, and what that invites is not hypothetical:
somebody eventually sends the convenient one over a wire and finds that ids no longer match.
So the shell owns a set of view types, free to change whenever the interface wants something
different, because nothing verifies against them.

**The webview never builds a command.** It names an intent with plain arguments — a channel
id, a body — and the shell constructs the `Command`. That is not a weakening of §3's first
property, since the command still names its target and the gate still resolves permission by
replay; it is one fewer place where a front end can hand the core a shape it did not expect,
and it keeps `kols-api` free of `serde` entirely.

`kols-core` must stay I/O-free and deterministic. Merge ordering, permission resolution
and retention decisions are exactly the code that has to produce identical answers on
every node, and pure functions over explicit inputs are how that stays testable.

---

## 3. The API Boundary

Everything the interface can do is one of a fixed set of commands; everything it learns
arrives as an event. No shared memory, no callbacks holding protocol types, no key
material crossing in either direction, ever.

```
Command  = OpenChannel { channel_id, before: Option<Hlc>, limit }
         | SendMessage { channel_id, body, attachments, reply_to }
         | EditMessage | DeleteMessage | React | Pin
         | CreateChannel | UpdateChannel | SetPermission
         | JoinVoice { channel_id } | LeaveVoice | SetMute | SetDeafen
         | StartStage | PromoteSpeaker
         | Search { scope, query }
         | StartDirectMessage { with: identity, in_network }   — creates a network, `03` §4.3
         | AcceptDirectMessage | DeclineDirectMessage
         | InviteCreate | InviteRedeem | AdmitWaitingMember | RevokeMember
         | SetContribution { storage_offered, bandwidth_cap, relay_willing }
         | …

Event    = Records { channel_id, records }        — live and backfilled alike
         | ChannelState | PermissionsChanged | MemberPresence
         | VoiceState { participants, topology, transport: Delivery }
         | GovernanceReorg { voided: [Action] }    — §4
         | KeyStatus { channel_id, have_key: bool }
         | DirectMessageRequest { from: identity, link_verified: bool }
         | SyncProgress | Degraded { reason }
```

Three properties this boundary must hold, because the sandbox path (§7) depends on all
three and retrofitting any of them is expensive:

1. **No ambient authority.** Every command names its target explicitly; the core
   re-checks permissions on receipt rather than trusting that the interface only offered
   buttons the user was allowed to press. **Held by a type rather than by discipline**
   (D25): the gate returns an `Authorized`, which has no public constructor, so an
   executor takes one of those instead of a `Command` and being handed something nobody
   checked is not expressible. The compiler runs that claim as a `compile_fail` doctest.
   A channel's category is *looked up* from replayed state rather than carried on the
   command, for the same reason — whoever supplies the category chooses which grant
   applies.
2. **Consent is a decorator, not a redesign.** App Hosting §3.3 requires that any signed
   action on the user's behalf pass a platform-level prompt when the code is sandboxed.
   Commands are therefore tagged with a sensitivity class, and the sandboxed build wraps
   sensitive ones in a prompt. The native build does not prompt; nothing else differs.
   **The class follows the tier of the capability the command needs** (D26), not a
   judgement about consequence — the same rule spec 07 §3.8 settled for channel entries.
   A drift test resolves every command's verb against the vocabulary's own tier table, so
   re-tiering a verb and forgetting the classification fails there rather than in a prompt
   that quietly stopped appearing. A finer class is never a weaker one: prompting for
   everything above `Local` satisfies §3.3, and the finer grade only changes *how* it
   prompts.
3. **Events are idempotent and re-deliverable.** `Records` may arrive twice, out of order,
   or after a gap — the interface renders from the merged projection, never by appending
   what it just received. This falls straight out of `01` §7's rule that live delivery is
   an optimization. **It is the consumer's property, not the emitter's**, and it cannot be
   otherwise: a record pushed over gossip is *also* inside the segment that follows it, so
   duplicate delivery is the normal case. Merge — by record id, through `ChannelView` —
   never append.

**A command produces an `Outcome`, and the thing that produces it prints nothing.** An
executor that rendered would be one no interface could reuse, which is the whole reason this
boundary exists; `kols` renders the same values a webview would render differently. There is
one submit path — authorize, then run — and the `Authorized` never escapes it, because the
run step is what requires one and nothing else can produce one. So the check is not something
a future caller can be *asked* to remember.

**What the executor can answer that the gate cannot.** The gate reaches no store, on purpose,
which leaves two questions to the layer that does: whether an edit targets a message this
member wrote (a fact about the record set, not about replayed state), and the rate ceiling
(computed over the author's own HLC readings — §10.2 of `01`). Both are refused before
anything is signed. Neither is *enforcement*: nobody can write into another author's log, and
readers refuse an over-rate record whatever the writer believed. This is the author's client
telling them first, which is the division `01` §10.2 draws.

**The event vocabulary was written from the engine rather than ahead of it.** The sketch above
has nine variants; what exists has six, and each has something producing it — §4's loop had
been reporting all of them in words for weeks. Two categories are deliberately excluded: this
node's transport, because a sandboxed build gets no ambient host access (App Hosting §3.2),
and the startup report, because that is what the node *is* rather than something that happened.

**What is still designed rather than built.** The commands for direct messages, search, voice
and stage, each of which has a line above and no code behind it. `STATUS.md` §6 tracks those,
along with the fact that events currently reach a terminal and no projection-holding client —
which is a missing consumer rather than a missing contract.

---

## 4. Sync Engine

`kols-net` runs one `MemberNode` per joined network on a single-threaded event loop —
and because a direct message conversation *is* a network (`03` §4), a user with fifty
conversations is running fifty of them. They are cheap (two members, no relay duty, a
governance log of a handful of entries) but they are not free, so node lifecycle is a
real concern: idle DM networks should be suspended and woken on demand rather than all
held live. *Flagged: the suspend/wake threshold wants measurement, not a guess.*

Each loop, DM or server, is structured identically, with the domain layer talking to it
through channels. Care is needed with one implemented invariant: `next_swarm_event` drains its `pending` queue only on entry, so an event pushed
from inside the loop is delivered on the *next* call — which never comes if nothing else
happens. The crate's own guidance is to push to `pending` only from an arm that returns
immediately. The client must respect that rather than rediscover it.

Loop responsibilities, in priority order:

1. **Live** — gossip subscriptions for open and notified channels (`01` §7).
2. **Head sync** — resolve author-log pointers for visible channels; delta-fetch head
   segments.
3. **Backfill** — walk `previous_segment` chains on demand, bounded concurrency. Landed
   in `kols serve`, which walks to the start of history because it has no scroll
   position to bound it; a UI bounds this by pages instead (`01` §5). Each hop resolves
   its own key, since each segment lives under its own pointer (`01` §3.1.0) — a hop whose
   wrapping this node cannot open is where the walk ends, and is also what reading past a
   retention boundary looks like.
4. **Publish** — seal open segments on the size/age thresholds (`01` §3.1); publish
   immediately when the user goes idle, so a message is never stuck unpublished behind a
   half-full segment.
5. **Maintenance** — governance sync, ledger gossip, DEK re-wrapping after rotation,
   participant-index and search-posting re-announcement inside their TTLs.

Two protocol behaviours the client is specifically obliged to act on rather than log:

- **The voided-actions report** (Core §2.7.1 point 5). When reconciliation voids an
  action this node submitted, the client must resubmit or prompt. This matters most for a
  voided revocation, where the alternative is a removed member quietly becoming current
  again because nobody was assigned to notice. It surfaces as `GovernanceReorg`.
- **Kademlia server mode.** Provider records go unanswered while a node is in client mode,
  which is the default until it has a confirmed external address — on a LAN or loopback
  every lookup returns nobody. `set_dht_server_mode(true)` where appropriate, and expect
  publicly addressable nodes to carry records in production.

**Ledger before fetch.** A holder that has not advertised upload capacity is dropped by
source selection as not having volunteered, so the capability ledger must be populated
before a fetch can use a source the DHT found. Layering is governance, then ledger, then
fetch — a fetch that finds nothing on a fresh node is usually this, not a bug.

---

## 5. Local State

**SQLite** holds a projection, never the source of truth: messages (id, channel, author,
HLC, body, flags), reactions, channel and role state from replay, read watermarks,
attachment cache metadata, and the local FTS index (`03` §6 — the only search available
for private channels). It can be deleted and rebuilt from the network.

**Blob cache** holds fetched chunks, which is simultaneously how this node participates in
swarm serving (Storage §4.2): anything fetched makes this node a source. That should be
visible in settings, with a size cap the user sets, because it is their disk.

**Keys** live in the OS keychain where one exists, with each network's seed encrypted at
rest under a user passphrase. Per network rather than one master seed (`02` §6.3), so the
thing a passphrase protects is a set rather than a single object that links every identity
its holder has. `intranet-*` key types implement no `Debug` and no
serialization deliberately — use their `fingerprint()` methods for logging and tests, and
do not derive around it.

**Two UI honesty requirements**, carried from the guarantees the protocol actually makes:

- **Deleted means hidden, not unsent** (`01` §6). The confirmation dialog says so.
- **Retention is not deletion** (`01` §8). The setting says so.

Nothing in the interface should imply the system can retract bytes somebody already has.

---

## 6. Multi-Device (Designed Now, Built Later)

Core §1.3 is already implemented: devices are independently seeded and linked by
certificates recorded in the governance log; revoking a device does not rotate the
identity. The client ships single-device but must not make choices that block this:

1. **Every record carries the signing device** (`01` §3.3), so a device revocation can
   invalidate that device's records without touching the identity's other work.
2. **Read state is keyed by (device, channel)** from day one (`01` §9), so cross-device
   sync later is a merge of watermarks rather than a schema change.
3. **Nothing assumes a single writer per identity per channel.** An author log is
   single-writer by *identity*, not by device, so two devices appending concurrently
   would collide on the pointer version. The protocol resolves that deterministically —
   lower record hash wins, loser retries at an incremented version (Storage §2.2) — but
   the loser's records must be re-published, not dropped. Build the publish path with
   that retry from the start; discovering it during a P5 rewrite is the expensive way.
4. **A network's seed stays on as few devices as possible.** Additional devices get
   certificates, not the seed. Enrollment is per-network (Core §1.3 point 7) — which is now
   the shape of the secret as well as the shape of the enrolment, so the UX is "add this
   device to these servers" rather than one global action, and there is no single object
   that would grant all of them at once.

---

## 7. The Sandbox Path

The eventual goal is running this same interface as an in-network `app-bundle`, rendered
by a generic protocol-aware client. What that requires, and what it does not:

**Already satisfied by this architecture:** the interface is HTML/CSS/JS; it holds no
keys; every privileged action crosses a narrow named API; consent is a decorator.

**Required from the platform, not from us:**
- An execution sandbox meeting App Hosting §3.2 — origin isolation per `app_id`, no
  ambient host access, platform-enforced CSP. **This is deliberately outside the
  protocol** (§3.2.1): nothing in `intranet-app` will tell a client an app is safe to
  run. A client that executes an `app-bundle` without its own sandbox has skipped a step
  the protocol was never in a position to take.
- Enforcement of `network-storage-read`/`write` (supported in v1) and `realtime-media`
  (declared, not yet enforced) — the latter is precisely the extension point a
  communications app needs, and it does not exist yet.

**What the sandboxed build gives up**, and why the desktop build stays the primary
target for now: no OS keychain, no tray or native notifications, no direct audio device
access without `realtime-media` being enforced, and a webview it does not control. Voice
in particular is unlikely to work in the sandbox until that capability is real.

The honest framing: **the sandbox build is a P5 packaging target for text chat, not a
replacement for the desktop client.** Designing for it now costs a disciplined API
boundary, which is worth having regardless.

---

## 8. Testing

| Layer | Approach | State |
|---|---|---|
| `kols-core` | Property tests on merge ordering: any permutation and any partition of a record set must converge to the same rendered history. This is the correctness claim of the whole design | **Done** — 40 permutations, reversal, duplicates, two-sided partition |
| Encoding | Frozen vectors, round-trip, injectivity, domain separation, id stability (`08` §3) | **Done**, including on big-endian via `scripts/cross-check.sh` |
| Wire | Two live nodes: publish, pointer sync, fetch, reassemble, render identically | **Done** — plus the delta measurement between fetch rounds |
| Permissions | Table-driven cases over replayed governance states, including the tricky ones — frozen pointers after a narrowed grant, waiting-room members, voided revocations | Partial — non-member, forged signature and wrong-channel covered at the reader; both post gates, channel/category/network scope and governance tier covered at the boundary; frozen pointers and waiting-room members not |
| API boundary (§3) | Every command against a replayed log, not a hand-built state: a grant reaches the scope it names and no further, an ordinary verb does not buy a governance-tier one, and the consent class agrees with the vocabulary's tier table | **Done** for the command half — 18 cases, plus a `compile_fail` doctest for the unforgeable token. Nothing for events, which do not exist |
| Executor | Every record kind end to end through the real binary: the record is written, the merge renders it, and the refusals the gate cannot make — somebody else's message, the rate ceiling — happen before anything is signed | **Done** — 10 cases over a keyed node |
| Events (§3, property 3) | A consumer that merges rather than appends: the same event twice, events out of order, a gap filled later, and the case that actually happens — a record arriving live and again inside the segment behind it | **Done** — 5 cases. The daemon's own wording is asserted by the two-node tests, which is what makes the emitter's refactor behaviour-preserving |
| Keying | A removed member must fail to decrypt content wrapped after the rotation, and must still decrypt what they held. Assert the honest guarantee, not a stronger one | Not started (P2) |
| Multi-node | Extend the existing Docker NAT harness with chat scenarios: partition two members over a real network, heal, assert identical history | Not started — the in-process partition test is not this |
| Media | Loss and jitter injection against both `MediaTransport` impls; the fallback is expected to degrade badly and the test should record how badly, not skip it | Not started (P3) |

The protocol repo's gate applies to this work too: `cargo test --workspace` and
`cargo clippy --workspace --all-targets` both stay clean, and a run that skipped clippy
because the toolchain lacked it has checked half the gate and should say so.

**One measurement discipline learned in P0, worth keeping.** A test that asserts on bytes
moved must measure at the point the cost is actually incurred. The first version of the
wire test ran both fetch rounds — manifest, then chunks — and then asked what the reader
still wanted, which is zero by construction and proves nothing. The number that matters is
what it wanted *between* the rounds. A green test that asserts nothing is worse than no
test, because it is believed.
