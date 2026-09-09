# Client Architecture

**Document status:** v1.23 — §5's `kols-store` is built and deliberately not switched on: the index, the verdict fold and the page read all exist and are proved against `kols_core::withheld` itself, and wiring them into `open_channel` waits on the interface gaining paging, since rendering a page into a window that cannot ask for the next one would hide history. Previously v1.22 — §5 carries what `kols-store` has to be, derived from the measurements rather than assumed: a page is three queries, the rate verdict must be *stored* because folding over a page would make a message appear and vanish depending on how much was loaded, held is deliberately not stored, and the record bytes stay in files. Previously v1.21 — §5 carries the projection, measured and half built: the replay half needed no SQLite and is done, taking one ordinary command from 152 ms to 1.5 ms against three hundred channels, while the read half — opening a channel reads every record in it, four seconds at a hundred thousand — is what actually wants an index and is what remains of O4. Previously v1.20 — §8's content-routing row is done, and its claim that this needed the Docker NAT matrix was over-specified: what it needed was topology control, which the transport's in-process tests have and the client's daemons cannot. Previously v1.19 — §5's second half is built too: an append re-chunks its tail rather than the whole segment, 6.5 ms to 0.12 ms at ten thousand records and flat against linear, with Storage §1.3 now requiring the property it rests on. The measurement caught a second bottleneck the first had been hiding — a publish that cloned the object it returned. Previously v1.18 — §5's plan is built: a send is flat in history, 298 ms to 1.8 ms at a thousand of an author's records and 2.1 ms at six thousand, and the background pass no longer rebuilds a whole chain per tick. Two things it turned up: the removed rebuild was carrying the fail-closed check that an unkeyed member cannot write, and *published* and *announced* are different kinds of fact — one survives a restart and one is about a swarm the restart replaced. Previously v1.17 — §5 carried the plan for making a send cost the same at a hundred thousand records as at a thousand: the executor stops building and publishing an author log it never usefully published, three full record scans per send become a small index of this member's own readings, and `serve`'s per-tick rebuild becomes a checkpoint of the open segment. The mechanism was already in `01` §3.1 — sealing exists to bound re-chunking on append — and `rebuild_log` never sealed. Previously v1.16 — §5's superlinear half is fixed: the cost was re-publishing rather than re-reading, since every `append` re-encoded the whole segment, so a rebuild did n²/2 records' worth of cryptography and kept one answer. `push` and `publish_current` separate the two; a send at a thousand records goes from 298 ms to 71 ms and the curve becomes linear, and `serve`'s per-tick pass stops being quadratic too. Previously v1.15 — §5 records what the repeated work actually costs, measured rather than argued. The two halves are not the same size — a send is superlinear in the author's own record count and unbounded by sealing or retention, at about 300 ms once a thousand records are behind it, while replay grows with structure and costs an order of magnitude less at the design target. Previously v1.14 — §8 gains the ephemeral-broadcast row, whose point is how to test a negative: an absence is only evidence when something known-good is crossing the same path at the same time. Previously v1.13 — §5.1's last owed item is settled rather than built: a node holds only what it can read, kept for the forward secrecy it buys, and what that will cost private channels is recorded in `03` §3.5 as a requirement of the work that makes it true. Previously v1.12 — §3 gains `SetPresence` and `MemberPresence`. The event is deliberately one-directional: a member goes quiet because nothing arrived, which is not an event and can never be one, so a consumer decides that by a beat going stale against its own clock. Previously v1.11 — §5.1 is new and carries the storage work built on 2026-09-07 and 08: tiers as reasons rather than places, replica duty over sealed segments, the two ceilings, the order things are given up in under pressure, and the one rule the whole thing turns on — that a provider count is safe in one direction only, so *nobody answered* and *nobody holds it* must never collapse into one answer. Previously v1.10 — §3's event list matches the enum, which it had stopped doing in three places at once: `Backfill` was named as an event when it is a variant of `Arrival` inside `Records`, the count said six when nine existed, and §8 said there was nothing for events "which do not exist" beside a row testing five cases over them. All three are the same gap — the command half of the boundary has a compile-time drift guard and the event half has none — and §8 now carries that guard as owed rather than the count as a fact. Previously v1.9 — §1 records the second Tauri default to remove a feature silently: the native drag handler takes the drag before the page sees it, so channel reordering never worked. Twice is a pattern, and §8's row is now about shell configuration rather than the ACL alone. Previously v1.8 — §1.1 is new: closing the window *is* the shutdown path, so no durable write may happen in place, and what is held rather than written wants stopping. Previously v1.7 — §1 records the shell's second boundary: Tauri's ACL refuses every `plugin:` command an application declares no capability for, silently, and this client shipped with none — so no node event ever reached the window and three polls were written as fixes for what was one denial. §8 gains the row that keeps it fixed. Previously v1.6 — §4 takes the single-node-per-network claim and its six-second expiry, §5 takes what the missing projection costs, and §8 gains the content-routing row; all three moved here from a status file that was carrying them. Previously v1.5 — §3 lists `CreateCategory` and `UpdateCategory`, which landed in the code before they reached this page. Previously v1.4 — §1 and §2 describe the layout that was built: `kols-node` holds the executor, the daemon and the event loop, and `kols-net` is publish and fetch over it. §3 separates what crosses the boundary from what is designed and unbuilt, and `GovernanceReorg` has moved into the first list. The store and media crates still do not exist
**Depends on:** all preceding documents; App Hosting Spec §1–§3 for the sandbox path
**Consumed by:** implementation; `09` for the interface built on §3's boundary

This document is the client's *architecture*. What the interface looks like and how it
behaves is `09` — including the one thing §1's diagram leaves open, that `kols-node` owns one
`MemberNode` **per network**, and a user belongs to several.

---

## 1. Shape

A standalone desktop application: a Rust core linking the `intranet-*` crates directly,
and an HTML/CSS/JS interface rendered in a webview, with **Tauri v2** as the shell.

```
┌─────────────────────────────────────────────────────────┐
│  kols-ui        HTML/CSS/JS in a webview                │
│                 holds no keys, no sockets, no files     │
└───────────────────────────┬─────────────────────────────┘
                            │  kols-api  (§3) — the only crossing
┌───────────────────────────┴─────────────────────────────┐
│  kols-core      domain: channels, records, merge order, │
│                 permissions, keys, retention, search    │
│  kols-node      the executor, the node daemon, and a    │
│                 file-backed store — owns the event loop │
│  kols-net       publish and fetch over a running node   │
│  kols-store     not built — kols-node stands in for it  │
└───────────────────────────┬─────────────────────────────┘
                            │
              intranet-* crates (protocol v1.0)
```

**Why Tauri rather than a native Rust GUI.** The stated goal is to eventually run this
inside the app-bundle sandbox, and App Hosting §1.2 fixes that sandbox as a webview
running HTML/CSS/JS. A native GUI would mean writing the interface twice. Building on a
webview now means the sandboxed variant is a re-host of the same interface against a
narrower API (§7), not a second product.

**Why not Electron.** Same webview model, considerably heavier install, and an IPC
boundary to a separate Rust process instead of an in-process call. Tauri's boundary is
the one we want to define carefully anyway (§3), so getting it for free is worth more
than Electron's ecosystem advantage.

**The shell has a second boundary, and it fails silently.** Tauri v2 gates every `plugin:`
command on an access-control list assembled from capability files, and an application that
declares none gets an empty one — every such command refused. `kols-api`'s own commands are
not affected, which is exactly what makes this hard to see: the window opens, the channels
draw, messages send, and the whole thing looks healthy.

**This client shipped that way, and the cost was mistaken for four separate bugs.**
`listen` is `plugin:event|listen`. With no capability it was refused, `watch()` rejected on
its first `await`, and *none* of the node's events were ever delivered to the window — not
records, not governance, not the relay's standing, not the reorg report, not `Degraded`.
What kept the interface alive was the polling added in response: the channel every two
seconds, the waiting room every four, the relay on its own timer. Each of those was written
as a fix for "a pushed event was the only path to a redraw", and each was really a
workaround for the same denial one layer down. The features with no poll behind them —
unread counts, the degraded banner — simply never worked, and read as unbuilt.

**And it happened again, in the other direction.** Tauri installs a native drag-and-drop
handler on the webview by default — it is how a window receives files dropped from the desktop
— and it takes the drag before the page sees it, so HTML5 drag-and-drop does not work. Tauri's
own documentation on the field says disabling it is *required* to use HTML5 drag and drop on
the frontend. Channel reordering was drag-only, so for as long as folders have existed there
was no way to reorder a channel; the front end was correct throughout and the events simply
never arrived. `dragDropEnabled: false` is the whole fix, and the trade it makes is that this
window cannot receive dropped files — which costs nothing while there are no attachments and is
a decision to revisit when `kols-media` exists.

