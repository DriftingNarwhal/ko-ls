# Required Protocol Extensions

**Document status:** v2.2 — **E16 added**: there is no way to leave a network, because every membership change is gated on `revoke-node` and the one member who knows they are leaving is the one who cannot say so; §16 also settles the rejoin question the client was carrying as open, which turns out to be answered already for `forget` and settled since for the leave that would have kept the seed — the client is deliberately not growing one, and §16 says why the protocol should permit it anyway. Previously v2.1 — §13 records that D29 turned E13 from a friction item into the mechanism, and §12 records that `Discovery::Off` is a privacy requirement rather than a saving; both were being carried in a status file. Previously v2.0 — **E14 landed**, as leaf replacement rather than the re-delivery this document asked for;  E1 and E3 withdrawn, **E9, E2, E5, E4, E11 and E12 landed**; E12 narrowed to its protocol half on landing, E13 added from `09`, E14 added from a bug, **E15 added from a divergence this document should have been carrying already**. §2's branch-length and profile-enforcement claims corrected to what landed
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
| ~~E2~~ | Channel governance entries — **landed generically**, Core §2.7.2; chat payloads in spec 07 §3.8 | — | Done |
| ~~E3~~ | ~~Derived pointer ids~~ — **already possible**, no change needed | — | None |
| ~~E4~~ | Gossipsub behaviour for live delivery — **landed**, Core §5.1 | — | Done |
| ~~E5~~ | Media fan-out at the relay — **landed**, Real-Time §2.2.1 | — | Done |
| E6 | QUIC datagram media path | P3 (quality) | Large, partly upstream |
| E7 | Channel-scoped MLS groups | P2 | Large |
| E8 | Track metadata in sealed media payloads | P4 | Small |
| ~~E9~~ | App-layer policy map in `NetworkPolicy` — **landed**, Core §2.6.2 | — | Done |
| E10 | Direct member-to-member delivery for DM invitations | P2 | Small |
| ~~E11~~ | Namespace registration for extension capabilities — **landed**, Core §2.2.1 | — | Done |
| ~~E12~~ | Optional peer discovery — **landed**, Core §5.1.1. Asked as tiered liveness; only the behaviour set was the protocol's, and the tiering stayed in the client | — | Done |
| E13 | Cross-network connection bootstrap, for direct messages — **load-bearing since D29, not merely convenient**: without it a conversation across NAT needs its own relay, so there are no DMs at all (§13) | P2 | Medium |
| ~~E14~~ | Idempotent epoch-key delivery — **landed**, Core §3.5.1. Asked as key re-delivery; landed as leaf replacement, because re-delivery restores no group state and re-adding silently breaks revocation | — | Done |
| E15 | Independent per-network seeds — Core §1.1's master seed is not what this client implements (D28) | Nothing; conformance rather than function | Spec text only |
| E16 | Self-removal from a group — a member cannot say they are leaving, because every membership change is gated on `revoke-node` (§16) | Leaving a network at all | Small |

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

**This section asked for the opposite of what landed, and the reversal is worth keeping.**
It argued all four are capability-gated and therefore *count* toward branch length (Core
§2.7.1 point 2), since each requires `chat:manage-channel` or `chat:create-channel` and
none is free to mint. Core §2.7.2 excludes them instead, and is right: whether an
application entry is cheap to mint depends on the tier of the capability it declares, and
answering that means resolving a tier against replayed state — which the branch-length
metric deliberately cannot do. A generic carrier cannot tell a scarce declaration from a
cheap one, so counting them would reopen the grinding path point 2 exists to close.

**What it costs is real and bounded.** Channel structure carries no weight in fork choice,
so a partition may void a definition a competing branch never saw. That is acceptable
because everything which *must* survive a partition — membership, revocation, policy,
epoch rotation — is a core entry that still counts, and a voided channel entry is
resubmittable from the voided-actions report like any other (`02` §5, `05` §4). Spec 07
§1.3 states this obligation normatively; a client that ignored the report would silently
lose channels to a heal.

`ChannelRotation` is the channel analogue of `EpochRotation` and inherits its discipline
wholesale: tentative until finality (k = 10 capability-gated actions **and** T = 30
minutes), prior channel-epoch secrets retained until then, re-welcome on a voided branch.

