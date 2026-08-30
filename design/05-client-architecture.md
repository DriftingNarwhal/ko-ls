# Client Architecture

**Document status:** v1.9 — §1 records the second Tauri default to remove a feature silently: the native drag handler takes the drag before the page sees it, so channel reordering never worked. Twice is a pattern, and §8's row is now about shell configuration rather than the ACL alone. Previously v1.8 — §1.1 is new: closing the window *is* the shutdown path, so no durable write may happen in place, and what is held rather than written wants stopping. Previously v1.7 — §1 records the shell's second boundary: Tauri's ACL refuses every `plugin:` command an application declares no capability for, silently, and this client shipped with none — so no node event ever reached the window and three polls were written as fixes for what was one denial. §8 gains the row that keeps it fixed. Previously v1.6 — §4 takes the single-node-per-network claim and its six-second expiry, §5 takes what the missing projection costs, and §8 gains the content-routing row; all three moved here from a status file that was carrying them. Previously v1.5 — §3 lists `CreateCategory` and `UpdateCategory`, which landed in the code before they reached this page. Previously v1.4 — §1 and §2 describe the layout that was built: `kols-node` holds the executor, the daemon and the event loop, and `kols-net` is publish and fetch over it. §3 separates what crosses the boundary from what is designed and unbuilt, and `GovernanceReorg` has moved into the first list. The store and media crates still do not exist
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

Event    = Records { channel_id, records }        — live and backfilled alike
         | Backfill | Governance | Adopted | EpochRotated | MemberKeyed
         | JoinAnswered | Relay { reserved, designated } | Degraded { reason }
         | GovernanceReorg { mine: [VoidedAction], others }  — §4, Core §2.7.1 pt 5

Designed here and not built:

Command  | Search { scope, query }
         | JoinVoice { channel_id } | LeaveVoice | SetMute | SetDeafen
         | StartStage | PromoteSpeaker
         | StartDirectMessage { with: identity, in_network }   — creates a network, `03` §4.3
         | AcceptDirectMessage | DeclineDirectMessage
         | SetContribution { storage_offered, bandwidth_cap, relay_willing }

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

**`SetPermission` landed with three commands rather than one**, which is what building it
showed. A permissions surface needs roles to exist and to have members, and `define-group`
and `manage-membership` are separate acts at separate bars (Core §2.2) — collapsing them into
one command would have flattened the asymmetry `02` §1 asks the interface to reflect. There
is deliberately **no `DeleteRole`**: `EntryBody` expresses no group removal, so a role's
capabilities and members can be emptied and its name stays in replayed history. The interface
says so rather than offering a control that cannot work.

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

**The event vocabulary was written from the engine rather than ahead of it.** The sketch above
has nine variants; what exists has six, and each has something producing it — §4's loop had
been reporting all of them in words for weeks. Two categories are deliberately excluded: this
node's transport, because a sandboxed build gets no ambient host access (App Hosting §3.2),
and the startup report, because that is what the node *is* rather than something that happened.

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

*Owed: a node suspended past the window can have its claim taken over while it still believes
it holds one — the check is wall-clock, so a sleeping laptop is indistinguishable from a dead
one. Making it impossible needs the holder to re-check ownership as it beats. Rare rather than
impossible today, because taking over requires somebody to start a second node inside that
window.*

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

**None of that projection exists yet, and `kols-node` carries a file-backed store instead.**
Nothing has needed one: the terminal replays the governance log on every invocation, which is
slow and correct, and the window re-reads on a two-second tick. The projection is worth
building when something renders fast enough to notice it is not there.

Two costs sit against it meanwhile, and both are the same work being repeated rather than a
defect. Replay walks the log once per question, so reading channels and reading categories are
two walks over the same entries. And **the executor rebuilds an author's whole log to append
one record** — `rebuild_log` replays every record this member has written in a channel on every
write, which is correct, because a segment is a pure function of its record sequence (`01`
§3.1), and is linear in a log that only grows. Both want measuring before they are optimised
rather than after; the projection is where they stop being recomputed.

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
| API boundary (§3) | Every command against a replayed log, not a hand-built state: a grant reaches the scope it names and no further, an ordinary verb does not buy a governance-tier one, and the consent class agrees with the vocabulary's tier table | **Done** for the command half — 18 cases, plus a `compile_fail` doctest for the unforgeable token. Nothing for events, which do not exist |
| Executor | Every record kind end to end through the real binary: the record is written, the merge renders it, and the refusals the gate cannot make — somebody else's message, the rate ceiling — happen before anything is signed | **Done** — 10 cases over a keyed node |
| Events (§3, property 3) | A consumer that merges rather than appends: the same event twice, events out of order, a gap filled later, and the case that actually happens — a record arriving live and again inside the segment behind it | **Done** — 5 cases. The daemon's own wording is asserted by the two-node tests, which is what makes the emitter's refactor behaviour-preserving |
| Keying | A removed member must fail to decrypt content wrapped after the rotation, and must still decrypt what they held. Assert the honest guarantee, not a stronger one | Not started (P2) |
| Platform | The code that differs per operating system, run where it differs: the seed's permissions and the home directory's resolution. Not the daemon suite, which tests merge and gossip and is platform-neutral | **Partial** — the store's resolution is a pure function with cases; the seed's permissions are asserted on Unix by `cargo test` and on Windows only against the **built artifact** in CI, because the Rust test for it is `#[cfg(unix)]` and compiles out |
| Multi-node | Extend the existing Docker NAT harness with chat scenarios: partition two members over a real network, heal, assert identical history | Not started — the in-process partition test is not this |
| Content routing | Three nodes with a **forced** indirect path — A and B unable to reach each other directly while both reach C — asserting A ends up holding B's records. Two nodes cannot demonstrate this: with nobody to route *through*, a one-hop table and a working DHT behave identically | Not started, and **never once observed**. The DHT is bootstrapped and `fetch_chunks` pulls from whichever holder answers, so this should work; nothing has shown that it does. Belongs with the NAT scenarios in harness spec §2.3, which already simulate the topology |
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
