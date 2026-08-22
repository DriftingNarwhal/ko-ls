# Confidentiality: Channel Keying, Private Channels and Direct Messages

**Document status:** v1.0 — design reviewed. Only the network tier is implemented; channel and session tiers are P2/P3
**Depends on:** Core Protocol Spec §3 (epoch keying), Storage Spec §5 (envelope encryption), Search Spec §3, Real-Time Spec §3.5
**Consumed by:** `01-messaging-model`, `04-realtime`, `05-client-architecture`

---

## 1. The Gap This Document Closes

The protocol gives a network one epoch key, shared by every member and rotated on
membership change. Everything encrypted under it is readable by every current member.
That is the correct default for a network-wide chat channel and it is exactly wrong for a
private channel, whose whole point is that other members cannot read it. (Direct messages
solve this differently again — they are their own networks, §4.)

The protocol names this gap rather than filling it. Real-Time §3.5 flags that a
restricted-audience broadcast "cannot reuse epoch-key encryption for confidentiality,
since the epoch key is shared network-wide by definition — such a feature would need its
own scoped key, closer in spirit to how call encryption already works". Private channels
are the same problem in the text domain, and this document answers it the way §3.5 says
to: with a scoped key.

---

## 2. Three Keying Tiers

| Tier | Used by | Key source | Who can read |
|---|---|---|---|
| **Network** | Public channels, attachments in them, public search postings | Per-object DEK wrapped under the network epoch key (Storage §5) | Every current member |
| **Channel** | Private channels, their attachments, their voice | Per-channel MLS group, §3 | Current channel roster |
| **Session** | Voice/video in public channels | `CallKey`, sealed per participant (`intranet-realtime`) | Current call participants |

### 2.1 Public channels: derive, don't reuse

Public channel content is stored exactly as the protocol specifies — a per-object DEK,
wrapped under the epoch key, re-wrappable by any current member on rotation (Storage
§5.3). Nothing new.

For the **live path** (`01` §7), which carries records before any object exists, records
are sealed under a derived key rather than the raw epoch key:

```
channel_content_key = KDF(epoch_key_for(rotation_ref), "chat:channel", channel_id)
```

The derivation is app-layer, adds no trust assumption (it is derived from a key the
member legitimately holds), and buys one real thing: the live and private paths become
one code path with two key sources, rather than two code paths. Every live payload
carries the `rotation_ref` it was sealed under, so a receiver mid-rotation picks the
right key instead of failing.

---

## 3. Private Channels: a Per-Channel MLS Subgroup

**Decision (D9): each private channel gets its own MLS group**, whose membership is the
channel roster, created and rotated with the same machinery `intranet-epoch` already
provides for the network group (`GroupSession::create / add_member / remove_member /
rotate / apply_commit`).

### 3.1 Why not the simpler thing

The obvious alternative is a per-channel symmetric key, sealed to each member's identity
key — precisely what `CallKeyEnvelope` does for calls. It is genuinely simpler, and it is
rejected for three reasons:

1. **Rekey cost is O(n) per roster change**, against MLS's O(log n). At 50 members that
   is nothing; at 500 it is a re-seal storm on every join and leave.
2. **No forward secrecy.** A member removed today keeps every key they were ever sealed,
   so re-keying means minting an unrelated key and re-sealing to everyone — which is the
   naive scheme Core §3.2 explicitly rejects for the network, for the same reasons.
3. **It would be a second key-management mechanism** to write, test and get wrong, when
   the codebase already contains a working, spec-conformant one.

The protocol solved group keying once. A private channel is that problem at a smaller
scope, and the solution transfers whole.

### 3.2 Anchoring: rotations must be ordered and durable

MLS needs commits to be ordered, and standard deployments use a central Delivery Service
for that — which this project rejects. Core §3.3 resolves it for the network by making
each rotation a governance log entry. **Channel rotations resolve the same way**, via new
entry variants (`06` §2):

```
EntryBody::ChannelMembership { channel_id, action: Add | Remove, identity }
EntryBody::ChannelRotation   { channel_id, commit_ref, reason }
```