**One validity rule beyond ordinary capability checking:** a `ChannelDefinition` (or any
other channel entry) is **refused in a `conversation`-profile network** — `03` §4.1's
profile distinction. The profile lives in replayed policy state (E9), so every reader that
understands the `chat` namespace reaches the same verdict.

**Whose replay, though, and the generic form changed the answer.** This section originally
said "rejected on replay" without qualifying it, which read as the protocol enforcing the
rule. It cannot: it carries `chat` payloads without decoding them, so the rejection is an
application-layer obligation and spec 07 §1.2 states it as one. What that costs is bounded
— minting the entry still needs the declared capability, and every conformant client
refuses it — but a client that ignored the profile would see a channel where others see
none, which is a weaker guarantee than the original wording promised.

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

## 4. E4 — Gossipsub Behaviour for Live Delivery ✅ **landed**

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

**Acceptance — all met.** In `intranet-transport/tests/live_delivery.rs`: a payload reaches
a subscriber byte-identical on the topic it was published to, a node that never subscribed
hears nothing, unsubscribing stops delivery, and publishing to an empty topic is not a
failure worth reporting. In `kols-core`: a payload round-trips to the identical record with
the same id, refuses to open under the wrong epoch or a swapped rotation, refuses a record
relayed into another channel, and fails closed on tampering. Over two live `kols` nodes: a
message posted while a peer is watching arrives live and **exactly once**, since the durable
copy that follows is the same content-addressed record; and history still converges with
nothing arriving live at all.

**Three things landed differently from the proposal, each for a reason.**

- **Message signing is off, not "left to the payload" as an aside.** A chat record already
  carries its own signature over its own canonical bytes, so gossipsub signing them again
  with the transport keypair would give a receiver two authorities for "who wrote this" and
  no rule for choosing. `gossip_behaviour` takes no keypair at all, so the absence is
  visible in the signature rather than implied.
- **Message ids are content hashes.** The default is derived from sender and sequence
  number, which would make the same record arriving live and again in a segment two
  different messages — and idempotent duplicate delivery is a requirement, not a nicety.
- **The transport validates nothing.** It cannot: it does not know what a payload means.
  Signature, membership and `chat:post` are all checked in the client, and doing half of it
  in the transport would be worse than none, because a caller would read it as done.

**The live payload is sealed** under spec 07 §5.2's channel content key, derived from the
epoch and bound to both channel and rotation — so a payload cannot be opened by a
non-member on the mesh, cannot be replayed into another channel, and survives being
published either side of a rotation because it carries the `rotation_ref` its sender used.

*Alternative if per-channel mesh overhead proves heavy at scale: an HRW-selected fanout
tier, which `intranet_realtime::assign_tier` already computes. Measure first.*

---

## 5. E5 — Media Fan-Out at the Relay ✅ **landed**

**This was a correctness gap in the relay, not merely an optimization.**
`MediaEnvelope` carried one `to`, and `MemberNode::relay_call` forwarded one envelope to
that one recipient, so a sender in an n-party relayed call emitted n−1 envelopes per
frame. The relay therefore did not reduce sender upload at all — which is the entire
stated reason Real-Time §1.1 switches to a relay past 4–5 participants.

The change: a fan-out form in which the sender emits **one** envelope per frame and the
relay replicates it to the participant set minus the sender.

```
MediaEnvelope { call, from, to: Recipient, frame }
Recipient = One(PerNetworkIdentityId) | Participants
```

**Landed as proposed**, specified in Real-Time §2.2.1. Four things are worth recording —
the first is why the shape above was right, and the other three were found while building
it rather than by review:

- **`Participants` carries no list, and that is the safety property.** The fan-out set is
  the participant list the relay was told when it agreed to carry the call — never one
  travelling in the envelope. This makes fan-out *stricter* than the per-recipient form it
  replaces: under the named form a sender in a carried call names its own target and the
  relay checks it, while under fan-out a sender has no field in which to ask for a
  non-participant at all.
- **The relay readdresses each forwarded copy to `One(recipient)`.** Without this a
  participant would receive a `Participants` envelope, and a participant that also relays
  would fan it out again. Readdressing means a participant never holds a fan-out envelope,
  so a forwarding loop has nowhere to start, and a receiver's "is this for me" check is
  the same on both paths. Rewriting is safe for the reason misrouting always was: routing
  metadata is outside the AEAD, and a misdelivered frame still fails to open.
