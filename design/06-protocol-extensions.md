# Required Protocol Extensions

**Document status:** v1.2 — E1 and E3 withdrawn, E11 added, **E9 and E2 landed**
**Depends on:** all preceding documents
**Consumed by:** work in `distributed-intranet`

---

## 0. Scope and Posture

Everything this chat application needs that the protocol does not already provide,
gathered in one place so the protocol work can be scoped, sequenced and reviewed
independently of the client work.

Two rules govern this list:

- **Nothing here weakens an existing guarantee.** Every item is additive. Where an item
  touches a mechanism the specs reason about carefully (the governance log, media
  delivery, capability tiers), it follows that mechanism's existing pattern rather than
  introducing a parallel one.
- **The specs are authoritative, so extensions belong in a spec, not only in code.** The
  natural home is a seventh document — `07-chat-application-spec.md` — in
  `distributed-intranet/specs/`, consuming the platform the way App Hosting and Search
  do. Adding entry types and a wire protocol without writing them down would leave the
  repo's central claim (the specs are authoritative and the code implements them) false.

| # | Extension | Blocks | Size |
|---|---|---|---|
| ~~E1~~ | ~~Extension-capability tier registry~~ — **already implemented**, needs configuration only | — | None |
| ~~E2~~ | Channel governance entries — **landed generically**, Core §2.7.2 | — | Done |
| ~~E3~~ | ~~Derived pointer ids~~ — **already possible**, no change needed | — | None |
| E4 | Gossipsub behaviour for live delivery | P1 | Medium |
| E5 | Media fan-out at the relay | P3 | Medium |
| E6 | QUIC datagram media path | P3 (quality) | Large, partly upstream |
| E7 | Channel-scoped MLS groups | P2 | Large |
| E8 | Track metadata in sealed media payloads | P4 | Small |
| ~~E9~~ | App-layer policy map in `NetworkPolicy` — **landed**, Core §2.6.2 | — | Done |
| E10 | Direct member-to-member delivery for DM invitations | P2 | Small |
| E11 | Namespace registration for extension capabilities | P1 | Small |

---

## 1. E1 — Extension Capability Registry (Not Needed)

**Withdrawn: this already exists.** `NetworkPolicy::extension_capabilities` is a
`BTreeMap<String, Tier>`, `GovernanceState::tier_of` resolves an `Extension` capability
through it, and an unregistered name returns `UnregisteredExtensionCapability` rather
than defaulting to ordinary — which is exactly the fail-closed behaviour the invariant
needs.

What remains is **configuration, not code**: register the chat vocabulary from `02` §2.2
into the map at genesis, with the tiers stated there. Worth one conformance test
confirming that a registered governance-tier chat capability is refused when granted to
`everyone`, since that invariant is the reason the registry exists and it is cheap to
assert.

---

## 2. E2 — Channel Governance Entries ✅ **landed, in generalised form**

`EntryBody` is a closed enum. Channels need durable, ordered, non-lapsing state, which is
exactly what App Hosting §4.3 concluded for names after finding append-sets insufficient
on both counts.

**What landed is not what this section proposed, and the change is the point.** Four
chat-shaped variants in the core enum would have been the same mistake E9 was careful to
avoid one section later — Core §0 says the platform must not be shaped around one
application. It had already happened once (`AppNameRegistration` is App Hosting's record in
the core vocabulary); four more would have made a pattern of the exception. So the log
gained **one generic application entry** (Core §2.7.2): namespace, kind, declared
capability, opaque payload. Chat's four records below are payloads in the `chat` namespace,
decoded by `kols-core`.

```
EntryBody::ChannelDefinition { channel_id, name, category, kind, privacy, topic, slowmode }
EntryBody::ChannelUpdate     { channel_id, change }        — rename, recategorise, archive, delete
EntryBody::ChannelMembership { channel_id, action, identity }   — private channels only
EntryBody::ChannelRotation   { channel_id, commit_ref, reason } — private channels only
```

All four are **capability-gated and therefore count toward branch length** under the
fork-choice rule (Core §2.7.1 point 2) — unlike device certificates, which are excluded
because they need no capability. That is the correct classification: each requires
`chat:manage-channel` or `chat:create-channel`, so none of them is free to mint, and
none opens the grinding path point 2 exists to close.

`ChannelRotation` is the channel analogue of `EpochRotation` and inherits its discipline
wholesale: tentative until finality (k = 10 capability-gated actions **and** T = 30
minutes), prior channel-epoch secrets retained until then, re-welcome on a voided branch.

**One validity rule beyond ordinary capability checking:** a `ChannelDefinition` (or any
other channel entry) is **rejected on replay in a `conversation`-profile network** —
`03` §4.1's profile distinction, enforced where it can actually be enforced. The profile
lives in replayed policy state (E9), so this is a deterministic verdict every node
reaches identically, not a client-side convention a modified client could ignore.

