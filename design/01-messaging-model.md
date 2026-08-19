# Messaging Model

**Document status:** v1.0 — implemented in `kols-core`; spec 07 is normative where they overlap
**Depends on:** Core Protocol Spec §2 (governance log), Storage Spec §1–§5, Search Spec §3
**Consumed by:** `02-membership-and-permissions`, `03-confidentiality`, `05-client-architecture`

---

## 0. The Problem This Document Solves

Chat is high-frequency, small-payload, multi-writer, append-mostly, and expected to
retain an ordered history that anyone can scroll indefinitely. The storage layer's unit
of work is none of those things: a publish means chunking, encrypting, hashing, a pointer
version bump, a DHT provider announcement and a set of search postings. Doing that once
per message is not merely inefficient — at a few messages per second it would saturate
the DHT with provider records for objects smaller than the records describing them.

Neither storage primitive fits directly:

- A **mutable pointer** (Storage §2.2) is single-writer by construction. A channel has
  many writers.
- A **distributed append-set** (Storage §2.5) is multi-writer, but its entries lapse when
  nobody re-announces them. Storage §2.5 warns explicitly that this model is wrong "for
  anything where losing an entry due to the publisher's node being offline would itself
  be a problem". A message vanishing because its author took a holiday is exactly that.

The resolution is to stop trying to make a channel a single object. **A channel is a set
of single-writer logs that readers merge.**

---

## 1. Structure at a Glance

```
network (a "server")
└── channel  (governance-anchored definition; id derived at creation)
    ├── author log  A   ── mutable pointer ──▶ segment N ──▶ segment N-1 ──▶ … (CID chain)
    ├── author log  B   ── mutable pointer ──▶ segment M ──▶ …
    ├── author log  C   ── …
    └── moderation log  (per moderator, same shape, carries redactions)

reader view of a channel = merge(all author logs) − redactions, ordered by HLC
```

Each author owns exactly one log per channel, writes only to it, and can write to nobody
else's. Every property below falls out of that.

---

## 2. Channels

### 2.1 Definition Record

A channel is defined by a governance log entry — `EntryBody::ChannelDefinition`, a new
variant (`06` §2). This follows App Hosting §4.3's correction exactly: an append-set
alone cannot supply trustworthy ordering (a squatter can backdate) or durability (a
registration lapses when unrefreshed), and a channel's name, privacy flag and permission
bindings need both.

```
ChannelDefinition {
  channel_id:    derived from network id and a nonce — Chat Application Spec §3.6 is normative
  name:          human-readable, unique within the network at definition time
  category:      Option<CategoryId>                          — permission scope, §4 of `02`
  kind:          Text | Voice | Stage
  privacy:       Public | Private                            — see `03`
  topic:         short description, indexed (Search §2.1)
  slowmode:      seconds between one author's messages, 0 = off — §10.3
}
```

Renames, re-categorisation, archival and deletion are further governance entries against
the same `channel_id`; current state is whatever replay produces.

**Creating and changing are gated differently, and deliberately** (spec 07 §3.8): the
definition itself needs `chat:create-channel`, which is ordinary, while every later change —
and every roster or rotation entry — needs `chat:manage-channel`, which is governance-tier.
A definition grants nobody access to anything, because a new private channel's roster is
empty until a membership entry fills it; the tier follows what an action can widen rather
than how consequential it feels. `kols-core::channel` implements the mapping and refuses an
entry declaring the wrong one.

**All of this applies to `server`-profile networks only.** A `conversation`-profile
network (`03` §4.1) has exactly one channel, its id derived as
spec 07 §3.6 specifies, declared nowhere — and a `ChannelDefinition` entry
in such a network is **invalid and rejected on replay**, so the distinction is enforced
rather than merely presented. Everything else in this document — segments, ordering,
edits, reactions, retention, the live path — applies identically to both profiles, since
a conversation is a channel like any other once you are inside it.

**A best-effort append-set mirrors every definition** to
`collection_id(network_id, "chat:channels")` (spec 07 §3.6), purely so a client can enumerate
channels without walking the log. It is never authoritative — if it disagrees with
replay, replay wins, and a missing entry costs a slower listing, not a lost channel.