- **The claimed sender must be the peer that connected — a check that did not previously
  exist on this path.** A media envelope carries no signature by design (§2.2 puts
  authenticity in the AEAD), so `from` is a claim, and the relay is the one node that
  cannot check a claim against the frame. It was already worth one forwarded frame to an
  attacker under the named form; under fan-out it is worth N−1 sends at the relay's
  expense to anyone who knows a carried call's id. The relay now binds `from` to the
  connection, the same answer `intranet-storage`'s chunk requests and the signalling path
  already use. **Found while implementing the fan-out, not by review.**
- **The wire break is versioned rather than smuggled.** The recipient field gains a
  discriminant, so the media envelope's domain tag advances to
  `intranet.wire.call-media.v2`. Under the v1 tag an old envelope's first recipient byte
  would have been read as the discriminant and, twice in every 256 envelopes, parsed into
  a plausible wrong answer. Domain separation turns that into a decode failure, which is
  what it is for.

**Amplification is bounded by the relay's own agreement, not by the protocol.** One
envelope in becomes N−1 out, which is both the point and the shape of an amplifier. The
bound is that a relay chose the call and the participant set it accepted; a node unwilling
to carry a large call declines at that point.

**That bound is now enforced rather than assumed** — Real-Time §2.2.2, landed immediately
after this and prompted by it. The gap was that a node's advertised `bandwidth_cap` was
read by every node *except* the one that declared it: it steered other members' relay and
source selection while the volunteer itself enforced nothing. Survivable when a relay
forwarded one envelope per envelope received; a multiplier once fan-out landed. A media
relay now bounds concurrent calls, participants per call, and sustained bytes forwarded —
charged for what *leaves* the node, since charging the inbound size would under-meter by
exactly the fan-out factor the ceiling exists to bound.

Two consequences the client owns:

- **Refusing is ordinary.** A relay at its ceiling declines, and the call renegotiates onto
  another through the mechanism it already uses when a relay drops (`04` §7). The client
  must treat a decline as a topology event, not a call failure, and show the user nothing.
- **The node's own refusals belong in the contribution UI** (`02` §6.4). A run of them is
  what "I volunteered more upload than I have" looks like from inside the node, and no
  other signal says it.

The values stay unspecified and stay local, for the reason §2.3 gives about relay
selection: a ceiling describes one node's hardware, so a network able to set it could
compel a member to spend bandwidth it never offered — the inversion of Core §4.3's opt-in.

Constraints that survived the change, as required:

- The relay forwards **only** for calls it agreed to carry and **only** to participants it
  was told about — without both checks it is an open reflector (Real-Time §2.2).
- The relay still cannot decrypt: `Participants` is routing metadata outside the AEAD,
  which is exactly what §2.2 already permits it to see.
- A misrouted frame still fails to open, because the nonce binds the call.

**Acceptance — all met**, in `intranet-transport/tests/call_media.rs` over four live nodes
and `intranet-realtime/src/wire.rs` for the encoding: a sender in a three-party relayed
call emits one envelope and the relay emits two, neither of them back to the sender; a
relay refuses fan-out for a call it never agreed to carry, for a sender outside the call,
and for a sender whose claim does not match its connection; each forwarded copy arrives
readdressed to its recipient; a relay that is also a participant hears the frame without
sending it to itself, including when it is the only other participant and has nothing to
forward; the per-recipient form still works unchanged for mesh; and the two forms cannot
decode as each other, nor a v1 envelope as a v2 one. For §2.2.2's ceilings: a relay at its
call ceiling declines and then carries nothing for that call even if frames arrive anyway;
a byte allowance runs out mid-call, refuses whole rather than partially, says why, and
resumes when refilled; and ten unit tests cover the charging arithmetic, the refill curve,
a backwards clock, and a carrier that is also a participant.

**The client may now switch to a relay at the threshold** (`04` §3.1). The advice to stay
in mesh existed only because a relay that did not fan out made calls worse than mesh, and
it no longer does.

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

## 11. E11 — Namespace Registration for Extension Capabilities ✅ **landed**

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