Two lessons, and the second is the general one:

- **A capability file is not optional configuration**, and its absence is not a smaller
  version of having one. `crates/kols-app/capabilities/default.json` grants the core default
  set and the two window commands outside it, and nothing else — there are no plugins here,
  because everything this client does crosses `kols-api`.
- **A denial that produces no output is a test's job**, since no amount of running the
  application reveals it. `crates/kols-app/tests/permissions.rs` resolves every `plugin:`
  command the interface calls against the real configuration, asserts the window label
  the capability names is the one the config actually creates — the two default
  independently and can drift apart without either file looking wrong — and asserts the
  native drag handler is off.
- **Twice is a pattern, and the pattern is the defaults.** Both of these were shipped
  behaviour that no amount of using the application would reveal, arrived at by leaving a
  Tauri setting alone. The rule this leaves is that a shell setting this application depends
  on gets asserted against the real configuration, whether or not it was written down —
  because what is not written down is exactly what defaults out from under you.

### 1.1 Closing the window is the shutdown path, so it has to be survivable

There is no shutdown protocol in front of it and there should not need to be one. The window
closes, the process ends, and whatever was in flight was in flight. Two consequences, and only
the first is about correctness:

**Nothing may be written in place.** `fs::write` truncates the destination and then fills it,
so a process ending between those two steps leaves a file that is neither the old contents nor
the new. For most of what this store keeps that is an empty list the next tick rewrites. For
`entries/` it is a governance log that no longer decodes — and `Store::log` refuses the whole
log rather than the one file, correctly, because a governance log with a hole in it is not a
smaller governance log. The window is milliseconds wide and what is on the other side of it is
the network, which is the wrong side of that trade to leave to chance. Every durable write goes
through a temporary and a rename, and the temporary lives in the store's own `tmp/` rather than
beside its destination, because the directories this store keeps are all scanned: a leaked
temporary in `entries/` is a corrupt entry, in `chunks/` a corrupt chunk, and `append_entry`
numbers by counting the directory so it would take an index twice.

**This is atomicity and not durability**, and the distinction is worth keeping straight. The
bytes may still be in the page cache when the process ends; they survive the process dying,
which is what this is for, and they would not survive the machine losing power. Guarding that
means an `fsync` per record, which is a real cost to take deliberately — and losing the last
message to a power cut is a different order of problem from losing the network to a window
closing.

**What is *held* rather than written still wants stopping.** The node claim is released on drop
and otherwise expires on a six-second timer, so a process that simply ends makes the next launch
sit waiting for a claim nobody holds — the window opens and the node behind it does not start for
several seconds. The relay reservation is a slot on somebody else's machine, held until it times
out. So the shell stops the node on `ExitRequested`: aborting the task drops the future, dropping
the future drops the claim, and it waits for that because an abort that is never polled has
dropped nothing. Bounded, because closing a window must never be the thing that hangs, and
everything the bound gives up on is what the expiry already covers.

---

## 2. Crate Layout

| Crate | Owns | Deliberately does not own |
|---|---|---|
| `kols-core` | Record types and canonical encoding, merge ordering, permission resolution, channel/session key management, retention policy, local search | Any I/O, any libp2p type |
| `kols-node` | The executor behind §3's boundary, the `kols` binary, the `MemberNode` event loop, gossip subscriptions and sync back-off, and the file-backed store standing in for `kols-store` | The interface, and any view shape |
| `kols-net` | Publishing and fetching over a running node — chunk store and announce, pointer accept, segment reassembly | The event loop, domain rules, UI state |
| `kols-store` | **Not built.** SQLite schema and queries, blob cache, migrations | Domain rules |
| `kols-media` | **Not built.** Capture, encode, jitter buffer, playback, `MediaTransport` impls (`04` §5) | Signalling policy |
| `kols-api` | The command/event surface (§3) and its consent decorators | Anything else |
| `kols-app` | Tauri shell, window/tray/notifications, OS keychain, and the view shapes the webview receives | Domain rules |
| `kols-ui` | The interface | Everything above |

**`kols-node` is not in the original drawing, and that is the correction rather than an
addition.** This layout was drawn before there was an executor. When one arrived it needed the
store, the daemon and the event loop in the same place — an executor that cannot reach the log
cannot refuse an edit aimed at somebody else's message — and `kols-net` kept only what does not
need the loop. `kols-store` remains the intended projection (§5); until it exists,
saying `kols-net` owns the event loop describes a client nobody built.

**`kols-app` converts rather than deriving.** The domain's records have exactly one
serialization and it is normative — spec 07 §3's canonical encoding, hand-written because a
record's id is the hash of those bytes. Putting `Serialize` on the same types would create a
second serialization living beside the first, and what that invites is not hypothetical:
somebody eventually sends the convenient one over a wire and finds that ids no longer match.
So the shell owns a set of view types, free to change whenever the interface wants something
different, because nothing verifies against them.

**The webview never builds a command.** It names an intent with plain arguments — a channel
id, a body — and the shell constructs the `Command`. That is not a weakening of §3's first
property, since the command still names its target and the gate still resolves permission by
replay; it is one fewer place where a front end can hand the core a shape it did not expect,
and it keeps `kols-api` free of `serde` entirely.

`kols-core` must stay I/O-free and deterministic. Merge ordering, permission resolution
and retention decisions are exactly the code that has to produce identical answers on
every node, and pure functions over explicit inputs are how that stays testable.

---

## 3. The API Boundary

Everything the interface can do is one of a fixed set of commands; everything it learns
arrives as an event. No shared memory, no callbacks holding protocol types, no key
material crossing in either direction, ever.

```
Built, and crossing the boundary today:

Command  = OpenChannel { channel_id, before: Option<Hlc>, limit }
         | SendMessage { channel_id, body, attachments, reply_to }
         | EditMessage | DeleteMessage | React | Pin
         | CreateChannel { .., category: Option<CategoryId> } | UpdateChannel
         | CreateCategory { name, position } | UpdateCategory      — spec 07 §1.8
         | SetName | CreateInvite | AdmitMember | RevokeMember
         | SetBootstrapRelays | SetNetworkName                     — D32, spec 07 §1.7
         | SetChatSetting { setting, value }                       — spec 07 §4.3, §2.8
         | SetAdmissionMode { mode }                               — Core §2.4
         | CreateRole { group }                                    — `02` §1
         | SetPermission { group, verb, scope, grant }
         | SetRoleMember { group, identity, member }
         | LeaveNetwork                                            — Core §2.5.1, `02` §6.5
         | SetContribution { storage_offered }                     — Core §4.3, `02` §6.4
         | SetPresence { state }                                   — `01` §9, `09` §4.1

Event    = Records { channel, records, arrival }
               — Arrival = Live | Head | Backfill { segments }
         | Governance { learned } | Adopted { entries } | EpochRotated { excluded }
         | MemberKeyed { identity } | JoinAnswered { joiner, accepted }
         | MemberPresence { identity, state }                — `01` §9, `09` §4.1
         | Relay { reserved, designated, failures } | Degraded { reason }
         | GovernanceReorg { mine: [VoidedAction], others }  — §4, Core §2.7.1 pt 5

Designed here and not built:

Command  | Search { scope, query }
         | JoinVoice { channel_id } | LeaveVoice | SetMute | SetDeafen
         | StartStage | PromoteSpeaker
         | StartDirectMessage { with: identity, in_network }   — creates a network, `03` §4.3
         | AcceptDirectMessage | DeclineDirectMessage

Event    | ChannelState | PermissionsChanged | MemberPresence
         | VoiceState { participants, topology, transport: Delivery }
         | KeyStatus { channel_id, have_key: bool }
         | DirectMessageRequest { from: identity, link_verified: bool }
         | SyncProgress
```

**The split is the point, and this document used to blur it.** One list of everything, built
and intended together, reads as a description of the boundary and is not one — it named
`SetPermission` beside `SendMessage` as though both worked. Two names changed on the way
(`InviteCreate` → `CreateInvite`, `AdmitWaitingMember` → `AdmitMember`), `InviteRedeem` never
existed under that name because redemption happens in `join` rather than at this boundary, and
four commands arrived without being written down here: `SetName`, `CreateInvite`,
`SetBootstrapRelays` and `AdmitMember`. The relay one came in with the window's relay panel and
never came back to this page, which is how a boundary document stops describing its boundary — and the
same thing happened again the day this paragraph was written, when `CreateCategory` and
`UpdateCategory` landed and were caught only by a sweep at the end of the session. The lesson is
not that people should remember. It is that a boundary is worth checking against its code
mechanically, which is cheap, rather than by intention, which is not reliable.