This inherits, without new machinery: a total order on commits; the fork-choice and
bounded-finality rules (Core §2.7.1); the tentative-retention discipline (Core §3.3 — a
channel rotation is tentative until buried under k=10 capability-gated actions *and* 30
minutes old, and prior channel-epoch secrets are retained until then, exactly as for the
network); and a `rotation_ref` that `DekWrapping` can reference for channel-keyed objects
the same way it references network rotations.

**Cost, stated plainly:** private-channel roster churn writes to the governance log that
every node replays. Roster changes are rare compared to messages — no message ever enters
the log — but a network with hundreds of private channels and heavy churn will grow it.
`00` §6 item 3 carries this as an open question with checkpointing as the mitigation.

### 3.3 Authorization and keys are separate, and both are required

`chat:read:<channel>` (`02` §2.2) records *who is entitled* to read a private channel.
The channel MLS group decides *who can actually decrypt* it. They must be kept in step,
and where they disagree the design fails closed:

- A member in the roster but without the capability is refused service by honest nodes
  anyway (Storage §5.4 gates on `read-content`, and the client additionally checks the
  channel capability before rendering).
- A member with the capability but not in the MLS group simply cannot decrypt. The client
  surfaces this as "you have access but no key yet — ask a channel manager", not as an
  error, because it is the normal state between a grant and the next commit.

`chat:manage-channel` is the capability that changes both, in one action, which is why
`02` §2.2 tiers it as governance-tier.

### 3.4 What a private channel does not hide

- **Its existence and name**, since `ChannelDefinition` is in the governance log. Hiding
  the definition would mean giving up ordered, durable channel state. A network wanting
  a genuinely invisible space should use a separate network, which is free.
- **Its roster**, for the same reason.
- **Traffic patterns.** Members can see that objects are being published under a
  channel-scoped pointer, and roughly how much. Core §1.2 already declines to defend
  against timing correlation.
- **Anything from its own members.** Membership means readership; that is what it means
  everywhere in this design.

---

## 4. A DM Is Its Own Network

**Decision (D10), revised.** An earlier draft made a DM a private channel with a
two-member roster. That is rejected. **A direct message conversation is a separate
two-person network**, created for the purpose, and the chat application presents a list
of them as a DM inbox.

### 4.1 A Conversation Is a Different Kind of Network From a Server

**Decision (D20).** Two network *profiles* exist, declared at genesis in the app policy
map (`06` §9) as `chat:network-profile`:

| | `server` | `conversation` |
|---|---|---|
| Channels | Many, created and organised into categories | **Exactly one, implied** — never declared, never created |
| Channel id | Per `ChannelDefinition` entry (`01` §2.1) | Derived: `H(network_id ‖ "chat:conversation")` |
| Roles | Groups with capability sets, category-scoped permissions | None beyond the implicit two |
| Membership | Founders, moderators, `everyone` | **Every participant is a Founder** (§4.4) |
| `app-bundle` | Allowlist choice | Never on the allowlist |
| Voice | Voice and stage channels | Calls in the one channel, no stage |
| Search | Network postings for public channels | Local index only — a conversation is too small to be worth DHT postings |
| Presented as | A server in the server list | A contact in the friends list |

**This is enforced, not conventional — by readers rather than by the protocol.** In a
`conversation`-profile network, a `ChannelDefinition` entry is **invalid and refused by
every reader that understands the `chat` namespace** (spec 07 §1.2). The profile lives in
replayed policy state, so they all reach the same verdict, and a client that tried to
create a channel in a conversation would find nobody accepting the entry. Categories,
stage channels and app-bundle publishing are refused on the same basis.

**What that is worth, stated precisely**, since E2 landed generically and the protocol
carries `chat` payloads without decoding them: minting the entry still requires the
declared capability, and every conformant client refuses it — but the platform cannot
enforce a rule it cannot read, so a modified client would see a channel where others see
none. Bounded, and weaker than "rejected on replay" implies.

The one thing that is *not* claimed: permanence. A network's founders can change policy,
and a conversation whose founders deliberately reconfigure it into a server becomes one —
at which point clients correctly present it as a server. The profile is a declaration of
what a network is for, enforced while it holds, not a cage.

**What a conversation deliberately keeps** is the machinery that makes it a real network
rather than a chat log: identity, an epoch key, and membership. That last one matters —
removing somebody from a group conversation rotates the epoch key, so they cannot read
what is said afterward. A conversation is minimal in *surface*, not in guarantees.