**Landed** as Core §2.2.1. A registration ending in `:` covers the namespace beneath it and
any other registration matches exactly, with the longest match winning. The separator
requirement was the one thing this section left open and it turned out to matter: plain
prefix matching would let a registration for `chat:post` also cover `chat:postmortem`, a
different capability silently inheriting a tier nobody chose for it.

`kols-core::capabilities::namespaces` replaces the workaround — one entry per verb, at
genesis, once. Removing it also showed the workaround was **not** actually workable as
described: it said a scope's names are registered when that scope is created, and nothing in
the client ever did that, so `chat:<verb>:<channel>` was in no registry anybody wrote and a
per-channel grant was refused at replay. The override mechanism `design/02` §4 describes
could not be used at all.

---

## 12. E12 — Optional Peer Discovery ✅ **landed, narrowed**

**Found designing the interface (`09` §2), not by review.** A client runs one node per
network, and a direct message *is* a network (`03` §4.3), so a user with a handful of servers
and thirty conversations runs thirty-odd nodes in one process. `MemberBehaviour` was a fixed
struct — Kademlia, mDNS, identify, ping, relay client, dcutr, and every request-response
protocol, on every node. That means thirty Kademlia routing tables running periodic bootstrap
queries and thirty swarms doing mDNS multicast on one LAN, to serve conversations that need
none of it.

The shape that fixes it comes from what these networks *are*: **a two-member network has
nobody to discover.** No Kademlia, no mDNS, no provider records — the one peer that matters is
known by construction.

**What landed is the behaviour set, and not the tiering, and that split is the point.** This
was written asking for three tiers — hot, warm and cold — as one extension. Only the first
half of that was ever the protocol's: the behaviour set is `intranet-transport`'s and a
consuming client cannot assemble a partial one, so **Core §5.1.1** now says a node MAY be
built without Kademlia and mDNS and keep everything else. `MemberNode::with_discovery(..,
Discovery::Off)` is that node — still listening, dialable, relaying, hole-punching, gossiping
and serving every request-response protocol.

The rest was never a protocol concern, and asking for it here would have put client policy in
a specification. *Whether a node exists at this moment, and whether it holds a relay
reservation while nothing is happening, is a decision a client makes over time* — the
protocol has no view on it and needs none. So hot/warm/cold stays in `09` §2 as client
behaviour, built on the reservation and dial primitives that already exist. **This is a
property of the network, fixed when the node is built; liveness is a property of the moment.
They read as one feature and are two, and conflating them is what would have made a
specification of something no spec should hold.**

**One consequence surfaced while implementing it.** The routing table is also the address
book, so a node without discovery dials **by address** and never by peer id alone, and
caching an address against a peer is a no-op there. A pairwise network pays nothing for that —
its peers' addresses arrive with their membership — but it is a real constraint on any other
use, and it is now in Core §5.1.1 rather than left to be discovered.

**How absence is reported, which is the part worth not getting wrong.** Provider queries and
collection enumeration are discovery operations, so on such a node there is no query to run:
`find_providers` and `enumerate_collection` return `Option` and `None` says exactly that.
Returning a query id that never resolves would be indistinguishable from *content that
genuinely has no holders* — the confusion `set_dht_server_mode` already exists to prevent, and
the reason that call has its own API surface at all.

**Acceptance, as landed:** two nodes without discovery connect by address and converge their
governance logs; discovery operations return `None` rather than a query that never resolves;
storing, serving and announcing content are unaffected. `crates/intranet-transport/tests/discovery_off.rs`.

**What the client still owes** (`09` §2, not this document): choosing `Discovery::Off` for a
conversation-profile network, and the hot/warm/cold policy over reservations.

**Neither is built, and the first stopped being an efficiency item.** `kols init` writes no
`chat:network-profile` key, so every network the client creates is a `server` and there is no
conversation network to build the leaner node for — `kols_core::policy::conversation_genesis_values`
exists for one and only a test calls it. What changed the weight of that is D29: with discovery
on, a DM node meeting a peer at the shared network's relay lands in that relay's routing table,
which is the shared-routing-table correlation D29 forbids. So `Discovery::Off` for a conversation
network is a **privacy requirement** rather than a saving, and `09` §3 depends on it being one.

