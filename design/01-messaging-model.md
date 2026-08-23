# Messaging Model

**Document status:** v1.1 — §2.3 makes categories nameable and ordered, §2.4 fixes the sidebar's two-level order. Neither is implemented; spec 07 is normative where they overlap
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
in such a network is **invalid and refused by every reader that understands the `chat`
namespace** (spec 07 §1.2). The profile lives in replayed policy state, so they all reach
the same verdict; the protocol carries the payload without decoding it and so enforces
nothing here itself. Everything else in this document — segments, ordering,
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

### 2.3 Categories are named and ordered, and are not channels

Today a category is an id and nothing else. A `CategoryId` is referenced by a channel
definition and used as a permission scope (`02` §4) — it has no name, no definition entry and
no order anywhere. That is not a gap anybody chose; it is why the folders `00` §5 promises
cannot be drawn, since there is nothing to put on the label.

**A category gets a definition entry carrying a name and a position, and nothing else.** No
membership, no content, no keying, no privacy. It is furniture, not a channel, and the entry
that describes it should not be able to grow into one.

**The definition is metadata over a scope that already exists.** This is the part a reader will
get wrong if it is not said plainly. Permission resolution reads the *channel's* `category`
field and binds against `cat:<id>` (`02` §4, spec 07 §4.2). It does not consult a category
definition today and **must not start**. The definition supplies a label and a sort key; the
scope is the id, and the id is carried by the channels themselves.

Two things follow, and both are the reason to build it this way:

- **Deleting a category cannot widen or narrow anybody's access.** It removes a name and a
  position, not a scope. A channel whose category definition is gone is still in that category,
  still resolves exactly the same permissions, and merely has no label to render. If deletion
  *could* move access, tidying the sidebar would be an access-control act wearing a cosmetic
  disguise — which is precisely what §2.4 refuses to let reordering become.
- **A category needs no definition to work.** Every network that exists today has categories in
  this sense already. Naming them is additive, and no existing network's permissions change when
  the feature lands.

A client that means "delete this folder and move its channels out" issues `Recategorise(None)`
for each channel alongside the delete. That is a client's compound action and belongs to no
single entry, because the log has no transactions and pretending otherwise would be a lie about
atomicity.

**Category entries are governance-tier, under `chat:manage-channel`.** Not `create-channel`,
though a definition widens nothing and that capability's tier is argued from what an action can
widen. Two reasons override the analogy. Categories are deliberately few — `02` §4 sizes them at
roughly one per role, against hundreds of channels — so they are not the routine act
`create-channel` is tuned for. And channel *structural* mutation is already tiered exactly this
way: rename, topic, slowmode, archive and delete all require `manage-channel` and none of them
widens access either. A category is the furniture permissions bind to by default; the entries
that name and order it belong with the rest of the structure.

Scope has one wrinkle worth stating, because it is not symmetric with channels. A category
definition must be scoped `*`, network-wide: nothing encloses a category, so there is no
narrower grant that could authorize creating one. An update may be scoped `*` or to the
category's own id, which is grantable once the category exists.

A category id derives the way a channel id does — `H(domain ‖ network id ‖ nonce)`, spec 07
§3.6 — so a category entry cannot be replayed into another network.

### 2.4 The order of the sidebar

D31 fixed channel order as a network default a member may override locally. Categories need the
same, and adding them makes the sort **two-level rather than flat**, which is a correction to
what D31 assumed rather than an addition to it.

The network default is computed like this, and every node reaches the same answer:

1. **Uncategorised channels sort before every category**, as an implicit top-level group.
2. **Categories sort among themselves** ascending by position, ties broken by category id.
3. **Channels sort within their own category** ascending by position, ties broken by channel id.

At every level, a sibling that has never been given a position sorts after every sibling that
has. Positions are comparable only among siblings: a channel's position says nothing about where
its category sits, and two channels in different categories are never compared at all.

**Uncategorised first, rather than last, and the reason is not tradition.** It matches the
clients people have used, which is worth something — but the load-bearing reason is that a
channel which loses its category has somewhere obvious to appear. Sorting the uncategorised
group last would make `Recategorise(None)` look like deletion to anybody with more than a
screenful of channels.

