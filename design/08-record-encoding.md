# Canonical Record Encoding

**Document status:** v0.1 — normative for implementation; folds into `distributed-intranet/specs/07` (F2)
**Depends on:** `intranet_crypto::{Enc, Dec}`, `01-messaging-model`, `03-confidentiality`
**Consumed by:** `kols-core`, and any other implementation of this application

---

## 1. Why This Document Is Normative

`01` §3.3 specifies record shapes. This specifies their bytes, and it has to, because
three things in this design are functions of exact bytes:

- **`message_id = H(canonical record bytes)`.** Every reply, reaction, redaction and pin
  references a message by that hash. A one-byte disagreement between two implementations
  produces two ids for the same message, and references silently stop resolving.
- **Signatures.** A record is verified against the bytes it claims to be. Encoding drift
  means a valid record failing verification on another node.
- **Merge ordering.** The final tie-break is the record hash (`01` §4), so two nodes that
  encode differently order concurrent messages differently and render different history.

The protocol already made this decision and wrote down why: encoding is **hand-written per
type**, never derived, because a derive macro's output is a property of field order, map
iteration order and library version — none of which the protocol controls. This document
follows that rule rather than restating the argument.

---

## 2. Inherited Rules

From `intranet_crypto::Enc`, unchanged and not restated per type:

| Rule | Detail |
|---|---|
| Domain separation | Every signable or hashable type begins with its own tag, so a signature over one type can never verify as another |
| Fixed-width integers | `u8`, `u32`, `u64`, `i64`, big-endian, unframed |
| Variable-length fields | Length-prefixed with a `u64` — bytes and strings alike |
| Fixed arrays | Unframed, safe because the width is a compile-time constant |
| Sum types | A `u8` discriminant **before** the payload |
| Options | `0x00` absent, `0x01` followed by the value |
| Sequences | `u64` count, then elements; **order must be deterministic** at the call site |
| Booleans | `0x00` / `0x01` |

Three prohibitions, inherited and absolute: **no floating point** anywhere in an encoding;
**no hash-ordered collections** feeding a sequence; **no system time** read during
encoding — every timestamp is an explicit field value.

---

## 3. Domain Tag Registry

Tags follow the protocol's existing `intranet.<thing>.v<n>` convention, with `wire.` for
transport-only types. **A tag is permanent.** Changing what a tag covers means a new tag
with an incremented version, never a redefinition — an old node must never verify a new
value under an old tag's assumptions.

| Tag | Covers |
|---|---|
| `intranet.chat-record.v1` | A single record's signed payload (§5) |
| `intranet.chat-segment.v1` | A segment container (§6) |
| `intranet.chat-channel-id.v1` | Server-profile channel id derivation (§7) |
| `intranet.chat-conversation-id.v1` | Conversation-profile channel id derivation (§7) |
| `intranet.chat-thread-id.v1` | Thread channel id derivation (§7) |
| `intranet.chat-log-pointer.v1` | Author log pointer id derivation (§7) |
| `intranet.chat-moderation-pointer.v1` | Moderation log pointer id derivation (§7) |
| `intranet.chat-channel-key.v1` | Channel content key derivation (§8) |
| `intranet.chat-topic.v1` | Gossip topic derivation (§7) |
| `intranet.wire.chat-live.v1` | Live gossip payload (E4) |
| `intranet.wire.chat-dm-invite.v1` | DM invitation (E10) |

---

## 4. Primitives

### 4.1 Hybrid logical clock

```
Hlc.encode(enc):
    enc.i64(wall_millis)      // Timestamp semantics: ms since Unix epoch, signed
    enc.u32(counter)
```

Ordering is `(wall_millis, counter)` ascending, compared as signed then unsigned. Signed
milliseconds match `intranet_crypto::Timestamp` exactly, so no conversion exists to get
wrong.

**The counter is monotonic per (author, channel), and an HLC must strictly increase within
one author's log.** Two records from the same author in the same channel sharing an HLC
are **invalid** — both of them, not just the later one, since "later" is what is in
dispute. This gives three properties at once: no duplicate ids from one author, gap
detection within a segment, and a total order over an author's own records that needs no
separate sequence field.

### 4.2 Identifiers

All are 32-byte values encoded with `fixed`, never length-prefixed:

| Type | Bytes | Note |
|---|---|---|
| `ChannelId` | 32 | Derived, §7 |
| `MessageId` | 32 | `H(record bytes)`, §5.4 |
| `PerNetworkIdentityId` | 32 | Ed25519 public key, encoded by the protocol's own `encode` |
| `DeviceKey` | 32 | Ed25519 public key of the signing device (Core §1.3) |
| `Hash` / `Cid` | 32 | Governance heads, manifest CIDs |

### 4.3 Text

Strings are length-prefixed UTF-8, encoded **exactly as given**. The encoder performs no
normalization, trimming or case folding.

