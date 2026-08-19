# Record Encoding — Implementation and Conformance

**Document status:** v1.0 — **not normative.** `distributed-intranet/specs/07` §3 is the
contract; this document is where the client's obligations against it live.
**Depends on:** Chat Application Spec §3 (normative), `intranet_crypto::{Enc, Dec}`
**Consumed by:** `kols-core`

---

## 1. Where the Encoding Now Lives

The canonical encoding was drafted here and moved upstream in F2. **Chat Application Spec
§3 is normative**: domain tags, the record header, kind discriminants and their rate
classes, the record id, the segment container and its count-prefix exception, derived
identifiers, and the evolution rules. Where this document and that one appear to disagree,
the spec is right and this document is a stale copy to be fixed.

Two normative descriptions of one wire format is the drift this project criticises
elsewhere, so this document deliberately stopped being one rather than being kept in sync
by hand.

What remains here is what the spec has no business specifying: which crate implements
what, and what the client must prove about its own implementation.

---

## 2. Module Map

| Concern | Where |
|---|---|
| `Hlc`, and the next-reading rule | `kols-core::hlc` |
| `ChannelId`, `MessageId`, every derivation in spec §3.6 | `kols-core::ids` |
| `Record`, `RecordBody`, `RecordClass`, encode/decode/sign/verify | `kols-core::record` |
| `Segment`, ordering validity | `kols-core::segment` |
| Capability names and their tiers | `kols-core::capabilities` |
| Frozen test vectors | `kols-core/tests/encoding.rs` |

---

## 3. Conformance Obligations

Required of any implementation, and permanent once written:

1. **Fixed test vectors.** `(logical value, expected hex)` for every record kind, a
   multi-record segment, and every derived identifier. **These are the contract.** A change
   that alters one is a wire break to be recognised and versioned, never a value to
   re-bless because a test went red.
2. **Round-trip.** `decode(encode(v)) == v` for every type, including absent and present
   options, empty sequences, and maximum-length fields.
3. **Injectivity.** No two distinct logical values encode alike — property-tested, since
   that is what the length-prefixing exists to guarantee.
4. **Domain separation.** A record signature must not verify as a segment, an append-set
   entry, or a governance entry under the same key.
5. **Id stability.** The same record signed twice yields the same id, and re-encoding a
   decoded record reproduces byte-identical output.
6. **Cross-platform.** Vectors verified on a big-endian target, or the endianness
   discipline is stated rather than tested.

Obligations 1–5 are met by `kols-core/tests/encoding.rs`. **Obligation 6 is not met**:
there is no big-endian target in CI, so byte order is currently correct by construction
(`Enc` writes big-endian explicitly) rather than by demonstration. Recorded here rather
than quietly counted as passing.

---

## 4. What Measurement Changed

Both of the spec's more surprising rules came from this implementation rather than from
review, and the numbers are kept here because the spec keeps the rule:

| Finding | Before | After |
|---|---|---|
| Segment record list carries no count prefix (spec §3.5) | 51,405 bytes of 176,123 moved per appended message | **1,556 of 176,115**, one new chunk of eight |
| Same, measured across the wire between two live nodes | — | reader refetches **1 chunk of 3** |

The second rule — HLC strictness per (author, device) rather than per author (spec §2.6) —
produced no number, only a failing case: a merged segment necessarily interleaves two
devices, so the per-author rule declared every concurrent-version recovery invalid.