**Note on the wake path, recorded because it was nearly specified as its own mechanism.** No
wake-up message is needed. Being dialable is what a reservation provides, and the dial *is* the
signal — an inbound stream wakes the handler. Core §5.3's separation of reservation limits from
circuit limits is what makes this work: a circuit is capped at 60 seconds and 256 KB precisely
because a relay assists connection establishment rather than carrying traffic, so a held
connection was never the primitive to reach for. *(120 seconds and 8 MB when this was written;
lowered 2026-08-22, which strengthens the argument rather than changing it.)*

---

## 13. E13 — Cross-Network Connection Bootstrap

**Two people starting a conversation cannot be asked to stand up a relay first** (`09` §3).
The DM network is fresh, has two members and no infrastructure, and the friction of
provisioning any would make the feature unusable.

**This item was scoped as removing friction, and D29 turned it into the mechanism.** The
escalation is recorded here rather than left in a status note, because it changes what
failing to build this costs. A relay may not be shared between two of a member's networks
(D29, `09` §3), and every direct message is its own network (D10). Those two together mean a
conversation between two NATed people would need **its own deployed relay** — which nobody
will do. So without E13 there are no direct messages across NAT at all, rather than
direct messages that are tiresome to start. It is a P2 item whose absence removes a feature,
not one that degrades it.

The material already exists. `03` §4.3 delivers the DM invite over a direct peer-to-peer
stream *inside a shared network*, carrying a voluntary identity link — a signed
common-ownership proof (Core §1.2). By the time the DM network exists, each party knows
exactly which shared-network identity the other one is.

So: **exchange the new network's addresses over the connection that already exists**, and
coordinate a simultaneous open. This is DCUtR's shape with the signalling channel being
another network's connection rather than a relay circuit. Nothing new is disclosed — each
party already knows the other's address and identity from the shared network.

**The gate, without which this is a catastrophe.** Address disclosure for network X over
network Y is permitted **only to a peer who is a member of X**, verified by replaying X's
governance log. Ungated, this is an oracle for "enumerate your other identities and their
addresses", and the unlinkability Core §1.2 provides is gone wholesale — not weakened,
gone. For a direct message the check is tight by construction, since X has two members.

**Rejected alternative:** having a shared-network node relay the DM traffic. It works, needs
no new mechanism, and is worse — it tells a third party which two identities are conversing,
where address exchange tells nobody anything they did not know. It remains the *fallback*
when hole-punching fails, not the mechanism.

**Acceptance:** an address request naming a network the requester is not a member of is
refused; a bootstrapped connection carries no shared-network identity into the new network;
hole-punch failure falls back without the disclosure gate being bypassed.

---

## 14. E14 — Idempotent Epoch-Key Delivery ✅ **landed, in a different form**

**Found by a bug, not by review, and it is the shape this project keeps meeting: an operation
with no way to recover.** `MemberNode::answer_epoch_key` calls `GroupSession::add_member`
unconditionally. There is no check for a requester already in the group, so a second request for
the same identity adds a second leaf and appends a **second `EpochRotation`** — which forks the
governance log against the entry that admitted them. Observed: a keyed member losing a grant it
held, in a test that had passed for days.

The consequence is not the duplicate rotation. It is that **the request can never be retried**,
and a request that is never answered strands a member permanently: they hold an identity, they
are in the log, honest nodes will serve them, and they can open none of it. A founder has
ordinary reasons not to answer at the moment one arrives — answering appends a rotation and so
takes the store's append lock that every one-shot command also takes — and a lost race reports a
degradation on the *founder's* terminal and tells the asker nothing.

The client can make the single request *reliable*, and now does (`05` §4). It cannot make asking
*repeatable*, and must not until this lands.

```
answer_epoch_key(request, identity, now):
    if the requester's credential already holds a leaf:
        replace that leaf with the requester's key package, in ONE commit
    else:
        add, rotate, welcome — as today
```

**This section asked for key re-delivery, and that turned out to be half a fix.** The reasoning
was that a Welcome cannot be reissued — true — and that a member already in the group therefore
needs only the key material the keyring holds. That second half is wrong, and the error is worth
keeping because it is easy to make twice. **A requester asking again has no group state by
construction:** it asks because it holds no key, and it holds no key because the Welcome that
would have carried *both* never arrived. Keys alone leave it able to read what exists now and
unable to apply any later commit, so it falls out of the group at the next membership change —
silently, having looked recovered. `apply_pending_rotations` returns immediately for a node with
no group, which is where that shows up in the code.