This is deliberate and worth being explicit about, because the tempting alternative is
wrong: normalizing at encode time would alter the user's text between what they typed and
what everyone stores, and would make two visually identical bodies collapse to one id —
which then breaks edit and reply references. **Unicode normalization is a rendering and
search concern** (search tokenization already normalizes, Search §3.1) and never an
encoding one.

Validity bounds, all refusing rather than truncating:

- `Message.body` and `Edit.body` — **network policy**, `chat:message-max-bytes` (`01`
  §10.1), default 8 KiB of UTF-8. A network that wants longer posts can raise it, and
  every node then agrees on the same limit.
- `Reaction.key` at most 64 bytes and attachment `name` at most 255 bytes — **fixed
  encoding constants**, not policy. Nothing is gained by letting these vary, and each
  policy key is one more value two nodes could disagree about mid-propagation.

---

## 5. Records

### 5.1 Common header

Every record, of every kind, begins identically:

```
record_payload(r) -> Enc:
    e = Enc::domain("intranet.chat-record.v1")
    e.fixed(r.channel_id)          // 32
    r.author.encode(e)             // 32
    e.fixed(r.device)              // 32
    r.hlc.encode(e)                // 12
    e.variant(r.kind)              // 1
    <kind-specific body, §5.2>
```

**`network_id` is deliberately absent.** Every `ChannelId` is derived from the network id
(§7), so a channel id already commits to its network and a record cannot be replayed into
a different one. Carrying the network id as well would add 32 bytes per record to restate
a fact the channel id already fixes.

**`device` is present from v1 even though v1 ships single-device** (`05` §6). Adding an
authenticated field later would mean a v2 record tag and two encodings to support forever;
32 bytes now is much cheaper than that.

### 5.2 Kinds, and why the discriminant carries a class

```
0x01  Message    { body: str, reply_to: Option<MessageId>, attachments: seq<Attachment> }
0x02  Edit       { target: MessageId, body: str }
0x03  Tombstone  { target: MessageId }

0x40  Reaction   { target: MessageId, key: str, remove: bool }
0x41  Pin        { target: MessageId, remove: bool }

0x80  Redaction  { target: MessageId, governance_head: Hash }     // moderation logs only

Attachment       { manifest_cid: Hash, byte_len: u64, media_type: str, name: str }
```

Discriminants are grouped by **rate class**, and the grouping is load-bearing rather than
tidy:

| Range | Class | Counted against |
|---|---|---|
| `0x01`–`0x3F` | message-class | `chat:message-rate-per-minute` |
| `0x40`–`0x7F` | reaction-class | `chat:reaction-rate-per-minute` |
| `0x80`–`0xBF` | control-class | Neither; validity governed by capability |
| `0xC0`–`0xFF` | reserved | Refused |

The reason is §9's forward-compatibility problem. Rate ceilings are *validity* rules, so
every node must count the same records (`01` §10.2) — including nodes that do not
understand a record kind introduced after they shipped. Because the class is derivable
from the discriminant alone, an old node counts a new kind correctly without knowing what
it is. Had the class been a property of the variant's meaning, two client versions would
have reached different verdicts on the same records, which is precisely the divergence the
rate rule exists to avoid.

### 5.3 What is signed

The signature covers `record_payload(r)` — the domain tag and every field above, and
nothing else. Signing is by the **device key**, verified against a non-revoked device
certificate binding that device to `author` in this network (Core §1.3 point 4). A record
whose device certificate is absent or revoked at the point in the chain where the record
falls is invalid.

### 5.4 The record id

```
message_id(r) = hash_bytes( record_payload(r) ++ signature )
```

The signature is included, matching `AppendSetEntry::entry_id` — the closest existing
analogue, and precedent worth following rather than diverging from. Two consequences,
both intended:

- The id commits to the **exact bytes delivered**, so a record fetched from a segment and
  the same record received over gossip are provably the same object, not merely equal
  field by field.
- The id cannot be computed before signing. Nothing needs it earlier: `reply_to` always
  references an already-published record.

Ed25519 signatures are deterministic (RFC 8032), so this is stable — the same record
signed twice by the same key produces the same id.

---

## 6. Segments

```
segment_bytes(s) -> Enc:
    e = Enc::domain("intranet.chat-segment.v1")
    e.fixed(s.channel_id)
    s.author.encode(e)
    e.u64(s.sequence)                              // monotonic per (channel, author)
    e.option(s.previous_segment, |e, cid| e.fixed(cid))
    e.seq(s.records, |e, r| e.bytes(r.canonical_bytes()))
```

Each record is embedded as its **complete canonical bytes including its signature**,
length-prefixed. Three properties follow, and all three matter:

1. **A segment is self-verifying.** A reader checks every record's signature from the
   segment alone, without the pointer, and a stale pointer costs freshness rather than
   trust.
2. **Records re-emit byte-identically.** A record read from a segment and republished over
   gossip is the same bytes, so its id is stable across paths — the invariant `01` §3.3
   depends on.
3. **An unknown record kind is still framed.** A reader that does not understand a
   discriminant can still skip it, count it by class (§5.2), and carry it forward.