### 2.2 Threads

A thread is a channel whose id is **derived, not declared**:

```
thread_channel_id = H(domain ‖ parent_channel_id ‖ root_message_id)      — spec 07 §3.6
```

It inherits the parent's privacy, keying and permissions, and needs no governance entry
at all — the first reply creates it implicitly, and its existence is discoverable from
the root message. This keeps a busy server from writing a governance entry every time
somebody replies in a side conversation, which is exactly the log-growth pressure `00` §6
flags as the risk with D4.

---

## 3. The Author Log

### 3.1 Segments

An author's contributions to one channel are stored as a **chain of segment objects**.
A segment is an ordinary content object (Storage §1): plaintext-chunked with CDC, each
chunk encrypted deterministically under the object's own DEK, content-addressed.

```
Segment {
  channel_id
  author:            per-network identity id
  sequence:          monotonic, per (channel, author)
  previous_segment:  Option<Cid>          — hash chain backward through history
  records:           [Record]             — append-only within this segment
}
```

Two thresholds close a segment and start the next one, whichever comes first:

- **size**, target 4 MiB of records — bounds the cost of re-chunking on append; and
- **age**, target 24 hours — bounds how much history is trapped behind one object for
  retention purposes (§8).

Both are **local publishing tuning**, not validity rules — a node that seals early or
late is not producing invalid history. The one network-wide bound is
`chat:segment-max-bytes` (§10.1), which readers enforce: a segment above it is refused,
so no author can compel every reader to fetch an arbitrarily large object. The local
target sits well below the network bound on purpose.

Within an open segment, appending a record republishes the *same object* under the same
DEK. Everything ahead of the record list is fixed-width and the list itself carries no
count prefix (spec 07 §3.5), so an append changes only the tail — measured at **1,556 bytes
moved out of 176,115** for one message appended to a full segment. Because chunking happens on plaintext before deterministic encryption (Storage
§1.2), every chunk except the tail re-derives to an identical CID, so readers delta-fetch
only what is new. This is the single property that makes the model affordable, and it is
also why the DEK must stay fixed for the object's lifetime — which it does by
construction (Storage §1.2).

**Why segments rather than one ever-growing object:** re-chunking cost grows with object
size, manifests grow without bound, and — decisively — retention (§8) needs to drop old
history, which means old history must be *separate objects with separate DEKs*. A single
object cannot be partially forgotten.

### 3.1.1 Two Writers, One Log

Two devices of one identity can publish the same author log concurrently — neither has
seen the other, both publish at the same version. Storage §2.2 settles which *record* is
canonical (lower record hash) and then says plainly that it supplies no content-merge
semantics, leaving what the two versions mean together to the application layer. This is
that decision.

**The loser adopts the winner's pointer and republishes the union at the next version.**
Nothing is dropped. Merging is a union by record id followed by a sort, with no field-level
conflict to invent a rule for, because a segment is a set of independently signed,
content-addressed records rather than a document. The loser's records were validly
published and their author has no way to learn they lost except by being told, so
discarding them would be the one unacceptable outcome.

The winner's chunk set is **recomputed locally rather than fetched** — chunk encryption is
deterministic per (chunk, DEK), so re-encoding the winning segment reproduces the winner's
exact chunks.

**What a rebase costs depends on where the loser's records sort**, and this is worth
knowing before it surprises someone. A segment is prefix-stable, so a loser whose records
sort *after* the winner's re-uploads only the tail — measured at 15,901 of 164,956 bytes
for one late record against 600. A loser whose records interleave rewrites from the first
interleaved point onward. A device returning after an absence is therefore the cheap case,
and two devices writing simultaneously the expensive one; neither loses anything.

### 3.2 Locating a Log Without a Directory

Each author log has exactly one mutable pointer, whose id is **derived**:

```
pointer_id = H(domain ‖ channel_id ‖ author_id)                          — spec 07 §3.6
```