**And re-adding is worse than this section said, which the tests found rather than review.** The
objection here was honesty: a second leaf for one member is a lie in a log every member replays.
The real cost is a security failure. Removal is expressed against an *identity* and applied to a
*leaf*, so `leaf_index_for` finds the **first** leaf holding that credential — a revocation
therefore removes the abandoned leaf, rotates, and hands the new epoch key to the member it was
asked to exclude, who is still on the leaf nobody removed. **The removal reports success and
Core §3.1's guarantee is gone.**

**What landed: replace the leaf.** Remove the stale one and add the requester's key package in a
single commit, producing one rotation and a Welcome the requester can open. It restores the
member completely, it is the honest record of what happened, and it needs no capability of its
own since the roster before and after is identical. Specified as **Core §3.5.1**, with
`GroupSession::replace_member` staging both proposals and clearing them if either fails — a
dangling remove proposal would otherwise be swept into whatever commit came next and drop a
member nobody decided to drop.

**Acceptance — all met.** In `intranet-epoch/tests/conformance.rs`: a replacement leaves one
leaf rather than two and welcomes the member again; a replaced member can apply a later commit
and converge on the same key, which is the property key-only delivery would have lacked; and a
failed replacement removes nobody. In `intranet-transport/tests/revocation.rs`: a member who
asked for a key twice is **still revocable** — which fails against the old behaviour with the
revoked member's key fingerprint identical to the founder's. A requester may re-send the *same*
key package, which is what a node that never completed a join actually holds.

**One test was written, run against the old code, and thrown away.** It asserted that answering
twice kept the governance log linear — and it passed under both behaviours, because answering
parents on the current tip, so a duplicate add never forked anything. The fork in the original
report came from concurrent writers, not from the duplicate add; the duplicate add's damage is
the second leaf. A green test that asserts nothing is worse than no test.

*The client's retry is the point of this, and it landed with it:* `kols serve` now re-asks every
30 seconds while unkeyed rather than once, deliberately slow against a two-second tick because
every answer is a real rotation and a governance entry.

---

## 15. E15 — Independent Per-Network Seeds

**Found by reading this document against the protocol it depends on, and it had been true
for a while: the client does not implement Core §1.1's identity model and does not intend
to.** D28 (`02` §6.3) generates fresh entropy per network. Core §1.1 specifies one master
seed per *person*, "the single source of truth from which all other keys are derived", and
Core §1.2 derives each network identity from it as
`derive(master_seed, network_id, "identity")`. Those are different systems, and §0 of this
document says a needed protocol change is recorded here rather than assumed into
existence. This one was not — the divergence lived in the implementation, then in a
decision register, and never in the place that tracks what the protocol owes.

**The guarantees Core §1.2 exists for are preserved or strengthened, which is why this is
an amendment rather than a bug.** Unlinkability holds by construction rather than by
derivation: two per-network public keys are independent random keypairs, so they are
uncorrelatable without the *weaker* assumption that a derivation function hides its
inputs. Provable common ownership is unaffected, being a voluntary signed statement over
two public keys (`03` §4.3) that never referenced the seed. Determinism within one network
is unaffected. What changes is the blast radius: under a master seed, one compromised
backup is every identity its holder has, in every network, forever — and it is precisely
the object §1.1 tells a user to write down. Per-network seeds mean one leaked phrase
exposes one network.

**What the amendment must actually say**, since three sections lean on the master seed:

- **§1.1** must permit an identity's per-network keypairs to be independently generated
  rather than derived from one seed, and state the trade in both directions: derivation
  buys recovery from a single phrase, independence buys a blast radius of one network.
  Neither is wrong; a specification that names only the first leaves a conformant client
  unable to choose the second.
- **§1.2** must state its properties — unlinkability, determinism, provable common
  ownership — as requirements on the *result* rather than as consequences of the
  derivation, since independent generation satisfies all three by other means.
- **§1.3 point 3** must be restated in terms of the seed *for that network*. Enrollment
  already is per-network ("point 7"), so this is the wording catching up with the rule:
  what signs a device certificate is the per-network identity key, and where that key
  comes from is not the certificate's business.