**That check now exists, and writing it found the drift it was written to prevent.**
`kols-api`'s consent suite carries a sample of every command, and the sample list had fallen
four behind the enum — so the drift test that gives `Sensitivity` its meaning was silently
classifying eleven commands out of fifteen. Two guards replace the intention: an exhaustive
`match` with no wildcard arm, which stops the suite compiling until a new variant is named,
and an assertion that the sample list covers every name. Neither alone is enough, since the
first compiles happily with the list untouched.

**`SetContribution` carries all four of `02` §6.4's contributions** — storage, upload,
download and relay willingness. It briefly carried only storage, on a reading of "keep it
simple" that turned out to be the wrong simplification: the other three are things a member on
a phone or a metered link has a real opinion about, and leaving them at a shipped default is
the client deciding on their behalf, which is what §6.4 exists to forbid.

**Relay willingness is offered only where it could work.** A bootstrap relay's job is being
dialable by two peers who cannot dial each other (Core §5.5), so a node behind NAT
volunteering for it advertises something it cannot do. §4 now records
`ExternalAddressConfirmed`, and the interface hides the control until a public address has
been seen — hiding, never enforcing, per `09` §5: the command still accepts the flag and the
terminal can still set it. *Not confirmed* is deliberately weaker than *not reachable*, and
the surface says the weaker thing.

**It is also the first command gated on nothing that is not `Governs`, which broke an
assumption in the drift test.** `verb()` returns `None` for two reasons that had been treated
as one: gated on a *protocol* governance capability, or gated on no capability at all.
`LeaveNetwork` is the second kind and is still `Governs`, on the stricter-reading rule — it
writes the same irreversible entry `RevokeMember` does. `SetContribution` is the second kind
and is ordinary and reversible. So the test now names the capability-free commands explicitly
rather than assuming they are all governance, and the exhaustive match is what stops a new one
arriving unclassified.

**Not `Local`, though, and the reason is the sandbox.** `Local` means nothing is signed and
nothing leaves this node on the user's behalf, and the second half is false here: this changes
a public, signed claim about what the machine will give away, which the node re-advertises on
its next tick. Under §7's sandbox path `Local` would let hosted code raise a member's donated
disk with no prompt, which is the act App Hosting §3.3's consent decorator exists for.

**`SetPermission` landed with three commands rather than one**, which is what building it
showed. A permissions surface needs roles to exist and to have members, and `define-group`
and `manage-membership` are separate acts at separate bars (Core §2.2) — collapsing them into
one command would have flattened the asymmetry `02` §1 asks the interface to reflect. There
is deliberately **no `DeleteRole`**: `EntryBody` expresses no group removal, so a role's
capabilities and members can be emptied and its name stays in replayed history. The interface
says so rather than offering a control that cannot work.

**Accepted as permanent, 2026-09-07, rather than carried as a debt.** This was `STATUS.md`'s
O19 for as long as that file has had one, which read as a fix nobody had got round to. It is
not: no protocol change is being asked for, because nobody has needed one — a role that has
been emptied of capabilities and members holds nothing and grants nothing, and what remains is
a name in history that history is entitled to keep. The cost is a member meeting a limit the
interface has to explain, and explaining it is cheaper than an entry kind the log would carry
forever. If somebody ever does need it, it is E17 and it starts here.

**One grant at a time, not a capability set.** `DefineGroup` carries a whole set, so every
edit is a read-modify-write; a set-shaped command would make each edit overwrite whatever a
concurrent manager had just written, and the loser would silently revert a grant nobody meant
to withdraw.

**A setting is a value, not a key.** `SetChatSetting` carries a `ChatSetting` rather than a
policy key string, and the reason is quieter than `Scope`'s: an unrecognised app-policy key
is **not** refused. Core §2.6.2 makes absent mean the consuming spec's default, deliberately
unlike the capability registry — so a mistyped key would be written, replayed by every joiner
forever, and ignored, with the setting it was meant to change still reading as its default
and nothing anywhere reporting a problem. An enum makes that unsayable.

Writing a setting *back to its default removes the key* rather than storing the number. The
default is the same thing as absence, so writing it explicitly would freeze today's value
into a network that would otherwise pick up a revised one.

**The append lock is therefore held across the read as well as the write**, which is where
the first version of this had it wrong. Every other command reads the state `submit` replayed
and then locks only to append, and that is right for them, because none of them writes a value
derived from what it read. This one does, so a lock taken after the read leaves the window it
was meant to close: two managers each build a set from what they saw, the second lands, and the
first's grant is gone. **The verify-by-replay does not catch it** — it asks about the capability
this call changed, which is exactly the one that survived. Reading inside the lock makes the
pair atomic on this node, and what remains is the genuinely distributed case, which nothing
here can repair and the log records in an order every reader agrees on. `SetChatSetting` and
`SetAdmissionMode` take the same shape for the same reason, since `PolicyChange` carries the
whole policy record and is therefore read-modify-write by construction.

Three properties this boundary must hold, because the sandbox path (§7) depends on all
three and retrofitting any of them is expensive:

1. **No ambient authority.** Every command names its target explicitly; the core
   re-checks permissions on receipt rather than trusting that the interface only offered
   buttons the user was allowed to press. **Held by a type rather than by discipline**
   (D25): the gate returns an `Authorized`, which has no public constructor, so an
   executor takes one of those instead of a `Command` and being handed something nobody
   checked is not expressible. The compiler runs that claim as a `compile_fail` doctest.
   A channel's category is *looked up* from replayed state rather than carried on the
   command, for the same reason — whoever supplies the category chooses which grant
   applies.
2. **Consent is a decorator, not a redesign.** App Hosting §3.3 requires that any signed
   action on the user's behalf pass a platform-level prompt when the code is sandboxed.
   Commands are therefore tagged with a sensitivity class, and the sandboxed build wraps
   sensitive ones in a prompt. The native build does not prompt; nothing else differs.
   **The class follows the tier of the capability the command needs** (D26), not a
   judgement about consequence — the same rule spec 07 §3.8 settled for channel entries.
   A drift test resolves every command's verb against the vocabulary's own tier table, so
   re-tiering a verb and forgetting the classification fails there rather than in a prompt
   that quietly stopped appearing. A finer class is never a weaker one: prompting for
   everything above `Local` satisfies §3.3, and the finer grade only changes *how* it
   prompts.
3. **Events are idempotent and re-deliverable.** `Records` may arrive twice, out of order,
   or after a gap — the interface renders from the merged projection, never by appending
   what it just received. This falls straight out of `01` §7's rule that live delivery is
   an optimization. **It is the consumer's property, not the emitter's**, and it cannot be
   otherwise: a record pushed over gossip is *also* inside the segment that follows it, so
   duplicate delivery is the normal case. Merge — by record id, through `ChannelView` —
   never append.

**The node loop prints nothing either, and that took a second seam.** `Sink` carries
*events* and `design/05` §3 deliberately keeps the startup report out of that vocabulary —
it is what the node *is* rather than something that happened — so those lines were simply
`println!`ed from inside `serve`, which made the loop a layer that decides how something
looks. Exactly what `Sink`'s own justification says a second interface cannot reuse.

**It stopped being an inelegance and became a crash on Windows.** A GUI-subsystem binary
launched from Explorer has no console: `GetStdHandle` returns null, Rust's stdio turns that
into a write error, and `print_to` *panics* rather than dropping the line. So suppressing the
console — which is all the window ever needed — would have made the window crash on the
node's first line of output, on the one platform that cannot be run from the development
container. The fix for a stray terminal would have been strictly worse than the terminal.

`kols_node::Report` is the seam: the loop hands its lifecycle lines to whoever is listening,
`kols` prints them, and the window passes `quiet`. No diagnostics are actually lost — what a
window needs from those lines (relay standing, degradation, whether it is keyed) already
reaches it as events, and reaches it more usefully than as text it would have to parse.

**A command produces an `Outcome`, and the thing that produces it prints nothing.** An
executor that rendered would be one no interface could reuse, which is the whole reason this
boundary exists; `kols` renders the same values a webview would render differently. There is
one submit path — authorize, then run — and the `Authorized` never escapes it, because the
run step is what requires one and nothing else can produce one. So the check is not something
a future caller can be *asked* to remember.

