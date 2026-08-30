# Membership and Permissions

**Document status:** v1.2 — §6.5 settles the shape: `forget` stays all-or-nothing by decision rather than by default, what it owes is the announcement, and the executor guard that would defeat `06` §16 on its own is named. Previously v1.1 — §6.5 records that leaving a network is a gap rather than a design, and what the client owes once `06` §16 lands. Previously v1.0 — permission resolution implemented and reached through `kols-api`'s gate; E11 landed, so §2.2's per-scope registration problem is gone
**Depends on:** Core Protocol Spec §1 (identity), §2 (governance), §5.6 (invites)
**Consumed by:** `01-messaging-model`, `03-confidentiality`, `04-realtime`

---

## 1. Roles Are Groups

Discord's role model and the protocol's group model line up almost exactly, which is
fortunate and slightly suspicious, so it is worth naming where they differ.

| Discord | Protocol | Difference |
|---|---|---|
| Role | Group (Core §2.1) | None material |
| Role holds permissions | Group holds capabilities | Capabilities can **only** be held by groups, never by an individual — there is no per-user permission grant anywhere, by design |
| Roles are flat | Groups are flat, no nesting (Core §2.1) | Same, and the protocol enforces it |
| `@everyone` | `everyone` implicit group (Core §2.4) | Same idea; the protocol additionally **forbids** `everyone` from ever holding a governance-tier capability, under any configuration |
| Owner | `Founders` group (Core §2.3) | An ordinary group with an unrestricted capability set, not a special-cased person |
| Role hierarchy / position | **No equivalent** | See §5 |

Two consequences to design around rather than fight:

- **There is no per-user permission override.** Discord lets you grant one person access
  to one channel. Here that means creating a group containing that person. The client
  should make single-member groups a first-class, cheap UI action ("give access to…"
  creates and manages the group behind the scenes) rather than exposing the user to the
  fact that the protocol has no other way to express it.
- **Defining what a role can do is a higher-bar action than deciding who holds it.**
  `define-group` is governance-tier; `manage-membership:<group>` is ordinary *unless the
  target group holds governance power*, checked dynamically (Core §2.4). That asymmetry
  is the protocol's guard against permission sprawl, and the UI should reflect it: adding
  someone to Moderators is a normal admin action, changing what Moderators *can do* is a
  deliberate, rarer one.

---

## 2. Capability Vocabulary

The protocol's built-in capabilities cover the network-level questions; chat-specific
questions need extension capabilities. `Capability::Extension(String)` exists for exactly
this, and its tier is looked up from the network's policy registry rather than carried
inline — deliberately, so nobody can declare a governance-tier capability as ordinary and
slip it onto `everyone`.

### 2.1 Protocol capabilities this app relies on

| Capability | Use here |
|---|---|
| `read-content` | The actual gate on being served any content bytes (Storage §5.4). A waiting-room member holds nothing and is correctly refused |
| `publish:chat-log` | Network-level right to write *any* author log. Channel-level rights are separate (§3) |
| `publish:chat-moderation` | Right to write a moderation log (`01` §6) |
| `publish:image` / `video` / `audio` / `file` | Attachment types the network allows |
| `moderate-content` | Delisting a whole pointer — the heavy instrument (`01` §6) |
| `approve-node`, `revoke-node` | Invites and bans |
| `define-group` | Creating and re-scoping roles |
| `manage-membership:<group>` | Assigning roles |
| `define-policy` | Network-wide abuse limits, admission mode, history access (`01` §10, §8) |
| `define-content-policy` | Which content types — and therefore which attachment kinds — may exist here |

### 2.2 Extension capabilities this app defines

Each is registered with its tier at genesis, per Core §2.2's requirement that consuming
specs tier-tag anything they define. **Registration is mandatory**: an unregistered
extension name is refused outright rather than assumed ordinary, which is the fail-closed
behaviour that keeps a governance-tier capability from being slipped onto `everyone`.

**One entry per verb covers every scope of it**, since E11 landed (Core §2.2.1, `06` §11): a
registration ending in `:` covers the namespace beneath it, longest match winning. Before
that the registry matched whole names, so every scope needed its own entry added by a policy
change — which meant creating a channel with a permission override required amending network
policy, and the registry grew with the channel count forever. `kols-core::capabilities`
builds the entries, and its `VERBS` table is where the tiers below actually live — `kols-api`
resolves each command's class against it rather than against a second copy.

