# Voice, Video and Stage

**Document status:** v1.0 — design reviewed. Nothing implemented; P3/P4
**Depends on:** Real-Time Transport Spec (all), Core Protocol Spec §4.4 (relay roles), `03-confidentiality`
**Consumed by:** `05-client-architecture`, `06-protocol-extensions`

---

## 1. Voice Channels Are Long-Lived Calls

A Discord voice channel is a room you walk into: it exists whether or not anyone is in
it, joining is one click, and there is no ringing. That maps onto the protocol's call
primitive with one adjustment — the call is bound to a channel and outlives any
particular participant set.

```
voice channel (ChannelDefinition, kind: Voice)
└── call session   call_id = H(channel_id ‖ "chat:voice")     — stable, derived
    ├── participants: joins and leaves are ordinary Signal traffic
    ├── topology:     mesh → relay → stage, chosen by size (§3, §4)
    └── key:          session key (public channel) or channel key (private) — `03` §2
```

`call_id` is derived rather than generated, so any member can join a voice channel by
computing its id from the channel — the same reasoning as derived pointer ids in
`01` §3.2. No rendezvous record, no directory lookup, no race between two people being
first to "create" the call.

**Presence in a voice channel is public to the channel's readers** and rides the same
ephemeral gossip as text presence (`01` §9), not the call signalling path — you can see
who is in a voice channel without joining it, which is table stakes for this UX and which
the signalling channel alone would not give you.

---

## 2. Keying

| Channel privacy | Media key | Rekey trigger |
|---|---|---|
| Public | `CallKey`, generated per session, sealed to each participant via `CallKeyEnvelope` | Participant joins or leaves |
| Private | Derived from the channel's MLS group (`03` §3) | Channel roster change |

Public voice rekeys on **every** join and leave. Without it, someone who joins at 14:00
can decrypt frames captured at 13:00 if they recorded the ciphertext, and someone who
leaves keeps decrypting until the call ends. Rekey is O(n) sealed envelopes, which is
cheap for the sizes the mesh and single-relay topologies cover, and is one of the reasons
large rooms move to a different mechanism entirely (§4).

Private voice does not need its own key at all: the channel MLS group already produces
one with forward secrecy and O(log n) rekey, and the roster change *is* the rekey
trigger. This is the payoff for `03` D9 — private voice comes free with private text.

**A media relay is blind in both cases** (Real-Time §2.2): it speaks
`/intranet/call-media/1.0.0` and never `/intranet/call-signal/1.0.0`, so key envelopes
never cross a channel it carries. That is an architectural property, not a promise, and
it is worth preserving carefully in the client: any temptation to route signalling
through a relay for convenience destroys it.

---

## 3. Topology by Size

Following Real-Time §1.2, with the threshold as network policy:

| Participants | Topology | Cost to each sender |
|---|---|---|
| 2–4 (policy default) | **Mesh** — every participant sends to every other | (n−1) × bitrate upload |
| 5 – ~50 | **Blind relay** — one selected relay forwards | 1 × bitrate upload, once §3.1 is fixed |
| ~50+ or stage | **Broadcast** — live-stream distribution (§4) | 1 × bitrate upload, flat in audience |

Transitions use the protocol's one renegotiation mechanism — trigger, propose,
tie-break, make-before-break (Real-Time §1.4) — which `intranet-realtime::CallSession`
already implements (`evaluate`, `propose`, `receive_proposal`, `complete_handover`). The
client's job is to drive it, not to reimplement it.

Relay selection comes from `intranet_realtime::relay::select` over
`relay_media_willing` ledger entries, weighted toward latency and jitter, and permitted
to use local `reliability_signal` — one of only two places in the entire design where
that signal may be read (Real-Time §2.3).

### 3.1 A relay currently does not reduce sender upload — this must be fixed