**Acceptance:** replay produces current channel state deterministically; a channel entry
in a conversation-profile network is refused by every node; a `ChannelRotation` on a
losing branch is voided and appears in the voided-actions report; a `DekWrapping` whose
`rotation_ref` names a voided channel rotation is treated as stale and re-wrapped
(Storage §5.3.1, unchanged mechanism).

**Known cost:** log growth at high channel counts and high private-channel churn. No
message, edit, reaction or redaction ever enters the log — only channel structure does.
Carried as `00` §6 item 3 with checkpointed replay as the mitigation.

---

## 3. E3 — Derived Pointer Ids (Not Needed)

**Withdrawn: the existing API already permits this.** `intranet_storage::new_pointer_id`
mints randomly, which is what prompted this item — but `PointerId::from_bytes` is a public
`const fn`, so a client derives an id by hashing whatever it likes and wrapping the
result. No addition to the storage crate is required, and none should be made: the
derivation inputs and their domain separation are the *client's* business, and putting a
chat-shaped constructor in the storage layer would push application concerns down into a
crate that deliberately does not have them.

Implemented in `kols-core::ids` instead (`author_log_pointer`, `moderation_log_pointer`),
with the derivations pinned by test vectors.

**Verified:** the derived id is stable across runs; existing pointer ownership, version
and collision rules apply unchanged — in particular two devices of one identity colliding
on a version resolve by lower record hash with the loser retrying (Storage §2.2), which
`05` §6 relies on.

---

## 4. E4 — Gossipsub Behaviour for Live Delivery

`MemberBehaviour` carries a fixed set of request/response protocols and no
publish/subscribe. Chat's live path (`01` §7) needs one topic per channel, subscribed on
demand.

- Add gossipsub to `MemberBehaviour`, with message signing left to the payload (records
  are already individually signed — `01` §3.3) and the topic id derived from the channel.
- Expose subscribe/unsubscribe/publish on `MemberNode`, and surface received messages as
  a `NodeEvent` variant.
- Validate on receipt: signature, current membership, and the author's `chat:post`
  permission against replayed state — the same three-part discipline Storage §2.5 requires
  for append-set entries, applied here because the reasoning is identical.

**Acceptance:** a client with gossip disabled converges on identical history through the
durable path alone; duplicate and out-of-order delivery are idempotent; unsubscribing a
channel stops its mesh maintenance.

*Alternative if per-channel mesh overhead proves heavy at scale: an HRW-selected fanout
tier, which `intranet_realtime::assign_tier` already computes. Measure first.*

---

## 5. E5 — Media Fan-Out at the Relay

**This is a correctness gap in the current relay, not merely an optimization.**
`MediaEnvelope` carries one `to`, and `MemberNode::relay_call` forwards one envelope to
that one recipient, so a sender in an n-party relayed call emits n−1 envelopes per frame.
The relay therefore does not reduce sender upload at all — which is the entire stated
reason Real-Time §1.1 switches to a relay past 4–5 participants.

Required change: a fan-out form in which the sender emits **one** envelope per frame and
the relay replicates it to the participant set minus the sender.

```
MediaEnvelope { call, from, to: Recipient, frame }
Recipient = One(PerNetworkIdentityId) | Participants
```

Constraints that must survive the change:

- The relay still forwards **only** for calls it agreed to carry and **only** to
  participants it was told about — without both checks it is an open reflector
  (Real-Time §2.2, and the current implementation is careful about this).
- The relay still cannot decrypt: `Participants` is routing metadata outside the AEAD,
  which is exactly what §2.2 already permits it to see.
- A misrouted frame still fails to open, because the nonce binds the call.

**Acceptance:** in an n-party relayed call, sender upload is one frame per interval
regardless of n; a relay refuses `Participants` fan-out for a call it never agreed to
carry; the existing per-recipient form still works for mesh.

**Until this lands, the client stays in mesh** (`04` §3.1) — switching to a relay that
does not reduce upload makes calls worse, not better.

---

## 6. E6 — QUIC Datagram Media Path

Real-Time §1.5 requires unreliable, unordered media delivery. What ships is
request/response over a reliable stream, which §1.5 permits only as a fallback. The
blocker is upstream: quinn supports datagrams, `libp2p-quic` disables them at
construction (`datagram_receive_buffer_size(None)`) and exposes no datagram API.

Two routes:

- **Fork or patch `libp2p-quic` to enable datagrams and expose them**, and pursue the
  change upstream. Cleaner, keeps one transport and one NAT traversal path, and benefits
  the protocol rather than just this app.
- **A side UDP socket**, negotiated over call signalling and secured by the existing call
  key. Entirely in our hands, and duplicates hole-punching we would rather not duplicate.