### 4.2 Why this is better than a private channel

- **The data never lands on an uninvolved node.** A private channel's messages are
  ordinary network content: replicated by HRW placement across whichever members offered
  storage, encrypted so those holders cannot read a byte of it. It works, but it spends
  other people's disks storing ciphertext that is useless to them. In a two-person
  network, the only eligible replica holders *are* the two participants — Storage §3.2
  degrades gracefully to as many nodes as exist rather than refusing to operate — so the
  conversation is stored by exactly the people having it.
- **Nothing about it burdens anyone else.** No channel definitions, no roster entries and
  no rotations in a shared governance log that every member of the server replays forever.
  This was the design's largest hidden growth driver: a 50-member network has up to 1,225
  possible pairs, a 500-member one over 120,000, and every one of them would have been
  writing structural entries into a log nobody else benefits from.
- **It is what the protocol already expects.** Core §2.8 explicitly anticipates one group
  of people holding several independent networks rather than one network doing several
  jobs, and networks are deliberately cheap to create. A DM is simply the smallest case.
- **Metadata isolation comes free.** In a private channel, every member of the server can
  see that a private channel exists between two identities and roughly how active it is
  (§3.4). A separate network reveals none of that to the server *in its own content or its
  own log*. One qualification, because the earlier flat statement was too strong: where a
  conversation cannot hole-punch and falls back to the shared network's bootstrap relay
  (`09` §3), that relay's operator sees one address acting both as a member and as a party
  to a conversation, and can infer that two of its members are talking. Nothing about *what*
  they say is exposed, and no other member learns anything — but the isolation is from the
  server's members and its log, not from an operator watching addresses.

**What it costs:** each conversation carries its own genesis, governance log, MLS group
and DHT namespace. That is more fixed state per conversation than a channel would be —
but it is paid by the two participants alone, rather than by everyone. The trade is
correct in both directions.

### 4.3 Starting one: the invite has to travel somehow

The friction this creates is real and is the only part needing design. You can only start
a DM with someone you can already reach, so **the shared network is the rendezvous**, and
"message Alice" is one click over the following flow:

1. The sender's client creates a new network with itself as sole Founder, configured for
   two people: auto-admit off, replication naturally degraded, no `app-bundle`.
2. It mints an invite (Core §5.6) for that network.
3. It delivers the invite **over a direct peer-to-peer stream** to the recipient, inside
   the shared network — never as published content, never in a channel, never in a log.
   Nothing about the request is stored anywhere, by anyone.
4. The invite is accompanied by a **voluntary identity link**: a signed statement that the
   identity issuing this invite is the same person as this identity in the shared network.
   Core §1.2 defines exactly this mechanism — provable common ownership at the user's
   discretion, derivable by nobody otherwise — and this is its first real use.
5. The recipient's client verifies the link, shows "Alice wants to message you", and on
   acceptance joins the new network. From then on it is an ordinary network that the UI
   happens to render as a conversation.

**Honest limitation:** direct delivery requires both parties to be reachable. An
undelivered DM request stays queued in the sender's client and retries; it does not sit
in an inbox somewhere, because there is no inbox and building one would put the request
back onto other people's nodes. A request to someone who never comes online simply never
arrives.

### 4.4 Group Conversations

**Decision (D21): a group conversation is its own `conversation`-profile network,
distinct from every pairwise conversation among the same people.** Alice–Bob, Alice–Carol
and Alice–Bob–Carol are three separate networks with three separate histories, and each
participant holds a different identity in each, uncorrelatable **by key** (the standing
qualification — `00` §1, `09` §1). That is not overhead to
apologise for — it is the reason a group chat cannot leak into a private one.

**Creating one must not feel like founding a server**, and does not: the client flow is
"select two or more contacts → start conversation". Behind it, a `conversation`-profile
network is created and E10 invites are delivered directly to each participant, exactly as
for a pairwise conversation (§4.3). Nobody names a server, picks a profile, configures
roles or creates a channel, because a conversation profile has none of those to configure.