**Finding, from reading the implementation rather than the spec.** `MediaEnvelope` carries
a single `to`, and the relay forwards one envelope to that one recipient
(`MemberNode::relay_call` checks that both `from` and `to` are participants, then
forwards). A sender in a five-person relayed call therefore emits four envelopes per
frame — exactly the upload cost that Real-Time §1.1 says the relay exists to avoid.

The relay is faithfully blind and faithfully limited; it just isn't saving anybody
anything. **A fan-out form is required**: the sender emits one envelope per frame, and
the relay replicates it to the participant set minus the sender. Recorded as a required
protocol change in `06` §5, with the wire detail there.

Until it lands, relayed calls behave like mesh calls with an extra hop, which is worse
than mesh. The client should therefore stay in mesh until the fix exists rather than
switching at the threshold and getting slower.

---

## 4. Stage / Large Rooms Use the Stream Path

Past roughly fifty listeners, a call is the wrong primitive regardless of how well the
relay fans out: everyone is receiving n streams, key envelopes are O(n) per membership
change, and the interaction pattern is one-to-many anyway.

**Stage mode is Real-Time §3's live-propagating swarm**, not a call:

- A small **speaker set** runs an ordinary call among themselves (mesh or relay), mixes,
  and publishes the mixed output as a live stream.
- Distribution uses `LiveStream` with the HRW first tier over gossiped `bandwidth_cap`
  (Real-Time §3.3) — deterministic, coordination-free, no broadcaster nomination.
  `intranet_realtime::assign_tier` already computes it.
- Listener count costs the speakers nothing: broadcaster upload is flat in audience size,
  which is the property that makes a 3,000-listener room possible at all.
- Promoting a listener to speaker is adding them to the speaker call. Demoting is the
  reverse. Neither disturbs distribution.

Two consequences to be honest about:

- **Stage listeners are seconds behind**, by design (Real-Time §3.1). That is correct for
  a broadcast and unacceptable for conversation, which is why speakers stay on the call
  path and only the outbound mix is streamed.
- **Stream redistributors are not blind** (Real-Time §3.5). They are ordinary members
  holding the epoch key, so they *could* decrypt what they forward. For a public stage
  that is fine — everyone forwarding is entitled to listen. **A stage in a private
  channel must be keyed to the channel group** (`03` §3), which is exactly the scoped-key
  case Real-Time §3.5 flags and declines to solve; this design solves it, because private
  channels already have the key.

**Recording a stage is free.** Real-Time §4 converts a finished broadcast into ordinary
immutable content with no re-encryption, so "record this stage" is a per-broadcast flag,
not a feature. Opt-out prevents publication of a discoverable record; it cannot stop a
listener keeping their own copy (§4.2 of that spec), and the UI must say so rather than
implying a stronger guarantee.

---

## 5. The Media Transport Interface

Real-Time §1.5 is unambiguous: call media must be delivered **unreliably and unordered**,
because a frame past its playout deadline is worthless and a reliable ordered channel
turns one lost packet into a multi-frame gap. The reference implementation ships
request/response over a reliable stream — which §1.5 permits only as a fallback — and
the README records the reason: `libp2p-quic` disables QUIC datagrams at construction and
exposes no datagram API, so closing it needs an upstream change.

**Decision (D14): the client defines its own media transport interface and implements it
twice.**

```
trait MediaTransport {
    fn send(&mut self, envelope: MediaEnvelope) -> Result<(), MediaError>;
    fn poll_recv(&mut self) -> Option<MediaEnvelope>;
    fn characteristics(&self) -> Delivery;    // Unreliable | ReliableFallback
}
```

- `ReliableFallback` wraps the existing `/intranet/call-media/1.0.0` path. Ships first.
- `Datagram` is the conformant path, and is what the jitter buffer, pacing and loss
  concealment are all written against.

Because frames are already AEAD-sealed by the call key before they reach the transport,
the two implementations are genuinely interchangeable — the transport carries opaque
bytes plus routing metadata and has no other job.