| Capability | Effect | Tier |
|---|---|---|
| `chat:create-channel:<scope>` | Create a new channel, in a category or network-wide. **Authorizes a `channel-definition` entry and nothing else** (spec 07 §3.8) — a definition grants nobody access to anything, since a new private channel's roster is empty until a governance-tier membership entry fills it | Ordinary |
| `chat:manage-channel:<scope>` | Rename, re-categorise, archive, delete; bind permissions; set slowmode (`01` §10.3); **change a private channel's roster**. Authorizes `channel-update`, `channel-membership` and `channel-rotation` entries | Governance-tier |
| `chat:post:<scope>` | Write messages in scope | Ordinary |
| `chat:read:<scope>` | Read scope. For private channels this is the record of intent; the *enforcement* is key possession (`03`) | Ordinary |
| `chat:moderate:<scope>` | Redact others' messages, pin, manage threads | Governance-tier |
| `chat:connect-voice:<scope>` | Join a voice channel and hear it | Ordinary |
| `chat:speak-voice:<scope>` | Transmit in a voice channel | Ordinary |
| `chat:manage-voice:<scope>` | Mute, move and disconnect others | Governance-tier |

**Why `chat:manage-channel` is governance-tier.** It can add an identity to a private
channel's roster, which is the ability to grant read access to content the rest of the
network cannot see. Anything that can widen access to confidential material is governance
power regardless of how routine it feels, and tiering it correctly is what keeps
`everyone` from ever holding it (Core §2.4's structural invariant).

---

## 3. Permission Resolution

A permission question is always of the form *"does identity I hold capability C for
channel K?"*, answered by replaying the governance log — never by consulting a cached
role list, and never by asking a peer.

Resolution order, checked in sequence, first match wins:

1. **Channel override:** any group containing `I` holds `C:<channel_id>`.
2. **Category default:** any group containing `I` holds `C:<category_id>`.
3. **Network default:** any group containing `I` holds `C:*`.
4. **Otherwise: denied.** Fail-closed, no ambient grant.

There is exactly one level of inheritance and no recursion. This is app-layer *name*
resolution over flat capabilities — it does not nest groups and does not reintroduce the
graph traversal Core §2.1 rejects.

**One question resolves at scope only, and it is not an exception.** Creating a channel
cannot be authorized by a channel-scoped grant, because the channel's id is minted by the
entry that creates it — nobody could hold, or have registered, a grant naming it beforehand.
So a definition resolves against the category and the network and stops. `kols-core` exposes
that as its own function rather than as the ordinary path called with a placeholder id: a
caller that could invent an id to ask about would get an answer that quietly depended on it.

**Denials are absent grants, not negative grants.** There is no "deny" capability, because
a negative grant would need precedence rules over the union-of-groups model the protocol
defines, and that is exactly the sprawl Core §2.1 is built to prevent. To exclude someone
from a channel that a category grants broadly, the channel carries an override binding
the narrower group — not a deny entry against the person.

---

## 4. Category Scope Exists for Scale

Binding permissions per-channel is the obvious design and it does not scale: a group with
posting rights on 300 channels holds 300 capability entries, every one of which is
replayed by every node and re-evaluated on every check.

So **the default scope for a permission binding is the category**, and per-channel
overrides are supported but presented in the UI as the exception. A 300-channel server
with 12 categories carries roughly a dozen bindings per role instead of 300. This is the
mitigation `00` §4 names for capability-set growth, and it matches how large Discord
servers are actually administered — permissions live on categories and channels inherit.

---

## 5. There Is No Role Hierarchy, and That Changes Moderation UX

Discord uses role *position* to decide who can act on whom: you cannot ban someone whose
top role sits above yours. The protocol has no such concept — groups are an unordered
flat set, and a capability either is or is not held.

Consequences to design for deliberately:

- **Any `revoke-node` holder can remove any member, including a Founder.** There is no
  structural protection for the network's creator beyond who holds the capability.
- **Mutual removal is possible.** Two moderators can each remove the other; the fork rule
  (Core §2.7.1) decides which action survives if they act during a partition, by
  capability-gated branch length then lower entry hash.

Rather than inventing a position system, the design leans on the protocol's own answers:
keep governance-tier capabilities in small groups, use member-vote policy (Core §2.6.1)
for networks that want removals deliberated rather than unilateral, and surface the
**voided-actions report** (Core §2.7.1 point 5) in the UI so a revocation lost to
reconciliation is re-proposed rather than silently undone. That report is mandatory in
the protocol and is one of the few places where a client is expected to act on its own —
`05` §4 makes it a first-class notification, not a log line.

*Flagged: if role hierarchy turns out to be genuinely needed, it belongs in the protocol's
policy module (Core §2.6) as a governance rule, not bolted into the client where two
clients could disagree about who outranks whom.*

---

## 6. Onboarding

### 6.1 Invites

Invites are the protocol's (Core §5.6): signed, time-bounded, use-count-limited, carrying
bootstrap addresses, network id and issuer. The client's job is to make one shareable —
a single URI (`intranet-chat://join/<encoded-invite>`) that opens the app and starts the
join handshake.

**Built, and two things about it were not obvious until it was.** The protocol had no way to
serialize an invite on its own — its bytes existed only inside the join request that presents
it, which is the far end of the journey — so issuing one and having no way to hand it to
anybody was possible right up until something tried. That is now Core §5.6's obligation and
`encode_invite`/`decode_invite` upstream.

And **an invite needs an address, which only a running node knows.** A one-shot command
cannot invent one, so the daemon writes down what it is reachable on and `kols invite` reads
that — refusing to mint rather than producing a credential that connects to nothing. The
same applies in reverse: the addresses an invite carried are kept by whoever redeems it, so
a joiner never has to be told an address by hand after being handed one inside the invite.

**Which addresses, though, turned out to be most of the work.** The daemon first wrote down
everything it listened on minus loopback and link-local, and one ordinary Windows machine
produced twenty-five: three global IPv6 addresses and one real LAN address, and beside them a
Tailscale pair, three virtual-adapter subnets left by VirtualBox and two hypervisors, and
circuits through a relay that announced its private container addresses beside its public
name — each direct address doubled again by TCP and QUIC. That is a 4,400-character invite,
which is not something anybody sends in a chat message, and the length was not the worst of
it: a joiner dials the list in order, and every circuit through one relay shares a connection,
so a dead relay hop dialled first can cancel the live one queued behind it (Core §5.2). The
invite was carrying addresses that actively broke the join it existed to perform.

So the daemon selects rather than filters, on three rules that between them cut that machine
to nine addresses, and a change to the encoding took it the rest of the way. **An overlay address is dropped** — somebody on
your tailnet does not need an invite to find you, and somebody who is not on it can never use
the address. **A LAN address survives only if it is the one this machine actually uses**;
nothing in the text distinguishes `192.168.56.1` from `192.168.1.200`, so the routing table is
asked instead, and if it answers nothing useful every LAN address is kept — a long invite is a
better outcome than two machines in one house unable to find each other. **A relay already
offered at an address a stranger can reach is not also offered at a private one**, which
removes the leaked container addresses while keeping a member relaying on the LAN, for whom
the private hop is the only way in. What survives is capped, so no interface list can produce
an invite nobody can paste.

More than half of what was left after that was the node's peer id, repeated once per address,
and **the second half of the fix is upstream in how an invite is encoded.** The addresses in
one invite all name the same node, so they all end in the same `/p2p/<peer id>`. The invite's
wire encoding now writes the longest ending they share exactly once — a fact about those
strings rather than about multiaddrs, which is what keeps `intranet-invite` free of any notion
of what an address means. It is not what is signed: the signature covers a payload in which
the addresses appear whole, so this changed the URI and nothing a receiving node verifies.

Two richer schemes were measured and rejected, both for the same reason. A shared table of
`/`-separated components dedupes more in principle and comes out **larger**, because the
protocol's encoder frames every length with a fixed eight-byte `u64` and the table's prefixes
and index lists cost more than the repetition they remove. Compressing the whole encoding
would need a dependency, a bound against a decompression bomb, and a size that varies with the
data. Nine addresses went from 1,082 bytes to 634 for one string and one subtraction.

Together: **about 4,750 characters to 1,324** for that machine, which `invite_length.rs`
asserts and Core §5.6 now states as an obligation — an invite is carried out of band by a
human, so one too long to paste has failed the job the specification gives it.

The URI is a container, not a format: the bytes are the protocol's and normative, the scheme
and its base32 are the client's, chosen so an invite survives being pasted into a chat
message and copied back out.

The invite carries **nothing** beyond what the first authenticated connection needs
(Core §5.7). Channel lists, roles, history and keys all arrive afterward as ordinary
post-connection sync. A client that tried to stuff a server preview into an invite would
be leaking network state to anyone holding the link.

### 6.2 Admission modes

| Mode | Behaviour | Suits |
|---|---|---|
| **Auto-admit** | Valid invite ⇒ immediate `everyone` membership and epoch key delivery | Open communities; the low-friction default |
| **Explicit intake** | Valid invite ⇒ connectivity and identity only; **no groups, no keys**; the joiner sits in the waiting room, visible to `manage-membership:everyone` holders with issuer context | Small high-trust networks; membership screening |

Two protocol constraints the UI must respect rather than route around:

- **Member-vote policy cannot be combined with auto-admit**, and cannot be chosen at
  genesis (Core §2.6) — the first vote would need a quorum of an electorate that does not
  exist. The network-creation flow must not offer that combination, and the
  "switch to member-vote" action belongs in settings, after a founding electorate exists.
- **A waiting-room member is served nothing** — not content, not metadata, not bandwidth
  (Storage §5.4). The lobby UI must be built from what the joiner already has (the
  invite) plus whatever an admin chooses to send them, not from a channel list.

### 6.3 First-run

1. Generate a seed **for this network**, not one master seed for all of them. Each network
   a member joins gets its own, so no single secret is the thing that links every identity
   they hold — the same reasoning per-network derivation exists for (`00` §1), applied one
   level up. One phrase leaked exposes one network rather than all of them and every
   conversation.

   **What a backup is, stated precisely, because "restore" promises more than it delivers.**
   A seed derives an identity; it is not data and it is not a network. A phrase alone
   restores nothing, because a network's id cannot be derived from it. Coming back needs
   three things — the phrase, the network id, and an address to reach the network at, which
   is to say a relay. Given those, a member returns as *the same member*: the governance log
   already names them, their capabilities resolve, their display name is still bound to
   them, and their own messages come back like any other history, because their author log
   was published as content other members hold.

   What does not come back: anything written and never published, local-only state such as
   read watermarks, and — the one that actually bites — **the list of which networks they
   belonged to**, which lives in the client's workspace and in no seed.

   So a backup is a *bundle the client exports* rather than something a person writes down.
   A phrase per network is unmanageable as a literal instruction once direct messages are
   networks too; what a member keeps is the set.

   **A password must wrap these seeds and must never derive them.** The intended shape is a
   local account — a username and password this installation knows and no network ever sees
   — that unlocks the keyring the seeds live in. The tempting alternative is to *derive*
   each seed from the password and the network id, which needs no storage at all and is
   badly wrong: the network id is public, travelling in every invite and every address, and
   member ids are in the governance log, so an attacker can derive a candidate identity from
   a guessed password and **check it offline against a value the network publishes**. That
   is a brainwallet with a verification oracle. A random seed wrapped under a
   password-derived key has no such oracle — there is nothing public to check a guess
   against — and it keeps the seed's full entropy regardless of how weak the password is.
   The codebase already draws this line one level down, where epoch keys and MLS state are
   sealed at rest under a seed-derived key rather than derived from anything public.

   Losing the password then loses the seeds, which is precisely what the export bundle is
   for, and why the two features are one piece of work rather than two.
2. Redeem an invite, or create a network (which makes you the sole Founder).
3. Post-connection sync: peers, governance log replay, capability ledger, epoch key.
4. Resolve the channel list and render.

### 6.4 Resource contribution is a per-network choice

Contribution is opt-in and revocable per network (Core §4.3), and the client must expose
it as such rather than deciding on the user's behalf: how much disk this network may use
for replicas (`storage_offered`), bandwidth caps, willingness to act as a bootstrap relay
(`relay_bootstrap_willing`) and as a blind media relay (`relay_media_willing`).

Defaults should be modest and honest — a chat client that quietly volunteers a laptop as
a media relay for a 3,000-member server has misrepresented what the user agreed to. The
settings UI should show what the contribution is currently costing, which the capability
ledger and local counters already make available.

**The declared cap is now binding on the node that declared it** (Real-Time §2.2.2). It
used to be read by every node *except* the one that made it — it steered other members'
relay and source selection while the volunteer enforced nothing — which meant a user could
set a limit, watch the client ignore it, and have no way to tell. A media relay now bounds
concurrent calls, participants per call, and sustained bytes forwarded, and refuses when it
would exceed them. Two client consequences:

- **A refusal is not an error to surface.** A call whose relay declines renegotiates onto
  another, the same as if that relay had gone offline (`04` §7). The UI should say nothing.
- **The node's own refusals are worth showing in settings**, in the one place a user is
  already looking at what they volunteered. A steady stream of them is what "I offered more
  upload than I have" looks like from the inside, and there is no other signal for it.

### 6.5 Leaving, which the client currently cannot do

The client offers **forget**, and the word is honest about what it does: it deletes this
installation's store and the seed with it. It is deliberately not called *leave*, because
membership is governance state and nothing in the log expresses resignation — so to every
other member, nothing has happened.

**That is a gap rather than a design.** `06` §16 (E16) has the protocol half: a membership
removal naming its own author needs no capability, because it grants nothing, names nobody
but its signer, and is proved by the same signature that proves the identity it removes.
Until that lands there is no entry a departing member is allowed to write, and this section
records what the client should do once there is.

**One command, not two, and that is a decision rather than a default.** An earlier draft of
this section asked for a *leave* that keeps the seed beside the *forget* that destroys it, on
the grounds that they answer different questions. They do, and the second question turned out
not to be one anybody is asking: leaving a network is something a person does when they are
done with it, and being done with it means wanting it off the machine. A seed-preserving leave
buys the ability to be re-admitted as the same member later, which is a real property and one
nobody has wanted. Recorded here rather than deleted, because a section that quietly grew a
second command would be a design nobody chose.

So `forget` stays all-or-nothing, behind the native confirmation it already has, and what it
gains is the announcement.

**The order is load-bearing and easy to get backwards.** The departure entry is signed by
the key `forget` is about to destroy, so it must be written **and published** first. A node
that is offline, unkeyed, or without a route cannot announce anything — so the announcement
is best-effort, and the interface must distinguish *told the network* from *could not tell the
network* rather than reporting success either way. A forget that cannot announce is still a
forget; it is just one the network will not learn about, and saying so is the difference
between an honest limit and a lie.

**Announcing still matters even though nobody can come back.** It is tempting to read
all-or-nothing as making the departure entry pointless — the identity is gone either way. It
is not: the entry is the only thing that takes the leaf out of the MLS group, and without it
the network rotates key material for a member who deliberately destroyed their half of it,
forever. What the departure buys is the *network's* hygiene, not the leaver's.

**One guard has to move with it, and it is not in the protocol.** `kols-node`'s executor
refuses any self-removal from `everyone` outright, on the grounds that it would leave the
network unmanaged by the only node that can rotate its key. That is true of a *last* holder of
`revoke-node` and false of everybody else, and it would silently defeat `06` §16 on its own.
It becomes a check for the condition it actually means.

**What the network does with a departure is the network's business, not the leaver's.**
The removal takes effect on replay; the epoch does not rotate on the departure itself, for
the grinding reason `06` §16 gives. A member who left and kept their seed is precisely a
member who has not been revoked yet, which is the state the guarantee already covers.

---

## 7. Profiles

A profile is per-network by necessity — there is no cross-network identity to hang a
global one on (`00` §1).

A member's avatar and status live in a small mutable pointer they own
(`content_type: chat-profile`, `pointer_id = H("chat-profile" ‖ identity_id)`, derived on
the same principle as author logs). Avatars are ordinary content objects. Nickname
overrides imposed by moderators, if wanted, are a channel-scoped record in the
moderator's log rather than an edit to somebody else's profile — nobody writes another
member's objects, anywhere in this design.

**The display name is not in that pointer, and an earlier version of this section had it
wrong.** A name is expected to be unique within a network, and uniqueness is a question about
the whole network at a moment in time. A mutable pointer is single-writer by construction: it
can say what I call myself and has no ordering relative to what anybody else published, so two
members claiming one name produces two nodes that disagree about who is who with nothing to
settle it. Ordering is what makes uniqueness decidable and the log is what has ordering.

So **a name claim is a `chat` application entry** (spec 07 §3.9, normative), and the rest of
the profile stays in the pointer because none of it needs ordering. The split is the same one
`01` §2 draws for channels: what needs a total order goes in the log, and nothing else does.
This is the third time this project has reached that conclusion — App Hosting §4.3 for app
names, D4 for channels, and now this.

Four properties worth carrying here rather than only in the spec:

- **The claim carries no identity.** It binds whoever authored the entry, so claiming a name
  for somebody else is unsayable rather than refused. `chat:set-name` is therefore ordinary and
  granted to `everyone` at genesis without widening anything.
- **Uniqueness compares a normalized key**, not the typed form: NFKC, whitespace collapsed,
  lower-cased, with invisible code points refused outright. Display uses what was typed.
- **A key binds permanently, including after its holder leaves.** History renders by author id
  with names resolved at display time, so an inherited name would silently relabel somebody
  else's past messages — every line honestly attributed while the conversation as a whole
  becomes a lie.
- **A name is never sufficient on its own.** The key deliberately does not fold confusables, so
  an interface must render a name alongside enough of its holder's identity to tell two
  lookalikes apart (spec 07 §8). `09` §4 owes this the same honesty it owes presence.