**The segment itself carries no signature.** Its authenticity comes from the mutable
pointer that names its CID, signed by `owner_identity` (Storage §2.2), plus every record's
own signature. A second signature over the same facts would create a state where the inner
and outer signers disagree with no rule saying which wins — the exact reasoning
`ModerationEntry` gives for not carrying its own signature inside a `LogEntry`.

**A record must belong to the segment carrying it.** Verifying a segment checks, for every
record, that its `channel` and `author` match the segment's own — not only that its
signature is valid. Without this a validly-signed record could be lifted out of one
author's log and embedded in another's, where it would carry an authorship the pointer's
owner never had; the signature stays genuine throughout, so signature checking alone does
not catch it. *Added while implementing `Segment::verify`, not present in the first draft
of this document.*

Decoder bounds: at most 4096 records per segment, and `chat:segment-max-bytes` (default
8 MiB) over the whole encoding. Both refuse rather than truncate.

---

## 7. Derived Identifiers

Every derivation is `hash_bytes(Enc::domain(tag) ++ inputs)`, so each is domain-separated
from all others and none can be confused for another.

| Value | Derivation |
|---|---|
| Channel id (server) | `H("intranet.chat-channel-id.v1" ‖ network_id ‖ nonce:32)` |
| Channel id (conversation) | `H("intranet.chat-conversation-id.v1" ‖ network_id)` |
| Thread channel id | `H("intranet.chat-thread-id.v1" ‖ parent_channel_id ‖ root_message_id)` |
| Author log pointer id | `PointerId(H("intranet.chat-log-pointer.v1" ‖ channel_id ‖ author))` |
| Moderation log pointer id | `PointerId(H("intranet.chat-moderation-pointer.v1" ‖ channel_id ‖ moderator))` |
| Gossip topic | `H("intranet.chat-topic.v1" ‖ channel_id)`, rendered hex |
| Participant index collection | `appendset::collection_id(network_id, "chat:authors:" ++ hex(channel_id))` |
| Channel browse collection | `appendset::collection_id(network_id, "chat:channels")` |

The two collection ids use the protocol's existing `collection_id` helper rather than a
parallel derivation, so append-set entries land where the storage layer expects them.

---

## 8. Channel Content Key

For the live path (`03` §2.1) and for private channels:

```
channel_content_key = keyed_hash(
    key  = epoch_key_for(rotation_ref),          // network epoch key, or channel MLS key
    data = Enc::domain("intranet.chat-channel-key.v1")
             .fixed(channel_id)
             .fixed(rotation_ref)
             .finish()
)
```

`keyed_hash` is the protocol's existing BLAKE3 keyed hash — no new primitive. Binding
`rotation_ref` into the derivation means a key from one rotation cannot be mistaken for
another's, which is what lets a receiver mid-rotation hold both and pick correctly rather
than fail.

---

## 9. Evolution Rules

**Adding a kind:** allocate the next discriminant *within the correct class range* (§5.2)
and add it to the registry here. Never reuse a retired discriminant.

**Changing a kind:** not permitted. A changed shape is a new discriminant, or — if the
common header itself must change — a new record domain tag at `.v2`, with `.v1` still
decodable forever.

**Encountering an unknown kind:** the record is **retained, counted by class, and not
rendered.** Not rejected, and not silently dropped:

- *Rejected* would mean an old client refusing a segment containing one new record,
  losing every valid record beside it.
- *Dropped* would mean the record vanishing from a node's copy while still existing on
  others', so a node that later served that segment would serve something different from
  what it received.

Retention keeps the record set identical everywhere and confines the difference to what
each client version *displays*. That is a presentation difference between versions, not a
consistency failure — the merged record set, the ids, and the ordering are the same on
both. This is the one place this design accepts differing rendered output between
implementations, and it is accepted deliberately because every alternative is worse.

---

## 10. Conformance Tests

Required before P0 is considered done, and permanent thereafter:

1. **Fixed test vectors.** A checked-in file of `(logical value, expected hex)` pairs for
   every record kind, a multi-record segment, and every derived identifier in §7. These
   are the actual contract; a change that alters one is a wire break and must fail CI
   loudly rather than be re-blessed casually.
2. **Round-trip.** `decode(encode(v)) == v` for every type, including options absent and
   present, empty sequences, and maximum-length fields.
3. **Injectivity.** No two distinct logical values encode to the same bytes — property
   test over generated records, which is what the length-prefixing exists to guarantee.
4. **Signature domain separation.** A signature over a record must not verify as a
   signature over a segment, an append-set entry, or a governance entry under the same key.
5. **Id stability.** The same record signed by the same key twice yields the same id
   (Ed25519 determinism), and re-encoding a decoded record reproduces byte-identical
   output — the property §6 point 2 relies on.
6. **Cross-platform.** Vectors verified on both a little-endian and a big-endian target,
   or the endianness discipline is untested rather than merely stated.