The local override still overrides all of it, still writes nothing, and still reaches nobody.
What the network agrees on is the default; what a member sees is theirs.

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
late is not producing invalid history. `kols serve --seal-bytes` is exactly that knob;
tests set it low so a chain forms in a few messages rather than four megabytes.

**Age is measured across the segment's own records — newest minus oldest — not against
the clock.** The distinction is load-bearing. A publisher rebuilds its log by replaying
its stored records, so the boundaries have to be a pure function of that sequence: replay
them a month later and the same seals fall in the same places, producing the identical
chain. "Older than a day *right now*" would seal somewhere new on every restart, and a
node would publish a second chain competing with the one its readers already hold. This
is also why sealing needs no persisted state of its own — the record sequence *is* the
state. The one network-wide bound is
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

### 3.1.0 A Pointer Per Segment

Each segment lives under its own derived pointer, `author_segment_pointer(channel, author,
sequence)`, and therefore under its own DEK.

This is forced rather than chosen. A pointer commits to one DEK **for its entire life** —
`MutablePointer::update` carries `dek_commitment` forward deliberately, because Storage
§1.2 fixes a DEK for its object's lifetime — so every segment sharing a pointer shares a
key, and a key that opens the newest message also opens the oldest. Retention could then
only ever forget a whole log. A DEK that can be forgotten separately needs a pointer of
its own, and there is no way around that: wrappings travel only alongside the pointer
record they belong to, so a wrapping for a pointer nobody published never syncs at all.

Sealing therefore starts a new segment **and** a new key. The sealed segment keeps the key
it was written under, so nothing is ever re-encrypted — re-keying it would move every CID
in it, forcing every reader holding it to refetch the whole object, and would leave the
superseded ciphertext readable under a key nothing retires, which is exactly the forgetting
this exists to make possible.

**The cost is one indirection on the read side.** `author_log_pointer` — the derivation a
reader can compute from public information alone (§3.2) — no longer names the messages. It
names a *head index*: an otherwise empty segment whose `sequence` says which segment is
currently the head. From there every other address is derivable again, including each hop's
key as the backfill walk of §5 goes back. The index changes only when a segment is sealed,
not on every append, so an ongoing conversation refetches nothing.

The index pointer's version **is** the head sequence. Two pointer records at the same
version are settled by lower record hash (Storage §2.2), so an index republished at version
zero with a newer sequence would lose that coin-flip against the copy peers already hold,
about half the time, and the author's newer history would simply never be found.

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

*Resolved: no protocol change was needed. `intranet-storage` mints pointer ids randomly with
`new_pointer_id`, but `PointerId::from_bytes` is a public `const fn`, so a client derives one
by hashing whatever it likes — and the derivation inputs and their domain separation are the
client's business rather than the storage layer's. `kols-core::ids` implements it, with the
derivations pinned by test vectors, and `06` §3 records why E3 was withdrawn.*

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

The client walks the chain as far as its local chunk store can carry it and stops at the
first hop it does not hold, queueing that one for the ordinary fetch path rather than
blocking on it. So a node absorbs a chain it already has in a single pass, and pays a
round per hop only for what it still has to fetch. It has no scroll position to drive
"on demand" from yet, so it walks to the start; a UI would bound this by pages.

**A segment is marked absorbed only once the chain behind it is whole.** A mark meaning
"this segment is stored" reads correctly and behaves wrongly: the walk stops at the first
marked segment, so marking one whose own ancestors were still missing walls off
everything behind it permanently — a reader takes one hop of history and then stops, for
good. The cost of deferring is a re-walk of the held part each round, which stores
nothing, because records are content-addressed and already present.

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

**Landed as E4** (Core §5.1), gossipsub being a mature libp2p behaviour that already solves
per-topic mesh maintenance. Three configuration choices turned out to be load-bearing and are
recorded in `06` §4: transport-level message signing is off, because a record already carries
its own signature and two authorities for "who wrote this" is worse than one; message ids are
content hashes, because the same record legitimately arrives twice and deduplication has to
agree with the consumer's own content addressing; and the transport validates nothing,
because it cannot know what a payload means and half a check reads as a whole one.