**What the executor can answer that the gate cannot.** The gate reaches no store, on purpose,
which leaves two questions to the layer that does: whether an edit targets a message this
member wrote (a fact about the record set, not about replayed state), and the rate ceiling
(computed over the author's own HLC readings — §10.2 of `01`). Both are refused before
anything is signed. Neither is *enforcement*: nobody can write into another author's log, and
readers refuse an over-rate record whatever the writer believed. This is the author's client
telling them first, which is the division `01` §10.2 draws.

**The event vocabulary was written from the engine rather than ahead of it.** Nine variants
exist and each has something producing it — §4's loop had been reporting all of them in words
for weeks. Two categories are deliberately excluded: this node's transport, because a
sandboxed build gets no ambient host access (App Hosting §3.2), and the startup report,
because that is what the node *is* rather than something that happened.

**The event half of this list then drifted three times, and the reason is that only the
command half is checked mechanically.** This paragraph counted six when six existed; `Relay`
and `GovernanceReorg` landed the next day and the day after, and neither came back to this
page. The list above named `Backfill` as an event when it is a variant of `Arrival` *inside*
`Records` — so it was written twice, in one line, contradicting itself. §8's boundary row said
there was nothing for events "which do not exist" while the row two below it tested five
cases over them. Every one of these is the same mistake the commands made and stopped making:
the consent suite's exhaustive `match` and sample-list assertion catch a new `Command` at
compile time, and nothing does that for an `Event`. **The lesson is not that the count was
wrong. It is that one half of a boundary has a guard and the other half does not**, and the
half without one has now drifted every time it changed. §8 carries the guard as owed.

**What is still designed rather than built.** The commands for direct messages, search, voice
and stage, each of which has a line above and no code behind it. `00` §5 sequences them by
phase. Note also that events currently reach a terminal and no projection-holding client —
which is a missing consumer rather than a missing contract.

---

## 4. Sync Engine

`kols-node` runs one `MemberNode` per joined network on a single-threaded event loop —
and because a direct message conversation *is* a network (`03` §4), a user with fifty
conversations is running fifty of them. They are cheap (two members, no relay duty, a
governance log of a handful of entries) but they are not free, so node lifecycle is a
real concern: idle DM networks should be suspended and woken on demand rather than all
held live. *Flagged: the suspend/wake threshold wants measurement, not a guess.*

**Exactly one process may run a node for a given network, and the store enforces it.** The
MLS group is live state, so two nodes would each advance it without seeing the other, after
which whichever saved last decides the network's key — with no symptom at the moment it
happens. So a store carries a claim: `kols serve` on a store the window has open is refused,
and the other way round.

**The claim expires rather than only releasing on drop**, after six seconds without a
heartbeat (`kols_node::store::NODE_CLAIM_STALE`), and the expiry is the load-bearing half. A
window is closed by the window manager, which runs no destructors, so a claim released only
on `Drop` would leak on the *normal* way this application ends. A crash therefore costs a
pause rather than a stuck store. A pid check is the obvious alternative and is worse:
liveness is a different question on every platform, and a reused pid looks alive while
belonging to somebody else. Claiming also waits a stale claim out rather than refusing on
sight, since a restart is ordinary.

**A suspended node finds out that it lost its claim, rather than carrying on** — built
2026-09-07, and the shape is worth keeping because the obvious fix was the wrong one. The
expiry is wall-clock, so a sleeping laptop is indistinguishable from a dead one and its claim is
taken over. That part is *correct*: from the store's side the two really are the same
observation, and a longer window would only move the line rather than close anything. What was
wrong was the waking, when the first process went on believing it held a claim it did not.

So the claim carries an **owner token**, and the heartbeat checks whose claim it is refreshing
rather than only asserting that somebody is alive. Losing it is not a fault to recover from —
the other node is the holder and is right to be — so the loop stops and says so.

**One thing this had to not break, and it is the part a reader should look for.** Releasing a
claim on `Drop` was unconditional, which after this change would have had the *loser* delete the
*winner's* heartbeat and directory on its way out: a store that reads as unclaimed while a node
is actively running against it, which is a worse state than the one being fixed and reachable
only by fixing it. `Drop` checks ownership first, and
`tests/workspace.rs::a_claim_taken_over_while_its_holder_slept_is_reported_lost` asserts that
half specifically rather than trusting the reasoning.

Each loop, DM or server, is structured identically, with the domain layer talking to it
through channels. Care is needed with one implemented invariant: `next_swarm_event` drains its `pending` queue only on entry, so an event pushed
from inside the loop is delivered on the *next* call — which never comes if nothing else
happens. The crate's own guidance is to push to `pending` only from an arm that returns
immediately. The client must respect that rather than rediscover it.

Loop responsibilities, in priority order:

1. **Live** — gossip subscriptions for open and notified channels (`01` §7).
2. **Head sync** — resolve author-log pointers for visible channels; delta-fetch head
   segments.
3. **Backfill** — walk `previous_segment` chains on demand, bounded concurrency. Landed
   in `kols serve`, which walks to the start of history because it has no scroll
   position to bound it; a UI bounds this by pages instead (`01` §5). Each hop resolves
   its own key, since each segment lives under its own pointer (`01` §3.1.0) — a hop whose
   wrapping this node cannot open is where the walk ends, and is also what reading past a
   retention boundary looks like.
4. **Publish** — seal open segments on the size/age thresholds (`01` §3.1); publish
   immediately when the user goes idle, so a message is never stuck unpublished behind a
   half-full segment.
5. **Maintenance** — governance sync, ledger gossip, DEK re-wrapping after rotation,
   participant-index and search-posting re-announcement inside their TTLs.

Two protocol behaviours the client is specifically obliged to act on rather than log:

- **The voided-actions report** (Core §2.7.1 point 5). When reconciliation voids an
  action this node submitted, the client must resubmit or prompt. This matters most for a
  voided revocation, where the alternative is a removed member quietly becoming current
  again because nobody was assigned to notice. It surfaces as `GovernanceReorg`.
- **Kademlia server mode.** Provider records go unanswered while a node is in client mode,
  which is the default until it has a confirmed external address — on a LAN or loopback
  every lookup returns nobody. `set_dht_server_mode(true)` where appropriate, and expect
  publicly addressable nodes to carry records in production.

**Ledger before fetch.** A holder that has not advertised upload capacity is dropped by
source selection as not having volunteered, so the capability ledger must be populated
before a fetch can use a source the DHT found. Layering is governance, then ledger, then
fetch — a fetch that finds nothing on a fresh node is usually this, not a bug.

---

## 5. Local State

**SQLite** holds a projection, never the source of truth: messages (id, channel, author,
HLC, body, flags), reactions, channel and role state from replay, read watermarks,
attachment cache metadata, and the local FTS index (`03` §6 — the only search available
for private channels). It can be deleted and rebuilt from the network.

### The projection, measured first — half built 2026-09-08

**The replay half is built and it needed no SQLite.** A command asked the log three questions —
replayed state, the channel fold, the name fold — and each one **re-read every entry file,
decoded it and verified its signature on insert**. Measured at 152 ms for one ordinary command
against a network with three hundred channels, growing with the log, which only ever grows.

The store holds the log and its replayed state across calls now, invalidated by counting entry
files: entries are only appended and are numbered by the directory's own size, so an unchanged
count is an unchanged log — no read, no decode, no verification. A change reads only the files
past the count and **advances the state along the chain** rather than replaying from genesis,
which `GovernanceState::apply` has always supported; where fork choice has moved a branch out
from under the cached chain, the prefix comparison catches it and the fold is redone, because
`apply` walks forward and cannot unapply. The channel, category and name folds are cached the
same way, keyed on the same generation.

| Channels in the log | One command, before | A settled send, after | A governance write, after |
|---|---|---|---|
| 10 | 7 ms | 1.0 ms | 1 ms |
| 100 | 46 ms | 1.1 ms | 2 ms |
| 300 | 152 ms | 1.5 ms | 8 ms |
| 350 | — | 1.8 ms | 11 ms |

A send is what a member does repeatedly and it is now near flat; a governance write pays for the
rebuild its own entry causes, which is the honest place for that cost to fall.

**The read half is not built, and it is the larger one.** `open_channel` reads every record in
the channel from its own file, decodes it, merges the whole set and runs the reader-side rate
pass over it — so `before` and `limit` bound what is *returned* rather than what is read:

| Records in the channel | Opening it |
|---|---|
| 500 | 20.9 ms |
| 3,000 | 120.8 ms |
| 6,000 | 222.6 ms |

Linear at about 0.037 ms a record, so **roughly four seconds at a hundred thousand** — for a page
of fifty. Records accumulate far faster than structure does, so this is the one that bites first
at scale, and it is the half that genuinely wants an index rather than a cache: a page needs the
records in an HLC range *plus* whatever targets them, which is a query rather than a fold. That
is what `kols-store` is for, and it is what remains of O4.

#### What `kols-store` has to be, from the measurements above

**A page is a query, and three things make it one.**

1. **Which records are in the page.** The records of a channel ordered by HLC, taking `limit` of
   them before a given reading. An index on `(channel, hlc, id)` answers it; nothing else can,
   because the record files are named by content id and a content id says nothing about order.
2. **What acts on them.** An edit, withdrawal, reaction, pin or redaction may sit anywhere later
   in history, so a page needs the records *targeting* its messages. An index on
   `(channel, target)` answers that.
3. **Which records were refused.** This is the one that decides the shape of the whole thing, and
   it is not obvious.

**Why the rate verdict must be stored rather than recomputed per page.** `withheld` is a greedy
left-to-right fold: a record is refused when its author's ceiling is already met by the records
*already admitted* in its trailing window (`01` §10.4). Folding over a page alone would admit
records the full fold refuses, so the same message would appear when somebody scrolled to it and
vanish when they loaded the channel whole — the divergence §10.1 makes these network policy to
avoid, arriving from inside one client instead of between two.

So the projection stores each record's verdict, and the fold state that produces it is **bounded**
— admitted readings inside the rate window, plus the author's last message reading for slowmode.
That means a verdict is a function of the *stored verdicts* of that author's records in the
trailing window, which is a query rather than a walk: `(channel, author, class, hlc)` answers it.
A record arriving out of order — backfill reaches into the past — invalidates the verdicts after
it, so those are re-folded forward. In order, which is the live case, that is one record.

**A stored verdict is only true of the limits that produced it**, which is the rule that falls
out of storing it at all. The ceilings are network policy and slowmode is per channel, so a
`define-policy` change or a moderator calming a channel makes every verdict in scope stale — and
a stale refusal is a message that stays hidden after the rule that hid it was relaxed. So the
limits a channel's verdicts were folded under are stored beside them, and a change re-folds that
channel. Rare by construction: these are governance acts, not traffic.

**Held is not stored**, and the asymmetry is deliberate: a future-dated record is held by
comparing its reading to *now*, so it is a per-record question with a moving answer and storing it
would freeze a verdict that is supposed to expire.

**The record bytes stay in files.** The projection indexes and does not duplicate: a page reads
fifty record files rather than a hundred thousand, and the files remain the source of truth. This
store already keeps every message twice — once as a record and once inside a segment's chunks —
and a third copy would be paid for on every member's disk to save a decode that costs nothing at
fifty records.

**What `kols-core` has to change**, named here because it is the part that is not local:
`ChannelView::render` computes the withheld fold itself, so it cannot render a page. It takes the
refusals instead, computing `held` locally as it always did.

**And the ordinary discipline applies.** The projection is deletable and rebuildable from the
records, it is never consulted for anything replay answers, and a disagreement between it and the
files is settled by rebuilding — the same standing rule as every other cache here.

#### Built 2026-09-09, and deliberately not switched on

`kols-store` exists: the schema, the three indexes above, the verdict fold, and the fold
fingerprint that re-folds a channel when its limits change. `Store::page` is the read — it brings
a channel's rows up to date, asks for the page, reads those record files and the ones acting on
them, and hands back the refusals. `ChannelView::render_excluding` takes those refusals and
computes *held* itself, which is why it takes a clock rather than a whole `Withheld`: a caller
that could hand over a hold would be handing over a verdict that was true a minute ago.

**What it is not wired to is `open_channel`, and that is a product decision rather than
plumbing.** The window's `open_channel` takes a channel and nothing else — there is no paging in
the interface at all, and `before` and `limit` have been on the command since it was written
without ever being passed. Rendering a page into an interface that cannot ask for the next one
would hide history with no way to reach it, which is a worse failure than the slowness it fixes.
So the mechanism is built and proved, and turning it on is `09`'s question: what a channel does
when somebody scrolls past the top of what was loaded. `01` §5 has always said a UI bounds this
by pages; there is now something for it to bound.

The equivalence that makes the whole thing sound is asserted rather than argued:
`kols-store/tests/verdicts.rs` folds records one at a time and compares every verdict against
`kols_core::withheld` over the same set — the real pass, not a second implementation of it —
across bursts, sliding windows, two authors, slowmode and the no-ceiling sentinel.

**None of that projection exists yet, and `kols-node` carries a file-backed store instead.**
Nothing has needed one: the terminal replays the governance log on every invocation, which is
slow and correct, and the window re-reads on a two-second tick. The projection is worth
building when something renders fast enough to notice it is not there.

Two costs sit against it meanwhile, and both are the same work being repeated rather than a
defect. Replay walks the log once per question, so reading channels and reading categories are
two walks over the same entries. And **the executor rebuilds an author's whole log to append
one record** — `rebuild_log` replays every record this member has written in a channel on every
write, which is correct, because a segment is a pure function of its record sequence (`01`
§3.1), and is linear in a log that only grows.

### What that repetition costs, measured

**Measured 2026-09-08 rather than argued, which the paragraph above had been asking for since it
was written.** `crates/kols-node/tests/cost.rs` is the harness and stays, because it is also the
thing that will show whether the projection delivered. Release build; a debug one runs this path
about forty times slower and would have answered a question about `rustc -O0`.

**The two halves are not the same size, and that is the finding.**

| Own records in the channel | One send |
|---|---|
| 25 | 2.5 ms |
| 200 | 21.6 ms |
| 500 | 89 ms |
| 1,000 | 298 ms |

| Channels in the log | `state()` | `channels()` | One ordinary command |
|---|---|---|---|
| 10 | 1 ms | 1 ms | 5 ms |
| 50 | 5 ms | 3 ms | 24 ms |

**The send path is the real cost and it is worse than linear** — forty times the records costs
about a hundred and twenty times the work, because each of the author's records is re-read from
its own file, decoded, sorted and then replayed into a segment that is re-chunked and
re-encrypted as it grows. At a thousand of your own messages in one channel, sending one costs a
third of a second; the curve keeps going, and nothing bounds it. **Sealing does not**, because
the rebuild is over `own_records` — everything this member ever wrote in the channel — rather
than over the open segment. Neither does retention, which drops what is *published* and not what
this store holds.

**The replay path is real and an order of magnitude less pressing.** It grows with structure
rather than traffic — no message ever enters the governance log (`00` §4) — so at `00` §6's
design target of hundreds of channels, one command costs something like 150 ms rather than
something like a second. Worth fixing, and not the thing that breaks first.

So the two are separable, which is a correction: they were carried as one item on the reasoning
that the projection fixes both. It does, but only one of them is urgent, and a fix for the send
path does not have to wait for a SQLite schema.

**And the superlinear part was not the re-reading. It was re-publishing, and it is fixed.**
Replaying `n` records called `append` `n` times, and **every `append` re-chunked, re-encrypted
and re-hashed the whole segment so far** — so a rebuild did `n²/2` records' worth of
cryptography to reach a state that one encode of the final segment produces identically, and
then discarded all but the last answer. `AuthorLog::push` appends without publishing;
`publish_current` encodes once however many were pushed. The pointer version still advances once
per record, because a version is only reachable through the one before it — but signing a small
record `n` times is a different order of work from encrypting a growing segment `n` times, and
only the last of those pointers is ever announced.

| Own records | Before | After |
|---|---|---|
| 25 | 2.5 ms | 2.4 ms |
| 200 | 21.6 ms | 15.1 ms |
| 1,000 | 298 ms | 71 ms |

**The shape matters more than the ratio.** Before, 1.74× the records cost 2.56× the work; now it
costs 1.74×. What is left is genuinely linear — re-reading and decoding every record in the
channel to find this member's, the per-record pointer-version signatures, and two encodes — and
linear is what a cache or the projection removes. Superlinear is what nothing removes except
this.

**The same pass runs in `serve`, unconditionally on every sync tick**, and had the same shape:
it published every record as it rebuilt the chain and kept only the last result. That was
quadratic CPU burned in the background forever, which no amount of nobody-is-waiting makes
acceptable. It encodes once per segment now — at the seal, and once more for the head.

### Constant rather than merely bounded — in progress, 2026-09-08

Linear is not good enough and the reason is a date rather than a constant: a prolific member in
a long-lived network reaches a hundred thousand of their own records in one channel, and every
term below is multiplied by that. **The target is that sending the hundred-thousandth message
costs what the thousandth does.**

**The mechanism to do it with is already in this design and the write path is not using it.**
`01` §3.1 gives the segment size threshold a purpose in as many words — it "bounds the cost of
re-chunking on append" — and `rebuild_log` **never seals**. It opens a segment at sequence zero
and pushes every record the author has ever written in the channel into it, so the object the
executor encodes is the whole history as one segment, growing without limit, while `serve` seals
properly as it replays. The two build different chains. The executor's is fiction: it is used to
derive the next reading and to report `moved` and `total`, and those numbers are therefore
measured against a segment that does not exist.

**Where the time actually goes, per send.** Three full scans of the channel's record directory —
every record, every author, read from its own file, decoded and sorted — one for the rebuild and
one each for slowmode and the rate window. Then the pointer version is signed once per record,
and the segment is encoded.

#### What the executor actually needs, which is much less

It needs exactly three answers, and none of them requires an encoded segment:

1. **The author's newest reading in this channel**, for the next HLC.
2. **The author's newest `Message`-class reading**, for slowmode.
3. **How many of this class the author wrote in the trailing minute**, for the rate ceiling.

So the executor stops building an author log at all. **It does not publish**, which it never
usefully did: the durable write is `put_record`, `serve` does every publish that reaches the
network, and nothing consumes the bytes the executor reported — the window never reads `Wrote`,
no test asserts it, and `kols post` prints a number computed against the fictional segment above.
The property those numbers existed to make visible is asserted directly by
`kols-core/tests/author_log.rs` and measured by `tests/cost.rs`, which is where a regression
signal belongs.

The three answers come from a **small per-channel index of this member's own readings** — the
newest reading and the newest `Message` reading, both exact and kept forever, beside the
readings inside a few minutes, pruned on write. It is a **cache**: absent or unreadable, it is
recomputed from the records exactly as today, so nothing about it is a second source of truth.

#### And the same for the background pass, which is the larger half

`serve`'s `publish_own_logs` rebuilds every channel's whole chain from stored records and
publishes every segment, **unconditionally, on every sync tick**. Re-announcing is deliberate —
Kademlia provider records expire — but re-announcing needs chunk ids, not a re-encode, and the
chunks are already in this node's store. So the open segment gets a **checkpoint**: its records,
its sequence, its previous-segment CID, its pointer version and the last reading it carried. A
tick then republishes the head and re-announces what it already holds, rather than rebuilding
history it cannot change.

The last reading has to be in the checkpoint rather than read off the segment, and that is a
latent defect being fixed rather than a new requirement: `next_hlc` reads
`segment.records.last()`, which is `None` on a freshly sealed segment, and `append`'s
monotonicity check only looks within the current segment — so nothing today would catch a
reading that went backwards across a seal boundary.

#### Why none of this costs byte-identical history

- **A checkpoint is a cache and the records stay the source of truth.** Delete it and the
  identical chain is recomputed. This is the property `01` §3.1 already states — a segment is a
  pure function of its record sequence — being *used* rather than *re-executed*, and it is the
  same move Core §2.7 permits for the governance log under checkpointed replay.
- **Sealing is already deterministic.** `01` §3.1 measures age across the segment's own records,
  newest minus oldest, never against the clock, precisely so that a replay a month later lands
  the seals in the same places.
- **Nothing here changes what a record or a segment encodes to.** The indexes and checkpoints
  hold readings and identifiers, and the bytes that go on a wire are produced by the same code
  from the same inputs.

#### What it took, and what it turned out to be

**Built 2026-09-08.** The executor no longer builds an author log at all, and the numbers are
what the section asked for:

| Own records | Before | After |
|---|---|---|
| 25 | 2.5 ms | 1.6 ms |
| 1,000 | 298 ms | 1.8 ms |
| 6,000 | — | 2.1 ms |

Flat, which is the whole point: the hundred-thousandth message costs what the first did.

**One line of the removed rebuild was load-bearing and is kept explicitly.** It derived the
log's DEK, which requires an epoch key — so a member who held none could not write. That is
fail-closed and worth keeping (`00` §2): a node that minted a key instead would write content no
other member can read and read nothing it wrote before, which is divergence that looks like
working software. It is O(1) and was being paid per record only because it sat inside a rebuild.

**The index is maintained in `put_record` rather than at the executor**, because the executor is
not the only writer. A record this member wrote also arrives over the wire — refetched from a
segment this node published and later lost, or written by another of their devices once §6
lands. An index that only saw the executor's writes would be behind in exactly those cases, and
behind means the next reading is not greater than one already published.

**And `next_hlc` takes a reading rather than a log, which fixed a latent defect.** Read off the
open segment it answered from nothing on a freshly sealed one, so the reading before the seal
was invisible — and `push` checks monotonicity only within the segment it pushes to, so nothing
would have caught a reading going backwards across the boundary.

#### The background pass, and the marker that must not be persisted

`publish_own_logs` now asks whether a channel has anything new before doing anything, comparing
the newest reading this member wrote against the one the log was last published through. An idle
node costs one small file read per channel per tick instead of re-reading, re-encoding and
re-announcing its entire history every two seconds.

Re-announcement keeps its own hourly cadence, because provider records expire (Search §3.2's
24-hour TTL) and a log nobody has added to still has to stay findable.

**The two markers are not the same kind of fact, and treating them alike was a real bug.**
*Published through* is about this disk and survives a restart; *last announced* is about a swarm
that a restart replaces. Persisting both left a restarted node holding content nobody could find
for an hour, looking entirely healthy — caught by `three_nodes`, which watched a restarted
keeper skip the publish it existed to make. Announcements are forgotten when a node starts.

#### What this reaches, and what it does not

Cost per send becomes **flat in history**, bounded by the open segment rather than by everything
behind it. What it does not reach is cost proportional to the *message*: appending still
republishes the open segment, so the seal threshold becomes the ceiling on per-send work and the
knob that trades it against the number of objects.

**Making it constant rather than bounded needed one more change, in the storage layer, and it is
done.** Content-defined chunking derives boundaries from a rolling window that restarts at every
cut, so an append can only disturb the last chunk — but `intranet_storage::encode` re-chunked and
re-sealed the whole plaintext to discover that. `AppendOnlyObject` keeps the last chunk's
plaintext and re-chunks only that plus what arrived, and Storage §1.3 now **requires** what it
rests on: boundaries depend only on the bytes since the previous boundary, and an incremental
encoding must be byte-identical to a whole one.

| Records in the segment | Whole encode per append | Incremental |
|---|---|---|
| 500 | 0.275 ms | 0.110 ms |
| 4,000 | 2.562 ms | 0.143 ms |
| 10,000 | 6.524 ms | 0.116 ms |

Flat against linear, so the ratio is 56× at ten thousand records and grows without bound.

`AuthorLog` holds the encoder and extends it per push, which is why `Segment` gained
`header_bytes` and `framed`: the bytes ahead of the record list are fixed once a segment exists,
and each record is framed independently, so appending a record appends its bytes and disturbs
nothing before them. That is spec 07 §3.5's count-free record list read one level up — the reason
it exists is that a count at the head would change on every append, and here that same property
is what lets the chunker keep everything it has already sealed. The encoder is rebuilt at a seal,
where the header changes, and at a rebase, which replaces the record set rather than appending to
it.

**And one thing the measurement caught that reasoning had not.** With the encoding incremental,
the cost was still climbing — because `publish_current` handed back a *clone* of the object, and
copying a segment per append is the same cost arriving one layer up. The object is shared behind
an `Arc` now, mutated through `make_mut`, so a caller still holding a previous publish gets a copy
at that moment and nobody sees an object change under them. It is worth recording that the second
bottleneck was invisible until the first was gone.

**Blob cache** holds fetched chunks, which is simultaneously how this node participates in
swarm serving (Storage §4.2): anything fetched makes this node a source. That should be
visible in settings, with a size cap the user sets, because it is their disk.

**Keys** live in the OS keychain where one exists, with each network's seed encrypted at
rest under a user passphrase. Per network rather than one master seed (`02` §6.3), so the
thing a passphrase protects is a set rather than a single object that links every identity
its holder has — and the passphrase **wraps** those seeds rather than deriving them, for the
reason `02` §6.3 gives: derivation from public inputs is offline-checkable against ids the
network publishes. `intranet-*` key types implement no `Debug` and no
serialization deliberately — use their `fingerprint()` methods for logging and tests, and
do not derive around it.

**None of that is built, and the paragraph above is a target rather than a description.** What
exists today is a seed written to a file, unencrypted, restricted to the account that wrote it —
a `chmod 0600` on Unix, a protected DACL on Windows — and refused outright where it cannot be
restricted, since a secret another account can read is worse than one that was not written. So
anything with read access to that user's disk is that member. `02` §6.3 settles the shape it
must take, and `00` §5 carries it as a release gate rather than a feature.

**Two UI honesty requirements**, carried from the guarantees the protocol actually makes:

- **Deleted means hidden, not unsent** (`01` §6). The confirmation dialog says so.
- **Retention is not deletion** (`01` §8). The setting says so.

Nothing in the interface should imply the system can retract bytes somebody already has.

---

## 5.1 What This Machine Holds, and For Whom

Built over 2026-09-07 and 08. `02` §6.4 owns what a member *offers*; this owns what the client
does with the offer, because it is architecture rather than policy.

**Tiers are reasons, not places.** A chunk is not *in* a tier; it is held for a set of reasons
and is droppable when none remain:

| Reason | Held because | Bounded by | Given up |
|---|---|---|---|
| **duty** | placement ranked this node for it | `storage_offered` | With evidence — see below |
| **cache** | this member fetched it to read | nothing this setting governs | Freely |

One object is commonly both, and that is the case the model exists for: **withdrawing a
contribution must never drop bytes a member is still reading.** Two directories with a chunk in
one or the other reads simpler and is wrong — it forces a choice about a chunk that is
genuinely both, and either answer breaks one of the two rules.

### Duty

`kols_core`'s placement is not ours to invent: `intranet_ledger::placement` is deterministic
given a ledger and weighted only by gossiped capacity, and reimplementing or re-weighting it
would break the property the scheme rests on (Storage §3.3, Core §4.6). This client chooses
*which keys to rank over* and does no arithmetic of its own.

**Sealed segments only, decided by the chain rather than a flag.** A head is republished on
every append, so ranking over it would re-place the network on every message; a head is nobody's
predecessor, so the set named as some held segment's `previous` is exactly the set that can never
be republished.

Three behaviours fall out of the protocol rather than being built here, which is the sign the
layering is right. `select` returns fewer than asked for, so a network too small to meet its own
replication factor has everybody hold everything — Storage §3.2's degraded operation by the
ordinary path. `rank` excludes a zero-weight node entirely, so *contributing nothing* needs no
check. And releasing duty leaves the object alone, because the mark is a reason.

**Ledgers converge; they do not agree.** A node that wrongly takes duty holds a harmless extra
copy; one that wrongly declines is caught when the ledger settles. No step may require agreement
at an instant.

### Two ceilings

`storage_offered` bounds the duty tier per network. An **installation-wide** ceiling bounds
everything — a disk does not know how many networks are on it, and at P2 every conversation is
one, so a per-network limit would bound the total at nothing. It lives in the workspace and is
set outside the `kols-api` vocabulary for the reason creating a network is: that boundary is per
network and this is a fact about the workspace above them.

**The arithmetic saturates.** A ceiling can be lowered below what is already held, and a plain
subtraction underflows into an enormous allowance exactly then.

### Under pressure, in order

1. **Stop fetching discretionary history.** Heads are always fetched — they make a channel
   readable, and the application working is not a contribution. History is refetchable and is
   what grows without bound, so it stops at the ceiling.
2. **Shed cached copies**, oldest in their chain first. A cached copy is not a replica, so this
   needs no evidence at all. **Records are never touched**: shedding drops the *servable* copy,
   so the cost falls on what this node gives the network and not on what its member can see.
3. **Give back replicas two other nodes demonstrably hold**, clamped to how many members have
   volunteered anything — a network of three cannot produce evidence of four.
4. **Hold a last known copy past the ceiling** for seven days, saying so, then give it up and
   record it. Exceeding a ceiling briefly beats destroying data, and the overshoot is bounded in
   time and in size, being reached only after 1–3 are exhausted.

**A provider count is a hint and is safe in one direction only.** It may overstate — records
outlive the bytes they name — so it is a filter and never a proof, which is why the bar is two
holders rather than one and why a stale answer reads as unknown rather than as its last value.
*Nobody answered* and *nobody holds it* are opposite states, and a design that collapses them
drops a last copy because the network was slow.

**A node that sheds must stop announcing what it shed.** A provider record outlives the bytes,
so a node that drops quietly sends every peer that believes it on a fetch that fails — and a
failed fetch counts against the *serving* node. Kademlia has no un-publish; not republishing is
the most that can be done and doing none of it was the bug.

### What a member is told

Three situations reach the same place and only one is ordinary, so they are three notices rather
than one "storage is full": history has stopped arriving; content is *about to be* lost and only
this machine holds it; content already *has been*, and here is the record. A member who was told
and did nothing has made a choice; one who was never told had it made for them.

**A member may ask for history the ceiling stopped**, bounded by the oldest reading they can see
— which is what `OpenChannel`'s `before` was always for. Asking is not getting: the executor
holds no node, so a want is recorded and the daemon honours it on its next pass. What arrives is
pinned for an hour, because shedding takes the oldest history first and that is precisely what
somebody scrolling back has just asked for.

### Repair

Built 2026-09-08, and it is what makes a generous member a backstop rather than a coincidence.
Placement alone decides what a node holds *when everything is well*; it says nothing about
content whose holders have gone, so before this a member offering a great deal of disk caught
falling content only where the ranking had already put them.

**Repair is the same ranking read further down, not a second policy.** Storage §3.4 asks for the
shortfall to be re-placed onto *the next nodes in the same deterministic HRW order*, which is
what keeps repair as independently recomputable as placement — every node computes the same list
from the same ledger, so which nodes step in is determined rather than raced.

**Depth follows the size of the hole.** A node ranked at `replication_factor + k` steps in when
the census says the object is at least `k + 1` copies short, and otherwise does not. One missing
copy wakes one standby; three wake three. Without that rule every node that noticed would adopt,
and a network would answer one missing copy with as many new copies as it has members.

**A repair ends only on evidence.** A standby is by definition outside the replica set, so the
ordinary release rule — *the ledger stopped naming me, so somebody else has it* — would give a
repair back the instant it was taken. So repair duty is marked as such, and is released only when
a **fresh** count says the network has its target without this node. No answer and a stale answer
both keep it, for the same reason the eviction rule above gives: not having been told is not the
same as having been told nobody needs it. Placement catching up is the other ending, and is a
promotion rather than a change — the object stops being repair and becomes ordinary duty.

**The census had to widen for this.** It used to ask only about duty, which answers *is what I
promised still safe to give back*. Repair asks the opposite question about objects this node has
no duty for, so those are asked about too — after the ones a decision may be pending on, and
still a few per tick, because this is background work nothing is waiting on.

**The ceiling still wins.** Repair is offered the room pass one left and takes on only what fits;
a node at its ceiling contributes no repair and sheds the copy like any other. That ordering is
what resolves the one place the tier model strains: a cached copy of under-replicated content is
not really a spare, but adopting it would mean promising bytes this node does not have.

### Not going looking is the design (D38)

Repair works over content this node **can name** — the objects it holds and the links it walked —
and deliberately does not crawl. Content arrives because placement ranked this node for it, and
the offer's job is to weight that ranking, not to be a quota the node fills.

**That is not the limitation it first looks like**, and checking rather than assuming is what
settled it. A node under its ceiling walks every chain it can read to the start, because
`history_budget` is unbounded while there is room. So it already holds every sealed segment it
could be ranked for or be the standby for, and duty and repair are complete over the network's
readable content. A node *at* its ceiling holds less — and has no room to repair with either, so
there is nothing it could have done with the knowledge. The two cases meet, which is why no fetch
path is owed here.

The reading that falls out and belongs on the settings screen: an offer is a **ceiling on
willingness rather than a target**. A 50 GB offer on a network holding 2 GB holds 2 GB.

### A node holds only what it can read, and that is kept (O25)

The walk stops at a segment whose DEK it cannot unwrap, so a member outside a channel's roster
never fetches, links or holds that channel's content and never takes duty for it. The pointer is
public and carries the object's CID, so the ciphertext *is* fetchable without the key — nothing
does it, and that is the decision rather than the omission.

**What it buys is forward secrecy.** An epoch key compromised later cannot open bytes this node
never kept, which is the property `03` §3.1 chose MLS for in the first place. Holding ciphertext
leaks nothing today — the holder can no more read it than a stranger could — so the trade is
entirely between that hardening and durability, and the hardening wins.

**It costs nothing at all right now.** Roster keying is unbuilt: `channel_dek` derives from the
network epoch regardless of a channel's privacy flag, so every member can decrypt every channel
and there is no content a node is excluded from. What it will cost when private channels land —
placement ranking over the roster rather than the ledger, or the effective replica set becomes
*roster ∩ top-k* and can be empty — is recorded as a requirement of that work in `03` §3.5 and
against E2, rather than left to be found during it.

---

## 6. Multi-Device (Designed Now, Built Later)

Core §1.3 is already implemented: devices are independently seeded and linked by
certificates recorded in the governance log; revoking a device does not rotate the
identity. The client ships single-device but must not make choices that block this:

1. **Every record carries the signing device** (`01` §3.3), so a device revocation can
   invalidate that device's records without touching the identity's other work.
2. **Read state is keyed by (device, channel)** from day one (`01` §9), so cross-device
   sync later is a merge of watermarks rather than a schema change.
3. **Nothing assumes a single writer per identity per channel.** An author log is
   single-writer by *identity*, not by device, so two devices appending concurrently
   would collide on the pointer version. The protocol resolves that deterministically —
   lower record hash wins, loser retries at an incremented version (Storage §2.2) — but
   the loser's records must be re-published, not dropped. Build the publish path with
   that retry from the start; discovering it during a P5 rewrite is the expensive way.
4. **A network's seed stays on as few devices as possible.** Additional devices get
   certificates, not the seed. Enrollment is per-network (Core §1.3 point 7) — which is now
   the shape of the secret as well as the shape of the enrolment, so the UX is "add this
   device to these servers" rather than one global action, and there is no single object
   that would grant all of them at once.

---

## 7. The Sandbox Path

The eventual goal is running this same interface as an in-network `app-bundle`, rendered
by a generic protocol-aware client. What that requires, and what it does not:

**Already satisfied by this architecture:** the interface is HTML/CSS/JS; it holds no
keys; every privileged action crosses a narrow named API; consent is a decorator.

**Required from the platform, not from us:**
- An execution sandbox meeting App Hosting §3.2 — origin isolation per `app_id`, no
  ambient host access, platform-enforced CSP. **This is deliberately outside the
  protocol** (§3.2.1): nothing in `intranet-app` will tell a client an app is safe to
  run. A client that executes an `app-bundle` without its own sandbox has skipped a step
  the protocol was never in a position to take.
- Enforcement of `network-storage-read`/`write` (supported in v1) and `realtime-media`
  (declared, not yet enforced) — the latter is precisely the extension point a
  communications app needs, and it does not exist yet.

**What the sandboxed build gives up**, and why the desktop build stays the primary
target for now: no OS keychain, no tray or native notifications, no direct audio device
access without `realtime-media` being enforced, and a webview it does not control. Voice
in particular is unlikely to work in the sandbox until that capability is real.

The honest framing: **the sandbox build is a P5 packaging target for text chat, not a
replacement for the desktop client.** Designing for it now costs a disciplined API
boundary, which is worth having regardless.

---

## 8. Testing

| Layer | Approach | State |
|---|---|---|
| `kols-core` | Property tests on merge ordering: any permutation and any partition of a record set must converge to the same rendered history. This is the correctness claim of the whole design | **Done** — 40 permutations, reversal, duplicates, two-sided partition |
| Encoding | Frozen vectors, round-trip, injectivity, domain separation, id stability (`08` §3) | **Done**, including on big-endian via `scripts/cross-check.sh` |
| Wire | Two live nodes: publish, pointer sync, fetch, reassemble, render identically | **Done** — plus the delta measurement between fetch rounds |
| Permissions | Table-driven cases over replayed governance states, including the tricky ones — frozen pointers after a narrowed grant, waiting-room members, voided revocations | Partial — non-member, forged signature and wrong-channel covered at the reader; both post gates, channel/category/network scope and governance tier covered at the boundary; frozen pointers and waiting-room members not |
| API boundary (§3) | Every command against a replayed log, not a hand-built state: a grant reaches the scope it names and no further, an ordinary verb does not buy a governance-tier one, and the consent class agrees with the vocabulary's tier table | **Done** for the command half — 18 cases, plus a `compile_fail` doctest for the unforgeable token. **Owed for the event half**: nine variants exist and no test names them, so nothing fails when one is added. The command half's two guards — an exhaustive `match` with no wildcard arm, and an assertion that the sample list covers every name — are what this wants, and their absence is why §3's event list drifted three times |
| Executor | Every record kind end to end through the real binary: the record is written, the merge renders it, and the refusals the gate cannot make — somebody else's message, the rate ceiling — happen before anything is signed | **Done** — 10 cases over a keyed node |
| Events (§3, property 3) | A consumer that merges rather than appends: the same event twice, events out of order, a gap filled later, and the case that actually happens — a record arriving live and again inside the segment behind it | **Done** — 5 cases. The daemon's own wording is asserted by the two-node tests, which is what makes the emitter's refactor behaviour-preserving |
| Keying | A removed member must fail to decrypt content wrapped after the rotation, and must still decrypt what they held. Assert the honest guarantee, not a stronger one | Not started (P2) |
| Platform | The code that differs per operating system, run where it differs: the seed's permissions and the home directory's resolution. Not the daemon suite, which tests merge and gossip and is platform-neutral | **Partial** — the store's resolution is a pure function with cases; the seed's permissions are asserted on Unix by `cargo test` and on Windows only against the **built artifact** in CI, because the Rust test for it is `#[cfg(unix)]` and compiles out |
| Multi-node | Extend the existing Docker NAT harness with chat scenarios: partition two members over a real network, heal, assert identical history | Not started — the in-process partition test is not this |
| Content routing | Provider discovery **through a peer that is not the holder** — a fetcher that has never been given the holder's address, finding it anyway. This is the part a one-hop table cannot fake | **Done**, upstream and in-process: `intranet-transport/tests/provider_discovery.rs`. The fetcher dials one node and nothing else, the holder dials that same node and nothing else, and the intermediary is asserted to hold no copy — so the only way the fetcher can learn who has the chunk is by asking somebody who does not. Probed by removing the announcement, which empties the answer. **The row used to say this needed the Docker NAT matrix, and that was over-specified**: what it needs is *topology control*, not address translation. The client's own daemons cannot provide it — every `kols` process can dial every other, and there is no way to withhold an address from one without turning off the discovery under test — but the transport's in-process tests construct the graph directly. mDNS cannot produce a false pass either way, since it discovers addresses and carries no provider records: knowing where a peer is is not knowing what it holds |
| Ephemeral broadcast (`01` §9) | Presence, where the claim that matters is a **negative**: an invisible member publishes nothing. A negative alone is satisfied by a broken mesh, a node that never connected, or a test that did not wait — so it is bracketed. The invisible phase runs first, a record crosses live between the same two nodes to prove the path is carrying traffic, and only then is the absence of a beat evidence. The wire half is unit-tested apart from the daemon: a beat cannot be forged or re-dated by whoever received it, an unknown state is refused rather than rounded down, and freshness uses this node's clock and never the sender's | **Done** — 7 wire cases, one two-node test covering both phases, and 7 interface checks, among them that the roster never renders "offline". Both halves mutation-checked, because a guard over an absence is exactly the kind that passes for the wrong reason |
| Shell configuration (§1) | Every `plugin:` command the interface calls, resolved against the real capability file and the real window label; and the settings whose defaults remove a feature silently. Tauri refuses what no capability names, and its native drag handler swallows HTML5 drag events, and neither produces any output | **Done** — 6 commands, the label the capability is scoped to, and `dragDropEnabled`. Written after shipping with no capabilities at all, which refused every event for the life of the client, and extended after the same shape of bug took drag-and-drop |
| Media | Loss and jitter injection against both `MediaTransport` impls; the fallback is expected to degrade badly and the test should record how badly, not skip it | Not started (P3) |

The protocol repo's gate applies to this work too: `cargo test --workspace` and
`cargo clippy --workspace --all-targets` both stay clean, and a run that skipped clippy
because the toolchain lacked it has checked half the gate and should say so.

**Where these run, and why it is not CI.** The suite runs in the development container, where
the output is complete and a failure can be reproduced in the minute it appears. CI builds; it
does not gate. The exception is the platform row above, which is the one thing a Linux container
cannot do for itself, and it runs as part of a build rather than on every push.

**Run the daemon tests starved as well as fast.** `taskset -c 0,1` is not a curiosity: these
tests spawn two or three processes that sign and encrypt, and a wide machine wins every race a
narrow one loses. A keying bug that had been latent for weeks — a joiner that learned its own
admission and never asked to be keyed in, because the ask was nested under an event that need
not recur — was invisible at full speed and reproducible in one run at two cores. Timeouts scale
with available parallelism (`tests/common::patience`) so the same suite is honest on both.

**And clean up.** This container's storage is the host's. Test helpers remove their directories
on `Drop` rather than at the end of a test, because `Drop` runs on an unwind and a failing test
is exactly when scratch is left behind.

**One measurement discipline learned in P0, worth keeping.** A test that asserts on bytes
moved must measure at the point the cost is actually incurred. The first version of the
wire test ran both fetch rounds — manifest, then chunks — and then asked what the reader
still wanted, which is zero by construction and proves nothing. The number that matters is
what it wanted *between* the rounds. A green test that asserts nothing is worse than no
test, because it is believed.