- **§1.1's recovery clause** must say what recovery costs under each model. This is the
  half the client owes an answer to and does not yet have: a phrase alone restores nothing
  either way, because a network's id cannot be derived from it — coming back needs the
  phrase, the network id and a relay address (`02` §6.3). Under independent seeds a backup
  is a *set the client exports*, which is O7 and a release gate.

**Nothing upstream has to change for the client to keep working**, which is why this is
sequenced late rather than urgently: the protocol's own crates never see a seed, only the
per-network keypairs it produces, so the divergence costs conformance rather than
function. That is exactly the kind of debt this document exists to stop being invisible.

**Acceptance:** Core §1.1–§1.3 describe both models and require the properties rather than
the mechanism; a client generating independent per-network seeds is conformant; §1.3's
enrollment wording names the per-network key rather than the master seed; and the recovery
clause states what a backup must carry under each model rather than implying a phrase is
sufficient.

---

## 16. E16 — Self-Removal From a Group

**There is no way to leave a network, and the client's `forget` is not one.** It deletes
this installation's store and the seed with it. To every other member nothing has happened:
the departed identity is still in `everyone`, still in every role it was put in, still a
leaf in the MLS group, and still shown in the roster as a member who is simply never
connected. Every subsequent epoch rotation wraps the group's key material for a leaf whose
private half was deliberately destroyed.

None of that is a break — a member who deleted their seed cannot read anything, so nothing
leaks — but three things are wrong in a way that compounds:

- **The group only grows.** MLS removal is the only thing that takes a leaf out, and
  nothing here ever calls for one. A network that has churned through fifty members carries
  fifty leaves and rotates all of them, forever.
- **The roster lies by omission.** "Not connected" already covers away, unreachable and
  never dialled (`09` §4.1), and it now also covers *gone permanently*, which is the one
  case a reader would actually want distinguished and the only one this client could
  know about.
- **A leak that happened before the deletion never expires.** If the seed was ever copied,
  its holder keeps receiving key material for every future epoch. Revocation exists for
  exactly this and nobody thought to invoke it, because the person who knows is the person
  leaving.

### What the log can express, and what it cannot

`EntryBody::MembershipChange { group, identity, action: Remove { cascade } }` is the right
entry and already exists. It is gated on `Capability::RevokeNode`, which a departing member
almost never holds — so the one member who knows they are leaving is the one member who
cannot say so. The protocol has no concept of leaving at all: Core §2 covers admission and
revocation, and both are things done *to* a member.

### The rule this asks for

**A `MembershipChange` removing the entry's own author requires no capability.**

The reason it is safe is the reason it is worth stating as a rule rather than a special
case: the entry is **monotone downward and self-directed**. It grants nothing, it names
nobody but its signer, and its signature is the same one that proves the identity it
removes. There is no version of it that escalates, and no version of it that touches
another member. That is a materially different object from the capability-gated
`MembershipChange` beside it, which is why it does not weaken the gate that entry keeps.

Two things the amendment has to settle rather than leave implied:

- **Whether a departure rotates the epoch, and who pays for it.** It has to eventually —
  a leaf that stays in the tree is the whole point above — but `RotationReason::MemberRevoked`
  is `revoke-node`'s to trigger, and letting a departure force one directly creates a
  grinding path: under `admission: auto` (Core §2.4) a member holding a multi-use invite
  can join, leave, join and leave, forcing a real rotation and a replayed governance entry
  each time. The cheap resolution is that a departure **records** the departure and does not
  itself rotate; the next rotation excludes them, and a `revoke-node` holder may rotate on
  seeing one. That keeps the cost on the side that already bears it and leaves the guarantee
  intact, because a departed member who kept their seed is exactly a member who has not been
  revoked yet.
- **That it is a statement, not a request.** The removal takes effect on replay like any
  other entry. A design where leaving is a request some admin approves would mean a member
  cannot leave a network whose admins have all gone, which is the state a person most wants
  to leave.

### Coming back, which is not the question it looks like

Whether a departed member may rejoin **is already answered for `forget`, and not by this
extension**: `forget` destroys the seed, so that identity cannot return under any rule we
write. The old record is orphaned by construction. Nothing has to decide it.