**Recommendation: pursue the fork; do not build the side socket unless the fork is
refused upstream and voice quality is blocking users.** The client is insulated either
way by the `MediaTransport` interface (`04` §5), so this can land after voice ships.

**Acceptance:** under 5% packet loss, audio degrades to isolated concealed frames rather
than multi-frame gaps; `characteristics()` reports `Unreliable`; the fallback remains
selectable for peers where no unreliable path exists, which §1.5 permits explicitly.

---

## 7. E7 — Channel-Scoped MLS Groups

`intranet-epoch` provides `GroupSession` for the network group and an `EpochKeyring`
keyed by rotation reference. Private channels — `#mods`, `#planning`, a subset of a
server's members sharing that server's roles and context, **not** direct messages, which
are their own networks (`03` §4) — need the same machinery at channel scope: one group per private channel, its own keyring, rotations anchored by
`ChannelRotation` (E2).

Most of this is composition rather than new cryptography — `GroupSession` is already
generic over its membership, and `EpochKeyring` already tracks rotations by reference.
What is new is scoping: a keyring per channel, key delivery to a channel's new members
over the existing `/intranet/epoch-key/1.0.0` path extended with a scope, and the
tentative-retention window applied per channel rather than once per network.

**Acceptance:** a member removed from a private channel cannot decrypt content wrapped
after that channel's rotation, and can still decrypt what they already held; a channel
rotation voided by reconciliation is recoverable by re-welcome from retained prior
secrets; network rotations and channel rotations do not interfere.

---

## 8. E8 — Track Metadata in Sealed Media Payloads

Video, screen share and audio can be sent simultaneously by one participant, so frames
need track identification. Putting `track_id` and `kind` **inside** the sealed plaintext
(`04` §6) keeps the relay's visible metadata to what Real-Time §2.2 permits and avoids a
wire change when a track type is added later. This is mostly a client-side convention;
the protocol change is limited to the nonce binding call, track and sequence rather than
call and sequence.

**Acceptance:** a frame replayed into a different track fails to open.

---

## 9. E9 — App-Layer Policy Map ✅ **landed**

Chat's abuse limits (`01` §10) must be network policy: they are validity rules, so every
node has to reach the same verdict, and they spend other members' storage and bandwidth.
`NetworkPolicy` is the right home — it is replayed, ordered, gated on `define-policy`,
and already carries tunables of exactly this shape (`replication_factor`,
`mesh_relay_threshold`, `target_chunk_size`).

The question is whether chat's keys go in as named fields. **They should not.** Core §0
is explicit that the platform is deliberately not shaped around one application, and a
`chat_message_rate_per_minute` field in the core protocol's policy record would be
precisely that. Instead:

```
NetworkPolicy {
  …existing fields…
  app_policy: BTreeMap<String, PolicyValue>,     // namespaced keys, e.g. "chat:message-rate-per-minute"
}
```

The protocol **stores, orders and encodes** these values; it does not interpret them.
This is the same division `extension_capabilities` already uses — a registry the
governance layer carries on behalf of consuming specs without knowing what the entries
mean — so it is a precedent being followed rather than a new idea.

One key is load-bearing beyond tuning: **`chat:network-profile`** (`server` or
`conversation`, `03` §4.1) is what E2's replay rule keys off, so it must be set at genesis
and encoded like any other policy value. A network with no profile declared is treated as
`server`, which is the safe reading — it permits channel entries rather than rejecting
history a client might legitimately hold.

Namespacing is required (`chat:` here), so two applications on one network cannot collide
on a key. An unrecognised key is preserved on replay and ignored by clients that do not
know it, which is what lets a network run a newer client alongside an older one without a
policy migration.

**Acceptance — all met**, tested in `intranet-governance/tests/conformance.rs`: values
survive replay identically; a change is an ordinary `PolicyChange` gated on
`define-policy`, and an ordinary member's attempt is refused; encoding is order-independent
so two nodes building the same logical policy produce identical bytes; an unknown key
round-trips unchanged; an unnamespaced key is refused at the decoder.

Specified in **Core §2.6.2**, which also records the one asymmetry worth knowing: an absent
policy value means the consuming spec's default, while an absent *capability* tier is
refused. A missing tier would let a governance-tier grant pass as ordinary; a missing
setting just means nobody changed it.

Client-side accessors are in `kols-core::policy` — `ChatPolicy`, the key names, the
defaults from spec 07 §4.3, and the profile reading.

*Flagged: a later refinement could allow delegated, namespace-scoped policy edits — so a
moderators group could tune chat limits without holding `define-policy` over admission
mode and governance model. Not needed for v1, because per-channel slowmode (`01` §10.3)
already covers the delegable case that actually comes up.*