*Flagged: if per-channel mesh overhead proves significant at high channel counts, the
fallback is an HRW-selected fanout tier — `intranet_realtime::assign_tier` already computes
exactly this shape, weighted by gossiped `bandwidth_cap` (Real-Time §3.3). Measure before
switching.*

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

**Judged on a segment's newest record, not its oldest.** A segment somebody is still
writing to is live however far back it reaches, and retiring it because its first message
is old would drop an active conversation.

**Per segment, not per log** — which is what §3.1.0's pointer-per-segment buys. Each segment
holds its own key, so an author stops republishing and stops re-wrapping the segments that
have aged out and keeps maintaining the rest. A reader walking back through history reaches
a segment whose wrapping it can no longer open, and stops there.

That boundary is deliberately **indistinguishable from history that has not arrived yet**,
and a reader should not try to tell the two apart: a missing wrapping, a wrapping under an
epoch this node has not caught up to, and a segment retired last year all look identical,
and a client that reported "this was deleted" would be asserting something it cannot know.

**Axis 2 — joiner access (protocol policy, Core §3.4).** Which epoch keys a new member
receives: `CurrentEpochForward` (the protocol's conservative default) or `Full`
(historical keys delivered at join). Already implemented —
`EpochKeyring::keys_for_new_member(HistoryAccess)`.

| Preset | Retention | Joiner access | Feels like |
|---|---|---|---|
| Open archive | `Forever` | `Full` | Discord with full scrollback for everyone |
| Fresh start | `Forever` | `CurrentEpochForward` | History persists, but joining starts your clock |
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

### 10.4 Where These Are Enforced, and Two Decisions the Specs Leave Open

Both halves are built, and the shape they took is worth recording because one of them was
forced by the determinism §10.2 claims rather than chosen for convenience.

**The rate window cannot be evaluated as records land.** "How many has this author written
in the last minute" has no arrival-order-independent answer while the set is still being
assembled — the same records delivered in two orders would refuse two different ones, and
two members would render different histories from identical records, which is the exact
outcome §10.1 gives as the reason these are network policy. So the ceilings are applied the
way every other effect in the merge is applied: as a function of the sorted set, after it
is assembled. `kols_core::withheld` is that pass, and `ChannelView::render` runs it. The
per-record bounds — a body's size — stay in the per-record check, because those genuinely
are properties of one record.

The author's records are then walked in canonical order and admitted greedily, so a record
is refused when the ceiling is already met by the records *already admitted* in its
trailing window. A refused record does not itself occupy a slot: one burst costs an author
exactly the records that exceeded the rate, and does not silence them afterwards.

**Held is not refused, and the interface must not conflate them.** A future-dated record
(§4) stays in the record set, is served like any other, and renders the moment local time
reaches its claim — only *display* waits. So the reader reports two categories rather than
one, and only the permanent one is shown as a refusal. Saying "refused" about something
that will appear in four minutes is a client asserting what it does not know.

**Slowmode is applied to the message class, so an edit or a withdrawal is paced along with a
post. Decided rather than flagged**, 2026-08-22. Neither spec 07 §4.3 nor §10.3 says whether
slowmode reaches past a `Message`, and class is the answer that keeps two client versions
agreeing — the whole argument for encoding rate class in the discriminant range (spec 07 §3.3)
is that an old node counts a new kind correctly without understanding it, and a rule keyed on a
variant's *meaning* gives that up.

The cost is named rather than hidden: in a channel with a long slowmode, fixing a typo waits as
long as posting again does. That is accepted on the expectation that slowmode is a rarely-used
instrument — it is a moderator calming one busy channel, not a setting a network runs with. If
that expectation turns out to be wrong, the fix is a second and shorter bound for the corrective
kinds, still keyed on class; it is **not** a rule that reaches into what a variant means, which
is the one shape that would have two implementations disagreeing.

### 10.5 Beyond Limits

Spam is a moderation problem with a moderation answer (§6), and persistent
abuse is a `revoke-node` problem with the epoch rotation that implies. Note the useful
side effect of Storage §2.3's freeze consequence: narrowing `publish:chat-log` away from
an identity freezes their existing logs — their history stays readable and servable, and
they cannot extend it. That is exactly the right behaviour for a timeout, and it comes
free.