The question would be live for a **leave that keeps the seed**, and `02` §6.5 records that the
client is deliberately not growing one: leaving is what people do when they are done with a
network, and re-admission as the same member is a property nobody asked for. So for this
consumer the question is settled in both directions.

**The protocol should still permit it, and this is the one place that distinction earns its
keep.** A rule saying a self-removal is valid says nothing about whether the remover kept
their keys, and it must not — another consumer of this platform may well want a leave that can
be undone, and a spec that assumed the seed goes with the departure would have written one
client's product decision into the wire. Re-admission after a self-removal is ordinary
admission: the log records *added, removed, added*, and everything they wrote stays attributed
to the identity that wrote it. Anything stronger would be a ban, and this protocol has no such
concept — building one in as a side effect of leaving would be the largest thing in this
section and nobody asked for it.

**Ordering, which the client owes and the protocol should say plainly.** The departure entry
is signed by the key being destroyed, so it must be written *and published* before the seed
goes. A node that is offline or has no route cannot announce anything, so leaving is
best-effort and the interface must say which of the two happened rather than reporting
success either way.

**Recorded in the protocol repository as of 2026-08-31**, as a row in `specs/07` §7 — where
the platform work is actually scoped. It was specified here for a week and nowhere over there,
which meant the one list a protocol reader consults did not carry it.

**Acceptance:** Core §2 states that a membership removal naming its own author is valid
without a capability, and says what it does and does not do to the epoch; `intranet-governance`
accepts such an entry and refuses one that removes anybody else; a member with no grants can
remove themselves from `everyone` and every group they are in; and the departure is visible
to other members as a member who left rather than as one who is not connected.

---

## 17. Sequencing

```
P0   — nothing; E3 turned out to need no protocol change
P1   E2 → E4 ✅ ; E9 ✅, E11 ✅ (independent, small).  E5 landed here, ahead of its phase
P1   E12 ✅ (independent of everything else; landed as its protocol half only)
P1   E14 ✅ (independent, small — and a joiner stalls forever without it)
P2   E7   (depends on E2's ChannelRotation) ; E10 (independent, small) ; E13 (pairs with E10,
          and gates DMs across NAT entirely rather than easing them — §13)
P3   E6   — lands when it lands
P4   E8
—    E15  (blocks nothing; land it beside the credentials and backup work `00` §5 carries as a
          release gate and `02` §6.3 shapes, which is what it describes)
—    E16  (blocks leaving a network, which nothing else waits on; small, and independent of
          every other item here)
```

E9 is small and unblocks the rest. E2 is the first item requiring real spec work.

**E15 carries no phase deliberately.** It blocks nothing and nothing waits on it, because
the divergence costs conformance rather than function — so sequencing it against a phase
would overstate its urgency. It belongs next to the credentials and backup work (`00` §5's
release gate, shaped by `02` §6.3), since that is where the client finally has to say what a
per-network seed means to a user, and amending the spec while writing that is cheaper than
amending it twice.

**E5 was pulled forward out of P3 and landed in P1**, which the sequencing above always
permitted: it was scoped as a protocol bug fix independent of the chat client, and it
improves any consumer of the call path rather than only this one. Doing it early also cost
less than doing it late — the media envelope's wire format changed, and every additional
consumer written against the old shape would have been another thing to migrate.

---

## 18. What This Design Deliberately Does Not Ask For

Recorded so the boundary is visible, and so nobody adds them under the impression they
were oversights:

- **No app execution sandbox in the protocol layer.** App Hosting §3.2.1 places it in the
  client, deliberately, and `distributed-intranet`'s own guidance says not to add one
  there. The chat client provides its own isolation if and when it renders published apps.
- **No cross-network shared state.** No shared identity, no federated search, no directory
  of networks — every one of those would break the unlinkability the identity model exists
  to provide (Core §1.2). Note what this does *not* forbid: a client holding the workspace
  knows locally that several networks are all yours, which is what makes a DM inbox
  spanning networks possible (`03` §4.6). The knowledge comes from that directory and not
  from a shared secret — seeds are per network (D28, `02` §6.3), so there is no master key
  whose compromise would correlate them either. That correlation exists only inside the
  client, and nothing about it is exposed to any peer.
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