---

## 10. E10 — Direct Delivery for DM Invitations

A direct message conversation is its own network (`03` §4), which leaves exactly one
thing to solve: getting the invitation to somebody you can currently only reach inside a
shared network. It must not be published content — putting it in a channel makes it
public, and putting it in any durable store puts a private request onto other people's
nodes, which is the whole thing this design avoids.

A small request/response protocol, `/chat/dm-invite/1.0.0`, member to member:

```
DmInvite {
  invite:        the new network's invite (Core §5.6)
  identity_link: signed statement binding the sender's identity in this network
                 to the identity issuing the invite (Core §1.2)
  signature
}
DmInviteAck = Accepted | Declined | Blocked
```

Nothing is stored by anyone but the two parties, and nothing enters any log. Delivery
requires both to be reachable; an undelivered request retries from the sender's client.

**Acceptance:** the recipient can verify the identity link without any third party; a
forged link fails verification and the request is refused; the invite is never written to
storage or to a governance log by either side; rate limiting applies per sender identity,
so this cannot become a spam channel.

*Flagged: blocking is a client-side list, since there is no shared state to record it in
and no reason to want one. A blocked sender's requests are refused before display.*

---

## 11. E11 — Namespace Registration for Extension Capabilities

**Found while implementing permission resolution, not by review.** The tier registry
(`NetworkPolicy::extension_capabilities`) matches capability names **exactly**. Every chat
permission is parametrized by scope — `chat:post:<channel>`, `chat:manage-channel:<cat>` —
so as it stands each scope needs its own registry entry, added by a `PolicyChange`
governance entry. Creating a channel with a permission override would mean amending
network policy, which is a heavyweight action for a routine one, and the registry would
grow with the channel count.

Note the protocol did not hit this itself because its own parametrized capabilities are
**built-in variants** with computed tiers — `ManageMembership(GroupId)` derives its tier
dynamically from the target group. Extensions get only `Extension(String)` plus exact
match, so a consuming spec with parametrized capabilities has nowhere to put them.

The fix is to let a registry entry cover a namespace rather than one name:

```
extension_capabilities: BTreeMap<String, Tier>   // "chat:post:" → Ordinary, by prefix
```

Resolution takes the **longest matching registered prefix**, so a more specific
registration still wins and an unregistered name is still refused. One network-policy
entry per verb then covers every scope of that verb, forever.

**Acceptance:** an unregistered name is still refused outright; a governance-tier
namespace still cannot be granted to `everyone` at any scope within it; longest-prefix
resolution is deterministic across nodes; existing exact-match registrations keep working.

**Workaround until then**, implemented in `kols-core::capabilities`: register the
network-wide form of each verb (`chat:post:*`) at genesis, and a scope's names when that
scope is created. Workable, and it makes per-channel overrides cost a policy change —
which is the friction E11 removes.

---

## 12. Sequencing

```
P0   — nothing; E3 turned out to need no protocol change
P1   E2 → E4 ; E9, E11 (independent, small)
P2   E7   (depends on E2's ChannelRotation) ; E10 (independent, small)
P3   E5   (before relayed voice is worth enabling) ; E6 in parallel, lands when it lands
P4   E8
```

E9 is small and unblocks the rest. E2 is the first item requiring real spec work. E5 should be treated as a protocol bug fix and can be done independently of the
chat client — it improves any consumer of the call path.

---

## 13. What This Design Deliberately Does Not Ask For

Recorded so the boundary is visible, and so nobody adds them under the impression they
were oversights:

- **No app execution sandbox in the protocol layer.** App Hosting §3.2.1 places it in the
  client, deliberately, and `distributed-intranet`'s own guidance says not to add one
  there. The chat client provides its own isolation if and when it renders published apps.
- **No cross-network shared state.** No shared identity, no federated search, no directory
  of networks — every one of those would break the unlinkability the identity model exists
  to provide (Core §1.2). Note what this does *not* forbid: a client holding the master
  seed knows locally that several networks are all yours, which is what makes a DM inbox
  spanning networks possible (`03` §4.6). That correlation exists only inside the client,
  and nothing about it is exposed to any peer.
- **No reputation input to placement or ordering.** `reliability_signal` stays local-only
  and may feed exactly two selection decisions (swarm source selection, media relay
  selection). Nothing in this design reads it anywhere else, and nothing should.
- **No new content-merge semantics.** Storage §2.2 settles which record wins on a version
  collision and explicitly declines to define what two concurrent edits *mean* together.
  This design does not need merge semantics, because author logs are single-writer: two
  edits to the same message can only come from the same author, in that author's own
  order.
- **No role hierarchy.** Discussed and declined in `02` §5; if it is ever wanted, it
  belongs in the governance policy module, not in the client.