`owner_identity` is the author. `content_type` is `chat-log`, which the network's
allowlist must include and which `publish:chat-log` gates (Core §2.8's two gates).

Derivation, rather than a random id plus a registry, buys the property that matters at
scale: **any member can compute where any other member's messages in any channel would
live, from public information alone.** Channel reading therefore has an authoritative,
complete enumeration path — for each member in replayed governance state, compute the
pointer id and ask for it — with no dependence on any index being fresh or complete.

That path is O(members) and too slow to use on every channel open, so a **best-effort
participant index** sits in front of it: an append-set at
`collection_id(network_id, "chat:authors:" ++ hex(channel_id))` (spec 07 §3.6) where an author
announces having posted here. Stale or missing entries cost a slower first load, never a
lost message — which is precisely the durability property Storage §2.5 says an append-set
must not be relied on for, and precisely why it is not relied on here.

*Flagged: `intranet-storage` currently mints pointer ids randomly (`new_pointer_id`).
Derived construction is an additive change — `06` §3.*

### 3.3 Records

```
Record =
  | Message   { id, hlc, reply_to: Option<MessageId>, body, attachments: [Cid] }
  | Edit      { id, hlc, target: MessageId, body }
  | Tombstone { id, hlc, target: MessageId }               — author deleting their own
  | Reaction  { id, hlc, target: MessageId, key, remove: bool }
  | Pin       { id, hlc, target: MessageId, remove: bool }
  | Redaction { id, hlc, target: MessageId, governance_head }   — moderator logs only, §6

every record additionally carries: author, device, signature
message_id = H(canonical record bytes)
```

**The canonical bytes are normative and specified in Chat Application Spec §3** (`distributed-intranet/specs/07`), not left to an implementation — every reference below is that hash, so encoding drift means references silently failing to resolve.

**Every record is individually signed**, over its canonical encoding, by the author's
per-network identity key (with the signing device recorded — `05` §6). This is
redundant for the durable path, where the pointer's signature over the segment CID
already authenticates everything transitively. It is not redundant for the live path
(§7), which delivers records before any segment containing them exists.

The payoff is an invariant worth stating plainly: **a record delivered live and the same
record read out of a segment months later are byte-identical and independently
verifiable.** The live path can therefore be lossy, out of order, or entirely absent
without changing what a reader converges on. It also means a segment is self-verifying
even when the reader holds a stale pointer.

Cost: 64 bytes of signature per record, and one verification per record. At chat volumes
this is not the bottleneck; the alternative — trusting a delivery path — is not
acceptable under a fail-closed design.

### 3.4 Attachments

An attachment is an ordinary content object of its own (`content_type: image`, `video`,
`audio`, `file`), published independently and referenced by CID from the message record.
It is fetched by ordinary swarm serving, gets rarest-first multi-source fetch for free
(Storage §4.4), and is keyed exactly like the channel it was posted in (`03`).

Attachments are *not* embedded in segments. A 20 MB video inside a segment would drag
that segment's whole chain into every reader's delta-fetch and would defeat the
size-threshold reasoning in §3.1.

---

## 4. Ordering

Ordering is the part of any distributed chat system where honesty is cheap and
overclaiming is expensive.

**Each record carries a hybrid logical clock:** `hlc = (wall_millis, counter)`, advanced
per the standard HLC rule against the maximum HLC the author has observed in that
channel. Merge order across all logs in a channel is:

1. ascending `hlc.wall_millis`, then
2. ascending `hlc.counter`, then
3. **ascending record hash** — the same lower-hash-wins tie-break the protocol already
   uses for sibling governance entries (Core §2.7.1) and for pointer version collisions
   (Storage §2.2). No new rule, and any two nodes holding the same record set compute
   the same order.

What this delivers, precisely:

- **Per-author order is exact.** An author's own records are totally ordered by their own
  monotonic clock, and their log is a hash chain, so gaps are detectable.
- **Causal order across authors is respected where it was observed.** Because HLC
  advances against observed clocks, a reply that was written after seeing a message sorts
  after it, regardless of wall-clock skew.
- **Concurrent messages have an arbitrary but universally agreed order.** Two people
  typing at once get a deterministic order that no node disputes and that neither of them
  can control by lying about the time.

**Clock-skew defence.** A record whose `wall_millis` is more than
`chat:max-future-skew-millis` (§10.1, default 5 minutes) ahead of the receiver's clock is
**held, not dropped** — it is not rendered until local time reaches it, then admitted normally. A
record far in the past is admitted and simply sorts where it claims; it cannot displace
history a reader has already rendered, because rendering is a function of the merged set,
not of arrival order. A malicious author can therefore make their own message appear
slightly out of place, and cannot do anything else. That is the honest limit: **without
a central sequencer, no chat system can do better, and the ones that appear to are
trusting a server clock.**

---

## 5. Reading a Channel

Opening a channel is a bounded operation, not a full history replay.

1. Resolve the participant index (§3.2) to a candidate author set; fall back to the
   member roster from governance replay when the index is empty or the channel is new.
2. For each author, resolve their pointer (Storage §2.2) and fetch the **head segment
   only**.
3. Merge, order (§4), apply redactions (§6), render the most recent screenful.
4. Backfill on demand: scrolling up walks `previous_segment` CIDs, fetching in parallel
   across authors, bounded by a local concurrency setting (Storage §4.4 makes this a
   per-node choice, not network policy).

**Backfill is bounded by pages, not by authors.** A channel with 2,000 historical posters
but 30 active ones costs 30 head-segment fetches to open, and reaches the older 1,970
only if a reader actually scrolls that far back. This is the mitigation `00` §4 names for
fan-in, and it is the reason head segments are capped by age as well as size — a stale
author's head segment must not be a year of scrollback.

---

## 6. Deletion, Redaction and Moderation

Three distinct actions, deliberately not collapsed:

**Author deletes their own message.** A `Tombstone` in their own log. Structurally
guaranteed to be self-service — nobody else can write that log — and needs no permission
check beyond having written the message.

**A moderator removes somebody else's message.** A moderator cannot write into another
author's log, so a redaction lives in the **moderator's own log** for that channel
(`content_type: chat-moderation`, same segment mechanics), carrying the target message id
and the governance log head the moderator observed. A reader honours a `Redaction` if,
replaying governance state as of that head, its author held `chat:moderate` for the
channel (`02` §3). This is durable (a mutable pointer, not a lapsing append-set), scales
(nothing per-message enters the governance log), and stays verifiable by replay like
every other authorization question in the system.

**A moderator removes an entire log.** For floods and for content that must stop being
served rather than merely stop being shown, `ModerationEntry` (Core §2.7) delists the
author's channel *pointer*. Honest nodes then refuse to serve or index it, and append-set
validation check (c) (Storage §2.5) drops its discovery entries. This is the heavy
instrument: it removes everything that author wrote in that channel, not one message.

**What redaction does and does not do.** It causes conformant clients to stop rendering a
message and stop returning it in search, and it stops honest nodes re-serving the
delisted case. It cannot retract bytes already fetched by members, and a modified client
can render anything its holder already has. This is the same limit Core §3.1 states for
revocation and Real-Time §4.2 states for VOD opt-out, and the UI must not imply otherwise
— see `05` §5.

---

## 7. Live Delivery

The durable path costs a segment publish, a pointer update and a fetch; that is seconds,
not milliseconds. Chat needs milliseconds, so records are *also* pushed as they are
written.

**Mechanism: gossipsub, one topic per channel**, topic id `H(channel_id ‖ "chat:live")`,
added to the member behaviour (`06` §4). A client subscribes to a channel's topic while
the channel is open, recently active, or flagged for notification, and unsubscribes
otherwise — so a member of a 400-channel server carries a handful of meshes, not 400.

A live payload is exactly the signed record of §3.3, sealed under the channel's content
key (`03` §2) with the `rotation_ref` it was sealed under. Receivers verify the
signature, check the author's `chat:post` permission against replayed state, and admit
the record into the merged view immediately. The author's segment publish follows on its
own schedule.

**Nothing depends on this path.** Missed records arrive with the next segment fetch;
duplicates are idempotent because records are content-addressed by id; out-of-order
arrival is irrelevant because order is computed, not received. A client with gossip
entirely disabled is slower and completely correct.

*Flagged: gossipsub is the recommendation because it is a mature libp2p behaviour that
already solves per-topic mesh maintenance. If per-channel mesh overhead proves
significant at high channel counts, the fallback is an HRW-selected fanout tier —
`intranet_realtime::assign_tier` already computes exactly this shape, weighted by
gossiped `bandwidth_cap` (Real-Time §3.3). Measure before switching.*

---

## 8. History: Retention and Access Are Two Switches

The requirement is that a network chooses its own history behaviour, including a rolling
window whose remaining history is fully visible to new joiners. That is not one setting
with three values; it is **two orthogonal settings**, and every requested model falls out
of their combination.

**Axis 1 — retention (app-layer network policy).** How long content is kept:
- `Forever` — nothing is ever dropped. **The shipped default**, see below.
- `Days(n)` — content older than the window stops being replicated, and, decisively,
  **stops being re-wrapped on epoch rotation**. Storage §5.2 already specifies that
  content with no live wrapping simply goes dark; retention needs no new mechanism, only
  the decision to stop re-wrapping.

**Two windows, not one: `chat:retain-messages-days` and `chat:retain-attachments-days`.**
Text and attachments differ in cost by orders of magnitude, so a single window has to be
wrong for one of them. A message is capped at 8 KiB by `chat:message-max-bytes` and its
flow is capped by the rate limits, so a million messages is a few gigabytes network-wide —
years of a busy network. One attachment may be 25 MiB, ten to a message, so a single heavy
week outweighs all of that text. A network bounding what it spends on other members' disks
nearly always means the attachments, and one shared window would charge it the scrollback
as well.

**The default is `Forever` for both**, which revises this document's earlier default of a
rolling window. The reasoning is asymmetry rather than preference: retention can be
switched on whenever a network decides it wants it, and content already allowed to go dark
cannot be brought back. Shipping a window by default means every network that never thinks
about the setting quietly loses history it assumed it had. A zero, a negative or an absurd
value all read as `Forever` for the same reason — a policy value that arrived corrupted
must not start discarding history.

**Judged on a log's newest record, not its oldest.** A log somebody is still writing to is
live however far back it reaches, and retiring it because its first message is old would
drop an active conversation.

**Axis 2 — joiner access (protocol policy, Core §3.4).** Which epoch keys a new member
receives: `CurrentEpochForward` (the protocol's conservative default) or `Full`
(historical keys delivered at join). Already implemented —
`EpochKeyring::keys_for_new_member(HistoryAccess)`.

| Preset | Retention | Joiner access | Feels like |
|---|---|---|---|
| Open archive | `Unbounded` | `Full` | Discord with full scrollback for everyone |
| Fresh start | `Unbounded` | `CurrentEpochForward` | History persists, but joining starts your clock |
| Rolling window | `Days(n)` | `Full` | The network decides how far back anything is kept; joiners see all of what remains |

All three are genuine configurations of the same two switches, chosen at genesis and
changeable afterward by a capability holder. **Open archive is the shipped default**, per
the reasoning above — an earlier draft of this document defaulted to the rolling window,
and that is revised.

**Retiring a superseded epoch key follows from retention rather than being its own
setting.** A key becomes droppable once nothing still inside the window is wrapped under
it, which the re-wrap-on-read path arranges by refreshing live wrappings forward. A
separate "retire keys after n days" knob would be able to contradict the retention window
— keep content a year, drop its key at six months — and make retained content silently
unreadable.

**Honest limits.** Retention is not deletion: a member who already fetched an old segment
and its DEK keeps both, forever, and no rotation can take that back (Core §3.1). Dropping
a segment makes it unavailable to people who did not already have it; it does not erase
it. Likewise `CurrentEpochForward` prevents a joiner from *obtaining* older keys — it
does not prevent an existing member from pasting old content into a new message.

---

## 9. Presence, Typing and Read State

**Presence and typing are ephemeral and never stored.** They ride a gossipsub topic and
are dropped on restart: presence heartbeats every 30 s with a coarse state
(`online | idle | dnd | invisible`), typing indicators with a 5 s TTL. `invisible` is a
real per-user setting, not a UI courtesy — presence in this system is visible to every
member of the network, and a user who does not want that must be able to say so.

At larger scales presence is subscribed **per channel currently in view**, not
network-wide, so a 5,000-member server does not gossip a full roster heartbeat to
everybody. `00` §4 names this as a known scaling pressure.

**Read state is local**, per (device, channel): the HLC watermark up to which the user
has read, plus mention counts derived locally. It is deliberately not a network object in
P1. When multi-device lands (`05` §6) it becomes a small private object owned by the
user, encrypted to themselves — the shape is already compatible; only the storage
location changes.

---

## 10. Abuse Control

### 10.1 Limits Are Network Policy, Set by the Founder

Every limit below is a **network policy value with a shipped default, changeable by
whoever holds `define-policy`** — the Founders group by default. Two independent reasons
force this, and either alone would be sufficient:

- **Determinism.** These are *validity* rules: a record beyond the ceiling is refused by
  readers, not merely discouraged. A local limit would mean two members rendering
  different histories from the same records, which is the cross-node divergence this
  whole design is otherwise careful to avoid.
- **They spend other people's resources.** Replica holders did not choose to store
  somebody's uploads, and at replication factor 3 a 25 MiB attachment costs 75 MiB of
  network-wide storage. The person accountable for the network's health is the one who
  should be setting that.

They live in the network policy record (`06` §9), so a change is an ordinary governance
entry: ordered, replayable, tamper-evident, and visible to every member.

| Key | Default | What it bounds |
|---|---|---|
| `chat:message-rate-per-minute` | 30 | `Message`, `Edit` and `Tombstone` records, per author per channel |
| `chat:reaction-rate-per-minute` | 60 | `Reaction` and `Pin` records, per author per channel |
| `chat:message-max-bytes` | 8 KiB | One message or edit body, as UTF-8 (spec 07 §4.3) |
| `chat:attachment-max-bytes` | 25 MiB | One attachment |
| `chat:attachment-max-count` | 10 | Attachments on one message |
| `chat:segment-max-bytes` | 8 MiB | A published segment — without this, one author can force every reader to fetch an arbitrarily large object |
| `chat:max-future-skew-millis` | 300 000 | How far ahead of local time a record may claim to be (§4) |
| `chat:slowmode-max-seconds` | 21 600 | The largest per-channel slowmode a channel manager may set (§10.3) |

Defaults are deliberately generous — 30 messages a minute is one every two seconds, which
no human sustains and every flood exceeds. The purpose is to bound abuse, not to pace
conversation; pacing is §10.3's job.

### 10.2 Why the Rate Rule Is Actually Deterministic

A sliding window over *arrival* time would give every node a different answer. The window
is therefore computed over **the author's own HLC timestamps**, which are monotonic per
author per channel by construction (§4) — so every node evaluating the same records
computes the same window occupancy and reaches the same verdict, regardless of when the
records arrived or how skewed anyone's clock is.

The obvious evasion — claiming timestamps spaced far apart while actually sending fast —
defeats itself: records dated ahead of the receiver's clock are **held until local time
reaches them** (§4). An author who lies about pacing to escape the ceiling gets exactly
the pacing they claimed. No extra mechanism needed.

**The author's own client enforces the ceiling first**, so a user who types too fast sees
"you're going too fast" rather than watching messages be silently discarded by everyone
else. Reader-side enforcement is the backstop against a modified client, not the primary
UX.

### 10.3 Slowmode Is a Separate, Delegable Knob

The network ceiling is an abuse bound, not a moderation tool — a moderator wanting a
busy channel to calm down should not need `define-policy`, which would hand them
authority over admission mode and governance model at the same time.

**Per-channel slowmode is a field on `ChannelDefinition`** (§2.1), set by holders of
`chat:manage-channel` for that channel, bounded above by `chat:slowmode-max-seconds`.
Because channel definitions are governance-anchored and replayed, slowmode is as
deterministic as the network ceiling — every node computes the same verdict — while
being delegable to the people who actually moderate.

Effective limit for a record is the stricter of the two. Slowmode of `0` means off,
which is the default for a new channel.

### 10.4 Beyond Limits

Spam is a moderation problem with a moderation answer (§6), and persistent
abuse is a `revoke-node` problem with the epoch rotation that implies. Note the useful
side effect of Storage §2.3's freeze consequence: narrowing `publish:chat-log` away from
an identity freezes their existing logs — their history stays readable and servable, and
they cannot extend it. That is exactly the right behaviour for a timeout, and it comes
free.