**Every participant is a Founder.** A group conversation has no owner, and the protocol
gives no way to express "any member may add people" otherwise: `approve-node` is
governance-tier, and `everyone` may never hold a governance-tier capability under any
configuration (Core §2.4, a hardcoded invariant). Putting all participants in `Founders`
is the honest encoding of a conversation among peers.

*Consequence, stated because it is real: any participant can add or remove any other.*
There is no hierarchy to appeal to (`02` §5) — which is the correct model for a group
chat among friends and the wrong one for anything larger. If a conversation grows past
the point where that is comfortable, it has become a server, and the participants should
make one.

**Membership changes are ordinary admission and revocation**, with the epoch rotation
each implies. Two consequences worth naming:

- **Adding someone to a pairwise conversation is not possible** — the roster of a
  two-person conversation is the conversation. Adding a third person creates a new group
  conversation, with its own history starting empty. This matches what people already
  expect from messaging apps, and it falls out of the model rather than being imposed.
- **Removing someone rotates the key**, so they cannot read anything said afterward. What
  they already fetched, they keep — the honest floor of any symmetric-key system (§7),
  and no different here than anywhere else in this design.

**Identity between strangers.** Alice may start a conversation with Bob and Carol who do
not know each other. Each participant's display name comes from their profile in that
network (`02` §7); the identity link (§4.3) is a *voluntary* proof offered to whoever a
participant cares to prove themselves to. So Bob sees "Carol" because Carol says so, and
if Bob wants cryptographic assurance that this is the Carol he knows from a shared server,
Carol can send him a link. The protocol offers exactly this and nothing stronger, which
is the correct amount: identity linkage is the user's to give.

### 4.5 Delivery Is Asynchronous, With One Real Constraint

A two-person network has exactly two replica holders, which changes the delivery model
from what a server-backed messenger does — but not in the way it first appears.

**Sending does not require the recipient to be online.** The sender publishes their
segment to their own node, which is a complete, valid publish; there is simply nobody
else holding a copy yet. The recipient fetches it whenever they next sync.

**Delivery requires the two to overlap at some point.** The recipient can only fetch from
the one node that holds it, so the *sender* must be reachable when the recipient comes
back. Message someone at midnight, close your laptop, and they receive it the next time
you are both online — not the next time they open the app.

This gives three honest states the UI must distinguish, because collapsing them is the
kind of small lie that erodes trust in a messenger:

| State | Means |
|---|---|
| **Sent** | Written and published locally. Nobody else holds it yet |
| **Delivered** | The recipient's node has fetched it. Provable — they became a holder |
| **Read** | The recipient's client says so. Trust-based, and disable-able by them |

**Multi-device materially improves this**, which is worth knowing before P5 is scoped: a
person with a phone and a desktop both enrolled in a DM network has two nodes that can
serve, so the overlap window widens considerably. It is the natural answer to "my
messages take a day to arrive", and it is a better answer than putting a copy on somebody
else's machine.

*Flagged: a bootstrap relay does not help here, and should not be made to. Relays hold no
state and cap circuits at 60 seconds and 256 KB by design (Core §5.3) — a relay that
stored messages for later would be exactly the always-on infrastructure this project
refuses to depend on. The ceilings were 120 seconds and 8 MB until 2026-08-22 and the
conclusion only gets firmer: at 256 KB a circuit cannot carry a conversation, let alone
hold one for a recipient who is not there.*

### 4.6 What this means for the DM list

The client holds the workspace — the set of networks this installation belongs to — so it
knows locally that these are all yours and can present them as one inbox. Note that the
knowledge comes from the directory rather than from a shared secret: seeds are per network
(`02` §6.3), so there is no master key whose compromise would correlate them either. That is **local knowledge only** — no other party can
correlate your identity in a DM network with your identity in the server you met through
*from the keys alone*, unless you sent them the §4.3 link. Unlinkability is preserved exactly
as Core §1.2 intends, including its limit: an observer positioned to see both — a peer in both
networks, or a relay carrying both — can still correlate by address, which §1.2 places out of
scope rather than solving. The convenience is entirely client-side; so is the exposure.

This is why the DM surface is presented as a **friends list and an instant-messaging
pane**, not as a channel list: it behaves like an IM service, one conversation per
contact, with presence and delivery state per contact. A contact *is* a DM network you
both belong to, so "add a friend" and "start a conversation" are the same action (§4.3),
and removing a friend is leaving that network — which, for a two-person network, ends it.