**The client must know which one it is on and say so.** `characteristics()` exists so the
UI can show a degraded-quality indicator on the fallback path rather than leaving users to
infer it from bad audio, and so bug reports arrive with the transport identified. Real-Time
§1.5 asks implementations to state which fallback situation they are in; this is that,
made visible instead of buried in a comment.

Two routes to the real path, both in `06` §6: a forked or patched `libp2p-quic` that
enables datagrams, or a side UDP socket negotiated over signalling and secured by the
existing call key. The first is cleaner and depends on upstream; the second is entirely
in our hands and duplicates NAT traversal we would rather not duplicate. **Recommendation:
pursue the fork, keep the fallback shipping, and do not build the side socket unless the
fork is refused upstream and voice quality is blocking users.**

---

## 6. Codecs, Framing and Concealment

Implementation-level per Real-Time §6, but these are the defaults to build against:

| Parameter | Default | Why |
|---|---|---|
| Audio codec | Opus, 20 ms frames | Universal, royalty-free, excellent at 16–32 kbps, built-in PLC |
| Audio bitrate | 24 kbps mono voice, adaptive 16–48 | Fits residential upload even in mesh |
| Jitter buffer | Adaptive, 40–200 ms target | The mechanism that turns unreliable delivery into intelligible audio |
| Late-frame policy | **Drop**, never wait | Real-Time §1.5 — a late frame is worthless and delays every frame behind it |
| Loss concealment | Opus PLC, then comfort noise | Codec concern per §1.5 |
| Video codec | VP8 first, AV1 later | VP8 for encoder ubiquity; AV1 when hardware encode is reliably available |
| Video framing | Keyframe every 2 s, temporal layers | Bounds join latency and gives the relay something to drop under pressure |
| Screen share | Same path, higher resolution, lower frame rate | Text legibility beats motion smoothness |

**Track multiplexing goes inside the sealed payload, not the envelope.** A participant
may send microphone, camera and screen simultaneously; each needs identifying. Putting
`track_id` and `kind` inside the AEAD-sealed plaintext keeps the relay's visible routing
metadata to exactly what Real-Time §2.2 allows it — call, sender, recipient — and means
adding a track type later is not a wire change. The relay cannot selectively drop one
track without decrypting, which is the tradeoff; that only matters for congestion
management, and dropping whole participants is an acceptable first answer.

**Sequence numbers are per (call, track)**, and the nonce binds call, track and sequence,
so a misrouted or replayed frame fails to open rather than being rendered somewhere
unexpected — extending the property Real-Time §2.2 already relies on.

---

## 7. Failure Behaviour

| Failure | Response |
|---|---|
| Relay drops | Renegotiate per Real-Time §1.4/§2.4 — propose, tie-break, make-before-break onto another willing relay. Audio gap should be sub-second |
| No relay available | Fall back to mesh with a participant cap and tell the user why, rather than failing the call |
| A participant's media stalls | Their tracks are dropped from the mix; the call continues. Never let one bad link stall a room |
| Datagram path unavailable mid-call | Downgrade to fallback transport in place, flag degraded quality |
| Epoch or channel rotation mid-call | Frames carry `rotation_ref`; receivers hold both keys across the transition — the same tentative-retention discipline Core §3.3 requires |

---

## 8. Open Questions

1. **Bandwidth estimation and congestion control.** QUIC's congestion control governs the
   connection; it does not tell an encoder to reduce its bitrate. Some receiver-side
   feedback signal is needed, and there is no protocol home for it — likely a signalling
   message. Deferred to P3, flagged now because retrofitting rate control into a shipped
   media path is unpleasant.
2. **Server-side mixing versus client-side.** Stage mode currently assumes speakers mix
   locally before publishing. A relay cannot mix without decrypting, so it never will
   here — which means the speaker count is bounded by what one speaker's machine can mix.
   Acceptable at the stage sizes anticipated; revisit if not.
3. **Echo cancellation and noise suppression.** Platform APIs vary widely, and a poor
   choice here damages perceived quality more than any codec parameter. Evaluate during
   P3 against real hardware rather than picking now.