Private channels (§3) remain, and are now clearly scoped to what they are actually for:
**a subset of a server's members sharing that server's roles, moderation and context** —
`#mods`, `#planning`. That is a genuinely different thing from a private conversation,
and it is the case where being inside the network is the point.

---

## 5. Rotation Behaviour Under Membership Change

| Event | Network epoch | Channel epochs |
|---|---|---|
| Member joins network (auto-admit) | Rotates (MLS add is a commit) | Untouched |
| Member removed from network | Rotates | Every private channel containing them must also rotate |
| Member added to private channel | Untouched | That channel rotates |
| Member removed from private channel | Untouched | That channel rotates |

The second row is the one that needs care: **removing someone from the network does not
automatically remove them from private channel groups**, and if it did not cascade, they
would keep decrypting new private-channel content while being unable to fetch it from
honest nodes — a belt-and-braces situation that is fine right up until one brace slips.
The client that performs the revocation is responsible for proposing channel removals for
every private channel it can see the target in; nodes holding channels the revoker cannot
see are responsible for reacting to the revocation on replay. This is a **convergent
cascade, not an atomic one**, and the honest statement is that a removed member loses
channel access as each channel's managers process the revocation, not instantly.

*Flagged: an alternative is to derive channel groups from network membership so a network
removal implies removal everywhere. It is cleaner in theory and considerably heavier in
practice, because it couples every channel's MLS state to the network's rotation cadence.
Revisit if convergent cascade proves unreliable in testing.*

---

## 6. Search Must Not Leak Private Channels

Search postings are announced to the DHT under `hash(network_id ‖ term)` and are
readable by any member (Search §3.1). Publishing private-channel content into that index
would hand every member a term-level view of a channel they cannot read — the payload is
encrypted, but the *association* between a term and a pointer is not, and pointer ids for
private channels are derivable (§4).

**Rule, fail-closed: content in a private channel is never announced to the network
search index.** Private channels are searchable only through the client's own local
index, over what that client holds. The limitation is real and should be stated in the
UI: private-channel search covers what you have, not what exists.

*Optional extension, not the default: channel-scoped postings at
`hash(network_id ‖ channel_id ‖ term)` with payloads sealed under the channel key. A
non-member could enumerate the collection and learn that a channel contains some content
matching a term, without reading it. That leak is small but real; it buys complete search
for large private channels where local-only indexing is genuinely inadequate. Offer it as
a per-channel setting, off by default, with the leak described in the setting itself.*

Public channels index normally: default metadata always (Search §2.1), plus an
`IndexDocument` per message batch carrying the searchable body (Search §2.2). Note that
indexing is explicitly opt-in at field level — the client extracts message text and
nothing else, so local-only fields never leak into the index by accident.

---

## 7. Honest Guarantee Summary

Restating what this design delivers, in the style Core §3.1 and Storage §5.5 use:

- **Non-members of a network** get ciphertext and nothing else, and honest nodes will not
  serve them at all once replay converges on their non-membership.
- **Network members outside a private channel** can see the channel exists, who is in it,
  and roughly that it is active. They cannot read its content, its attachments, or hear
  its voice — that is enforced by key possession, not by policy.
- **A member removed from a private channel** cannot read anything wrapped after the
  resulting rotation. They keep whatever they already fetched and decrypted, forever. No
  symmetric-key system can do otherwise, and this one does not claim to.
- **A member removed from the network** additionally stops being served new bytes once
  honest nodes converge (Storage §5.4), and loses private channels as each one's rotation
  processes (§5) — convergently, not instantly.
- **Retention (`01` §8) is not deletion.** Dropping old segments makes them unavailable to
  those who did not already hold them.
- **Unlinkability between your identities is a property of keys, not of traffic.** Nobody can
  tell your identity in one network from your identity in another by the keys, and anyone
  positioned to see both addresses — a peer in both networks, a relay carrying both — can
  correlate them anyway. Core §1.2 places that explicitly out of scope; `09` §1 carries the
  statement in full, and every unlinkability claim in this document is bounded by it.
