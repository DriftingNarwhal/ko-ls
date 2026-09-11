# Engineering log

Newest first. Why the code and the documents are the way they are — the reasoning behind a
change, the thing that was tried and abandoned, the bug that turned out to be a different bug.

**This is history, not state.** Nothing here is a statement about how things are now: an entry
is true of the day it was written and is never revised afterwards. Where a later entry
contradicts an earlier one, the later one is what happened next rather than a correction to be
applied backwards. For current state read `STATUS.md`, and for how something is *meant* to
work read the design document or specification that owns it — an entry here may well describe
a decision those documents have since moved past.

**These entries were written inside `STATUS.md` and moved here unedited.** So where an entry
says "this file" it means `STATUS.md`, and where it cites `§0`, `§1`, `§4` or `§6` it means
that file's sections *as they stood on the day the entry was written* — before it was
reorganised into a map, and those numbers changed. The same goes for the `O` numbers: they
index a register `STATUS.md` §2 still carries, but the closed ones now live only here.

Nothing was rewritten to fix that, deliberately. Editing 96 entries to match a structure they
predate would mean deciding, ninety-six times, what an entry *meant* to say — which is how a
record of what happened turns into a record of what somebody later wished had happened.

Kept because this project keeps re-learning the same lessons and paying for them twice.

---

- **2026-09-11** — **`v0.13.2` did not open a window on Windows, and the suite could not
  have known.**

  The report: the app launches, unlocks, lists networks; clicking one opens a window that
  stays blank white and **cannot be closed**; the tray's quit does nothing; Task Manager is
  the way out. Three symptoms, and the useful move was to look for one cause rather than
  three bugs.

  It is one stuck thread. A synchronous `#[tauri::command]` runs its body **inline in the IPC
  handler** — `ExecutionContext::Blocking` is the command macro's default and becomes `Async`
  only when the function is `async` — and on Windows that handler is dispatched on the
  webview's own message-loop thread. `WebviewWindowBuilder::build()` there is the deadlock
  Tauri documents under *Known issues*, pointing at wry#583: WebView2's controller is created
  asynchronously and completes through the message loop, so the thread that has to pump it is
  the thread blocked waiting for it. The window gets created natively — hence white — and
  nothing after that returns, which takes the close with it and the tray's menu dispatch
  besides.

  **The fix is one keyword in two places**, which is the documentation's own remedy: *use
  `async` commands and separate threads when creating windows*. `open_network` and
  `open_conversation` are `async fn` now, so the body runs on the async runtime and the main
  thread is free to service the window it is being asked to build.

  **What took the time was not the fix but establishing it without Windows.** No reproduction
  was available, so the evidence came from reading: Tauri's own doc comment naming the
  deadlock, then the macro source to confirm what *synchronous* actually means for where a
  body runs, then a sweep for the same class — any window **getter** called on the main
  thread waits for a reply and would deadlock the same way, and there are none; the shell only
  uses setters and registry lookups. The query string in `index.html?network=…` was a second
  candidate for a blank page and was eliminated the same way: the asset handler splits on `?`
  before resolving, with a comment saying so.

  **Why nothing here failed, which is the part worth keeping.** Every gate this project runs
  executes on Linux, where WebKitGTK creates its webview synchronously on the calling thread
  and the identical code is correct. The container is not a weaker test of this property — it
  is a test of a different platform. And `v0.13.1` never had the shape: one window, created
  from the configuration during setup, never built from a command. The feature that needed
  windows made at runtime is the feature that introduced the trap, so the first build to
  contain it was the first build to break.

  So the guard is a **rule about the source**, which holds on every platform:
  `tests/window_creation.rs` asserts that no command building a window is synchronous and that
  no event handler builds one, following one level of call so a command reaching a helper
  counts — `open_network` builds through `show_network_window`, and a body scan alone would
  have missed it. It was written by reverting the fix and watching it name `open_network`,
  because a guard that passes on the broken code is not a guard.

  **Then the same question asked of macOS before building for it**, which was worth more than
  it looked. The bug itself is WebView2's: `send_user_message` runs a window creation *inline*
  when it is called on the main thread and *posts* it to the event loop otherwise, and only
  WebView2 needs that loop pumped to finish creating a webview — so v0.13.2 would most likely
  have opened its window on macOS. The check that mattered was the **other direction**: tao
  panics outright if a window is created off the main thread on macOS, so the fix could have
  traded a Windows deadlock for a macOS crash. It does not — `build()` from a worker only posts
  `Message::CreateWindow`, and the closure runs inside the event loop's `UserEvent` handler on
  the main thread, which is exactly where AppKit wants it. Only the request moved.

  Two macOS things came out of that reading. ⌘Q is fine, and now says so in a comment: the
  predefined quit item terminates the app, `applicationWillTerminate` becomes tao's
  `LoopDestroyed`, and that is `RunEvent::Exit` — while the `prevent_exit` beside it only ever
  sees `code: None` from *the last window being destroyed*, so it cannot swallow a quit. And
  `RunEvent::Reopen` — the **Dock icon** — was unhandled, which is the same shape as the bug
  being fixed: a documented way back that does not open. On macOS the Dock is the gesture
  somebody actually makes, and a member who had closed the window had the menu bar extra and
  nothing else.

  That arm could not be compiled here. `cargo check --target aarch64-apple-darwin` dies in
  `ring`'s build script for want of an Apple SDK, and the variant does not exist on Linux, so
  there is no cfg trick that gets it compiled either. What it rests on instead: `Cargo.lock`
  pins the exact `tauri` whose definition of the variant was read, and the call inside the arm
  is the one the tray handler already makes in compiled code. Stated because "could not be
  compiled locally" is the sort of thing that ought to be in the record before it matters
  rather than after.

  Third time a Tauri default or platform has removed a feature silently here, after the
  missing capability file and the native drag handler. The first two were denials that
  produced no output; this one produced no output *on the only platform the tests run on*.
  `CONTRIBUTING.md` now carries the second question that leaves: not only whether a shell
  behaviour is asserted against the real configuration, but whether that assertion could be
  true here and false where it ships.

- **2026-09-11** — **Slice 3, and three defects that were all one mistake.**

  Conversation windows, starting one from a roster, accepting and declining. The code went in
  without much argument; what the day was actually about is what launching it found, because
  the suite could not see any of it and all three had the same shape — **a window drawn once
  and then trusted**.

  The workspace window had no poll and no subscription. A network founded in it reported *not
  keyed in yet* for as long as the window stayed open: the epoch key is written a beat after
  the node starts, the row had already been drawn, and nothing asked again. I watched that for
  twenty seconds before believing it, then checked the disk — `epochs/` held the key and
  `rotation` matched it, so the state was right and the picture was old. The same gap ran
  straight through the feature being built: `09` §1.8 says a verified request appears in this
  list, and it would have appeared on whatever draw happened next, which might be tomorrow.
  Both mechanisms rather than either — the events the node already emits for what it announces,
  and a five-second poll for what nothing announces, which is most of §1.3's row contents.

  The other two were in `resume`, a startup path older than the workspace window that had not
  been re-read since D40 made a window a *view*. It marked a network as **in view** while
  nothing showed it, so the list drew the row as open and offered *leave* where it meant
  *forget* — for a network nobody had opened. And it gave up unless there was **exactly one**
  network, a single-network relic from before the list existed, which quietly meant a member
  with two networks unlocked and served **neither** until they clicked one. That one reaches
  other people: `09` §2 makes every joined network warm because a machine that serves nothing
  stops holding up replica duty it accepted, and it is the reason a conversation nobody has
  open can be reached at all.

  **Neither of those was a wrong line of code.** Each was a correct line whose meaning changed
  underneath it, which is the failure mode of a leftover rather than of a mistake, and reading
  the function would have caught both — the same lesson as *check the layer before building*,
  one layer further along: check the layer you changed the meaning of.

  **The visual check needed a conversation to look at, and there was none.** `dm::start`
  refuses yourself, so a single instance cannot produce one, and this container has no window
  manager to drive two. So the window was pointed at the conversation document with canned
  answers behind it, for one launch, with the files copied aside first and restored after —
  which is how the bottom-alignment bug turned up: a short conversation drew at the top with
  the composer a screen below. An auto top margin on the first row rather than
  `justify-content: flex-end`, which in some engines makes the overflow above unreachable.

  **And two checks in `drive.mjs` had never run.** They sat after the `process.exit` that ends
  the second pass — left there when the storage panel moved documents — and one of them was
  failing, because the stub it needed had been copied across without its body. A check that
  cannot run is worse than no check: it reads as coverage. Now 172 checks over three documents.

- **2026-09-11** — **Single instance, hand-rolled, and the plugin's source is why.**

  The hole slice 2 left: a tray icon can be built and never displayed, so the application can
  outlive its windows unreachably. §1.1 already specified the second door — a second launch
  raises the windows that already exist — and this is it.

  **The plugin was read rather than trusted.** `tauri-plugin-single-instance` uses a named
  mutex and `WM_COPYDATA` on Windows and **D-Bus on Linux**, and its Linux setup is
  `Builder::session().unwrap()` — which panics where there is no session bus. That is this
  container, and any minimal session, and it correlates with the very case the door exists to
  cover: a desktop with no tray host. A mechanism that fails where the problem it solves lives
  is not a solution to it. Licence and boundaries were fine — Apache/MIT, and raising a window
  signs nothing, so nothing about `kols-api` or the sandbox path objected. The objection was
  concrete rather than principled, which is worth separating.

  So it is the **node claim's shape one level up**: a directory, an owner token, a heartbeat, a
  staleness window. Platform-neutral, no dependency, and it keeps `05` §1's "no plugins here"
  true — which matters because the capability file's shortness is what makes it auditable.

  **A heartbeat is not evidence that anybody is answering.** The first version read the beat
  and exited if it was fresh, and killing a process showed what that costs: the last beat stays
  fresh for the rest of the staleness window, so a relaunch inside those seconds exited as a
  second instance with nobody to raise. That presents as *an application that will not start* —
  the worst failure available here, because the member's remedy is to try again and trying
  again is the gesture that keeps failing.

  The protocol is therefore an exchange: a launch **asks** and waits for the ask to be taken. A
  live holder consumes the marker on its next beat; a dead one never does, and the launch
  becomes the application. Verified with two real processes both ways — a second launch exits
  in under a second having raised the first, and a launch after a `kill` becomes the
  application with one window.

  **And the mirror of the same bug on the way out**: an exit does not drop a running task, so
  the claim had to be released where the nodes are stopped rather than left to its staleness
  window, or quitting and relaunching within a few seconds would look like the same failure.

  My own test failed when the protocol got stricter, which is the test having encoded the
  weaker one. Rewritten with a stand-in holder that actually answers, plus a new one for the
  heartbeat nobody stands behind.

- **2026-09-11** — **Slice 2: the tray, and the assumption underneath it that does not hold.**

  A tray icon, an explicit quit that stops every node and awaits them, a workspace window that
  hides rather than closing, and the one-time notice saying so. `05` §1.1's shutdown path moves
  off window-close onto that quit — amended rather than deleted, since atomic writes are still
  owed to crashes and a deliberate stop is now the one case that *can* release claims and relay
  slots properly.

  **The lock was written stronger than it needed to be, and building it showed which half
  mattered.** §1.12 said *hide all of them*. What the requirement actually is: nothing a locked
  installation should not show stays on screen. The network window plainly is that — it is a
  network's messages — and is closed rather than hidden, because a hidden window holds drawn
  state behind a lock that did not clear it. The workspace window is not: showing the lock
  screen, it names no network and lists nothing. Hiding it as well would make the tray the only
  way back from a lock, which is worse for no gain. The section is amended to the weaker, true
  statement.

  **And the tray is a desktop service rather than a window**, which the design had quietly
  assumed away. A Linux session with no tray host has none. So the builder's failure is not
  fatal and is not ignored either: where a tray cannot be built, closing the last window ends
  the application — the behaviour before this slice, and the honest one, since the alternative
  is a process a member cannot reach, cannot quit, and was told was fine. The notice is not
  shown there either, because it would be describing a tray that is not there.

  **The residual is the interesting part and is recorded rather than glossed.** A tray icon can
  be built *successfully* and never displayed — this container does exactly that, having no
  session bus, which is how it was found: the log said `Unable to get the session bus` while
  `build()` returned `Ok`. So the check catches a hard failure and cannot catch a silent one.
  The second door that would close it is already specified in §1.1 — **a second launch raises
  the windows that already exist** — which turns *relaunch it* into a way back. It is not
  built, so the promise has one platform-shaped hole and the way out of the hole is to quit
  from a window rather than closing it.

  Worth noting what found this: not review, and not the suite. The application was launched and
  its stderr read. The same shape as `v0.13.0`'s crash and slice 1's two defects — three times
  in two days that running it has been the only thing that worked.

- **2026-09-11** — **Running slice 1 found two defects that nothing else would have.**

  The window was built, the suite was green and the front-end driver was green, and then it
  was launched. Both of these were in the first ten seconds of using it.

  **The workspace document had no styles at all.** New markup with new class names, and the
  stylesheet's `.picker*` rules — sixteen of them — were now dead. It would have rendered as
  raw HTML, which is a worse answer to *not very aesthetically pleasant* than the thing being
  replaced. The picker's rules are now the workspace window's, rewritten for a 300px column:
  the row is the button and the leave control is taken **out of flow**, which is §5.1's second
  sizing rule applied where it bites hardest, since a control sharing a flex row with a name
  that may shrink makes the name's width a font question.

  **`class="sheet"` collided with an existing full-screen overlay.** `.sheet` is
  `position: fixed; inset: 0; display: flex` for the door and invite sheets, so the create
  dialog rendered as a column of single words down the left of the window. Renamed. Worth
  recording as the cost of reusing a stylesheet across a new document rather than as a typo:
  the second document inherits every name the first one took.

  **And Enter cancelled.** `<menu><button>cancel</button><button>create</button></menu>` — the
  first submit button in a form is what Enter activates, so typing a network name and pressing
  Return threw it away and closed the sheet. It produced no error and left no trace; the only
  reason it was found is that the driving script pressed Return and the network did not appear.
  Cancel is `type="button"` now, so the primary action is the only thing that submits.

  **What the launch did confirm**, and at the strength it holds: the workspace window opens at
  360x760 and is legible; an account is made and the copy is offered straight after, which is
  `02` §6.3's ordering; the create sheet renders and creates; the row draws with its open
  marker, its leave control and *not keyed in yet*; and **the network window opens titled
  `the workshop — 2fc7d5cc`**, which is D36 working for the first time since it was decided.

  **What it did not confirm is the reuse rule**, which is slice 1's central promise. Two
  networks in one window, the title clearing and changing with the content — unobserved. This
  container has no window manager, so `windowactivate` fails and clicks land wherever the
  pointer focus happens to be; further puppetry stopped earning its keep. It rests on the code
  path, the label being in the capability file, and the driver asserting the shell is the one
  asked. A person switching between two networks is what would actually settle it.

- **2026-09-11** — **Slice 1: the window model, and the trap it was sequenced to flush out first.**

  Two documents where there was one. `workspace.html` is declared at launch and
  `index.html` is created at runtime per network and reused as the member switches, which is
  what makes changing networks a click rather than backing out of one.

  **The capability file was the first change, deliberately.** It read `"windows": ["main"]`,
  and a capability is scoped to labels — so a new window whose label no capability names gets
  an empty allow-list, `listen` is refused, and no node event ever reaches it *silently*. That
  is the failure this shell already paid four apparent bugs and three polls for. Doing it
  before any second window existed meant the failure never had a chance to happen, and the
  existing test caught the rename the moment it landed, which is the guard doing exactly its
  job.

  **D36's title was never implemented.** It said `ko-ls` or `ko-ls (3)` — no network, no
  identity — and §6.5 had gated it on theming, which is unbuilt. D40 makes it load-bearing for
  a different reason, so it exists now and it is composed **by the shell**: the document
  supplies a count and never a name. A document that could write the network into the title
  could make one network wear another's, which is the whole of what D36 is for; a document
  lying about its own unread count is harmless. The reuse case clears the title before it
  redraws, because reuse turns that spoof into a temporal one.

  **Two things this slice found that were not in the plan.**

  The shell's first draft told a reused window which network it was drawing with `eval`,
  interpolating the network id into a script — in an application that spends a CSP keeping
  other people's code out of its documents. It emits an event instead, which is how every
  other push already reaches the interface.

  And `Workspace::containing` was written as "the parent directory", which is right for a
  store the workspace laid out and actively wrong otherwise: a `--home` pointing straight at
  one store is a supported shape, so the parent is whatever directory it happens to sit in —
  `/tmp` during a test run — and every unrelated store beside it would have been read as this
  installation's networks. It asks whether `path_for` would have placed the store there, which
  is exact. There is a test, because the loose version looks correct.

  **The front-end gate caught a real regression, which is the best thing that happened here.**
  Moving *create a network* to the workspace window left D29's shared-relay warning behind —
  and `09` §3 is explicit that creating a network with a relay is the **more common** of the
  two designations and the first one most people ever make. `drive.mjs` failed on it. It is
  ported, and the driver now runs two passes, one per document, because a gate that only
  covered the old document would have stopped covering half the interface.

  **One thing dragged more scope than expected, and it was the design being right rather than
  wrong.** The account gate had to move to the workspace window — it is the launch window —
  and `02` §6.3 binds the export offer to making an account. At a first run there are no
  networks, so there is no network window for the export to live in: `09` §1.13's *mine* group
  was asserting itself a slice earlier than planned. *this device* and *appearance* moved with
  it, which is where §1.13 puts them anyway.

  **What a launch proves, at the strength it holds.** The workspace window opens, at 360×760,
  titled. That is all it proves. The network window's creation, its title and the switch
  between two networks need a person — this is the shape that cost `v0.13.0` a crash, and the
  suite cannot see it for the same reason it could not then.

- **2026-09-11** — **The workspace became a window, and §7's oldest question kept its answer.**

  A second pass over the same section, after the user described what is actually wrong with the
  current interface: you have to back out of a network entirely to change networks, and join and
  create have permanent space on a page. They floated a dropdown and then an old instant-messenger
  buddy list, and said they would not mind several ko-ls windows open at once.

  **The buddy list is the right shape, and four questions settled the rest.** One network window
  reused as you switch, with a second on request rather than automatically; conversations in
  their own small windows; the application staying alive in the tray when its last window closes;
  and unlocking opening the workspace window alone.

  **It replaced what had been designed a few hours earlier in the same section.** That pass put a
  persistent rail inside the window, which was a reasonable reading of the documents and the
  wrong answer to a complaint the documents did not contain. Worth recording rather than quietly
  overwriting: the content decisions survived intact — what a row may claim, cold's staleness,
  no directory, no self-reordering, the invisible decline — and only the container changed. A
  design pass that has to be redone is cheaper when the reasoning was separable from the shape.

  **§7's first question keeps its answer, which is the part worth noticing.** It asked whether a
  fourth thing ever earns permanent space in the frame, and three field tests said no. It still
  says no — the workspace did not get a column, it left the frame. Overturning three field tests
  would have needed a better argument than "there is more to show now".

  Three consequences are load-bearing rather than decoration, and each comes from a decision
  already taken elsewhere:

  **A window is a view.** Closing a network's window must not set it aside or stop its node, or
  the defect `v0.13.0` was cut to fix comes straight back wearing a feature's clothes. So the
  application keeps running until somebody quits, which `00` §6 already implied by deciding that
  going offline is a separate act from locking, and which `05` §5.1 requires by having this
  machine hold replica duty for other people. `05` §1.1's "closing the window is the shutdown
  path" is amended rather than deleted: atomic writes are still owed to crashes and power cuts,
  and the deliberate Quit is now the one stop that *can* run a shutdown path properly.

  **Reuse reinvents D36's spoof in time rather than space.** Click a network, the content
  redraws, the title lags, and the message goes to the one you were looking at a moment ago. §1's
  existing clear-before-draw rule already covered the screen; the native title is now part of
  what is cleared, so a window mid-switch names neither network rather than one over the other's
  messages.

  **A tray menu would read the workspace out around the lock.** `02` §6.3 says the lock stops
  somebody at the keyboard seeing which networks this installation belongs to, and a tray listing
  them does exactly that without a password. While locked the tray offers unlock and quit and
  nothing else.

- **2026-09-11** — **The direct-message surface, and network management with it, because they are one surface.**

  Designed rather than built: `design/09` §1.1–§1.9. The brief was the DM surface; it took in
  network selection and management too, at the user's direction, and the merge is right for a
  reason better than convenience — **a conversation is a network** (D10), so a list of your
  conversations and a list of your networks are the same list, and building them apart would
  have been the interface disagreeing with the model.

  **What made it designable was a line drawn the day before.** `05` §3.1 had just separated
  acts that are about *one network* from acts that are about the *workspace above them*, to
  decide where the DM flow lived. That line turns out to answer "what belongs in network
  management" exactly: the switcher carries create, join, start a conversation, leave and set
  aside — which is precisely the workspace-act set — and the same rule is the test for whatever
  gets proposed for it next. A surface defined by a rule rather than by a layout is one that
  can refuse things later.

  **§7's first question closes, after three noes.** It asked whether a fourth thing ever earns
  permanent space in the frame. It does, once. The argument is not that conversations need
  somewhere to go: it is that D10 made the workspace big — a dozen servers is a list you open
  rarely and thirty contacts is a list you live in — so a modal in front of it is a modal in
  front of every message. Worth recording that the answer changed because a *decision from
  another document* changed the size of the thing, not because somebody looked at Discord
  again.

  **§7's third closes too, and answering it turned up a rule nobody had written.** §2 made cold
  *polled* rather than off. So a cold network's unread count is as old as its last poll, and a
  `0` on that row asserts the absence of news this machine never went and asked for — §4.1's
  rule about presence, at the scale of a whole network, and easier to get wrong here because a
  number looks authoritative where a missing word does not. The row says when it last looked.

  Three more that were decided rather than inherited. **There is no directory and there cannot
  be one**, so a name-shaped search box is not merely unbuilt — names are per network and not
  identifiers (spec 07 §1.7), so a box taking one is a phishing surface whose attacker's half
  is typing; the flow is pick a network, then a member, which is the shape of Core §1.2.
  **The list may not reorder itself on activity**, because a row moving between the look and
  the click is how a message is written into the wrong network — the failure D36 keeps a theme
  from causing, arriving through motion instead of styling. And **a decline is invisible**: the
  carrier acknowledged delivery and there is no application-level answer, so the person
  declining is told the sender will not learn it, and the sender's row says *delivered* and
  never *waiting*, since nobody can tell a decline from somebody who has not looked.

  One thing recorded as an exception rather than done quietly: the set-aside control sits on the
  row although §4.2's grouping would file it under *this machine, here*. Same argument §4.1
  already makes for the presence control — settings is a place you go to finish something and
  leave, and this is changed in the moment you are looking at the whole list. Two sections
  disagreeing about where a control goes is how a third ends up guessing.

- **2026-09-10** — **The daemon half of direct messages, and two things it found that were not on the list.**

  Offer, deliver, receive. The loop sends what is deliverable on an interval and stops on the
  carrier's acknowledgement; the receive arm decodes, checks the identity link binds the right
  pair, checks the sender is a current member, and only then keeps the request and raises the
  event.

  **D39's borrowed relay had never been called.** `borrowable_relay` landed with the decision
  on 09-10 and nothing in the client invoked it — so a conversation's node reserved no
  circuit, had no dialable address, and could not have put one in an invite. The first step of
  the flow was missing its own precondition, and the decision looked built because the
  function existed. It is wired now, recomputed at every start and **never cached**: every
  other relay path writes what it learned into the store so a node can dial before it has
  synced, and a cached loan would outlive the membership that justified it with nothing left
  to ask again.

  **The carrier could not tell a sender its payload had landed.** Core §5.1 says what an
  acknowledgement *means* — delivery-level only, never agreement — which reads as a complete
  description and was not one: the response arrived, libp2p handed it over, and the event loop
  had no arm for it. §5.1 also forbids the carrier queueing, so the retry is the sender's, and
  a sender with no ack has only two shapes available and both are wrong — re-send forever, or
  forget after one attempt and lose the request. Fixed upstream as Core v1.5's fourth
  obligation, with a live two-node test.

  **Fourth time, and the spec now says so rather than leaving it to be rediscovered.** §1.2's
  ownership proof had no serialized form; §5.6's invite had no bytes outside the join request;
  both were found by something finally trying to *send* one. The sentence added to §5.1 is
  that a mechanism specified from the receiving end is specified halfway — worth expecting
  when E13 lands, since address exchange is another send-shaped thing this carrier will carry.

  **And one hazard introduced and caught in the same sitting.** `Workspace::containing` first
  answered "the parent directory", which is right for a store the workspace laid out and
  actively dangerous otherwise: a `--home` pointing straight at one store is a shape this
  terminal has always supported, so the parent is whatever directory it happens to sit in —
  `/tmp` during a test run — and every unrelated store beside it would have been read as this
  installation's networks. The question that settles it is whether `path_for` would have
  placed the store exactly there, which is a pure function of the parent and the network id.
  There is a test on it, because the loose version looks correct.

  What is not built: accepting and declining, and the live two-daemon path — the terminal's
  `--home` names one store rather than a workspace, so a `two_nodes`-style test cannot reach
  this flow. The checks and the payload are tested against two stores instead and the wire is
  tested upstream, which is the same split `design/05` §8 already argues for provider
  discovery: test where the property is.

- **2026-09-10** — **Direct messages are not boundary commands, decided before the flow was built.**

  `design/05` §3 had carried three of them as `Command`s since before there was an executor —
  `StartDirectMessage { with, in_network }`, `AcceptDirectMessage`, `DeclineDirectMessage` —
  with *creates a network* written beside the first as though it were a footnote. It is the
  whole difficulty.

  **Three things point the same way, and the third is the one that decides it.** Starting a
  conversation creates a network and accepting one joins a network, which are the two acts §3
  already places outside the command vocabulary alongside `init` and `attach`. And there is
  nothing for the gate to do: creating a network needs no capability, and sending on Core
  §5.1's carrier needs only membership of the shared network, which the carrier meters and
  the *receiver* checks. A command whose gate is empty and whose target is the workspace is
  not what this boundary is shaped for.

  §5.1 had already stated the principle — the boundary is per network, and a fact or an act
  about the workspace above them sits outside it, with creating a network as the named
  example. §3's grammar was written first and §5.1 is the considered statement, so the
  grammar was the thing that was wrong. **Found by trying to type the signature**: the
  executor holds one `Store` and no workspace and no node, and every attempt to give it one
  was an attempt to argue with §5.1.

  **What crosses `kols-api` is the inbound event alone**, which is genuinely per network: the
  request arrives on the shared network's node, from a member of that network.

  Two corrections to the grammar came out of the same pass. **`in_network` is dropped** — the
  shared network is the one whose node carries the request, so a caller supplying it could
  name a network the sender is not in, which is the objection §3's first property already
  makes to carrying a channel's category on a command. And **`link_verified` is dropped from
  the event**, because spec 07 §6.2 requires the link verified *before the request is shown to
  anybody*: a flag could only ever be `true`, and a field that is always true invites an
  interface to render an unverified request with a badge, which is the disclosure §6.2 is
  written against. Emitting the event *is* the claim.

  **What this gives up is written down rather than discovered.** The sandbox path prompts on
  `Sensitivity`, so a flow outside the boundary gets no consent decorator and a hosted app
  cannot reach it — the same limit create-and-join already has, so it widens one hole rather
  than opening one, and §7's list of what the sandbox gives up is where it belongs.

  Built with it: the pending state on both sides, `dm::start`, and the event. Nothing emits
  the event yet, and §3's claim that every variant has a producer is corrected in place rather
  than left standing — that sentence is what made the list trustworthy.

- **2026-09-10** — **The two-machine test passed, on one network and on two.**

  Two machines, same network and separate networks. Everything more or less functioning; the
  tester came away with a short list of things to change and no defects. So the gap `STATUS.md`
  named around `v0.13.1` — *not tested: anything between two clients* — is closed, and the
  change both releases are about is the one that needed it.

  **Recorded at the strength it holds**, because this paragraph has been read as more than it
  was twice in three days. A session that works is evidence the ordinary paths work between
  two real machines on two real networks. It is not a measurement of the three failure modes
  `05` §8 names by name — contention between many nodes, delivery to a network nobody is
  looking at, claims released on close — and nobody watched a claim expire. Those want
  watching for on purpose rather than inferring from a good evening.

- **2026-09-10** — **O26 does not reproduce, and the thing its attribution was pointing at was real anyway.**

  The entry below is last night's. Measured again today, in the three shapes that matter:
  **green alone five times running** — which is the shape it failed in five times running —
  **green across all twelve of `two_nodes` at full width**, and **green across all twelve
  starved** under `taskset -c 0,1`. The machine was checked clean first, for the reason
  `CONTRIBUTING.md` gives.

  So the trigger was machine state that did not survive the session. That is what last night
  already suspected and could not name, and it still cannot: orphans, ports, scratch, disk and
  the fd limit were all clean then, and re-checking them today says nothing about a machine
  that has since restarted. **Recorded as unexplained rather than as fixed**, because a
  state-dependent fault and a fault that has gone away are indistinguishable on a green day —
  which is exactly the reasoning the `v0.13.1` entry below applies to a window that opens.

  **What did get fixed is a client defect on that path, found by reading rather than by the
  reproduction.** Both sites that ask to be keyed in were gated on `ready(..).is_ok()` —
  advertise into the ledger, *then* publish every author log — used as a proxy for *am I a
  member yet*. It is strictly stronger than that question: anything transient in the publish
  half made every tick skip the ask for as long as it lasted. **That is the same permanent
  strand the retry schedule was added to remove**, arrived at through the precondition instead
  of the cadence — and E14 exists because a member who cannot ask again is stranded rather
  than delayed. The ask now asks replayed state directly (`design/05` §4); advertising and
  publishing happen every tick in `adopt_local_changes` regardless and no longer decide it.

  **And the log line was diagnosing rather than reporting**, which is the part worth carrying.
  Whichever half of `ready` failed, startup said *not a member of this network yet* — so a
  publishing fault presented as a membership fault, and last night's attribution went looking
  at membership because the software told it to. It reports the actual error now. A wrong
  diagnosis in a log line is worse than no line, because it is believed and it is cheap: the
  bisection it cost ran to four commits across two repositories.

  Worth noting what this does **not** claim. It is not proven to be what failed last night —
  nothing reproduces, so nothing can be proven against it. It is a defect that was there, on
  the named path, with the failure mode described; the honest statement is that the register
  had one true half and one invented one, and only the true half was actionable.

- **2026-09-10** — **A joiner test started failing deterministically, and it is not from tonight.**

  `a_joiner_is_admitted_keyed_and_reads_what_was_written_before_they_arrived`: the joiner dials
  the founder, the founder writes and picks up the membership entry, and the joiner never leaves
  *not a member of this network yet*. Never keyed, wait expires at 45 s.

  **Attributed by removing things.** It fails at `main`, at the commit before the supervisor, at
  the commit before E10's client half, and at the exact `v0.12.0` state of both repositories.
  That last step is the one worth keeping: the protocol is a **path dependency**, so checking out
  an old client commit still builds it against *today's* protocol — moving only one repo back
  proves nothing, and I nearly stopped after doing exactly that.

  **And it passed on this machine an hour earlier at identical code**, which rules out a plain
  code fault and points at machine state. Orphans, ports, `/tmp` scratch and disk were all checked
  and clean; the fd limit is a million and the load average is under one.

  **It is isolated.** Eleven of twelve in that file pass, including
  `a_joiner_walks_back_through_sealed_segments_to_read_the_start` — also a joiner — and
  `a_founder_can_still_key_somebody_in_after_restarting`, which is the keying path. So neither
  joining nor keying is broken in general.

  Left open rather than chased at this hour, but **filed as urgent rather than as a flake**,
  because it is the admit-and-key path a two-machine test walks first. O20's signature is a test
  that fails under contention and passes alone; this one fails alone, five times running, which
  is a different animal and should not be filed with it.

- **2026-09-10** — **`v0.13.1` tested on one machine, and what that does and does not settle.**

  Confirmed by hand on Windows, portable build: the window starts, asks for the password, unlocks,
  lists the networks, **opens one without crashing** — which is what `v0.13.0` could not do —
  sends messages, and switches between networks.

  **What it settles** is the whole of the crash and most of the shell rewiring: opening a network
  no longer takes the process down, and switching between two no longer stops the one being left,
  which was the single-node behaviour this work existed to remove.

  **What it does not settle is the thing the release is about.** One machine cannot show whether
  several nodes contend for one runtime under load, whether a message reaches a network nobody is
  looking at, or whether claims are actually released when the window closes — that last one only
  appears as a *later* launch waiting out staleness. All three need a second client.

  Worth writing down as a distinction rather than a caveat, because the same shape has now cost
  this project twice in two days: a green suite and a window that opens are both real evidence for
  a narrower claim than the one they get read as. The release before this shipped on exactly that
  mistake — a smoke test that proved the window *opened* was reported as though it proved the
  window *worked*, and the crash was one click past where the check stopped.

- **2026-09-10** — **`v0.13.0` crashed on selecting a network, and the suite could not have caught it.**

  The window opened, unlocked, listed the networks, and died the moment one was chosen. The
  panic: *there is no reactor running, must be called from the context of a Tokio 1.x runtime*.

  **`tokio::spawn` finds the runtime entered on the calling thread, and panics when there is
  none.** The supervisor used it; `open_network` is a **synchronous** Tauri command, so no
  runtime is entered on the thread it runs on. The code it replaced used
  `tauri::async_runtime::spawn`, which works from any thread — so the regression was introduced
  by making the spawn *look* more ordinary than it was.

  **Why every test passed.** All of them reconcile inside `block_on`, which enters a runtime and
  makes the ambient lookup succeed. The dependency was invisible in the only place it was
  exercised and fatal in the only place it was used — a suite agreeing with the code about an
  assumption neither had stated.

  The fix is to stop asking the ambient context: `Nodes` holds a `tokio::runtime::Handle` given
  to it once, and a handle works from any thread. `Nodes::here()` keeps the convenient form for
  async callers and panics *at construction* where there is no runtime, which is the right place
  to fail — it names the problem where it can be fixed rather than at the first reconcile.

  **The regression test is the shape worth keeping**: reconcile from a plainly synchronous
  caller, outside any runtime, which is what the shell actually does. It fails against the
  shipped code with the exact panic a member saw.

  Recorded also because the smoke test run before tagging did not catch it and could not have: it
  launched the window against a fresh home, so it sat on the lock screen and never started a
  node. **"The window opens" and "the window works" are different claims**, and only the first
  was checked.

- **2026-09-09** — **The release build failed on the first tag after O7, and the local gate could
  not have caught it.**

  `v0.12.0` was tagged, both platform legs failed inside a minute of each other, `publish` was
  skipped, and the tag shipped nothing. Two tests in `kols-node --lib` panicked with
  `Locked("this installation has no account yet")`.

  **It is not a platform bug, which is the first thing worth recording, because the job it failed
  in is called *Platform tests* and both failures were on Windows and macOS.** It reproduces on
  Linux in four seconds. The failing thing is the *environment*: O7 made every store read go
  through an account, deliberately leaving **no unwrapped path even for tests**, and this step
  runs `cargo test -p kols-node --lib` with no `KOLS_PASSWORD` set.

  **Nothing this repository runs could have found it, and that is the part to fix rather than the
  two tests.** `CONTRIBUTING.md` instructs a developer to set `KOLS_PASSWORD`, so every local gate
  passes and always did — 389 green on `main` an hour before this. The release workflow runs only
  on a `v*` tag. So the one step that tested differently from how the project documents testing
  was also the one step nobody ran between releases, and the two facts hid each other for the
  eight days and nineteen commits between `v0.11.1` and this.

  **The first fix was one `env:` block and it was the wrong size**, which is the more useful half
  of this entry. Adding it to the failing step made that step pass and the build fail one step
  later, in the seed-permissions smoke check — which drives the *built binary* rather than the
  test harness, hits the same missing account, and had been sitting behind the first failure the
  whole time. Three steps needed it, not one: the test step and both platforms' smoke checks.

  **The mistake was fixing the reported failure instead of the class**, with the class in plain
  sight — O7 removed every unwrapped path, so *anything that writes a seed* needs an account, and
  the right move was to sweep the workflow for steps that run `kols` or `cargo test` rather than
  to patch the line in the error message. A one-line fix that makes the symptom go away is
  indistinguishable from a correct one until the next step runs.

  **What caught it was `workflow_dispatch` rather than another tag**, and that is the practice
  worth keeping: the trigger exists, had not been used since 09-01, and a tag is a bad first time
  to discover a build is broken because the tag is public by then. Two dispatch runs found two
  failures for the cost of no extra tags.

  The general shape is still the one to carry: **a CI step that does not run the documented
  command will disagree with the documentation exactly once, at a tag.** Anywhere a workflow
  re-states an invocation rather than calling the same entry point, it has quietly become a
  second definition of the gate.

  **The tag was moved rather than superseded**, which was the user's call between two defensible
  options. `v0.12.0` had been pushed and had published nothing — both legs failed and `publish`
  is gated on `refs/tags/`, so no release object and no assets ever existed under it. A tag that
  shipped nothing is not a released version, so re-pointing it costs nobody a download and keeps
  the version line honest; the alternative was cutting `v0.12.1` and leaving a permanent gap that
  every later reader would have to explain to themselves. The rewrite is a public ref either way,
  which is why it was asked rather than assumed.

- **2026-09-09** — **Both branches merged to `main`, and `v0.12.0` cut from it.**

  The register was the condition and the register is clear, so the two branches this project had
  been holding — `replica-duty-and-storage-ceilings` here and `close-the-moderation-head-question`
  in the protocol repo — went to `main`. **Fast-forward in both**, because `main` held no commit
  either branch lacked; there was nothing to reconcile and no merge commit worth making.

  **The protocol repo went first, and that ordering is not arbitrary.** The client depends on it
  by path, so a client `main` that built only against a branch would be a `main` nobody else could
  build. Its gate was re-run on `main` after the merge rather than trusted from the branch — 681
  passed, clippy clean — for the same reason a merge is not a build: the thing that was tested and
  the thing that is published are different commits until something says otherwise.

  **What `v0.11.1` was missing had changed in kind, which is the argument for cutting now rather
  than at a tidier moment.** Nineteen commits, and among them the whole of O7 — `design/00` §5 has
  called an account in front of the seeds a *release gate* since before there was code. Until
  today the client somebody downloaded from Releases was the last one that writes its seeds to
  disk in the clear, while `README.md` and `docs/two-machine-test.md` both send new users straight
  there. A release gate that holds only on a branch is not holding.

  **The version lives in two files and both had to move**: the workspace `Cargo.toml` and
  `crates/kols-app/tauri.conf.json`. They are independent, nothing checks them against each other,
  and the one that would have been missed is the Tauri one — it decides what the installer and the
  window's own metadata say, so a mismatch ships a binary that disagrees with its own tag and says
  so nowhere a build would fail.

- **2026-09-09** — **`WORKING.md` retired, and what checking its exit condition turned up.**

  It was written on 2026-09-07 to clear the owed register before P2, and it said from its first
  paragraph what would let it go: *every decision recorded below belongs in `design/` or
  `STATUS.md` by the time this file goes*. So retiring it is an audit rather than a deletion —
  the question is not whether the work is done but whether each decision reached a permanent
  home, and the answer was no in one place.

  **O16's second half had never landed.** The entry asked for three things: `design/00` §6's open
  question closed, O16 out of `STATUS.md` §2, and *`design/09` §3 gains the statement*. The first
  two were done on 09-07 and the Done log recorded them; the third was silently dropped, and the
  Done entry did not claim otherwise — it named `00` §6 alone. So the decision that this client
  does not dial what mDNS finds lived in the overview and not in the document an interface
  implementer reads, where §3 is exactly where D29's correlation argument is carried. Written
  there now as a cross-reference rather than a second copy, since `00` §6 owns it.

  **Three drifts in `design/05` §3, which is the list that has now drifted five times.**
  `FetchHistory` landed with O24 and never reached the page. `MemberPresence` was added to the
  built list on 09-08 and left standing in the *not built* list beside it, so the same event was
  described as both. And the prose still counted nine events where the list below it had ten.
  §8's row was stale in the same direction, recording the event half's drift guard as owed when
  it was built on 09-07.

  **The guard did its job and could not have caught any of these**, which is the part worth
  keeping. `tests/events.rs` fails to compile when a *variant* arrives unsampled, and that is
  what it was built for. A count written in prose is not a variant, and neither is an entry left
  behind in the wrong list — so the answer is not a better guard but fewer places to disagree,
  and the count is now stated once beside the list rather than twice.

  Nothing else was owed. The Done log's substance had already moved to this file, the storage
  design to `05` §5.1 and `02` §6.4, and the questions Q1–Q6 to the documents that own their
  mechanisms.

- **2026-09-09** — **O7 closed: the copy that survives the machine.**

  The lock and the export. `design/02` §6.3 has the decisions; what building them turned up:

  **A restore restores an identity and a way to reach the network — not history**, and that is
  the point rather than a shortfall. A returning member is already named in the governance log,
  so their own messages come back off the network like any other history. What the bundle has to
  carry is only what cannot be fetched: the seed, the network id, and a relay. Which is also why
  a phrase alone is not enough — a network id cannot be derived from a seed, and the list of
  networks a member belongs to lives in the workspace and in no seed at all.

  **Restoring never overwrites a seed.** Same reasoning as a second `init` refusing: the seed at
  that path cannot be recovered if it is lost. A network already present is skipped and said so.
  Skipping can leave somebody as the wrong member in that network, which is visible and fixable;
  overwriting is neither. The window says all three outcomes — taken, left alone, refused —
  because a restore reporting only what it added would be silent about the network somebody is
  most likely to be asking about.

  **The two Argon2 costs are deliberately different**, and it took writing the second to see it:
  the account is 64 MiB and three passes, the bundle 256 MiB and four. The bundle leaves the
  machine and may sit in a cloud drive for years, so an attacker's guessing budget against it is
  unbounded in a way it is not for a local file — and its cost is paid twice in a lifetime rather
  than at every login.

  **The export is offered right after the account is made and does not block it.** A first run
  that will not proceed until a file has been saved somewhere is a flow people learn to defeat,
  and this protects against losing the machine rather than against the next five minutes.

  Two probes on the load-bearing claim: restoring with fresh entropy instead of the bundle's seed
  fails *a restored member has to be the same member*, and dropping the collision guard fails
  *the seed already there must be untouched*.

  With this, **the owed register has nothing actionable left in it**: O1 is Wave 3's surface and
  waits on E10, E13, `03` §6's indexes and `kols-media`, none of which exist; O20 is a decision
  rather than an outstanding item.

- **2026-09-09** — **O7's keyring: the same construction one level up.**

  Seeds were the last thing on this disk in the clear. Everything else at rest — MLS group
  state, epoch keys, every DEK wrapping — was already sealed under a *seed*-derived key, so this
  is not a new mechanism; the only genuinely new part is where the secret comes from, because
  there is no seed above the seed.

  **The password wraps and never derives**, and the reason is worth keeping: deriving a seed from
  the password and the network id needs no storage and is badly wrong, because the network id
  travels in every invite and member ids are in the governance log — an attacker could derive a
  candidate identity from a guessed password and check it offline against a value the network
  publishes. A brainwallet with a verification oracle. A random seed wrapped under a
  password-derived key has nothing public to check a guess against.

  Four decisions taken with the user first. The account is **forced** on next launch, because a
  gate somebody can click past is a preference. The harness unlocks from the environment rather
  than getting an unwrapped mode — D30 keeps the terminal a test harness rather than a second
  interface, and it must not become a second security posture either. The OS keychain is offered
  and defaulted off, since §6.3's own argument is that a key usable without a secret is not
  protected by it. And the export is **portability, not recovery** — I had called it recovery,
  the user pushed back that there is no organisation and no one to be responsible but the user,
  and they were right about the name. What it is for is that the seed *is* the member and lives
  on exactly one disk: the password protects it from a thief and nothing protects it from the
  disk dying, and on a new machine a member without it is not locked out, they are a different
  person.

  **Two bugs, and the first is the kind that only appears at two.** The unlocked account started
  as one process-wide slot. Each installation has its own salt, so one workspace's key does not
  open another's — and the slot handed the first workspace's key to the second, which presented
  as *that password does not unlock this installation* against a password that was right. One
  installation per machine hides this completely; a test process that touches two does not. It is
  keyed by workspace now, which is also the truer statement: an account key is a property of a
  workspace, not of a process.

  The second was mine to cause: a standalone store looked for its account in its *parent*
  directory, so every bare store made under `/tmp` would have shared one account with every
  unrelated store beside it. A store opened on its own is its own workspace, and a `Workspace`
  provisions at its own root before making any store, so the multi-network case never reaches
  that fallback.

  **The KDF cost is deliberate and was also a hazard.** 64 MiB and three passes is what makes a
  guessing rig expensive; it is also 64 MiB per process, paid by a daemon suite that starts a
  great many of them, and O20 is already fragile under contention. Parameters live in the account
  file and a reader uses what the file says — already asserted — so the harness provisions
  cheaply and the shipped path keeps full cost. Same code path, honestly different numbers.

  Also worth recording as a habit: `cargo test` stops at the first failing *binary*, so two runs
  reported 199 and 248 passing and looked like a small break when five targets were red.
  `--no-fail-fast` is now in `CONTRIBUTING.md` beside the password.

  Three probes: writing the seed as itself fails both secrecy tests, and dropping `adopt`'s
  length guard fails the idempotence assertion. What remains of O7 is the surface and the export.

- **2026-09-09** — **Measured flat is not bounded, and the difference is the whole point.**

  Paging made a page cost a page and left the *tick* linear. Those are different properties: one
  is about what is drawn, the other about what is examined to decide nothing needs drawing. The
  second is what runs every two seconds for as long as somebody has a channel open.

  The reading that looked fine — 3.6 ms at five hundred records, 6.1 ms at eight thousand — is
  linear, and reads as flat because the constant is small. That curve only shows up at a scale
  nobody tests at, which is to say in somebody's client after two years. **A timing measurement
  cannot express *bounded*; it can only fail to notice.** So the guarantee is now a count of
  work — directory entries enumerated, files read, records decoded — asserted *equal* at two
  hundred records and at three thousand, with forty governance entries and forty held segments
  added on top. Equal at those sizes is equal at a hundred thousand.

  **Five things on that path grew, and every one had the same shape**: answering *has anything
  changed* by examining everything that might have. Reading and decoding every record file; then
  listing the record directory; then a `COUNT(*)`; a `COUNT(DISTINCT author)`; a listing and sort
  of the whole governance log; and a mark read per held segment. Each was a real saving over the
  one before and each left the growth one layer down. Worth naming as a habit rather than five
  bugs: the fix that gets you from *read everything* to *list everything* feels like the fix.

  **One primitive answers all five: a file that only ever grows, whose length is the signal.**
  Read with a `stat`, monotonic, and — unlike a counter written by read-modify-write — immune to
  two writers losing one another's update, which is the failure that would matter, since a marker
  returning to a value a reader had already seen would make that reader skip a record for good.
  It has to live in the filesystem rather than in memory because `serve` opens its **own** handle
  on the same store, so a counter in one would never see the other's writes.

  Records append their id, so the length is the count and the tail is exactly what is new.
  Governance entries append a byte, and there the tally is a change signal and **never** a count
  — nothing derives a filename from it, so two handles appending at once cost a rebuild rather
  than an entry. I had it deriving the entry number from the tally first, which reintroduced a
  numbering race in the name of removing a listing. Segments append a byte when a link is
  written, which is sound because shedding deliberately leaves links alone.

  Then the instrumentation itself was the thing to distrust: `listed: 0` meant *the five sites I
  remembered to instrument*. Charging every `read_dir` in the store — fourteen of them — is what
  makes the number a claim about the store rather than about my memory. It stayed zero.

  Three probes, each reverting one fix, each failing the test in its own way: 200→3000 listed for
  the records early-out, 12→172 for the log cache, 0→40 for the segment cache. And the test
  asserts a smaller page costs less, because constant is not the whole claim — a read that
  examined nothing would be constant too.

  **What it costs, stated rather than buried.** 32 bytes a record for the arrival log, about 3 MB
  at a hundred thousand messages beside records of hundreds of bytes each. A reliance on
  `O_APPEND` making small writes atomic against other appenders — a smaller assumption than the
  atomic `rename` the store already rests on, and not promised by POSIX at arbitrary sizes. And
  one more derived thing that can be wrong: a crash between the append and the record write
  leaves an id naming nothing, which the fold skips and a rebuild repairs. Every repair path
  lists a directory, and every one of them runs off the tick.

- **2026-09-09** — **Paging, and the boundary that could not be expressed.**

  O4's other half. `open_channel` now renders a *range* rather than a channel: a settled page is
  3.6 ms at five hundred records and 6.1 ms at eight thousand, against 22.8 ms and 397.7 ms for
  the whole channel — linear, and about five seconds at a hundred thousand, on a control the
  window re-runs every two seconds. `design/09` §4.4 is the design; the shape is a loaded range
  that only grows, local pages that load on scroll while the network fetch keeps its button, and
  three top-of-list states told apart.

  **The load-bearing fix was a type.** `Command::OpenChannel` had carried `before: Option<Hlc>`
  since it was written, and a clock reading cannot express a page boundary: records merge by
  reading and *then* by record id, because the counter is per author and device, so two members
  writing in the same millisecond share a reading legitimately. A boundary landing between such a
  pair excluded **both** — including the one that had never been drawn. A message disappears from
  the channel, nothing on screen reveals it, and every layer behaves exactly as written. It had
  never shipped only because neither field was ever read: the executor destructured them away.
  `Cursor` is the pair the merge sorts on, and `ChannelView` is keyed on it, so the order a page
  is cut on and the order the view merges in are one definition rather than two that agree today.

  **The measurement found the index being paid for and not used.** Before every page, the fold
  read *every record file in the channel and decoded it* — to compare a count. So a page still
  cost the channel, and the first reading showed the paged open slower than the unpaged one.
  Record files are named by their id, so the directory answers which records exist without
  opening one; and underneath that was the same mistake one layer smaller, materialising every
  identifier out of SQLite to discover none were new. This is the third time on this feature that
  the fix has been *stop reading what you already know*, which is worth saying out loud.

  **Twice the measurement itself was the thing that was wrong**, and both readings looked
  plausible. The first conflated folding a five-hundred-record batch with reading a page and
  reported 40 ms for something that costs 4. The second added a store-level column under
  hand-written limits that differed from the command's — so every call re-folded the channel, and
  the column measured the fold while appearing to measure the read. A verdict being true only of
  the limits that produced it is a property of this design; a measurement that varies them is
  measuring that property rather than the thing it asked about.

  **A ceiling that lied.** The reach limit began in the store, which silently truncated
  `Window::opening(usize::MAX)` — so `kols read` would have shown the last five hundred messages
  of a channel and said nothing about the rest. That is the same class of failure as the boundary
  above: the answer is wrong and nothing reveals it. It belongs where untrusted input arrives.

  Three smaller things the build settled. **Restoring the scroll position is a fix that predates
  paging** — emptying the list to redraw it clamps the scroll to the top and never puts it back,
  so a reader scrolled up was thrown to the top by every redraw that changed anything, which is
  why scrolling up has never been worth doing in this client. **The first-sight rule splits on the
  previously-drawn tail, not on the gesture**: marking nothing on a draw that reached backwards is
  almost right and loses a message, because one arriving in the same two seconds would be filed as
  seen and never marked. And **a want was kept alive by the wrong channel** — `history_incomplete`
  was node-wide, so a truncated chain anywhere stopped every channel's want from ever being
  forgotten; per-channel now, which the notice needed anyway.

  Two tests were green for the wrong reason and are not any more. The driver's `rows()` counted
  every child of the message list rather than every `.message`, so adding a notice shifted four
  mark checks by one index — the app's own comment had warned that a row which looks like a
  message and is not one is fine "until the day something counts them", and this was the thing
  counting them. And my own check that local history outranks the network notice passed with the
  branches *swapped*, because the fixture set both flags; the case that pins it is local history
  with no network boundary at all. The driver also aborted the whole run on the first missing
  element rather than reporting, which hid every check after it.

- **2026-09-09** — **The projection is built and switched off, and the reason it is switched off
  is not a technical one.**

  `kols-store` exists: the schema, the three indexes a page needs, the stored rate verdict, and
  the fingerprint that re-folds a channel when its limits change. `Store::page` reads a page and
  the records acting on it rather than the whole channel. All of it is tested and none of it is
  reachable from the interface, because **the window has no paging**. `open_channel` takes a
  channel and nothing else; `before` and `limit` have been on the command since it was written
  and have never once been passed. Rendering a page into an interface that cannot ask for the
  next one would hide history with no way to reach it, which is a worse failure than the four
  seconds it fixes.

  Worth recording as a shape: **the mechanism was the easy half.** The measurement said opening a
  channel costs the conversation, and the fix looked like an index — but the index only pays off
  if something can ask for a page, and nothing can. `01` §5 has said since it was written that a
  UI bounds this by pages; there is now something for it to bound, and that is `09`'s question
  rather than this crate's.

  **Two design rules came out of building it, and both are the kind that bite later.**

  The rate verdict has to be *stored* rather than recomputed per page. The pass is a greedy fold
  in merge order, so folding over a page alone admits records the whole-channel fold refuses —
  and the symptom is a message that appears when you scroll to it and vanishes when you load the
  channel whole. That is the divergence §10.1 makes these limits network policy to avoid,
  arriving from inside one client rather than between two.

  And a stored verdict is only true of the limits that produced it. Ceilings are network policy
  and slowmode belongs to the channel, so a `define-policy` change or a moderator calming a
  channel makes every verdict in scope stale — and a stale refusal is a message left hidden by a
  rule that no longer exists, which nothing else in the system would ever correct. The limits are
  stored beside the rows and a change re-folds.

  **The test that matters compares against the real pass, not a second implementation of it.**
  `verdicts.rs` folds records one at a time through the index and checks every verdict against
  `kols_core::withheld` over the same set. Probed twice: an off-by-one on the ceiling fails three
  of six, and ignoring slowmode fails the one that is about slowmode. Writing a second copy of
  the fold to compare against would have proved only that I can write the same bug twice.

- **2026-09-08** — **"Build the projection" was one entry and two costs, and only one of them
  wanted a database.**

  O4 has read as a single build since it was written: `kols-store` does not exist, so build it.
  Measuring first split it cleanly, and the halves want opposite things.

  **The replay half needed no SQLite at all.** A command asked the governance log three questions
  — replayed state, the channel fold, the name fold — and *each* re-read every entry file,
  decoded it and verified its signature on insert. Three full passes with cryptography, per
  command, growing with a log that only ever grows: 152 ms for one ordinary command against three
  hundred channels. The fix is a cache invalidated by **counting entry files**, which works
  because entries are only appended and are numbered by the directory's own size — so an
  unchanged count is an unchanged log, answered without reading anything. A settled send went to
  1.8 ms at three hundred and fifty channels and the folds measure at zero.

  Two details worth keeping. `GovernanceState::apply` already existed, so a change advances the
  state along the chain instead of replaying from genesis — the protocol had the incremental path
  and the client was not using it, which is the third time this project has gone looking for
  something to build and found the layer already had it. And the cached chain is compared as a
  *prefix* rather than assumed to extend, because fork choice can move a branch out from under it
  (§2.7.1) and `apply` walks forward with no way to unapply.

  **The read half is the larger one and is what a database is actually for.** `open_channel`
  reads every record in the channel, decodes it, merges the whole set and runs the reader-side
  rate pass — so `before` and `limit` bound what is *returned* and not what is read. 222 ms at
  six thousand records, about four seconds at a hundred thousand, for a page of fifty. Records
  accumulate far faster than structure does, so this bites first at scale.

  It cannot be answered by the same trick, and that is the interesting part: a fold can be cached
  because it is a pure function of the whole log, and a *page* is a query — the records in an HLC
  range plus whatever edits, withdrawals and redactions target them. Caching the answer to
  "everything" does not make "a page of it" cheap. That is what `kols-store` is for, and it is
  what remains of O4.

  **The general shape, since this is the third entry in a row about it:** an owed item names a
  remedy, and a measurement names a cost. When they disagree the entry is usually the one that is
  wrong, because it was written when somebody had a fix in mind rather than a number in hand.

- **2026-09-08** — **O15 named the wrong obstacle, and naming it wrongly is what kept it open.**

  The entry said the remaining content-routing test needed the Docker NAT matrix. It did not. It
  needed **topology control** — the fetcher must never be handed the holder's address — and
  address translation has nothing to do with that. Docker was where topology control happened to
  live, and the entry recorded the location rather than the requirement.

  What is true, and is the half worth keeping: the *client's* daemons cannot provide it. Every
  `kols` process can dial every other, and there is no way to withhold one address from one node
  without turning off the very discovery under test. So the conclusion "not with local daemons"
  was right and "therefore Docker" did not follow — the transport's own in-process tests build
  the graph directly, and that is where the test belongs.

  **Why no existing test caught this, which is the general shape.** `provider_discovery.rs` had
  seven tests and every one connected the fetcher to the holder. With the answer already in your
  routing table, an implementation that simply asks whichever peers it has a socket to passes all
  of them — and that is not routing, it is polling your neighbours. The discriminator is a single
  line of setup: dial the middle node and nothing else. Everything else in the test is
  consequence.

  Two things asserted rather than assumed, because both are ways this could pass hollow: the
  intermediary must hold no copy of its own, or the three-node shape collapses back to the
  one-hop case wearing more nodes; and the fetch must complete, because finding a holder is worth
  nothing if the bytes cannot then be got from it. Probed by removing the announcement — the
  lookup returns nothing, which is what says the DHT is carrying the answer.

  mDNS cannot produce a false pass in either direction here, and it is worth writing down why
  rather than reasoning about it again later: it discovers *addresses* and carries no provider
  records. Knowing where a peer is is not knowing what it holds.

- **2026-09-08** — **The second bottleneck was invisible until the first was gone, which is an
  argument for measuring after every step rather than after the last one.**

  Appending a record re-chunked and re-sealed the whole segment, so an append cost the size of
  everything already in it. `AppendOnlyObject` keeps the last chunk's plaintext and re-chunks
  only that plus what arrived — 6.5 ms per append at ten thousand records became 0.12 ms, flat
  against linear.

  Except it did not, at first. With the chunking incremental the cost was **still climbing**,
  because the publish handed back a *clone* of the encoded object: copying a segment per append
  is exactly the cost that had just been removed, arriving one layer up. Nothing about the first
  fix was wrong and nothing about it was sufficient, and the only reason it was noticed is that
  the measurement was re-run rather than assumed. Shared behind an `Arc` now, mutated through
  `make_mut` so a caller holding a previous publish gets a copy at that moment.

  **The spec gained a requirement it had only implied.** Storage §1.3 says chunking is
  content-defined and deterministic, and the incremental encoder needs something narrower:
  boundaries depend only on the bytes *since the previous boundary*. A chunker carrying a rolling
  hash across boundaries would satisfy everything §1.3 said and break this — and the failure
  would be two conformant implementations addressing identical content differently, which is
  content addressing quietly meaning two things. So it is stated, along with the consequence
  (an append cannot move any boundary but the last) and the obligation (an incremental encoding
  must be byte-identical to a whole one).

  **A probe found a hole in the tests rather than in the code, which is the more useful kind.**
  The mistake the code guards against is asking the small-content exemption about the *region*
  being re-chunked rather than about the *object* — and breaking it deliberately passed the
  entire suite. FastCDC normalises with a stricter mask below the target, so sub-target cuts are
  uncommon and thousands of appends over varied data never hit one. A bounded deterministic
  search found a 360-byte region that cuts; the test constructs the case now, and **fails if the
  search finds nothing**, because a test that quietly found no case would be worse than none.

- **2026-09-08** — **Flat, and the two things it turned up are worth more than the number.**

  A send at a thousand of the author's own records went from 298 ms to 1.8 ms, and stays at
  2.1 ms at six thousand. What made it flat was not a cleverer rebuild but noticing the executor
  had no reason to build anything: it needs three answers — the newest reading, the newest
  `Message` reading, and how many of a class fall in the trailing minute — and was rebuilding and
  encoding an entire author log to get them, then reporting a byte count that nothing consumed
  and that was measured against a segment which did not exist, because `rebuild_log` never
  sealed. Three questions, a small per-channel index, no publish.

  **The first thing it turned up: one line of the removed rebuild was carrying a guarantee.**
  Deriving the log's DEK requires an epoch key, so a member holding none could not write — and a
  test said so in as many words: *posting without an epoch key must fail*, because a node that
  minted a fresh key instead would write content no other member can read and read nothing it
  wrote before. That check had been an accident of where it sat. It is explicit now and costs
  O(1); it was being paid per record only because it lived inside a rebuild.

  **The second: two markers that look alike and are not.** The background pass now skips a
  channel with nothing new, which needs a record of what was published and a record of when it
  was announced. Persisting both is the obvious thing and it is wrong: chunks survive a restart
  because they are on this disk, and provider records do not, because they live in a DHT and in a
  swarm the restart replaced. A node trusting a persisted "announced an hour ago" comes back
  holding content nobody can find, and looks entirely healthy doing it.

  `three_nodes` caught it within a minute of the change — a restarted keeper waited twenty
  seconds for a publish it had decided to skip. Worth recording as a shape rather than an
  incident: **durable and live state can be the same shape and the same size and still not be
  the same kind of thing**, and the question to ask of any marker is not "is this expensive to
  recompute" but "is what it describes still true after the process ends".

  Also fixed on the way past, because it stopped being reachable: `next_hlc` read the last record
  of the *open* segment, so on a freshly sealed one it answered from nothing — and `push` checks
  monotonicity only within the segment it pushes to, so a reading going backwards across a seal
  boundary would have been caught by nothing. It takes the index's newest reading now, which
  spans the channel.

- **2026-09-08** — **The expensive thing was not the one the entry named, and asking "is that
  really necessary" was what found it.**

  O5 has said for weeks that "the executor rebuilds an author's whole log to append one record",
  and the obvious reading — re-reading every record is wasteful — is true and is not where the
  time went. **Every `append` republished.** Appending a record re-chunked, re-encrypted and
  re-hashed the entire segment so far, so replaying `n` records to rebuild a log did `n²/2`
  records' worth of cryptography, and `rebuild_log` **discarded every intermediate result** —
  the loop ignores the return value. A thousand encodes to reach a state one encode produces.

  That is why it was superlinear, and why the entry's framing pointed slightly wide of it: the
  re-reading is linear and ordinary, and the re-publishing was quadratic and invisible, because
  from the outside both look like "it replays the log".

  **The same shape was in `serve`, and worse.** `publish_own_logs` rebuilds every channel's chain
  from stored records and publishes as it goes, unconditionally, on every sync tick. Its comment
  explains that republishing is cheap because unchanged chunks re-derive to the same CID and it
  costs a re-announcement rather than a re-upload — which is true about *bytes on the wire* and
  says nothing about the CPU that derives them. Quadratic work in the background is still
  quadratic work; nobody waiting on it only means nobody notices until the fan does.

  **The fix is a split rather than a cache**, which matters because a cache would have needed an
  invalidation rule and this needed none. `push` appends without publishing; `publish_current`
  encodes once however many records were pushed. The pointer version still advances once per
  record — a version is only reachable through the one before it, and that rule is load-bearing —
  but signing a small record `n` times is a different order of work from encrypting a growing
  segment `n` times, and only the last of those pointers was ever announced anyway.

  298 ms to 71 ms at a thousand records, and the curve straightens: before, 1.74× the records
  cost 2.56× the work; now it costs 1.74×. Every existing test passed unchanged, which is the
  claim that mattered — the frozen encoding vectors, the delta-fetch assertion that append moves
  a tail rather than a segment, and the two- and three-node wire tests that converge history
  across processes.

  **What is left is linear and still real:** every record in the channel is read and decoded to
  find this member's, and the pointer version is signed once per record. Those are what a cache
  or the projection removes. Worth being exact about the difference, because it is the whole
  reason this was worth doing first: linear is a constant to argue about, and superlinear is a
  date at which the thing stops working.

- **2026-09-08** — **The first measurement was forty times wrong, and the direction it was
  wrong in is the one that gets things rebuilt for no reason.**

  O5 has been carried since it was written with an instruction attached: measure before
  optimising. The first run was an ordinary `cargo test`, so a debug build, and it said a send
  in a channel with 200 of your own records takes **876 ms** and one command with 50 channels
  takes **4.3 seconds**. Those are numbers you act on immediately.

  In release the same code is 21.6 ms and 24 ms. Almost the whole path is signing, hashing and
  encrypting, and `rustc -O0` does not optimise any of it — so the debug run answered a question
  about the compiler. **A measurement taken on the wrong build is not a rough measurement, it is
  a different one**, and it pointed at an emergency that is not there.

  **What the honest numbers say is still worth acting on, but differently.** A send is
  superlinear in the author's own record count — forty times the records for about a hundred and
  twenty times the work — reaching a third of a second at a thousand records and not stopping.
  Each send re-reads every record this member ever wrote in that channel from its own file,
  decodes it, sorts the lot, and replays it into a segment that re-chunks as it grows. **Sealing
  does not bound this**, which was the thing worth finding: the rebuild walks `own_records`
  rather than the open segment, so a sealed history is walked again on every message forever.
  Retention does not bound it either — that drops what is published, not what this store keeps.

  **And it split an item that had been carried as one.** `WORKING.md` said the projection fixes
  both halves of O5 "which is why they are one piece of work". True, and only one half is urgent:
  replay grows with *structure* rather than traffic, so at the design target of hundreds of
  channels it costs a fraction of what the send path costs at a thousand messages. Two items
  bundled by a shared remedy rather than by a shared size, which is how a small fix ends up
  waiting behind a large one.

  The harness stays as `tests/cost.rs`, `#[ignore]`d. `design/05` §5 asked for a number that
  would show whether the projection delivered; a measurement that exists only in the session that
  produced it cannot do that.

- **2026-09-08** — **Two green tests that proved nothing, in one sitting, and they were the
  same mistake wearing different clothes: a check written against my own intent rather than
  against the thing.**

  O2 wanted a conversation-profile network whose node runs without discovery. Both halves went
  in, and then both probes passed when they should have failed.

  **The first.** I added a guard in the executor refusing channel structure in a conversation.
  Probed it by deleting the guard: still green. The refusal was coming from `kols-api`'s gate —
  `Refusal::NotAServer`, at all four channel and category sites, with a doc comment giving the
  exact reasoning I had just written a second copy of one layer down. The rule had been there
  the whole time and was simply *unreachable*, because nothing could create a conversation to
  refuse it in. The guard was deleted. This is the third time this project has gone looking for
  something to build and found the layer already had it.

  **The second, which is subtler and worth more.** `serve` reported *discovery off* by testing
  its own local variable. So swapping `MemberNode::with_discovery` back to `MemberNode::new`
  left the node built wrong and the report still saying it was right — the test agreeing with
  the code's *intention* while the code did the opposite. A stored field would have lied the
  same way. The fix is to make the observable a fact about the object: `MemberNode::discovery()`
  upstream, read off the behaviour set that actually exists, so the client asks the node instead
  of restating what it asked for. With that, the probe fails as it should.

  **The general form, since it caught me twice in an hour.** A test on a *decision* is worth
  little; a test on the decision's *effect* is worth what you wanted. The distance between them
  is exactly where a passing suite stops meaning anything — and it does not announce itself,
  because in both cases the assertion I wrote was a true statement about something.

  **What it cost to find: two probes and about ten minutes.** What it would have cost otherwise
  is a conversation network quietly sitting in a shared relay's routing table, which is D29
  reached with nobody designating anything, and which produces no symptom at all — a node
  without discovery listens, dials, relays, hole-punches, gossips and serves exactly like one
  with it.

  **Also worth recording: the blocker was not what the entry said.** O2 read as two client gaps,
  "no profile written" and "`Discovery::` nowhere". The first is not a gap — `kols init` writing
  no profile is correct, since absent means `server` and writing today's default would freeze a
  network at it. What was missing was that nothing could create a conversation at all, so there
  was no network for the second half to apply to. The item's own framing pointed one layer above
  the thing.

- **2026-09-08** — **A decision recorded and not built reads exactly like a decision nobody
  took.**

  O11 — a relay shared between two of a member's networks — was settled on 2026-09-07 as Q2:
  warn at designation, never refuse. Wave 1 was docs-only, so the reasoning went into
  `design/09` §3 and the notice was never written. The entry then sat in the owed table for a
  day saying "nothing enforces it", which is true and is not what was owed: enforcement was
  ruled out deliberately, and what was owed was the sentence a member reads.

  **Worth naming as a shape rather than as one item.** A wave that records decisions and a wave
  that implements them is a good split, and it leaves a window where the register describes work
  as pending that has actually been *designed and not made* — indistinguishable, from the table,
  from work nobody has thought about. The other Wave 1 items closed cleanly because they were
  accepted limits with nothing to build. This one had a client half and looked the same.

  **The check is by peer id, and a string comparison is the bug it is written against.** One
  relay answers at several addresses — a DNS name and a bare IPv4, TCP beside QUIC — so comparing
  the strings reports *no overlap* in precisely the case D29 exists for, silently. Proved rather
  than argued: degrading `relay_host` to return the whole address leaves two of the three new
  tests passing and fails only `a_relay_another_network_uses_is_reported_however_it_is_addressed`.
  That is the version somebody writes in five minutes and it passes a naive test suite.

  **The second designation was nearly missed.** The relay panel is where the decision was written
  down, and creating a network with a relay is the other half — the *first* relay most people
  designate, and the one a warning scoped to the panel would never have covered.

  **And one thing deliberately not built: no warning on join.** A joiner adopts whatever relay
  the invite carried, so the overlap is just as real and the member chose nothing. Warning there
  is a question about a different act, and answering it here would have been scope nobody asked
  for. Written into `design/09` §3 as a stated non-goal rather than left as a gap somebody
  rediscovers.

- **2026-09-08** — **O25 was a requirement for unbuilt work, not a defect in built work.**

  Filed the day before as "a node never holds content it cannot decrypt", with a trade-off to
  settle: forward secrecy against durability. The user settled it — keep the current behaviour —
  and checking what to build then showed there was nothing to build **yet**, for a reason worth
  recording.

  **Roster keying does not exist.** `channel_dek` derives from the network epoch regardless of a
  channel's privacy flag, and nothing in the key path branches on it; `ChannelMembership` and
  `ChannelRotation` are E2, unbuilt. So today every member can decrypt every channel and there is
  no content any node is excluded from. O25 costs exactly nothing, and a fix would have been code
  written against a feature that is not there — the mistake this log already records three times
  from the storage work.

  **The real finding is sharper than the item was**, and it is about placement rather than about
  holding ciphertext. Replica placement ranks over the capability ledger — every node that offered
  storage — which is right precisely because everyone can read everything. The moment a channel is
  roster-keyed that stops being true: the ranking would assign its segments to nodes that cannot
  fetch them, so the effective replica set becomes **roster ∩ top-k**, which can be *empty* in a
  large network. Worse quietly: the roster members who can hold it are ranked out, so they keep it
  as ordinary cache — sheddable, with none of the census, two-holders rule or grace window that
  duty carries. A private channel could lose its last copy to disk pressure on a machine that
  could read it.

  So the outcome is three obligations written into `03` §3.5 and against E2, before the work
  rather than after it: rank over the roster, take duty for what nobody else can hold, and report
  under-replication against the roster size rather than the network's replication factor — or a
  three-person channel reports permanent degradation that no amount of volunteering can fix.

  O25 moves to the accepted-limits table beside O16 and O19. An accepted limit left in the owed
  register reads as a fix nobody got round to, and this one is a decision.

- **2026-09-08** — **Presence, and the word that had to be absent rather than unused.**

  O6 closed. `01` §9 had specified the mechanism for a long time and neither front end
  implemented it, so §4's third question — who is here — had no answer and the window showed a
  narrower honest thing instead: who this node holds a connection to. That ordering was correct
  and is worth recording as such, because the tempting shortcut was always available: draw a dot
  from the connection list and call it presence. It would have been reachability wearing
  presence's clothes, and it gets *more* wrong as a network grows.

  **The design turns on there being two facts, not one.** A ring is this node's own observation;
  a word beside a name is what that member said. Presence travels by gossip, so somebody can be
  *here* without this node holding a connection to them — which is exactly why the two marks
  cannot be merged, and why the roster carries a sentence about each.

  **No word at all is the case that took the care.** It covers a beat gone stale, a member who
  chose invisible, and a member never heard from; nothing distinguishes them, so the row says
  nothing. There is no `offline` anywhere — not the interface, not the boundary event, not the
  wire format. A word that cannot be justified is better absent than unused, because an unused
  one gets reached for. `drive.mjs` asserts the roster never renders it, and the guard was
  mutation-checked by making the absent case render "offline": two checks fired.

  **Invisible publishes nothing rather than publishing "invisible"**, which is the whole setting.
  A beat saying so tells every member this node is running and hiding. It is two types in the
  implementation rather than one enum used twice, so the mistake cannot be made by accident, and
  the choice is persisted even though the beats are not — a member who chose to hide must not be
  back on the roster after a restart. The two-node test proves the silence: Alice is invisible
  *first*, a record crosses live between the same two nodes to show the mesh works, and only then
  is the absence of a beat meaningful. Mutation-checked by ignoring the setting; it failed.

  **Two things the spec had left and the code had to settle.** `01` §9 said the states were
  `online | idle | dnd | invisible`; `online` is the word §4.1 forbids for the absence case and
  is misleading for the present one too, since there is no server to be on a line to. And a beat
  is **signed** as well as sealed: the seal keeps presence inside the epoch but says nothing
  about who sent what, and gossip lets any subscriber republish — so without it one member could
  announce another as present, or keep announcing them after they had gone. The signature covers
  the time as well, and freshness is judged by when a beat was *heard*, because a signed
  timestamp from a sender's clock would otherwise pin somebody present forever.

- **2026-09-08** — **An offer buys ranking, not a crawl (D38) — and checking that closed the
  question rather than answering it.**

  The open half of the entry below was whether a node should *fetch* to repair. The user's read
  was that it should not: a node offering fifty gigabytes "should just be ranked accordingly, and
  then it is used as a fallback if no other nodes can store something". Recorded as D38, and it is
  the right way round — the alternative turns a setting a member chose into a download budget they
  did not.

  **What made it a clean close rather than a deferral is that the code already behaves that way,
  and reading it showed the gap I had described does not exist.** A node under its ceiling walks
  every chain it can read to the start, because `history_budget` is unbounded while there is room.
  So it already holds every sealed segment it could be ranked for or be the standby for, and duty
  and repair are complete over the network's readable content. A node *at* its ceiling holds less
  and has no room to repair with either — so the case where knowing more would have helped is the
  case where it could not have acted. No fetch path is owed. I had written the residual up as a
  design decision waiting to be made; it was a decision waiting to be *checked*.

  **The check found a real one instead, and a different one: O25.** `resolve` returns nothing when
  no epoch key unwraps a segment's DEK, and the walk then skips the hop entirely — so a private
  channel a member is not keyed for is never fetched, linked or held. A storage offer therefore
  funds durability only for channels its owner can read, which is not what the offer means: Core
  §4.2 defines `storage_offered` over *replicated* content, and the user had said plainly they were
  fine with a node holding bytes it cannot read. The pointer is public and carries the object's
  CID, so the ciphertext is fetchable without the key. Nothing does it.

  Filed rather than built, because it is a fetch path and a duty tier over objects with no records,
  and this session's lesson is that the layer matters more than the speed.

- **2026-09-08** — **Repair is placement read further down, and that is the whole design.**

  The storage arc's last piece: a node now takes on content the network is short of, not only
  content the ranking assigned it. The gap it closes was worth stating plainly — a member
  offering fifty gigabytes as a backstop caught falling content **only where placement had
  already ranked them**, so generosity bought a proportionally larger share of the ordinary load
  and nothing at all of the failure case. Durability by coincidence, one layer up from the
  coincidence duty was built to remove.

  The temptation was a second mechanism: notice a shortfall, volunteer. Storage §3.4 rules that
  out in a sentence that is easy to read past — repair re-places onto *the next nodes in the same
  deterministic HRW ranking*. So there is no new policy here at all, only the same ranked list
  read past the replication factor, which is what keeps repair as independently recomputable as
  placement. A node at `factor + k` steps in when the object is `k + 1` or more copies short.
  **Depth follows the size of the hole**, so one missing copy wakes one standby rather than every
  node that happened to notice.

  **The bug that rule prevents was already written before it was reasoned about.** The first
  version released repair duty on the next tick, every time, because a standby is by definition
  outside the replica set and the ordinary release rule is *the ledger stopped naming me, so
  somebody else has it* — which is exactly the premise repair exists to deny. Repair duty is
  marked as such and ends only on a **fresh** count saying the hole closed; a stale count and no
  count both keep it, which is the same one-directional-evidence rule the eviction path already
  turns on.

  **Two smaller things fell out.** The census only ever asked about duty, which answers *is what
  I promised safe to give back*; repair asks the opposite question about objects this node has no
  duty for, so it had to widen. And `shed_cache`'s justification — "the nodes placement ranked
  for it still hold it" — is precisely the assumption repair exists because it can be false. The
  doc now says so, and the resolution is ordering rather than a new rule: repair runs after
  shedding and takes only what fits, so a node with room keeps the copy under all the protection
  duty gets, and a node at its ceiling gives it up like everything else. The cap does not bend
  for this any more than for anything else.

  **What was deliberately not built**, and why it is in `STATUS.md` as a decision rather than a
  gap: fetching to repair. A node that never held a segment cannot learn it is short, since this
  client has no catalogue of the network beyond its own chain links — but one that *shed*
  something still has the link and could go and get it back. That would turn "I offer 50 GB" into
  "download until 50 GB is full", which is a different promise from the one the settings screen
  makes. Worth doing on purpose or not at all.

- **2026-09-08** — **A storage offer became a cap, and the thing it was capping did not exist.**

  Two days of clearing the owed register turned into the storage work, by way of a finding that
  inverted the plan. `SetContribution` landed as one number, then as four; the cap was asked for
  next; and checking before building showed there was **nothing to cap**. `placement::rank` was
  called nowhere and `replication_factor` read nowhere, so every byte on disk was what this node
  had fetched to read. Content outlived its author because somebody happened to have opened that
  channel — durability was a happy accident, and a ceiling over it would have bounded the
  member's own working set, which is the one thing contribution is not.

  So the order became duty, then tiering, then the cap. Recorded because the *check* was the
  whole value: the plan was coherent, the specs supported it, and it was aimed at the wrong
  layer.

  **The same shape happened twice more.** "Fetch what you are ranked for" was proposed as the
  next step and is a **no-op** — the walk queued every hop of every chain with nothing consulting
  a ceiling, so the node already held everything it could be ranked for, and the disk was
  ballooning through the cache path that no ceiling touched. And O24 was filed as a missing fetch
  on a paging path when there is **no paging path**: the window opens channels with `before:
  None`. Each time the mistake was the same one — reasoning about what the code should be doing
  instead of reading what it does.

  **Three things were wrong in ways that only measuring found.** `stored_bytes` counted `chunks/`
  alone, and this store keeps every message twice — once as a record, which is what rendering
  reads, and once inside a segment's chunks — so a member setting two gigabytes could have been
  handed four. A node that sheds while still advertising sends every peer that believes it on a
  fetch that fails, which counts against the *serving* node. And a `Degraded` message shipped
  claiming that scrolling back would reach evicted history over the network, which was false in
  the same session it was written: `OpenChannel` renders from stored records and the executor
  holds no node at all.

  **The rule the whole thing turns on** is that a provider count is safe in one direction only.
  It may overstate — records outlive the bytes they name — so *nobody answered* and *nobody holds
  it* are opposite states, and a design that collapses them drops a last copy because the network
  was slow to reply. That is why the bar is two other holders rather than one, why a stale answer
  reads as unknown rather than as its last value, and why the seven-day window does not override
  freshness: a test found that a last copy past its deadline was held anyway, because the count
  behind it had aged out. That was better than what had been designed, and it stayed.

  **What a member is told turned out to be most of the design.** Three situations reach the same
  place — history has stopped arriving, content is about to be lost and only this machine holds
  it, content already has been — and one message for all three teaches somebody to ignore the two
  that matter. Contributing nothing is an ordinary configuration and the interface says so in a
  sentence, because a phone, a metered link and a full disk are all reasons and none of them
  makes anybody a lesser member.

  Also closed on the way, each with its own surprise: O9, where the fix nearly deleted the
  successor's claim on the way out; O21, where a lost join answer was reported as a failure and
  the retry spent a single-use invite; O3, where two tests cited `Hash::ZERO` as a governance
  head and so were testing what a reader does with an unverifiable claim rather than testing
  redaction at all.

---

- **2026-09-01** — **Leaving a network works, and building the client half found a rule that was exactly backwards.**

  The protocol gained the entry this morning (Core §2.5.1). The client can now write it:
  `Command::LeaveNetwork`, one membership removal per group the member is in, gated on nothing
  but current membership — the only command at this boundary that needs no capability, which the
  consent classification had no rule for and took the stricter reading of.

  **`forget` used to refuse the open network, and that was the one case that could work.**
  The reasoning behind the refusal was sound and remains true: deleting a store under a running
  node loses live MLS state with nothing reporting it. But a running node is the *only* thing
  that can publish anything, so requiring the network be closed first guaranteed no departure
  could ever be announced. The requirement was correct right up until there was something to
  announce, and then it was precisely inverted. The open network is now the good path — submit,
  give the node a tick to adopt and gossip, stop the node, delete — and a closed one still
  forgets and says plainly that nobody was told.

  **The guard that had to move was a true statement about one member applied to everybody.**
  The executor refused any self-removal from `everyone` on the grounds that it would leave the
  network unable to rotate its key. That is true of the *last* `revoke-node` holder and false of
  every other member, and left alone it would have defeated the protocol change one layer up.
  It now asks the question it meant — and asks it of the whole network rather than of one group,
  because the capability is held through whichever group grants it, so stepping out of a role
  that carries it strands the network exactly as leaving `everyone` would.

  **The limit that leaves, which is worth stating rather than discovering.** This client grants
  only the chat vocabulary and `revoke-node` is not in it — the capability comes from `Founders`,
  which holds everything. So the sole founder of a network cannot leave it, and the refusal says
  what to do: add somebody else to `Founders` first. That is the network's own constraint rather
  than a missing feature.

  **What the interface reports is who could have heard, never who did.** Gossip acknowledges
  nothing, so the honest figure is how many members this node held a connection to when the
  departure went out. Zero is reported as *not told* rather than as success, and pointedly:
  it is unrecoverable, because the seed that signed the entry is gone and it cannot be sent
  again.

  **One test was not written, and the reason is a budget rather than an oversight.** The
  untested composition is "another member's node accepts a departure over the wire", which would
  be a twelfth daemon test in `two_nodes.rs` — and O20 is already the suite's worst behaviour,
  made worse by exactly that. Both halves are covered separately: acceptance without a
  capability by six conformance tests in `intranet-governance`, and the wire path by the
  revocation test that carries an ordinary `MembershipChange` end to end. Recorded so the gap is
  a decision somebody can revisit rather than a thing nobody noticed.


- **2026-09-01** — **E16 landed, and it turned out to be two rules rather than one.**

  The rule as specified: a `MembershipChange` removing the entry's own author is valid without a
  capability. Core §2.5.1 now says it, `intranet-governance` enforces it, and six conformance
  tests cover it. The protocol had no concept of leaving at all before this — §2.4 covers
  admission and §2.5 covers revocation, and both are things done *to* a member by a capability
  holder, so the one member who knew they were leaving was the one who could not record it.

  **What implementing it found was a second rule, in a document that already generalised to
  it.** §2.7.1 measures fork-choice branch length in *capability-gated* actions specifically, so
  that entries free to mint cannot grind a branch to victory — a correction made after device
  certificates turned out to be exactly such an entry. It generalises past them in as many
  words: "any other future entry type that similarly requires no capability to produce". A
  self-removal is now such a type, and the cheapest one yet to grind: under `admission: auto` a
  multi-use invite mints identities freely, and each could then leave every group it was in,
  turning free entries into branch weight. Excluding it is not a refinement of E16 — it is the
  half of E16 that keeps the first half from being an attack.

  **The design document had not seen it, and the code's own comment had.** `design/06` §16 asked
  only for the capability exemption. The comment above `is_capability_gated` already named
  `RotationReason::SelfInitiated` as a member of the excluded class and said why, which is what
  made the question obvious the moment the exemption was written. A comment that explains a rule
  rather than restating it is what makes the next instance of that rule findable.

  **It moved a function's question from the body to the entry.** `EntryBody::is_capability_gated`
  is a pure function of the body and cannot see the author, so it cannot tell a departure from an
  ejection — they differ only in who signed. `LogEntry::is_capability_gated` can, and now answers
  there. The body still reports `MembershipChange` as gated, which stays right for the general
  case; the entry subtracts the self-directed one.

  **Both halves were verified by breaking them.** Disabling the authorization change turns four
  tests red; disabling the fork-choice exclusion turns exactly one red. An assertion nobody has
  watched fail is a comment.

  **The harness got the second half as a runnable check.** `governance grinding-check` already
  padded a losing branch with device certificates and asserted it still lost; it now takes
  `--with departures` and pads with self-removals instead. With the exclusion in place the honest
  branch wins; with it removed, twenty departures beat two genuine governance actions and the
  command says so. Spec 06 §3 is widened to ask for the whole class rather than the one member of
  it that was found first, which is what it should have said the second time this happened.

  **And the O20 flake picked this change to reappear on.** The full workspace failed
  `a_node_offline_across_a_rotation_catches_up_and_can_still_read` on the first run after a
  governance change — which reads exactly like the protocol change breaking a live-wire test.
  It is not: the named test passes alone in 77 s, the suite with the change reverted passes 306,
  the suite with it applied passes 306 on the next run, and the change is inert for this client,
  which never writes the entry it affects. Ten minutes of comparison, and the reason to spend
  them is that reading the diff had already produced the right answer with no way to tell it
  apart from a comfortable one. It also corrects `CONTRIBUTING.md`: three clean runs and one
  failure make this intermittent, not the every-run failure that paragraph claimed.

  E15 was not done, deliberately: `design/06` §17 sequences it to land beside the credentials
  work it describes, because that is where the client has to say what a per-network seed means to
  a user, and amending the spec while writing that is cheaper than amending it twice. Everything
  else the protocol still owes — E7, E10, E13, E6, E8 — belongs to P2 and later.


- **2026-09-01** — **`v0.11.1` is cut, and the interesting number is forty.**

  That is how many commits `main` had taken since `v0.11.0` on 2026-08-23 — eight days, and
  every fix that made the client work: the Tauri ACL that had refused every node event for the
  life of the application, the native drag handler that ate every drag, a shutdown path that did
  not exist, a re-dial loop that counted the relay as a connection, and a founder's network name
  that never reached the log. None of it was in a published release, while `README.md` and
  `docs/two-machine-test.md` both send a new user to Releases.

  **A distributable that trails `main` is indistinguishable, from the far end, from work that
  was never done.** Field testing here has been on CI dispatch artifacts, which is the right way
  to get a build into a tester's hands quickly and is invisible to anybody else — so the gap
  went unnoticed for exactly as long as the only people running it were being handed builds by
  hand. The audit found it by comparing the newest tag against the newest commit, which is a
  check worth making whenever `STATUS.md` gains a row saying **done**.

  **Both version fields moved**, `Cargo.toml` and `crates/kols-app/tauri.conf.json`, which is
  the lesson `v0.11.0` already paid for: the second names the installer the runbook tells people
  to download, and bumping only the first publishes a release containing an installer labelled
  with the previous version.

  Cut as a patch rather than a minor deliberately — nothing in it is new, and every one of the
  five items above is a thing that was always meant to work and did not.


- **2026-08-31** — **A full audit across the three repositories, and the drift was all in one direction.**

  Nothing was found wrong with the code. Every gate is green — 306 tests here, 666 in
  `distributed-intranet`, clippy clean in both, `drive.mjs`'s 44 checks green — every internal
  document link resolves, every `§` cross-reference resolves to a heading that exists, and there
  is not a `TODO`, a `dbg!` or a `console.log` in the tree. What the audit found instead was six
  places where a document had been overtaken by the work it describes, and they have a shape.

  **A document goes stale in the direction the work moved, and the repository that owns a
  subject is not the one that notices.** `design/06` §16 specifies E16 in full — the rule, why it
  is safe, what it must settle about epoch rotation, an acceptance criterion naming
  `intranet-governance`. All of that sat in the *client's* ledger. `specs/07` §7, which is the
  list the protocol repository actually works from, had nine amendments and no E16; the protocol
  README said "nine amendments, six landed"; its `CLAUDE.md` said the same. So the one place a
  reader would look to find out what the platform still owes was the one place the debt was not
  recorded. This project has a rule for exactly this — *a needed protocol change is recorded in
  `design/06` rather than assumed into existence* — and it turns out to have been running in one
  direction only. E16 is now a row in §7 with the rule stated normatively, and both summaries
  count ten.

  **The worst of the six was not in a document at all.** The picker's own help text told a
  founder that a relay is set under **relay** *in the sidebar*. It has not been in the sidebar
  since settings became a screen: it is under settings → network. A stale sentence in a design
  document costs a reader a minute; a stale sentence in the interface is a wrong instruction
  given to the person least able to check it, at the one step of setup that has an order they
  cannot guess. Worth carrying: moving a control means grepping the interface for the old name,
  not only the docs.

  **`docs/two-machine-test.md` still opened by saying the test had never been run.** It passed
  eight days ago, on three machines across three networks. The document carried its own
  instruction for that moment — *delete it or fold what survived into the README* — and neither
  happened, so it spent a week disagreeing with `STATUS.md` about the project's headline claim
  while also directing people to a relay panel and an invite button that had both moved. It is a
  runbook now, with the navigation corrected. The delete-or-fold instruction is not being obeyed
  literally: most of it is relay deployment and failure modes, which belong on a project's front
  page even less than they belong here.

  **Two numbers, in opposite directions, and the interesting one is whose.** `README.md` here
  claimed 189 tests against 306. The protocol repository claimed 656 against 666 — while
  `STATUS.md` in *this* repository had 666 exactly right. A count is easiest to get wrong about
  yourself, because nobody re-derives a number they already believe.

  **And the distributable trails `main` by everything that made it work.** The newest tag is
  `v0.11.0` from 2026-08-23. The event path, drag, shutdown, re-dial after sleep and the
  network's name all landed on the 30th, and two documents send a new user to Releases — so
  following the README today gets a build in which none of it works. Recorded in `STATUS.md` §1
  rather than fixed, because cutting a release is a decision rather than a repair.


- **2026-08-30** — **The first drag that actually ran found that two of its four destinations had no target.**

  It works, and the notes back were "the drop zones are too small" and "once every channel was
  in a folder it was impossible to drag them out". Neither is about dragging.

  **A drag needs a target for every destination, not for every thing.** The destinations in a
  sidebar are: before a row, after a row, inside a folder, and out at the top level. Rows meant
  *before this one* and folders meant *inside this one*, so two of the four were reachable. A
  list of five channels had five places to drop and six places to want one, and the only route
  to the end of a list was a folder that happened to sit below it.

  Every row is two targets now, split at the midpoint — above or below the thing being pointed
  at, which is the question somebody is actually asking. And the sidebar's own empty space is
  the top level, which is the area a person aims at when they mean "out here" anyway. Without
  it a sidebar could be arranged into a state nothing could be dragged out of, which is the
  report.

  Three things found while doing it, none of them reported:

  - **Dropping a channel on itself moved it to the top.** The channel being moved is filtered
    out of its own siblings, so "before itself" matched nothing — and `Math.max(0, findIndex)`
    turned that nothing into index zero. Caught by the same guard that now ignores the drop.
  - **`Math.max(0, …)` was hiding a -1 in general**, so any `before` naming a channel that was
    not a sibling meant the top of the list rather than the end. The wrong end, silently, for
    every caller.
  - **The grab cursor had stopped appearing.** `draggable` moved from the button to the row
    while chasing the Tauri default; the CSS selector did not follow. Nothing depended on it,
    which is why nothing noticed.

  And a two-pixel margin between folders belonged to nothing, so a drag crossing it fell through
  to the sidebar and read as "out to the top level" — in the middle of a list of folders, which
  is never what was meant. Padding now, so the space belongs to the folder above it.

- **2026-08-30** — **Drag-and-drop was swallowed by a Tauri default, which is the second time that sentence has been written this week.**

  The tester asked to actually solve it rather than remove it. Right, and the previous entry's
  diagnosis was wrong in the way that matters: it correctly established that the front end runs
  the whole path when the events are dispatched by hand, and then guessed that a `<button>` was
  refusing to start a drag. It was not the button.

  **Tauri installs a native drag-and-drop handler on the webview, `dragDropEnabled`, and it
  defaults to on.** It is how a window receives files dropped from the desktop, and it takes the
  drag before the page ever sees it. Tauri's own doc comment on the field says it plainly —
  *disabling it is required to use HTML5 drag and drop on the frontend* — and it is one line
  from a configuration file nobody had reason to open.

  So for as long as folders have existed there was no way to reorder a channel, because drag was
  the only route and the route was closed a layer below the interface. Nothing in the front end
  was wrong at any point.

  **This is the same bug as the ACL, and that is the finding.** Both were features removed by a
  Tauri default. Both were invisible from inside the application — no error, no console message,
  the code simply never runs. Both were found only by asking the shell's real configuration a
  direct question. So `permissions.rs` now asserts `dragDropEnabled` beside the command list, and
  the check was verified by flipping the flag back and watching it fail. The general rule in `05`
  §1: a shell setting this application depends on gets asserted against the real configuration,
  whether or not anybody wrote it down — because what nobody wrote down is exactly what defaults
  out from under you.

  **The trade this makes is real and currently free.** With the native handler off, the window
  cannot receive a file dropped from the desktop. There are no attachments — `kols-media` does
  not exist — so nothing is lost, and it is a decision to revisit rather than a corner cut.

  The menu keeps move up and move down. The tester's instinct was to remove them once the drag
  worked, and the reason to keep them is not redundancy: a drag is a gesture some people cannot
  make, and it is the one route here that no test can reach.

- **2026-08-30** — **Drag to reorder never worked, and the JavaScript was never the problem.**

  Reported after the channel and folder pass, with the tester's own read attached: *I actually
  believe it was never working.* They are right, and it had shipped that way since folders
  existed.

  **Diagnosed by driving it rather than by reading it.** Dispatching `dragstart`, `dragover` and
  `drop` at the real sidebar under `jsdom` runs the entire path: `state.dragging` is set,
  `dragover` is `preventDefault`ed so the drop is allowed, and `drop` reaches `move_channel` with
  the right arguments. The wiring is correct. What never happens in the shipped application is
  the webview *starting* a drag.

  The likely reason is that `draggable` was on the `<button>`. A button is a form control and a
  webview resolves a press on one before anything else gets to decide it was a drag. It hangs
  off the row now — which is an attempt, not a fix, because whether a webview begins a drag is
  another thing this container cannot answer, and guessing at exactly that kind of question cost
  three rounds on the sidebar last week.

  **So the real fix is that reordering no longer depends on it.** The channel menu gains *move
  up* and *move down* — which folders have had since they existed, and channels never got
  precisely because the drag was there and looked like the answer. Offered only where there is
  somewhere to go, so no entry does nothing.

  The rule worth keeping: a capability whose only route is one nobody can verify has no route.
  Where the unverifiable one is a convenience, the plain one belongs beside it rather than after
  it.

  Two smaller things found in passing. A folder's own `dragover` fired after a row inside it had
  already marked itself, so the drop marker showed the folder when the drop would have gone to
  the row — the handler stops propagation now, like the drop handler already did. And if the
  drag still does not start with the attribute moved, it comes out: an invisible control that
  does nothing is worth less than the code it takes.

- **2026-08-30** — **Asked to check the shutdown path, found there was none, and the danger was not the one being looked for.**

  The question was whether the application closes cleanly. It has no shutdown handling at all —
  the window closes, the process ends, the node task is killed wherever it was. The expected
  answer was about the node claim: released on `Drop`, and otherwise expiring on a six-second
  timer, so a process that just ends makes the next launch sit waiting for a claim nobody holds.
  Real, and the smaller half.

  **Every durable write went straight at its destination.** `fs::write` truncates and then
  fills, so a process ending between those two steps leaves a file that is neither the old
  contents nor the new — and "a process ending" here is the user closing the window. For most
  of what the store keeps that is an empty list the next tick rewrites. For `entries/` it is a
  governance log that no longer decodes, and `Store::log` refuses the **whole log** rather than
  the one file. Correctly: a governance log with a hole in it is not a smaller governance log.
  Milliseconds wide, and what is on the other side of it is the network.

  Now a temporary and a rename. The temporary lives in the store's own `tmp/` and deliberately
  not beside its destination, because every directory this store keeps is scanned by something:
  a leak in `entries/` is decoded as an entry, one in `chunks/` is served as a chunk, and
  `append_entry` numbers the next entry by *counting* the directory, so it would hand out an
  index twice. Swept when a process takes the node claim, which is the one moment it knows
  nothing else is writing there.

  The heartbeat is atomic too, for a sharper reason than the rest: a half-written one does not
  parse, an unparseable one reads as **stale**, and a stale claim is one another process may
  take over while this one is still running. One tick wide and self-healing, and still the one
  direction of failure that file must not have.

  **Atomicity, not durability**, and the difference is worth not blurring. The bytes may sit in
  the page cache when the process ends: they survive the process dying, which is the case this
  is for, and they would not survive the machine losing power. That needs an `fsync` per record
  and is a cost to take on purpose — losing the last message to a power cut is a different order
  of problem from losing the network to a window closing.

  And the shell now stops the node on `ExitRequested`, waiting for it: aborting a task drops the
  future and dropping the future drops the claim, but an abort nobody polls has dropped nothing.
  Bounded at a second and a half, because closing a window must never be the thing that hangs,
  and what the bound gives up on is exactly what the six-second expiry already covers.

  The test asserts the property rather than trying to interrupt a write: nothing appears in a
  directory a reader scans, the store still opens, and a temporary left behind by a process that
  died at the wrong moment is gone once somebody takes the claim.

- **2026-08-30** — **The handle is removed, and the two CSS lessons it cost are kept.**

  It worked in the end — drawn icon, out of flow, correct on both platforms — and the tester
  asked for it gone: the right-click menu is enough, and there is no need for two ways to the
  same menu. Removed, along with the SVG that existed only for it and a `has-menu` class that
  reserved space for it. The folder head gets back the `right-click for folder actions` tooltip
  it had before, which is a hint rather than a second control.

  Worth writing down because the *reason* it was added has not gone away. The first field test
  came back asking for channel controls that had shipped weeks earlier, because a right-click
  is a control only for somebody who already knows. The person who asked for it removed is the
  same person who could not find the menu three weeks ago and can now — which is exactly the
  position from which a discoverability affordance looks redundant. It is still their call and
  it is made; what is recorded here is that the next person to arrive will be where the tester
  was, not where they are.

  The two sizing rules in `09` §5.1 stay, because neither was about this control: a glyph is
  sized by whatever font carries it, and a control beside shrinkable text should be out of flow
  rather than merely measured correctly. Three rounds of Windows bugs bought those, and they
  outlive the thing that taught them.

- **2026-08-30** — **The sidebar broke a third time, and the fix was to stop the row having an opinion.**

  Second report from Windows: the handle now renders *"significantly larger than the channel
  names"*, the names show as blank, and the row is worse than before the last fix. Same
  symptom, different cause, which is why the previous change did not touch it.

  The handle was a `⋯` — U+22EF, midline horizontal ellipsis — sized by `flex: none`, which
  means sized by its content, which means sized by the font. **Segoe UI has no U+22EF.**
  Windows fell back to whatever does carry it, that font's advance is far wider, and a handle
  sized by content took most of a 260px row. The previous round fixed the flex arithmetic and
  left the glyph, so the arithmetic was correct and the input to it was not.

  Two changes, and the second is the one that matters. The icon is **drawn** now — three SVG
  circles in a 20×20 box — so nothing about the row depends on what fonts are installed. And
  the handle is **out of flow**, absolutely positioned over a strip of padding the button
  reserves, so the name's width does not depend on the handle's even in principle. Correct
  arithmetic is not the same as no arithmetic, and after two rounds of adjusting how a row
  divides itself the right move was to stop dividing it.

  The same treatment for the presence caret, which was `▾` in a flex row beside a channel title
  that is allowed to shrink — same shape, smaller blast radius, no reason to leave standing.
  Swept the rest: `…` and `×` are Latin-1 and safe, and the folder button's emoji sits in a
  header with nothing shrinkable in it.

  **This is the third round on one row, and all three were spent on something the development
  container cannot render.** The driver now asserts the property rather than the appearance —
  the handle carries no text and contains an `svg` — because "does this look right on Windows"
  is not a question this machine can answer, and "does a row's width depend on a font" is.

- **2026-08-30** — **A relay is a peer, so "nothing is connected" was never true and nothing was ever re-dialled.**

  Reported as a MacBook that sleeps and does not come back without restarting the
  application. The re-dial loop in `serve.rs` was guarded on `connected.is_empty()`, and every
  `ConnectionEstablished` becomes a `Connected` — the relay's included. A node holding a
  reservation is connected to its relay for as long as it holds one, because that *is* the
  reservation. So the guard was false for the whole life of any node with a working relay, and
  the loop ran in exactly one situation: a relay going down, which takes the relay connection
  with it. That is the case it was written for and the reason it looked like it worked.

  Sleep produces the other case. The peer connection dies, `relay_watch` gets the relay back on
  its own, and from then on the node is connected to a relay and to nobody, with no path back
  except a restart — which re-dials everything unconditionally at startup.

  Now `to_redial` answers per peer: an address is worth dialling if the peer it names is not
  connected. Pulled out as a function rather than fixed in place in the `select!` so the case
  that caused it could be written down — connected to a relay, one member known and away, and
  the address must come back. The destination is the address's **last** `/p2p/`, since a
  circuit address opens with the relay's; taking the first would have re-asked the question
  that caused the bug.

  Ping is on with libp2p's defaults, so the dead connection is reaped and `Disconnected` does
  fire. That half was working, which is part of why this looked like a transport problem.

- **2026-08-30** — **A network's name existed on one machine, and the spec had said so all along.**

  Reported as a joined network showing an id in the picker and "unnamed network" over the
  channel list. `network::genesis` set the bootstrap relays, the content-type allowlist, the
  capability namespaces and the history policy, and never wrote `chat:network-name` — so the
  name a founder types at creation went to the local label and nowhere else. The creator saw
  it; everybody invited saw nothing.

  Spec 07 §1.7 has carried this key throughout, the settings sheet has written it throughout,
  and `Me` has carried `network_name` throughout. Nothing was missing except the one write at
  the moment somebody is unambiguously naming the thing. **No spec change; a client that had
  not implemented what was already normative.** Blank stays absent rather than stored empty,
  for §1.7's own reason: a network with no name declared *has* no name, whereas an empty string
  in policy is a claim that it is called nothing, replayed forever.

  The interface had a second half of the same bug. `drawMe` read `me.label` — this
  installation's local label — and fell back to "unnamed network", so even a network whose name
  *was* in policy rendered as unnamed to anybody who had joined rather than created it. It now
  prefers the network's own name, which is the one that travels.

  And the picker, which cannot afford to replay every store's log to draw a list, keeps the
  label as a **cache** of the name: `me` writes it through whenever the two disagree. That is a
  write inside a read and worth being explicit about rather than tidy — the alternative is a
  picker that lists ids.

  Networks created before this have no name in policy and will not grow one; setting it once
  under settings → network fixes them for every member.

- **2026-08-30** — **Field notes on the marks, and a sidebar that only broke on Windows.**

  Four notes back from the second test. Three were about the same thing: a first-sight mark is
  a nudge and was behaving like state. It marked your own messages — the interface telling you
  that you had not seen something you had just typed — and once up it stayed up until you
  changed channel. Now it never marks your own, it clears on hover, and it clears fifteen
  seconds after the window becomes the focused one.

  **Focus is the trigger rather than arrival**, which was the user's own framing and is the
  right one: marks earned while somebody was away are exactly the ones worth keeping until they
  are back to see them. The timer is deliberately not restarted by a redraw — a channel busy
  enough to redraw every few seconds would never settle, and that is the case where the marks
  are worth least.

  **The sidebar drew channel names as vertical columns on Windows and was fine on macOS**, and
  the split is the interesting part. `.channels li button` carries `width: 100%`; as a flex item
  with `flex-basis: auto` that becomes a basis of the whole row, so the row overflows by the
  width of the new `⋯` handle and both items shrink. How far the button *may* shrink is its
  min-content width, and `overflow-wrap: anywhere` on the name inside reduces min-content to
  the widest single glyph — which is precisely what `break-word` does not do. One letter per
  line.

  The engine decided whether it was visible, not whether it was there: how much the row
  overflows by depends on how wide `⋯` renders, which is Segoe UI against the macOS system
  font. Fixed by sizing the flex items from `0` rather than `auto`, and by cutting names with
  an ellipsis rather than wrapping them — a 260px column is not a place to wrap a name, and the
  full one is on the row's `title`.

- **2026-08-30** — **A design grew a second command nobody asked for, and the user removed it in one line.**

  Yesterday's §6.5 asked for *leave* beside *forget*: leave writes the departure and keeps the
  seed, forget does that and then destroys it. The reasoning was that they answer different
  questions, which is true. Asked directly, the answer was "I'm fine with it being all or
  nothing" — the second question is not one anybody has.

  Worth recording because of how the extra command got there. It was not requested; it fell
  out of writing down what leaving *could* mean, and once written it read like a requirement.
  A seed-preserving leave buys re-admission as the same member later, which is a real property
  and one nobody wanted, and it would have cost a second confirmation, a second explanation of
  which one destroys what, and a whole class of "I clicked the wrong one".

  **The protocol half does not follow the client half, and that is the point of keeping them
  in separate documents.** E16 still says a self-removal is valid without a capability and
  says nothing about whether the remover kept their keys — because another consumer may want a
  leave that can be undone, and a spec that assumed the seed goes with the departure would have
  written one client's product decision into the wire.

  **Announcing still matters even though nobody can come back**, which is the reading
  all-or-nothing makes easy to get wrong. The departure entry is the only thing that takes the
  leaf out of the MLS group; without it the network rotates key material forever for a member
  who deliberately destroyed their half of it. What it buys is the network's hygiene, not the
  leaver's.

  Turned up while checking this: `kols-node`'s executor refuses *any* self-removal from
  `everyone`, on the grounds that it would leave the network unable to rotate its key. True of
  a last holder of `revoke-node`, false of everybody else, and it would have silently defeated
  E16 on its own — a protocol change landing into a client that refuses the entry before it is
  signed.

- **2026-08-30** — **The most destructive thing this client does was asserted by a dialog and by nothing else.**

  Forgetting a network deletes the store and the seed inside it, and the confirmation says
  you cannot come back as the same member. Nothing held that. The existing test asserted the
  directory was gone, which is a weaker claim: a directory can go while the thing that
  mattered about it survives somewhere.

  Now `forgetting_destroys_the_seed_so_a_later_join_is_a_stranger` creates a network, forgets
  it, and re-creates a store for **the same network id** — everything public about the
  network still known, because the id is public — and asserts the identity differs.

  The second half is what makes the first half mean anything: the same entropy in the same
  network reproduces the same identity, so an identity is a pure function of seed and id
  rather than of anything the store happens to hold. That is why deleting the seed is
  irreversible rather than merely inconvenient, and it is also why the id being public costs
  nothing — `Workspace::build` already refuses to derive one from the other, on the grounds
  that it would make an identity a function of public information.

  The property is about **this machine**, and the wording everywhere should stay that way. A
  copy of the directory on a second device, or a backup taken before the deletion, is that
  member still. Forget destroys this installation's copy; it does not reach the ones it
  cannot see.

- **2026-08-30** — **No event had ever reached the window, and three polls were written as fixes for it.**

  Found while adding first-sight marks on messages, which needed the unread counts to work —
  and the unread counts had never incremented once. The path they hang off is
  `listen("kols://records")`, and `listen` is `plugin:event|listen`.

  Tauri v2 gates every `plugin:` command on an ACL assembled from capability files. This
  application had none — no `capabilities/` directory, `gen/schemas/capabilities.json` an
  empty object — so the allow-list was empty and every one of those commands was refused. The
  application's *own* commands are not gated, which is exactly what hid it: `invoke("me")`,
  `invoke("open_channel")`, `invoke("send_message")` all worked, so the window opened, drew,
  posted and looked healthy. `watch()` rejected on its first `await` and registered nothing,
  and a rejected promise nobody awaits says nothing to anybody.

  **The cost was paid four times as four different bugs.** `CHANNEL_REFRESH_MILLIS`,
  `DOOR_REFRESH_MILLIS` and `watchRelay` are each documented in this codebase as a fix for "a
  pushed event was the only path to a redraw" — one of them says it is *the fourth bug in
  three days* of that shape. They are all the same denial one layer down. What survived was
  everything a poll covered; what did not was everything else: unread counts, the `Degraded`
  banner, the reorg report reaching a window that opened after the node reported, and the
  automatic reconnect when a network designates a relay while your node is already running.
  The field test reported reconnection as "a little janky", which is what that last one looks
  like from outside.

  **Reproduced before believing it**, because the alternative was another diagnosis from
  reading. `RuntimeAuthority::resolve_access` is public and `Context::runtime_authority_mut`
  reaches it, so `tests/permissions.rs` asks the real configuration whether each command the
  interface calls would be allowed. It listed all six as refused, which is the whole finding
  in one line of output. `capabilities/default.json` now grants `core:default` plus
  `set_title` and `request_user_attention`, and the test passes.

  The test also asserts the **window label**, which is the part that would rot: a capability
  names the windows it applies to, `tauri.conf.json` does not write a label down, and both
  default to `main` independently. They can drift apart without either file looking wrong,
  and the symptom would be this bug again.

  **The general lesson is about the shape of the failure rather than about Tauri.** A boundary
  that refuses silently cannot be found by using the product, however carefully — every
  session of manual testing this client has had was compatible with it. It can only be found
  by asking the boundary directly, which is a test, which is why `design/05` §8 now has a row
  for it.

- **2026-08-30** — **The first field test's interface list, and the two items on it that were already built.**

  Seven notes came back from the three-person test. Two of them — add, delete and rename
  channels — asked for controls that had shipped weeks earlier, on a right-click and nowhere
  else. That is not a control, it is a rumour, and the fix is a `⋯` on the row that opens the
  same menu rather than a second copy of it. It appears on hover, on keyboard focus, and
  permanently on the open channel, so there is always one visible instance to find by
  accident.

  **First sight of a message is a set, not a watermark, and the reason is the user's own
  observation.** A message is ordered by its author's clock, so one written while its author
  was offline lands *back* in the timeline when it finally arrives — behind any read position
  you could have stored. So what is remembered is which message ids were on screen when the
  channel was last looked at, per channel, replaced on each visit rather than accumulated:
  bounded by the size of the channel, and with no eviction policy to get wrong. A channel with
  no record at all marks nothing, because "never displayed here" honestly means *no idea*
  rather than *none of this has been seen*.

  Two things it got wrong first, both found by driving the interface rather than by reading
  it. Marks cleared on the next redraw, because `freshIn` recomputed from storage every two
  seconds — fixed by holding them for the length of a visit. Then they never cleared at all in
  a network with one channel, because clearing was a side effect of *changing* channel and
  there was nowhere to change to; now arriving clears and staying accumulates, and clicking
  the channel you are in counts as arriving.

  **The rest was arrangement.** The roster left the rail for a dropdown at the top right,
  keeping one number in the frame — how many other members this node is connected to — beside
  a dot for yourself lit when that is not zero. That dot is the only thing in the window that
  separates "the network is quiet" from "nothing I write leaves this machine", and it was
  previously answerable only by opening the roster and counting unlit rings. The door became a
  sheet behind a button carrying the number waiting, which is the only reason it was allowed
  to stop being a permanent section. Settings became a screen: it had been floating over a
  dimmed channel while asking people to read three paragraphs about permanent governance
  entries, in a column narrower than the window already available.

  **Driven under `jsdom` with a stubbed `invoke`**, thirty checks, which is how both of the
  first-sight bugs surfaced in minutes. Kept as `crates/kols-ui/drive.mjs` and deliberately
  not wired into the gate: it needs `npm install jsdom` and applies no CSS, so it answers
  whether the wiring runs and not whether anything is in the right place. Whether the front
  end gets a real gate is a decision to make on purpose rather than drift into, and adding a
  second toolchain to `cargo test` is most of what it costs.

- **2026-08-30** — **Leaving a network is a gap, and the rejoin question answers itself.**

  The seventh field note asked that forgetting a network tell the network. It cannot: every
  `MembershipChange` is gated on `revoke-node`, so the one member who knows they are leaving
  is the one member who cannot say so. `forget` drops the local store and the seed, and to
  everybody else nothing has happened — still in `everyone`, still in every role, still a leaf
  the epoch rotates for, forever.

  Recorded as E16 rather than built, per `design/06` §0: a removal naming its own author needs
  no capability, because it is monotone downward and self-directed — it grants nothing, names
  nobody but its signer, and is proved by the same signature that proves the identity it
  removes. Two things the spec work has to settle rather than imply: that a departure records
  and does not itself rotate, because under auto-admit a multi-use invite would otherwise buy a
  join/leave grinding loop; and that it takes effect on replay rather than on approval, since
  a network whose admins have all gone is the one a person most wants to leave.

  **The open question turned out to be already closed.** Whether a departed member may come
  back does not arise for `forget`, which destroys the seed — that identity cannot return under
  any rule we write, and the record is orphaned by construction. It is live only for a *leave*
  that keeps the seed, which the client does not have and should, and there the answer is that
  leaving is not banishment: the log records added, removed, added. Anything stronger would be
  a ban, and this protocol has no such concept.

---

- **2026-08-30** — **The network was only as durable as its authors' uptime, and the first three-node test found it.**

  Three people across three networks, and the third saw nothing the second had written
  until the second came back online — though the first had been connected to both the whole
  time and could display every message. The user's reading was right and worth stating: the
  first node *should* have passed those messages on without the author ever reappearing.

  **Nothing in the suite could have caught this, and the reason is structural.** With two
  nodes, every piece of content has exactly one other place to come from — its author. A
  network that only ever served content from whoever wrote it passes every test in
  `two_nodes.rs`. So the harness moved to `common/` and `three_nodes.rs` exists, where a third
  install can ask for something its author is not around to give.

  The property turned out to work: Bob writes, Bob leaves, Carol arrives and reads it from
  Alice. **What breaks it is a restart.** The transport's `ChunkStore` is a `BTreeMap` in
  memory and nothing wrote it down, and the same was true of other members' pointers and DEK
  wrappings. Storage §4.2 makes holding the bytes the whole of swarm membership, so a node was
  a member of the swarm for exactly as long as its process lived.

  Own content came back anyway, which is why this hid so well: `publish_own_logs` re-derives
  this node's segments from its stored records at startup and re-announces them. Nothing did
  that for anybody else's, and nobody else *can* — a segment is encrypted under its author's
  per-segment key and named by the CID of that ciphertext, so only the author can produce
  those bytes again. A member who read a message, closed the app and reopened it could still
  see the message and could no longer pass it on. The people in the field report had been
  closing and reopening clients all session.

  Now `Store::put_chunk`/`chunks` and `put_pointer`/`pointers`, with `restore_contribution`
  putting both back and announcing every chunk before the first tick. Pointers are stored as an
  encoded `PointerResponse::Records` holding one record — the wire's own type, chosen because
  it already carries exactly a pointer with its wrappings and already verifies every signature
  on the way back in; a private on-disk format would be a second encoding of the same thing,
  checked less. One file per pointer, because `MAX_POINTERS_PER_RESPONSE` caps a response at
  256 and a node holding more would write a file it could never read back.

  **Two false starts, both the same mistake, both caught by the test rather than by me.** The
  first version killed Bob seconds after his message went out live, and Carol failed — which I
  nearly reported as the defect. It was a race of the test's own making: receiving a record
  live and becoming able to serve it are separate things, several pointer-sync rounds apart.
  The second version then killed Bob before he had published a segment at all. Both now have a
  comment saying so, because the shape recurs: **a test that kills a daemon at the moment a
  user-visible thing happens is measuring the gap between that and the thing underneath it.**

  Instrumenting `request_foreign_segments` and `absorb_segments` with four `eprintln!`s and
  reading one run settled in minutes what an hour of reading the call graph had not. The
  diagnosis I was building from the code — that the live path was suppressing the fetch — was
  wrong, and the instrumentation said so immediately: `wanted=0`, no pointers known at all.

  **A tail worth recording, because it nearly became a false attribution.** The new file's
  own three tests timed out at full width — in the tests they had just added, which reads
  exactly like the feature being broken. Longer deadlines did not fix it; a process-wide
  mutex so only one of them runs at a time did. What remained was
  `a_node_offline_across_a_rotation_catches_up_and_can_still_read` failing on every full run
  and passing alone in 50 s, and the obvious story — the new file's four daemons starved it —
  was wrong. Moving `three_nodes.rs` out of `tests/` and re-running the whole workspace
  reproduces it identically. Pre-existing, and now deterministic on a 24-core box rather than
  intermittent: `patience`'s factor is 1 at that width, so nothing is scaled while fourteen
  tests' daemons compete. Recorded in CONTRIBUTING; not fixed here, because fixing it means
  rethinking how the suite bounds concurrency and that is not this change.

  Retention is deliberately not addressed. Storage §4.6 defines no eviction policy, and the
  agreed design (primary tier bounded by the `storage_offered` a node already advertises,
  under-replication earning a slot rather than arrival order, a freely droppable cache beside
  it) is the next piece of work — after real store sizes exist to design against rather than
  guesses. Chunks are unbounded until then, which is what the in-memory store already did.

- **2026-08-29** — **The milestone's first test passes, and the invite it passed with was 4,750 characters long.**

  Two of the user's own laptops on separate networks, one on a mobile hotspot: joined over the
  deployed relay, auto-admit on a valid invite worked, and both ends reconnected after being
  closed and reopened. That is exactly what §1 named as this milestone's first test, and it is
  the first time two nodes have met across networks rather than across one LAN.

  It nearly did not, and the reason is worth writing down because three diagnoses were wrong
  before the right one, and each was killed by the user's evidence rather than by my analysis.
  A friend on a third network could not join. I blamed a `?` in the dial loop that aborted
  before the circuit was reached (killed by "nobody answered within 30s" — the timeout is only
  reached *after* the loop completes), then a spent invite (killed by "I generated a new invite
  every time"), then O21 (killed by "they are not in the roster"). The actual cause was in the
  relay's deployment: it announced its private container addresses beside its public name, the
  private hops sorted first, and because every circuit through one relay peer shares a
  connection, dialling a dead hop left the relay abandoned and **cancelled the request behind
  it** — so the failure was reported against the address that would have worked. The user fixed
  the relay's public address in Railway and it connected.

  The transport now orders an unroutable relay hop last (Core §5.2, added as a SHOULD), which
  would have prevented it. But the invite was the other half: it carried twenty-five addresses
  of which most could not answer anybody, and that is what came next.

  **Which addresses an invite carries is now a selection rather than a filter.** One ordinary
  Windows machine listed three global IPv6 addresses and one real LAN address, and beside them
  a Tailscale pair, three virtual-adapter subnets from VirtualBox and two hypervisors, and four
  circuits through the relay's private addresses — every direct one doubled by TCP and QUIC.
  Twenty-five addresses, about 4,750 characters of URI, which is not something anybody sends in
  a chat message. Three rules cut it to nine: an overlay address is dropped
  (somebody on your tailnet does not need an invite to find you); a LAN address survives only if
  the routing table says it is the one this machine uses, since nothing in the text separates
  `192.168.56.1` from `192.168.1.200`; and a relay already offered at a reachable address is
  not also offered at a private one.

  Two things that shape those rules. The routing-table question is asked by connecting a UDP
  socket to a documentation address — no packet is sent, it is a route lookup — and it **fails
  open**: if the answer matches nothing this node listens on, every LAN address is kept, because
  a long invite is a better outcome than two machines in one house that cannot find each other.
  And the relay rule is not "drop private hops": a member relaying on the LAN looks identical
  from the address alone, and for that network it is the only way in. What distinguishes the
  leaked container address is that the *same relay peer* was also offered publicly.

  Writing the tests found two defects in the first version, both of the same kind — a rule
  about this machine's own interfaces applied to an address belonging to another machine. The
  private relay hop survived as if it were a LAN relay, and the preferred-source pruning was
  dropping LAN relays outright.

  More than half of what was left after that was the peer id, repeated once per address, and
  the user took the wire-format change rather than defer it — a new build costs nothing. **The
  addresses in one invite all name the same node, so they all end in the same `/p2p/<id>`, and
  the encoding now writes the longest ending they share exactly once.** Deliberately stated as
  a fact about those strings rather than about multiaddrs: `intranet-invite` holds addresses as
  opaque `String`s precisely so a signed credential does not acquire opinions about transport,
  and a suffix is something an encoder can find without acquiring any. It is not what is signed
  — the signature covers a payload where the addresses appear whole — so this changed the URI
  and nothing a receiving node verifies.

  **Two better-sounding schemes were measured and both lost to the framing.** A shared table of
  `/`-separated components dedupes strictly more, and comes out *larger*: `Enc` frames every
  length with a fixed eight-byte `u64`, so the table's per-entry prefixes plus a per-address
  index list cost 766 bytes where the plain encoding cost 1,082 and the suffix scheme cost 634.
  Compression would need a dependency, a decompression-bomb bound, and a data-dependent size.
  The cheap trick won on the merits, which is not the usual direction.

  I got the arithmetic wrong twice on the way, both times by estimating instead of measuring,
  and both times the test caught it. I predicted the addresses would "more than halve" — they
  fell 41%, because those eight-byte prefixes are a floor no scheme here gets under. And the
  first end-to-end figure was an estimate of "roughly 200 bytes of overhead" that was off by
  15%; minting a real invite and encoding it costs a keypair and answers exactly.

  **About 4,750 characters to 1,324**, roughly half from choosing addresses and half from the
  encoding. Core §5.6 now says why an implementation should care: an invite is carried out of
  band by a human, so its serialized size is part of the job the section gives it, not a
  representation detail — with the two drivers named, since neither is visible from the field
  list.

- **2026-08-23** — **A closing sweep, which found the same defect twice in one session.**

  Counts corrected: 247 here and 655 upstream, `kols-core` at 126 and `kols-api` at 33, all of
  which this file had at their pre-session values. §1's list of what the window can do now
  includes managing channels and folders and reporting a healed fork.

  **And `design/05` §3 was missing two commands again.** `CreateCategory` and `UpdateCategory`
  went into the boundary this session and never reached the page that describes it — which is
  precisely the failure the same section was corrected for earlier today, when `SetBootstrapRelays`
  turned out to have arrived with O12 and stayed unrecorded. Both were caught by the mechanical
  sweep rather than by anybody remembering, which is the useful part: a boundary document is
  worth checking against its code by machine, because intention has now failed at it twice.

  The identifier sweep across all seventeen documents is otherwise stable, every remaining miss
  attributable to an unbuilt `SetPermission`, a P3/P4 media type, or a name kept deliberately in
  a note about a rename. No broken links anywhere in the set.

- **2026-08-23** — **The settings sheet is designed, and a question I raised as open turns out
  to have been answered a week ago.**

  `09` §4.2 divides settings by **what a click costs** rather than by topic: changing a font is
  local, private and undoable; changing the network's name writes a governance entry every
  joiner replays and nobody can unwrite. A sheet that mixed them would teach people everything
  in it is a preference, which is wrong about half of it. The display name is the instructive
  case — it *feels* local and D27 makes it a governance claim that is never released, so it
  files under the network's side.

  Permissions is parked there provisionally and says so. It is governance, not a setting, and it
  belongs with membership and moderation in a surface that does not exist; `09` §7 now carries
  that as its eighth question rather than letting it settle in quietly.

  **The custom-CSS question was withdrawn, because it was never open.** `09` §6.2 had already
  argued it in full: CSS leaks only by causing a network request, `url()`, `@import` and
  `@font-face src` are the complete set of ways to cause one, and this app's CSP permits no
  remote origin — so arbitrary user CSS *cannot* phone home. I raised it in `00` §6 by reasoning
  from `05` §3's "no keys, no sockets, no files" instead of reading the document that owns the
  interface. That is precisely the mistake the audit which produced that section was written to
  catch, and it is recorded rather than quietly deleted.

  What was actually unsettled is smaller and is now decided. **D36**: CSP does nothing about
  spoofing, and the spoof that matters here is specific — networks are the privacy boundary D29
  protects, so a theme making one network resemble another does not confuse somebody, it puts
  their message in the wrong network. The title bar carries network and identity as native
  chrome; everything else stays fully themeable, hiding and layout included. **D37**: reset
  takes a native menu item *and* a launch flag, because a control inside the document is not a
  reset when a theme can hide it.

- **2026-08-23** — **A healed fork now tells somebody. O17 closed.**

  `serve` reconciles when governance arrives, which is exactly when a losing branch appears,
  and reports what was voided — this member's own actions separated from everybody else's,
  because those are the ones they can resubmit. Loud where the flag says the entry *removed*
  something: a revocation, a moderation, a rotation. 247 tests, clippy clean.

  **Reported once per heal, and per entry rather than per node.** Saying it on every sync after
  a fork would train somebody to ignore the one notice they get that a removed member is current
  again; latching after the first fork would be worse still, because the silence would look like
  safety. The dedupe is a set of entry hashes, held in memory — a restart re-reporting is
  correct, since nothing here knows whether anybody acted.

  **The window asks rather than only listening.** Replay follows the winning branch, so an
  action that lost leaves no trace in the projection: a window that opened after the node
  reported would never have learned. The node holds the last report and the interface asks for
  it on every refresh, which is the same fix the relay panel needed for the same reason.

  What is deliberately *not* done is automatic resubmission. Re-signing somebody's revocation on
  their behalf because a partition healed is a decision, not a repair, and Core §2.7.1 point 5
  accepts a prompt. The banner says what was undone and what it means; the member acts.

- **2026-08-23** — **The channel work reached the window.**

  Folders can be made, renamed, reordered and deleted; channels can be renamed, given a topic,
  archived, deleted, and dragged between folders. All of it gated on `chat:manage-channel` and
  all of it re-checked on receipt. 242 tests, clippy clean.

  **The order is computed once, in the core.** `sidebar_order` produces it and the webview draws
  what it is given. Sorting again in JavaScript would have put a second implementation of a
  normative rule beside the tested one, which is the arrangement that let a relay enforce
  ceilings its own source had stopped specifying.

  **Positions are sparse and split at the midpoint**, so an ordinary drag writes one governance
  entry rather than renumbering a folder. Where there is no room — or a sibling was never
  positioned, which sorts it last and leaves nothing to measure against — the folder is spaced
  out once and the drag retried. That pass costs an entry per channel and happens once per
  folder rather than once per drag. `move_channel` also checks whether the category is actually
  changing before writing a `Recategorise`: a no-op entry is replayed by every joiner forever,
  and spending that to record that somebody dragged a channel within the folder it was already
  in is not a trade worth making.

  **Two things this turned up.** `category_id` had never been implemented — §3.6 said a category
  id derives from (network id, nonce) and nothing derived one, so the domain tag was registered
  upstream (`c2438f4`) before anything could. And `dto::Category` went in and came straight back
  out: `SidebarRow::Category` carries the fields inline, so the separate type was a claim that
  something existed.

  Deleting a folder says what it does. Spec 07 §1.8 makes that removal of a name and a sort key
  rather than a scope, so the confirmation says the channels keep their permissions — a dialog
  implying otherwise would be describing a different system.

- **2026-08-23** — **Sidebar order, a network's name and named categories are built. O18 closed.**

  Spec 07 §1.6, §1.7 and §1.8 implemented in `kols-core`, with `kols-node` replaying both
  channel positions and category state. 235 tests, clippy clean.

  **The gate caught what review would not have.** Adding one `ChannelChange` variant broke two
  exhaustive matches, and neither was cosmetic: `kols-api` had no bounds arm for a position —
  there is no range to violate, which is now recorded rather than left implied — and
  `kols-node`'s replay had nowhere to put one, so a channel never positioned holds `None` rather
  than zero, which is what keeps a new channel sorting last instead of jumping to the top of
  everybody's sidebar.

  **`EntrySubject` is the piece worth remembering.** An entry's 32-byte subject is a channel id
  or a category id, and *which* is taken from the discriminant rather than declared beside it —
  so a decoded entry cannot disagree with itself about what it describes. What remains
  representable is a mismatch built in memory, which `check_bounds` refuses at the author rather
  than leaving for every reader to catch. The encoding is unchanged for channel entries: both
  ids were already 32 raw bytes in that position.

  Category replay is a second walk of the log rather than a second return value from `channels`,
  which nine callers destructure. Both walks are the cost O4's projection exists to remove, and
  O5 says measure before optimising.

  No terminal or window surface. `Command::UpdateChannel` already carries a `ChannelChange`
  unaltered, so the boundary needed nothing; the controls are the interface work `design/00` §5
  sequences next.

- **2026-08-23** — **D31 and D32 reached the document that is normative for them.**

  Spec 07 §1.6 fixes channel order and §1.7 a network's name, with `SetPosition` allocated as
  channel-update `0x07`. Upstream `1697b2a`; neither is implemented, which is O18.

  **Writing the bytes down changed one of the decisions.** `SetPosition` was going to be a field
  on the channel definition, which is where `slowmode` lives and where it looks like it belongs
  — except that adding it to the `0x01` body re-encodes every channel definition already
  written. As a change discriminant it costs nothing already stored, and it keeps "never
  positioned" distinct from "positioned at zero", which the sort rule needs. This is the
  argument for the spec step existing rather than going design straight to code.

  **And it found a gap neither the design nor the audit had.** A category is an id and nothing
  else — no name, no definition entry, no position. Channels can be ordered now; the folders
  §5 promises cannot be named or ordered at all, because there is no entry kind to carry it.
  That is a design question, not a spec one, so it is not answered here.

- **2026-08-23** — **The design set was read against the code, and the architecture document
  described a client nobody built.**

  A two-pass audit: a mechanical sweep of every backticked identifier, numeric constant and
  cross-link in all seventeen documents, then a deep read of what the sweep flagged. Most of
  the set held. Constants match spec 07 §4.3 exactly, `design/06`'s extension states agree with
  §4 here on every row, and no relative link is broken. The drift was concentrated.

  **`design/05` was the bad one.** It gave `kols-net` the `MemberNode` event loop, gave
  `kols-store` the projection, and did not mention `kols-node` — the executor, the daemon, the
  store and the `kols` binary — in either its diagram or its crate table. §3 had drifted both
  ways: it named `SetPermission` beside `SendMessage` as though both worked, and omitted four
  commands that do, `SetBootstrapRelays` among them, which arrived with O12 and never came back
  to the page. Of the ten events it named, the code has two. Corrected at `9a515d9`, with the
  reason the layout changed recorded rather than the table quietly rewritten.

  **O17 is the one build gap the audit found**, and it is the kind worth finding this way: it
  fails only under a partition, so nothing was ever going to surface it by running.

  **This file's version line was wrong about four documents**, and the fix is to stop stating
  them here at all. Each document carries its own version; restating them in a second place is
  O14's failure mode at documentation scale, and it had already happened.

  `design/00` §5 now carries the interface and channel-management work — most of it surface over
  models that already exist — sequenced ahead of the release gate, with D31–D33 recording what
  was decided. §6 no longer says no architectural questions remain open, because five do.

- **2026-08-23** — **Two machines met through the deployed relay, and messages crossed both
  ways.** The first time any of this has run outside a test harness. This supersedes the
  "deployed is not proven" caveat in the entry below it, which was written hours earlier.

  **What was run.** Two machines **on one LAN**, both running the window, meeting through the
  deployed DI-Relay: connected several times, reconnected several times, and chat content
  crossed in both directions.

  **The relay was load-bearing even on one network**, which is what keeps this from being a
  shortcut: nothing in the client dials a locally-discovered peer, so the two ends had no way
  to find each other without it. That gap is now **O16** — mDNS runs and the transport caches
  what it finds, but it never auto-dials and the client ignores the event it emits.

  **Two defects found by taking the relay away, both since fixed.** Bringing the relay down
  broke an established connection, and bringing it back did not restore one. The first
  contradicts Core §5.5, under which a vanishing relay costs a reconnection rather than a
  network; the second left a node stuck rather than merely disconnected, which is worse,
  because a stuck node looks like a working one. The behaviour now is the specified one: the
  relay is needed to *establish* a connection, and an established connection survives it going
  down. That is also the best evidence so far that the DCUtR upgrade works — a pair still
  talking with the relay gone is a pair that went direct, since §5.2 closes the circuit once
  the negotiation is done.

  **What it does not prove, stated because this file is believed.** Both ends were on one
  network. The milestone's own case is two ends behind *different* NATs, one on a mobile
  hotspot, and a hole punch across a single LAN is the easy version of the problem Core §5.5
  exists for. O15 is untouched: two nodes cannot demonstrate content routing on any network.

- **2026-08-23** — **A relay is deployed, `v0.11.0` is cut, and the DCO requirement is gone.**

  The two-machine test's one standing dependency is met. **Deployed is not proven**: nothing
  has reserved a circuit through it and its address is not recorded here, so §0 says that
  rather than implying the test is now routine.

  `v0.11.0` covers the three commits that had accumulated past `v0.10.0` — abuse limits
  enforced at the reader, the current relay ceilings, and the daemon-orphan diagnosis — plus
  the correction of this file's own header date, which had lagged a day behind its own body,
  and a release reference that named `v0.1.0` nine tags after the fact.
  `crates/kols-app/tauri.conf.json` moves with the workspace version because it names the
  installer `docs/two-machine-test.md` tells you to download; bumping only `Cargo.toml` would
  have published a `v0.11.0` release containing an installer labelled `0.10.0`.

  **The DCO requirement is removed from all three `CONTRIBUTING.md`.** It was never once
  followed — 0 of 197 commits across the three repos carried a `Signed-off-by`, including
  every one of the sole contributor's own. No licence here requires it: AGPL-3.0, MPL-2.0 and
  CC BY 4.0 all impose no contributor-certification obligation. And the rationale as written
  oversold what it bought, which is why it went rather than being enforced: a DCO does **not**
  grant relicensing rights, so relicensing would still need every copyright holder's
  permission individually — that is a CLA's job, and this deliberately was not one. What
  actually carries the inbound grant is the "Licensing of contributions" section, which stays
  in all three. Worth re-adding *and* enforcing in CI if these repos ever take an outside
  contribution, where certifying provenance is the real thing a DCO is for. Left in this log
  above as it was written, because the log records what happened rather than what is true now.

- **2026-08-22** — **The flaky daemon test was orphaned processes, and three separate defects
  kept that invisible.**

  Asked to find out why `two_nodes::a_node_offline_across_a_rotation_catches_up_and_can_still_read`
  was flaky. **It is not, and neither is anything else in the suite.** On a clean machine the file
  passes 11/11 in 36 seconds, five runs in a row. On a machine where an earlier run was
  interrupted it fails two or three tests every time, in 103 seconds.

  `Daemon::drop` kills its child, and `Drop` does not run when the *harness* is killed — a
  Ctrl-C, a timeout, a `pkill` on cargo. The orphans keep listening on 45101–45162 indefinitely,
  and every later run's daemons fail to bind. My own earlier measurements in this session were
  contaminated by my own orphans, including the one that made me report the flake as
  pre-existing. It **is** pre-existing; the cause was not what I said.

  **Why nobody could see it.** Three things, each of which alone would have been enough.
  Every daemon helper spawned with `stderr(Stdio::null())`, so a daemon that exited said why into
  a void. `wait_for` never asked whether the daemon was still alive, so it spent the whole
  deadline reporting the wrong thing. And `TransportError::Listen` carried the empty string,
  because `libp2p::TransportError::Other` writes nothing in `Display` and puts the reason in
  `source()` — so even a captured stderr said `could not listen: ` with nothing after it. All
  three are fixed; the case now diagnoses itself in under a second and names the port.

  **Two real test bugs found on the way, both of which had been passing for the wrong reason.**
  `wait_for` searched the *whole* log, so waiting for something a daemon says more than once
  matched a line from a minute earlier — one such wait returned in **thirteen microseconds**.
  Found by instrumenting every wait with the time it took, which turns a rare failure into a
  number: a wait that returns instantly is not waiting. A match now consumes the log up to
  itself. That immediately exposed the second: `a_revocation_rotates_the_epoch...` waited for
  `"picked up"` after a post, and a post is a record rather than a governance entry, so no such
  line was ever coming — it had always matched the channel definition's line and asserted
  nothing. It polls the re-wrap it was standing in for instead.

  And `invites::one_string_takes_a_stranger...` had the identical race its sibling in the same
  file was fixed for and it was not: `join` returns when the daemon *answers*, which is before
  the daemon persists governance and records the room. It polls now too.

  **Found and deliberately not fixed:** `serve`'s event loop takes the store's append lock with
  `?` in three places, and `Store::lock` gives up after ten seconds — so a one-shot command
  holding it too long does not delay a key answer, it **terminates the node**. Core §3.5.1 makes
  retrying safe, so degrading would cost nothing. It is a product bug rather than a test one and
  it is not what caused this.

  213 green six runs running, clippy clean; 655 upstream.

- **2026-08-22** — **Abuse control was built in halves, and the halves each cited the other.**
  Found by a full documents-and-code review. §4.3 calls the rate ceilings *validity rules* —
  "a record past the ceiling is refused by readers, so a local limit would mean two members
  rendering different histories" — and only the writer was enforcing them. So the limit *was*
  local: whatever the author's own client chose. `ChannelView::check` looked at channel,
  signature, membership, moderation and posting rights, and never at the rate.

  **The skew hold did not exist at all.** `max_future_skew_millis()` read its policy key and was
  called from nowhere. That one matters more than it sounds, because §10.2's whole argument that
  the ceiling cannot be gamed is a pointer at it: claim timestamps a minute apart while sending
  them all at once, and *"records dated ahead of the receiver's clock are held until local time
  reaches them. An author who lies about pacing gets exactly the pacing they claimed. No extra
  mechanism needed."* Without the hold there was no mechanism, and a record claiming next year sat
  at the top of the channel permanently. `spacing_claimed_timestamps_buys_exactly_the_pacing_claimed`
  is the test for the pair, and it fails if either half is removed.

  **Slowmode was settable, bounded, replayed, displayed and enforced by nothing.** No path read
  `channel.slowmode` to refuse a post.

  **The one design decision worth knowing about is where the rate check runs.** Not in `admit`:
  "how many has this author written in the last minute" has no arrival-order-independent answer
  while the set is still assembling, so the same records in two orders would refuse two different
  ones — which is precisely the divergence the rule exists to prevent. It is a pass over the
  sorted set instead (`kols_core::withheld`), like every other effect in the merge.
  `every_arrival_order_refuses_the_same_records` asserts that over 40 shuffles.

  **Held is not refused**, and the reader now reports the two separately. A future-dated record
  stays in the set, is served like anything else, and renders when local time reaches it; telling
  somebody it was refused would be the interface asserting what it knows to be untrue.

  **Decided, not flagged:** slowmode applies to the message **class**, so an edit is paced along
  with a post. The specs do not say, and class is what keeps two client versions agreeing (spec 07
  §3.3) — the cost is that a six-hour slowmode also delays fixing a typo, accepted because
  slowmode is an instrument a moderator reaches for occasionally rather than a setting a network
  runs with. `design/01` §10.4 carries the reasoning and the fix if that turns out to be wrong.

  16 new tests in `kols-core`, one through the real binary, and all nine of the ones that assert
  new behaviour were confirmed to fail against the unfixed build first. 213 green, clippy clean.

- **2026-08-22** — **Unread channels, votes that line up, and every colour behind a token.**

  **Unread is driven by arrival, not by scanning**, which is what makes it free. The node already
  reports what it learned; the shell now says whether any of it was a **message**, so a vote or an
  edit counts as activity without telling somebody to go and read something. A channel that is not
  open gains a count; opening it clears it. Kept in `localStorage`, keyed by network — what
  somebody has read is not a fact about the network, and writing it to the log would publish a
  reading habit to every member. It survives the app being closed for a pleasant reason: the node
  was not running either, so it learns the backlog on the next start and reports it then.

  **Votes were misaligned on your own messages.** They sat immediately left of the action bar,
  whose width depends on what you may do to *that* message — so rows carrying edit and withdraw
  pushed their votes further left than everybody else's. Actions now come first and votes take the
  right edge, so the always-visible control is the aligned one.

  **The theming question, answered by audit rather than impression.** No inline styles are set
  from JavaScript, so structure and classes are fully reachable — but **seven colours were
  literal hex** and therefore unreachable by any user theme, most of them added by me as the
  interface grew: row hover, the pinned tint, chip backgrounds, the settings veil, and a
  foreground used on accent-filled controls. All five are tokens now (`--hover`, `--pinned`,
  `--sunken`, `--veil`, `--on-accent`), bringing `:root` to 18 and leaving no literal colour
  anywhere else in the sheet. `design/09` §6's user themes were true of the original palette and
  had quietly stopped being true of everything grown around it.

  196 green, clippy clean.

- **2026-08-22** — **The DHT was never bootstrapped, so routing had no reach.** Raised as "routing
  should be built into this — a mesh cannot need every member connected to every other member".
  Correct, and the design already agrees: §5.1's Kademlia is the routing layer, provider records
  are published, provider queries are issued, and `fetch_chunks` pulls from **whichever holder
  answers** rather than from an author. So A can read C's records through B without ever
  connecting to C.

  Every piece was present except the one that gives it reach. `add_address` records peers this
  node is *already connected to*, and nothing else populated the table — so it was exactly one
  hop deep, and a provider query could only ask the two or three peers already on the other end
  of a socket. Content two hops away was unfindable, and that is indistinguishable from content
  genuinely having no providers. `kad.bootstrap()` appeared nowhere in either repo.

  Fixed upstream (`8c0e2c8`): `bootstrap_dht` walks the DHT with a query for the node's own key,
  which returns the peers closest to it and fills the table on the way. The client calls it on
  every new connection and every five minutes, since a table that is never refreshed thins out as
  peers leave until it can no longer route. §5.1 now states the requirement rather than leaving it
  to be noticed, and says why: content is fetched from whichever holder answers, and the DHT is
  what finds one.

  **A correction to what I said one message earlier.** I reported that this client "never attempts
  a full mesh" and implied the fix was to hold more connections. That was the wrong conclusion —
  you do not want a full mesh, you want a DHT, and the DHT was there and inert. Connection
  breadth still matters (`design/09` §2's tiering does not exist), but Kademlia's own queries dial
  as they route, so bootstrapping widens the graph as a side effect rather than needing a policy
  first. 655 upstream, 196 here.

- **2026-08-22** — **The waiting-room fix was too expensive, and a roster landed.**

  **`record_waiting` on every tick was a real cost**, flagged on review before it shipped
  anywhere. `Store::state` is not a lookup — it reads the whole governance log off disk and
  re-applies it — so this paid a full replay every two seconds, for the lifetime of a node, to
  answer a question whose answer is almost always "nobody". It now returns early when the room is
  empty *and* nothing is written down; both halves are needed, since the second is what still
  lets a room that has just emptied record that once. Steady state is an in-memory check and a
  small file read.

  **A roster, answering the narrow question honestly.** `people` returns every member with a
  connected marker, and the note under it says what the marker means: *connected to you right
  now*. An unlit member may be away, unreachable from here, or never dialled, and nothing on this
  machine distinguishes them — so it is drawn as an empty ring rather than a red light. Written
  on change rather than on a tick, because unlike the waiting room it needs nothing recomputed.

  **The membership question this raised is worth recording.** There is no routing (Core §5.2), so
  a member is reachable directly or by hole punch and otherwise not at all — and separately, this
  client dials the peers it has addresses for and never attempts a full mesh, because
  `design/09` §2's hot/warm/cold tiering does not exist. Convergence does not need one: it is
  transitive through any shared peer. But it means "connected" is a subset of "reachable" by
  policy and not only by network, which is exactly why the marker cannot be called presence.

  A flake found and fixed on the way: `the_window_takes_the_same_path_as_the_terminal_to_join`
  read the founder's waiting file the instant `redeem` returned. `answer_join` sends the response
  *before* the daemon persists governance and writes that file, so this was always a race — it
  just widened when recording the room started replaying. It polls now, like the rest of the file.

- **2026-08-22** — **Interface pass one: settings, votes, and a door that clears.**

  **The waiting room never emptied**, and the cause is worth keeping: the node's room is filled
  when a join is answered and emptied by nothing that admission passes through — admitting writes
  a governance entry, and no path from one reaches that room. So a founder was shown somebody
  they had already let in, beside an `admit` button already pressed. The published list is now
  recomputed every tick and filtered against **replayed membership**, which is the authority — so
  it is also right for members admitted by somebody else, and after a restart. Verified against
  the unfixed build first: the new assertion in `invites.rs` fails there.

  **The relay moved out of the rail into a settings sheet.** Core §5.5 calls a bootstrap relay
  scaffolding, and scaffolding does not belong in the frame you look at all day — but it is one
  keystroke away, because when it *is* wanted it is usually because nothing else works. Escape
  closes it.

  **Reactions became an up/down vote, with no protocol change.** Spec 07's
  `Reaction { target, key, remove }` carries a free-form key, so up and down are two of them.
  Mutually exclusive, which is the difference from a reaction: voting up while holding a down
  vote withdraws the down vote first — two records, both this member's, because the log has no
  notion of changing your mind, only of what you have said. Keys this client does not offer are
  still rendered as chips, since another client writing one is conformant and hiding it would
  make "no button for that" look like "never happened".

  196 green, clippy clean. **Presence — who is online in a network or a channel — is not built**;
  see the note in §1, since the honest version is narrower than the request.

- **2026-08-22** — **Circuit lifetime raised to 60s, and §5.3 now says what the ceilings do not
  bound.** Asked whether the lowered ceilings limit a network to hundreds of thousands of nodes.
  They do not, and the arithmetic runs the other way: `max_circuit_duration` and
  `max_circuit_bytes` bound *one* circuit, so shorter and smaller means a relay performs **more**
  introductions per hour, not fewer — at 32 concurrent circuits, 120s allowed ~960/hour and 60s
  allows ~1,900. What bounds a single relay is `max_reservations` (128) and `max_circuits` (32),
  neither of which changed, and what scales past them is more relays and `relay_bootstrap_willing`
  members (§5.5) rather than a larger allowance on one host — a relay that could serve 100k
  members would be exactly the infrastructure §5.3 exists to prevent.

  The question did surface a coupling I had created: 30 seconds was chosen against the old
  behaviour, where a cut-short negotiation left the pair on a relayed connection. §5.2 made that
  fatal in the same commit, so the cost of being slightly tight became two members who cannot
  talk. Raised to 60 — still half the original, still nowhere near a session — with the bytes
  ceiling left at 256KB, where the anti-abuse work actually happens. Upstream `9951282`.

- **2026-08-22** — **O14 closed: a relayed circuit now carries the negotiation and nothing else.**
  Three parts, and the first was the one actually leaking.

  **A connection arriving relayed no longer triggers a sync.** `ConnectionEstablished` asked for
  the governance log, the ledger and every pointer over the circuit *before any upgrade had been
  attempted* — so payload crossed a relay on every connection, whether or not the punch later
  succeeded. The sync now happens when the peer becomes usable: on a direct connection, or on a
  successful punch, which performs it.

  **A failed hole punch disconnects the peer.** The circuit existed for that negotiation; leaving
  it open is how a relay quietly becomes the path. `dcutr` reports failure only after
  `MAX_NUMBER_OF_UPGRADE_ATTEMPTS`, so this is not closing on a first stumble.

  **The ceilings dropped from 120s/8MB to 60s/256KB**, in §5.3 and the implementation together.
  (First set to 30s, then raised on review: 30 was chosen against the *old* behaviour, where a
  circuit ending mid-negotiation left the pair relayed. §5.2 made that fatal in the same change,
  so a ceiling a few seconds too tight now costs two members who cannot talk. A DCUtR exchange
  runs ten to fifteen seconds on a lossy mobile link — which is exactly the pair that needs one.)
  The old figures predated the prohibition and were loose enough that a client relaying a whole
  conversation never met a limit — the rule held only by clients choosing to obey it, and one did
  not. `default_limits_match_the_spec_baselines` failed the moment the two disagreed, which is
  its job, and a new `a_circuit_cannot_carry_a_conversation` asserts the property rather than the
  constants.

  **Not covered:** behaviour under a real failed punch, which needs the NAT scenarios in harness
  spec §2.3 — including the new §2.3.6, where an IPv4-only CGNAT pair must *not* connect. Those
  scenarios do not exist yet, so this is verified by construction and by the suite, not by a
  failing punch. 655 upstream.

- **2026-08-22** — **A channel the founder created after the joiner arrived never showed up for
  them.** Suspected to be permissions; it was not. Reproduced first: `two_nodes.rs` gained
  `a_channel_created_after_a_member_joins_reaches_them`, and **it passes** — every other test
  here creates its channels before the joiner arrives, so that path had never been covered, and
  the entry does travel.

  So the sidebar was the fifth surface where a pushed event was the only path to a redraw. The
  channel list was drawn on `kols://governance` and nowhere else. The two-second tick now also
  re-reads what replay decides — this member's standing and the channel list — each drawn only
  when a signature over it changes, so a tick that finds nothing leaves the sidebar alone rather
  than fighting somebody using it.

  **The new test failed once and it was mine.** It passed alone and failed in the full suite,
  which in this project is the signal for a real race — but here it was a port collision with
  `a_founder_can_still_key_somebody_in_after_restarting`, so the second daemon bound nothing,
  reported nothing, and looked exactly like the feature failing. Moved to free ports; 196 green,
  and the ports now carry a comment saying why they are not shared.

- **2026-08-22** — **The two-machine test passes, including the part that matters most.** Two
  machines on separate networks, through a relay, admitted, keyed, messages both ways — and then
  **the relay was taken down and messages kept flowing**, which is the hole punch working and the
  relay out of the data path, confirmed on real networks rather than asserted. Edits, withdrawals
  and reactions all confirmed too.

  **Pinning "did nothing", and it was doing everything except showing.** Checked by running it:
  the record round-trips and `kols read` prints `[pinned]`, and `records.rs` has covered this
  since it was written. The window's entire signal for a pinned message was
  `box-shadow: inset 2px 0 0` on a row with 4px of padding — present, invisible, and exactly what
  a dead button looks like. It now carries a **`pinned` flag in words**, beside `edited`, plus a
  heavier bar and a tinted row.

  Two real defects came out of looking. The redraw signature ignored pin state, so a pin by
  *another member* on any message but the last would never have been drawn — the same
  only-when-you-act failure as before, one layer up. And `--remove` had no coverage, which
  matters now that a window offers pin and unpin as one button: a toggle whose second half is
  untested is half a feature. Both fixed; 195 green.

  A correction: I said pinning had no test at all. It did — an earlier grep matched "wra**ppin**g"
  and `head -5` hid the real hits.

- **2026-08-22** — **Core §5.2 corrected by its author: a bootstrap relay carries no payload, ever.**
  I had read "a correctness guarantee, not a usable path" as permitting a capped fallback, and
  argued that reading back. The author's intent is stricter and the spec now says it: **there is
  no third tier.** A relayed circuit carries the DCUtR negotiation; when the upgrade fails the
  circuit MUST be closed and MUST NOT carry protocol or application payload of any kind. A pair
  that cannot hole-punch reaches each other over IPv6 or not at all.

  Two consequences now stated rather than implied. Members with no mutual path are not
  partitioned from the network, only from each other — everything is pull-based and
  content-addressed, so they converge through any member both can reach, which is an ordinary
  sync partner rather than a route. And a two-member network where neither can reach the other
  has **no remedy**, which is a real limitation stated plainly instead of papered over.

  §5.3's ceilings are recast as defence in depth behind the rule, with 120s and 8MB noted as now
  generous by orders of magnitude. The harness spec asserted the opposite outcome in two
  scenarios and has been rewritten, plus a new one asserting that an IPv4-only CGNAT pair does
  **not** connect. Upstream `0b085e4`.

  **E13 loses its fallback**, and `design/09` §3 records why: the shared network's bootstrap
  relay was going to serve a DM's circuit on the reasoning that it "carries bytes and never
  inspects a join". It may carry a negotiation and nothing more. What survives is the rendezvous,
  which is the friction goal anyway — and the privacy flag that sat there goes with the fallback,
  since a relay that carries only a handshake observes that two identities met rather than
  watching a conversation.

  **Implementation is now behind the spec, deliberately and briefly.** The client still permits a
  relayed circuit to carry traffic — that is what v0.6.0 is being tested against right now, and
  changing transport behaviour underneath a running test would waste it. What is owed: close the
  circuit on a failed DCUtR upgrade, refuse to send payload over a circuit, and lower the
  relay's own ceilings so it enforces the rule rather than trusting clients. Recorded as **O14**.

- **2026-08-22** — **The client offered no IPv6, which is the path the spec designates when a hole
  punch fails.** Raised as "the relay should never carry messages", which sent me to Core §5.2 —
  and the sentence that matters is this one:

  > Two peers who can never hole-punch are expected to reach each other over IPv6, not over a
  > relay. … A deployment that expects CGNAT users to depend on relayed circuits for ordinary
  > traffic has misread this ordering.

  That is what had been built. Both front ends bound `/ip4/0.0.0.0/tcp/0` and nothing else: no
  IPv6, no QUIC — visible in a real log where every listening line was IPv4 TCP. §5.1 requires
  both families and both transports, `MemberNode::listen_default` had provided exactly that all
  along, and its own doc comment says binding all four "is what gives two peers behind CGNAT a
  path at all". Both front ends now default to it; `--listen` still overrides.

  **Where the stronger claim does not hold, recorded because it will come up again.** §5.2 keeps
  tier 3 deliberately: a relayed circuit is "a correctness guarantee, not a usable path", so that
  two peers are never partitioned. It is bounded by §5.3's ceilings, which "are the design, not a
  throttle to be raised when users complain". So content over a relay is permitted, expected to
  hit the caps, and never something to depend on — the remedy being IPv6 or a member volunteering
  as a relay, never a larger allowance on the hosted one.

  Dual-stack also multiplied what an invite carries: one machine had five interfaces before IPv6
  and QUIC doubled each. Loopback, unspecified and IPv6 link-local are now filtered out of the
  published set — legitimate to listen on, useless to hand somebody else, and previously shipped
  in every invite. 195 tests here.

- **2026-08-22** — **Taking the relay away silenced two nodes, and bringing it back did not
  reconnect them.** A deliberate test — is the hole punch working? — and it found three faults,
  one of which was hurting every conversation whether or not a relay was involved.

  **A closing connection was reported as a disconnect even when another remained.** A
  hole-punched peer has two connections: the relayed one it arrived on and the direct one that
  upgraded it. §5.2 exists so losing the first costs nothing, but `ConnectionClosed` reported
  `Disconnected` unconditionally, and the client removes a disconnected peer from the set it
  syncs with. **A circuit is capped at 120 seconds**, so the relay closes the relayed connection
  about two minutes into every conversation — meaning a *successful* hole punch was followed by
  the client dropping the peer and syncing with nobody. `num_established` is the answer libp2p
  already provides. Fixed upstream (`9e83bbc`).

  **Nothing re-dialled a peer that went away.** Addresses were dialled once at startup, so a peer
  lost after that stayed lost — a relay going down and coming back left two nodes that could both
  see it, had both re-reserved on it, and never spoke again. Re-dials every 15 seconds while
  nothing is connected, and only then. A reconnect is already a sync, so the connection is the
  whole recovery.

  **Nothing said whether the hole punch happened.** `HolePunchSucceeded` and `HolePunchFailed`
  were emitted by the transport and dropped by the client, so "is this still going through the
  relay?" had no answer short of taking the relay away. Both are now reported.

  On whether the relay should be out of the path after a punch: it is, and the 120-second cap is
  what enforces it — the relayed connection is closed by the relay itself shortly after the
  direct one exists. What was missing was the client surviving that closure, which is the first
  fix above rather than anything about the relay.

- **2026-08-22** — **A message from the other side did not render until the reader posted.** The
  records were in the store the whole time: the daemon absorbs them on its own — `two_nodes.rs`'s
  `a_reply_travels_back_and_both_sides_agree_on_the_order` asserts exactly that, with nobody
  posting — and the composer's own re-read after sending was what finally revealed them.

  **The fourth bug in three days where a pushed event was the only path to a redraw.** The
  others: the relay panel missing its one startup report, `kols://joins` emitted with no
  listener, and the near-miss before it. So this is fixed structurally rather than by another
  attempt to get an event right.

  The open channel is re-read every two seconds, matching the daemon's own sync tick, and drawn
  only when a signature over it changes — so an unchanged channel costs a local replay and no DOM
  work. `design/05` §3 already requires a consumer to merge rather than append, and the interface
  already redraws from the projection every time, so asking again can never show the wrong thing.
  O4's projection is what makes the replay itself cheap later.

  Also removed: `if (event.payload === state.current)` on the records listener. It could skip a
  redraw and could never cause one, which makes it a micro-optimisation whose only possible
  effect was the bug.

  And a consequence of drawing on a timer rather than on the reader's own action: scrolling to
  the end is now conditional on already being there, or a reader who scrolled up to read history
  would be dragged back every two seconds.

  **The two-machine test now passes its core**: two machines on separate networks, through a
  relay, admitted, keyed, messages both ways.

- **2026-08-22** — **A relay reservation was obtained once and never kept.** Reported as having to
  make sure the founder was still connected before the joiner could redeem an invite — which is
  the shape of a reservation that was lost and never re-obtained.

  **The 120 seconds is not the reservation.** It is `max_circuit_duration`, the cap on a single
  relayed connection (Core §5.3): a relay introduces, DCUtR hole-punches, and the direct
  connection carries the conversation. The reservation lease is an hour and libp2p renews it
  itself — verified in `libp2p-relay` 0.21.1, `reservation_duration` and the handler's renewal
  timer. So expiry was never the problem.

  **The connection ending was.** A relay holds reservation state in memory and is stateless
  across restarts (§5.5), so every redeploy drops every reservation — and there were many
  redeploys while the DNS fault was being chased. Nothing re-reserved: `reserve_via_relay` ran
  once at startup, and `circuit_listeners` only ever *grew*, so the node believed it held a
  circuit forever and advertised an address nothing answered on. Undetectable from inside and
  invisible from outside.

  Fixed in both repos. Upstream (`3335267`): listener loss prunes the sets and surfaces
  `NodeEvent::ListenAddrGone`, plus `has_circuit()` — a question that could not be asked before,
  because the answer was always yes. Here: a 20-second watchdog re-reserves through the same path
  startup uses, and a lost circuit says so on the terminal and in the relay panel.

  **Not done deliberately:** reserving on demand when an invite is redeemed. The reservation must
  exist *before* the joiner tries to reach the founder — they arrive through the circuit — so
  refreshing when needed is always too late. Maintaining it is the only ordering that works.

- **2026-08-22** — **A founder watching the door saw nobody at it.** B redeemed an invite, landed
  in the waiting room and stayed there; A's window never showed anyone waiting.

  The node did its half correctly: `record_waiting` writes the room to the store *before*
  `JoinAnswered` is emitted, so the answer was on disk the whole time. The shell emitted
  `kols://joins`. **Nothing listened for it.** The waiting list was only re-read inside `drawMe`,
  which runs on `kols://governance` and `kols://keys` — and a join produces neither. So the one
  event that means "somebody is at the door" was the one event with no listener.

  Audited the rest rather than fixing only the reported one: six events are emitted and
  `joins` was the only orphan. All six now have listeners.

  The doorway also re-reads every four seconds while a member who can admit is looking at it.
  `design/09` §4 already calls the waiting room stale by construction — it is live state in the
  node, written down for anything else to read — so refreshing is the model rather than a patch
  over a missed event. **This is the third bug in two days of the same shape**: an answer that
  existed, an event that carried it, and a consumer that was not listening at that moment.

  Also confirmed: v0.3.4 published a `.dmg`, so Tauri accepts `signingIdentity: "-"` and the
  ad-hoc signing landed.

- **2026-08-22** — **The macOS build said it was "damaged".** Not a corrupted download and not a
  bundler failure: **an arm64 binary must carry a valid signature to execute at all**, so an
  unsigned `.app` inside a quarantined `.dmg` fails Gatekeeper's check outright — and macOS
  reports that as *damaged and can't be opened*, which points at the Trash rather than at
  right-click → Open. The affordance everybody knows is the one for an app that is merely
  *unidentified*, which is a different state.

  `signingIdentity: "-"` in `tauri.conf.json` ad-hoc signs it, which turns the first message into
  the second. It is not notarisation and is not pretended to be — first launch still needs
  right-click → Open, and notarising needs a paid certificate this project does not have. The
  release notes and the runbook now say which message means what, since "damaged" is the one that
  makes somebody delete a working build.

  Two-machine test note: **the relay path works.** DNS, the relay, the circuit — all of it, on
  real machines across real networks. This is packaging, downstream of that.

- **2026-08-22** — **A node could not dial an address written as a name.** The root cause of a
  two-machine test that could not get a circuit, and the quietest failure this project has found.

  `SwarmBuilder` composes a transport from what it is asked for, and name resolution is one of
  those things. Without `.with_dns()` a `/dns4/…` address is unsupported — and nothing says so:
  `listen_on` for a circuit succeeds, because registering a listener resolves nothing; the dial
  that follows is refused inside the swarm; **no call returns an error**; the relay never sees a
  connection; and the node reports only that no reservation arrived. Every address in this
  project had been an `/ip4/` literal, so nothing had ever asked the transport to resolve a name
  — and a deployed relay is normally reachable *only* by hostname, since its operator does not
  control the IP.

  Fixed in `distributed-intranet` (`b66b16a`) for both node types, with
  `tests/dns_addresses.rs` dialling `/dns4/localhost` — no DNS server needed, so it cannot flake
  on one. Verified against the unfixed build first: it fails there, by timing out with nothing
  reported, which is the fault's own shape. 654 green upstream.

  **And the client's message asserted more than it knew.** It said the relay "answered and
  granted no usable circuit — it returned no address of its own", which is a claim about the far
  end. `reserve_via_relay` returning `Ok` means the reservation was *started*, not that anything
  replied. So a correctly configured relay was investigated for an evening on the strength of a
  sentence the code had no basis for. It now says a circuit did not arrive and names both
  possibilities, pointing at the relay's own log to tell them apart.

  Prior fixes in this chain were real and are kept — `RELAY_PUBLIC_ADDR` genuinely is required
  behind a proxy, and DI-Relay now reads Railway's proxy variables itself. They were not *this*
  fault.

- **2026-08-22** — **The relay panel showed a symptom with two opposite causes.** A first
  deployment could not get a circuit, tried a rebuilt relay from scratch, and got the same red
  line — because the line said "no circuit was granted" for both *nothing answered there* and
  *something answered and named no address*. Those need opposite fixes, and the node knew which
  had happened: the reason went out as `Degraded`, into a different part of the interface, while
  the status line kept its summary.

  `Event::Relay` now carries `failures`, the reason per relay in the order tried, and the panel
  prints them rather than a summary of them. The `RELAY_PUBLIC_ADDR` note is shown only for the
  case it explains — it says nothing useful about a relay nothing could reach.

  Verified rather than assumed, in `libp2p-relay` 0.21.1 itself: a reservation's address list is
  built from the relay's **external addresses only** (`behaviour.rs`), and the client turns each
  into a circuit listener (`priv_client/transport.rs`). An empty list is therefore granted and
  then unusable, which confirms the announce theory — and equally confirms it explains only one
  of the two failures.

  Also checked and ruled out: protocol drift between the relay's pinned `v1.0.1` and this
  checkout. The only changes are E14's, and none touch the relay path or `PROTOCOL_VERSION`.

- **2026-08-22** — **The relay panel could miss the answer it was waiting for.** Found by the
  first real use of the window: the panel sat on "waiting for this node to report" about a node
  that had already reported.

  The standing was **only ever pushed**, as `kols://relay`. The node starts in Tauri's `setup`,
  so it can settle its relay before the webview has finished registering listeners — and an
  event with nobody listening is gone, permanently, with no second chance because the node
  settles this once at startup. The window then reported a state that had no way of changing.

  Now the node *holds* its standing and `relays` returns it, so the event's only job is to say
  "ask again"; a missed one is harmless. The interface also polls every two seconds while
  unreported, stopping at about 30 — reservation is bounded near 20 — which covers a missed
  event without waiting on one.

  Two things that made this hard to see and are also fixed. **Nothing said the node was working
  on it**: pressing *designate* restarted the node and the panel changed almost nothing, so there
  was no way to tell the button had done anything. There is now an *asking the relay for a
  circuit…* state, visually distinct from the three verdicts. And **the refusal named no cause**,
  which for this failure is nearly always one thing: a relay behind a proxy announces its private
  container address, grants the reservation, reports healthy, and hands back a list clients
  reject. The panel now says that, with the `RELAY_PUBLIC_ADDR` to set — and the runbook adds it
  as step 4.5 rather than leaving it to be discovered.

  192 green, clippy clean.

- **2026-08-21** — **The window can now carry the whole test, and two gaps had to close before
  that was true.** Asked for a window-only test; checking rather than assuming found that it was
  not possible yet, for reasons neither of us had noticed.

  **The relay's identity needed a terminal.** `RELAY_PHRASE` came from `intranet-harness
  identity new` — a tool in the *protocol* repository — and DI-Relay refuses to start without
  one, on purpose. So "no step needs a terminal" was false for the one step a founder cannot
  skip. The relay panel generates one now: `MasterSeed::generate` and `to_backup_phrase`, from a
  crate `kols-app` already depended on. Shown once, stored nowhere, and marked as the relay's
  private key rather than the member's — the member's seed has no interface at all and must not
  start looking like it does.

  **React, revise, withdraw and pin did not exist in the window.** Seven of `kols-api`'s
  thirteen commands were wired; these four were not, and step 11 asks for exactly them, because
  each takes a different path through the merge. They are per-message controls revealed on hover.
  Two details worth keeping: `mine` on a reaction now comes from the core rather than being
  guessed, since a reaction is a toggle and a client that guessed would send an add for one the
  member already holds — a no-op that reads as a dead button; and withdrawal asks for
  confirmation in the words `design/01` §6 requires, *hidden, not unsent*.

  Pin is gated on the network-wide `moderate-content` and so misses a per-channel moderator. That
  error runs the safe way — it hides a control somebody may hold, rather than offering one that
  will be refused — and is noted where it is written.

  The runbook is now window-only end to end, cleanup included, and says plainly that it tests two
  unproven things at once. 192 green, clippy clean.

- **2026-08-21** — **`kols-cli` renamed to `kols-node`.** The name described 853 of its 5,578
  lines and misdescribed the rest. What is in it is the store, the node loop, the executor, the
  workspace, joining, inviting and secret-writing — the window's entire backend, and its only
  ko-ls dependency besides `kols-api` and `kols-core`. Calling that "the CLI" is what made "can
  we get rid of the CLI" sound like a small question. `cargo build -p kols-node` now; the binary
  is still `kols`. 192 green, clippy clean.

  **The binary is deliberately still here**, and this is the reasoning rather than an omission.
  Deleting it before the two-machine test costs three things and buys nothing: the runbook is
  written in `kols` commands and would be dead on arrival; CI's seed-permission check drives
  `kols init` and is the only test that inspects the built artifact rather than the source; and
  the window has still never had a flow run through it, so the reference path would be gone at
  exactly the moment a failure needs localising. D30 already makes it a development tool — a
  status, not a deletion — and its lines cost nothing while it waits.

  The remaining gate is that seed check. It *can* move into Rust: `windows-sys` is already a
  dependency with `Win32_Security_Authorization`, so a `#[cfg(windows)]` test can read a DACL
  back. It has not, because a Windows test cannot be run from this container and shipping an
  unverified one the day before a two-machine test is the wrong trade.

- **2026-08-21** — **D30: the terminal is a development tool, not a product surface.** Stated by
  the user and worth recording, because it changes what "finished" means rather than what the
  code does. Nobody is expected to use this application from a command line, so `kols` owes the
  window **no feature parity**, no discoverability and no end-user documentation. What it must
  keep is the one property that makes it worth having: it crosses the same `kols-api` boundary a
  window does, so "works in `kols`, not in the window" localises a fault to the interface.

  Consequences applied: the README's *Trying it* now opens with the release rather than
  `cargo build -p kols-cli`, and marks everything below it as the development path. The release
  notes said "`kols-desktop` is the window and `kols` the terminal client", which read as two
  products on the download page; `kols` is now described as what it is. §1's Runnable row is
  ordered product-first.

  **What this decision is not:** an instruction to delete the crate. `kols-cli` is 5,578 lines
  and 853 of them are the binary — the rest is `serve`, `store`, `executor`, `workspace`,
  `join`, `invite`, `network`, `chat` and `secret`, which is the window's entire backend and its
  only ko-ls dependency besides `kols-api` and `kols-core`. The binary is also what CI drives to
  check the seed's permissions on the built artifact, which is the one test that inspects the
  artifact rather than the source. When the window has carried the two-machine test end to end,
  `main.rs` and the `[[bin]]` target can go and the crate wants renaming to `kols-node`; the
  seed check moves to a library test on the same `secret::write_private` path.

- **2026-08-21** — **Runbook brought level with the window, and one asymmetry the restart
  created.** `kols relay set` now says what it cannot do: a node dials relays when it starts, and
  a terminal cannot restart a daemon in another process the way the window restarts its own. A
  running `kols serve` designating a relay it never dialled was previously silent. The runbook
  gains that caveat at step 4, the window's four relay states in the troubleshooting list, rows
  for steps 10 and 12 in the front-end mapping, and a note that the two machines need not use the
  same front end — A in the window and B in a terminal exercises both and leaves the detailed log
  on the side more likely to be waiting.

  Also: the relay panel no longer shows a permanent "…" to a member in the waiting room, who has
  no readable policy to draw and is exactly the person most likely to be looking at it.

- **2026-08-21** — **Designating a relay now restarts the node itself, and a latent restart race
  is gone.** Asked as "why would we not build it into the flow" — and the honest answer was that
  naming the next step is worse than taking it. `set_relays` restarts the node it just made
  policy for; a relay learned from *replay* does the same through `restart_node`, but only when
  there is no circuit, so a working node is never interrupted for a relay it does not need.

  **Underneath it was a real bug, not just a missing convenience.** `start_node` aborted the
  previous node and spawned the next immediately. `JoinHandle::abort()` does not drop the
  future — it asks the task to stop at its next await — and the store's claim is released *on
  drop*. So the replacement raced its predecessor's claim, and `hold_node` does not fail fast on
  a claim that looks fresh: it waits the staleness window out. The symptom would have been a
  window that hangs for half a minute and then works, on the ordinary path of reopening a
  network. The abort is now awaited, inside the new task so nothing blocks the interface thread.

  Also corrected: **the window has been launched.** Once, before anything was wired to it, and
  it rendered roughly correctly. Three documents said it never had. What is true is narrower and
  more useful — no *flow* has been run through it, so every path it has is unexercised. And §0's
  item 3 no longer says `kols-desktop` needs a Windows machine, which stopped being true when
  CI started building it.

- **2026-08-21** — **O12 and O13 closed: the window can set up a relay, and says when one
  works.** Found by a question worth recording — "why am I using the CLI at all if there is a
  window?" — which §1 had answered wrongly. It claimed no step needed a terminal while O12, three
  sections down, said relay setup was exactly the step that did.

  The window now submits `SetBootstrapRelays`, which had existed with a correct gate and a
  working executor and no handler on this side. Alongside it: the **full** network id with a
  copy button, labelled as the `RELAY_NETWORK` a relay will not boot without — it was rendered
  `slice(0, 16)`, which is worse than showing nothing because it looks copyable — and a
  creation form that no longer says "you can add one later" about a path that did not exist.

  O13 is `Event::Relay { reserved, designated }`, emitted at startup in **every** case. Relay
  failures already crossed the boundary as `Degraded` while success was a `println!`, so the
  window could report relay trouble and never relay health, which is the wrong half to have when
  the question is "is my relay working". The count rides along so that "designates none" and
  "designates some, none usable" stay distinguishable — both leave you reachable only on your
  own addresses, only the second is a fault.

  Address validation went to `kols_cli::parse_relay` rather than into the window, so the
  terminal gained it too: an address naming no peer id is refused by `kols relay set` and
  `kols init --relay` as well. That check earns its place — designating a relay writes a
  governance entry **every member replays**, so a bad address is carried by everybody and fails
  later on somebody else's machine. 3 tests; 192 green, clippy clean.

- **2026-08-21** — **Dropped the Intel macOS build leg, and bumped the actions off the
  deprecated Node 20 runtime.** The `macos-13` leg never produced a build that went through,
  and the Mac this is built for is Apple Silicon, so the matrix is now Windows and
  `macos-latest` only. Separately, every run warned that Node 20 is deprecated: `checkout` to
  v5, `upload-artifact` to **v6** and `download-artifact` to **v7**. Those are the *lowest*
  major of each that runs Node 24, deliberately — `upload-artifact@v5` is still Node 20, so
  the obvious one-major bump would have looked like a fix and changed nothing, and
  `download-artifact@v8` stops unzipping unconditionally, which would silently change what
  `find artifacts -type f` hands to `gh release create`. The reasoning is a header comment in
  the workflow, where the next person to see a deprecation warning will read it.

- **2026-08-21** — **Made every unlinkability claim say the same thing, and wrote the two-machine
  runbook down.**

  Core §1.2 has always been honest — two per-network public keys cannot be tied together, and
  IP-level and timing correlation are explicitly out of scope — and the documents restating that
  guarantee had been dropping the second half. A reader met the strong version in `design/00` §1,
  `design/03` §4.2, §4.4 and §4.6, and spec 07 §0, §1.5 and §8, and the honest version once, in
  `design/09` §1. **§1 is now named as the canonical statement and the others defer to it**, with
  Core §6 and spec 07's own summary carrying the limit too, since those are the sections
  consuming work is told to assume from.

  **Two of them were overclaims rather than omissions.** `design/03` §4.2 and spec 07 §1.5 both
  said a separate network reveals nothing about a conversation to the server. That is right about
  its log and its storage and wrong about E13's relay fallback, where the operator carrying the
  circuit sees one address acting as a member and as a party to a conversation and can infer that
  two members are talking. Both now say which of the two they mean.

  Nothing was weakened. Three documents stopped describing a narrower guarantee as a broader one,
  which is the failure this project objects to everywhere else.

  **`docs/two-machine-test.md` is new**, and is written to be thrown away: it exists because the
  test has not been run, and once the path is either routine or wrong it should be deleted or
  folded into the README rather than kept as a second thing to disagree with this file. It carries
  the three traps in the places they bite — never bind a relay to loopback, the Railway domain and
  TCP proxy are both required, and `relay reserved a circuit on …` is the line that means steps 3
  and 4 worked.

  Also refreshed the protocol repo's `CLAUDE.md`, which still said five amendments were
  implemented and E10 was the only one outstanding — stale since E14 landed and E13 and E15 were
  recorded.

- **2026-08-21** — **D29 recorded, and the direct-message bootstrap is not an exception to it.**
  O11's decision now lives in `design/00`'s register and its reasoning in `design/09` §3, rather
  than only in a status log where it would have been lost.

  **The distinction that resolves the apparent conflict.** D29 refuses a relay *designated by two
  networks*, carrying both as standing infrastructure. E13 is not that: it exchanges the DM
  network's addresses over a connection that already exists between two people who have already
  identified themselves to each other, then opens directly. **No relay is involved on the primary
  path at all**, and nothing is disclosed that either party did not already hold.

  **The fallback is bounded rather than free**, and the bound is a dependency nobody had noticed.
  When hole-punching fails, the shared network's bootstrap relay carries the circuit and does
  briefly serve two networks. It does not create D29's structural problem, because a
  conversation-profile network runs `Discovery::Off` and so has no routing table and joins nobody
  else's — **which turns O2 from a resource optimisation into a privacy requirement**, since with
  discovery on that fallback would put a DM node straight into the shared relay's routing table.
  O2's row says so now.

  What remains is IP correlation, which is real: the relay's operator sees one address as a member
  and as a party to a conversation, and can infer that two of its members are talking. That
  **weakens a claim `design/03` §4.2 makes** — that a separate network reveals nothing about the
  conversation to the server — against one party, in the case where hole-punching failed. Kept
  anyway, because the alternative is that symmetric-NAT users have no direct messages, and now
  stated in `09` §3 rather than left inside an earlier flag.

  **One correction worth recording, because it was nearly reasoned from.** A participant's identity
  in a DM network is *not* their identity in the shared network — it is a separate per-network
  derivation, which is precisely why `design/03` §4.3 step 4 carries a **voluntary identity link**
  binding the two. That the identities differ is the property making a conversation unlinkable from
  the server to everybody except the person a participant chose to prove it to. If they were the
  same, the DM network would be linked to the shared one by construction.

- **2026-08-21** — **Reviewed how a relay is actually reached, and one finding turned into a
  decision that changes what direct messages need.** No code moved; this is the review the
  two-machine test was waiting on, recorded as O11–O13.

  **What a relay is, checked against the code rather than the docs.** `RelayBehaviour` is circuit
  relay v2, identify, ping and kad — no governance replay, no ledger, no membership check, exactly
  as Core §5.5 says ("not a member of the network it serves"). It holds no state and circuits are
  capped at 120s and 8MB, so it establishes connections rather than carrying them. **The only
  network-specific thing about a relay is its identity**: `RelayNode::new` takes a
  `PerNetworkIdentity`, derives a keypair, and that is the whole role `RELAY_NETWORK` plays in
  DI-Relay.

  **Which raised the question, and the answer is no.** Because a relay checks nothing, one
  deployment *can* serve several networks — and it must not. Two networks sharing a relay share a
  routing table: `kad` runs under the default protocol name and `PROTOCOL_VERSION` is
  `/intranet/0.1.0` for every network, so nothing keeps their members from discovering each other,
  and two of one person's identities meeting on one relay is exactly the correlation Core §1.2
  exists to prevent. `design/09` §1 already listed "any relay that sees both" as an honest limit;
  reuse would have made an incidental limit into the normal case. **Decided: a relay is not shared
  between networks.** Recorded as **O11**, along with the uncomfortable half — nothing enforces it
  today, and enforcing it properly means network-scoping the protocol names, which is a wire change
  rather than a client fix.

  **The decision makes E13 load-bearing rather than convenient**, and that is the consequence worth
  not burying. Every direct message is its own network (D10). If a relay cannot be shared, then
  without cross-network bootstrap a conversation between two NATed people needs its own deployed
  relay — which nobody will do. E13 was scoped as removing friction from a flow that would
  otherwise work; it is now the only thing that makes DMs work across NAT at all. §4 says so now.

  **The window cannot finish the relay journey, which contradicts §0's own claim** that no step
  needs a terminal. It never shows the network id — the one string DI-Relay demands before it will
  boot — and its creation form says "You can add one later" when nothing adds one later, since
  `SetBootstrapRelays` reaches only `kols`. So a founder who skips the relay there is stuck at
  every exit and is not told why. **O12.** Alongside it, **O13**: `kols serve` reports reserved,
  granted-but-unusable, unreachable and none-designated, and says which — but success is a
  `println!` while the failures are `Event::Degraded`, so the window surfaces relay trouble and
  never relay health. "Is my relay working" is the question two machines will actually raise.

  **None of this blocks the two-machine test**, which is the useful conclusion. The CLI path is
  complete and signposted — `kols init` prints the network id and the exact ordering when no relay
  is given — and DI-Relay's own README covers the traps, including the `RELAY_PUBLIC_ADDR` case
  that looks like nothing is wrong. Deploy, and run the test on that path.

- **2026-08-21** — **E14 landed, and it was a security fix wearing a liveness fix's clothes.** It
  was written up as "a joiner can stall forever", which is true and is the smaller half.

  The stall is real: key delivery is a request and a response, the answer can be lost, and a
  request that is never answered strands a member permanently — in the log, served by honest
  nodes, holding no key that opens any of it. The client asked exactly once because retrying was
  known to be unsafe, so a lost answer was terminal.

  **What the fix had to be was not what `design/06` §14 specified.** That asked for key
  re-delivery: answer a member already in the group with the key material and append nothing.
  It reads correctly and restores half a member. A requester asking again has no group state *by
  construction* — it asks because it holds no key, and it holds no key because the Welcome that
  would have carried both never arrived — so keys alone leave it able to read what exists now and
  unable to apply any later commit. `apply_pending_rotations` returns immediately for a node with
  no group. It would have fallen out of the network at the next membership change, silently,
  having looked recovered.

  **And re-adding turned out to break revocation, which nobody had noticed.** §14's objection to
  it was honesty — a second leaf for one member is a lie in a log every member replays. The
  actual cost is worse: removal is expressed against an *identity* and applied to a *leaf*, so
  `leaf_index_for` finds the **first** leaf holding that credential. Revoking a doubled member
  removes the abandoned leaf, rotates, and hands the new epoch key to the member it was asked to
  exclude, still sitting on the leaf nobody removed. The removal reports success and Core §3.1's
  guarantee is gone. The new test fails against the old behaviour with the revoked member's key
  fingerprint **identical to the founder's**.

  So answering replaces the leaf — remove the stale one and add the requester's key package in one
  commit, producing one rotation and a Welcome they can open. Core §3.5.1, `GroupSession::
  replace_member`, which clears both proposals if either fails, since a dangling remove would be
  swept into whatever commit came next and drop a member nobody decided to drop.

  **A test was written, run against the old code, and thrown away**, which is the part worth
  keeping. It asserted the log stayed linear across two answers — and passed under both
  behaviours, because answering parents on the current tip, so a duplicate add never forked
  anything. The fork in the original report came from concurrent writers; the duplicate add's
  damage is the second leaf. A green test that asserts nothing is worse than no test, so it was
  replaced with the revocation one, and that one was checked against the old code before being
  believed.

  The client re-asks every 30 seconds while unkeyed — deliberately slow against a two-second tick,
  because every answer is a real rotation and a governance entry every member replays forever.

  Also: spec 07 §7's amendment table gains **E13, E14 and E15**, which were tracked only in the
  client, so the protocol repo's own record said one chat amendment was outstanding when four
  were.

  649 → 653 upstream, 189 here, clippy clean in both. `two_nodes` passes 10/10 at full width;
  **pinned to two cores it now fails one where it used to fail two**, which is recorded rather
  than claimed as a fix — the retry plausibly recovers one of the starved cases, and one run is
  not evidence.

- **2026-08-21** — **All three repos are licensed, and two of them were wrong in opposite
  directions.** `distributed-intranet` had no LICENSE and no licence metadata, which under
  default copyright is all rights reserved — public and legally unusable, for the repo whose
  specs exist to be implemented by other people. `ko-ls` declared `MIT OR Apache-2.0` in
  `Cargo.toml` with no LICENSE file behind it, which is both ambiguous and the most permissive
  reading available: it expressly allows closing this and selling it.

  The requirement was "free for everyone, and nobody can sell it", and those cannot both hold
  literally — every open-source licence, copyleft included, permits sale, and the Open Source
  Definition forbids discriminating against commercial use. What it resolves to is that nobody
  may **enclose** it, which is a copyleft question rather than a price one. So: **AGPL-3.0-only**
  for the applications, **MPL-2.0** for the protocol crates, © DriftingNarwhal. Selling stays
  legal and becomes pointless, since the buyer receives the source and may redistribute it.

  The Affero clause is the one doing work here: the obvious way to enclose a chat system is to
  host a modified copy rather than ship one, and plain GPL would permit exactly that. The MPL
  side is the reverse trade — other implementations may build over the protocol and licence
  their own work freely, while changes to these files stay open. §8 records the constraint that
  couples them, which is that the MPL files must never carry Exhibit B.

  Checked rather than assumed: the whole dependency tree is permissive (MIT, Apache-2.0,
  MPL-2.0, BSD, Zlib, Unicode) with no GPL or non-commercial terms, so nothing blocked the
  relicense; every crate in both workspaces now resolves a licence where nine resolved
  `UNDECLARED`; and both gates are green — 189 here, 649 upstream, clippy clean in both.

  **The specs are licensed separately, under CC BY 4.0.** A software licence is the wrong
  instrument for prose — MPL is written about Source Code Form and executables, and an
  implementer quoting a section into their own documentation should not have to work out
  whether that makes their document Covered Software. Attribution is the whole and correct ask
  for documents that exist to be implemented by other people.

  **DI-Relay needed a second fix that licensing it alone would not have delivered.** It pulled
  `intranet-transport` and `intranet-identity` at tag `v1.0.0`, which predates the protocol's
  licence — so anyone building it was building against code that was still all rights reserved.
  A licence on `main` does not reach a tag. `v1.0.1` is the first protocol tag carrying MPL-2.0,
  and DI-Relay is repointed at it; its README had also claimed MIT/Apache "matching the protocol
  repository", which was wrong in both halves, since the protocol repository had no licence to
  match.

  **All three repos now carry a `CONTRIBUTING.md` requiring a DCO sign-off.** Deliberately not a
  copyright-assignment CLA — contributors keep their copyright. A copyleft licence is only as
  good as the project's established right to ship the code under it, and with the repos public
  that right needs recording per commit rather than reconstructing later by asking every past
  contributor individually.

  **Honest limit:** `ko-ls` was public for a day declaring `MIT OR Apache-2.0`. A permissive
  grant already given cannot be withdrawn from copies already taken, so anyone who cloned it in
  that window may hold that snapshot under those terms. No LICENSE file existed and nothing was
  published to crates.io, which makes the grant weak and the practical exposure nil — but it is
  worth writing down rather than describing the change as clean.

- **2026-08-21** — **A second documentation pass, over the design set against the protocol it
  depends on rather than against itself.** The previous pass read the client's documents for
  internal contradictions and found three. Reading them against the landed specs is a different
  exercise and found five more, one of which had been true for weeks in a place nothing tracked
  and one of which was a gap in the normative document rather than in this repo.

  **`design/06` §2 described the opposite of what E2 landed.** It said all four channel entries
  are capability-gated and therefore *count* toward branch length. Core §2.7.2 excludes them,
  and is right — whether an application entry is cheap to mint depends on the tier of the
  capability it declares, which the branch-length metric deliberately cannot resolve. This
  file's own E2 log entry recorded the reversal on the day it landed; the design document that
  asked for it was never updated, so the request and the outcome disagreed in writing for two
  months. Now it carries both, because the reversal is the more useful half.

  **The same section overclaimed profile enforcement, and so did two others.** "A
  `ChannelDefinition` in a conversation network is rejected on replay" reads as the protocol
  enforcing it. It cannot: E2 landed generically, so the platform carries `chat` payloads
  without decoding them, and spec 07 §1.2 corrected the wording upstream. `01` §2.1, `03` §4.1
  and `06` §2 each still had the strong form. Fixing one and leaving two is how a correction
  becomes drift, so all three say the same weaker and true thing — every conformant reader
  refuses it, the platform enforces nothing here, and a modified client would see a channel
  where others see none.

  **`design/06` §16 still assumed a master seed.** D28 removed it in August; `03` §4.6 was
  updated for that and `06` was missed, so the one document that tracks what the protocol owes
  was describing an identity model the client had abandoned.

  **Which is how E15 was found, and it is the finding worth keeping.** The client does not
  implement Core §1.1 — one master seed per person, per-network keypairs derived from it — and
  does not intend to. D28 generates fresh entropy per network. That is a divergence from a
  normative specification, and `design/06` §0's own rule is that a needed protocol change is
  recorded there rather than assumed into existence. It never was: the decision lived in the
  implementation, then in a decision register, and in no list of what the protocol owes. It
  blocks nothing — the protocol's crates never see a seed, only the keypairs it produces, so
  the cost is conformance rather than function — which is exactly why it stayed invisible.
  Recorded now with what the amendment has to say, including the part the client cannot yet
  answer: what a backup *is* when there is no phrase to write down, which is O7.

  **The retention vocabulary split turned out to be hiding a real gap in the normative
  document.** `01` §8 named the default `Forever` in its prose and `Unbounded` in its table
  one screen later, and spec 07 §2.8 said `Unbounded` too. Aligning the word was the small
  half. The large half is that §2.8 described retention as one setting, named neither policy
  key, and never said how a network expresses "forever" in a key whose value is a day count —
  so the one genuinely normative question here was unanswered: **what does `0` mean?** Two
  clients answering that differently render different history from the same records, which is
  precisely the divergence §4.3 exists to prevent. The client has always read absent, zero,
  negative and overflow as `Forever`, fail-safe, because content allowed to go dark cannot be
  brought back. §2.8 now says so as a MUST, names both keys, carries the two-windows reasoning
  and the newest-record rule, and states that a reader must not report an unwrappable segment
  as deleted. §4.3's key list gains both keys. Spec text only — the protocol stores app-layer
  policy values without interpreting them (Core §2.6.2), so there is no protocol code to
  change and no protocol test to write; the behaviour being specified is `kols-core::policy`'s
  and was already tested here.

  Also: this file called `design/09` v0.2 in two places after it went to v0.3.

  **No code changed and no test moved**, and that was checked rather than assumed: 189 passing
  here and clippy clean, counted from a file rather than a pipe for the reason a previous entry
  records. Nothing upstream was touched, so its 649 stands unre-run.

- **2026-08-21** — **A documentation pass, and it found three stale claims rather than none.**
  The point of reading all of it after a run of landings is that some of it is wrong, and it was.

  **The README contradicted itself.** "The window" said *"It runs no node, so it will not show
  you another member's messages and will not update while you watch"* — three paragraphs above
  "What exists", which said the window runs a node and updates as records arrive. Both were
  written a day apart and only one was true. It now describes what the window does, including
  minting an invite and admitting from the waiting room, and says plainly what it still cannot
  do, which is presence.

  **`design/00`'s roadmap was two landings behind**, listing as *not yet* an executor behind the
  API boundary, its event half, edits and reactions as commands, invites, and "every part of the
  Tauri client". All but threads are built. A roadmap that describes finished work as pending is
  worse than none, because somebody plans around it.

  **`design/09` §5 still owed its presentation half**, which is built: the shell resolves each
  capability against replayed state and hands the interface a flag per control, so there is no
  second permission model in the front end to drift from the first. §7's invite question is
  mostly answered too, and what is left of it is now stated precisely — use-count and expiry are
  *defaulted rather than decided*, which is a smaller and more honest open question.

  **`design/05` §5 described a keychain nobody has built.** It read as a description and was a
  target: what exists is an unencrypted seed file restricted to its owner, refused where it
  cannot be restricted. Now says so, and points at O7.

  **E14 is new in `design/06`**, which is where a required protocol change belongs — O10 was
  living only in `STATUS`. It carries the mechanism, why re-delivery rather than re-add (a
  Welcome cannot be reissued, and re-adding rotates a group whose membership did not change,
  which is a lie in a log every member replays), and acceptance criteria.

  Also recorded in `design/05` §8: run the daemon tests starved as well as fast, because a wide
  machine wins every race a narrow one loses; and clean up after them, because this container's
  storage is the host's.

- **2026-08-21** — **The gate is gone: tests run here, GitHub builds.** A CI gate on every push
  was never asked for — builds were — and it turned three rounds of debugging into somebody
  copy-pasting log fragments for failures that one local `taskset -c 0,1` run reproduced with
  complete output. CI could say *that* tests failed; the container said *why*.

  What CI genuinely adds is the one thing this container cannot do: execute on Windows and
  macOS. So that is all it does now, and only when a build is wanted — a `v*` tag, or a manual
  dispatch. Each platform leg runs `cargo test -p kols-cli --lib`, which is the store and the
  seed, and then checks the **artifact** it just built: create a network, look at the seed's
  permissions, refuse the build if anybody else can read it.

  **That artifact check exists because the obvious thing does not work.** `secret::tests` is
  `#[cfg(all(test, unix))]`, so the Windows DACL path compiles out of the Rust tests entirely —
  a `cargo test` leg on Windows would have run the pure-function store tests and reported
  success while proving nothing about the one function written specifically for Windows. A test
  that looks complete and is not is worse than none.

  Also: **the test suite now cleans up after itself**, because this container's storage is the
  host's. `Home` already removed its directory on `Drop`; the `secret` tests added yesterday did
  not, and a day of Windows cross-compiles had taken `target/` to 11 GB. `Drop` rather than a
  line at the end of a test, because `Drop` runs on an unwind and a failing test is exactly when
  the scratch is left behind.

- **2026-08-21** — **The joiner never asked, and reading the code had said otherwise twice.** The
  round before this reported a stall after sixty seconds, which was useful and was not the
  finding. The finding came from **reproducing it** — the suite pinned to two cores, with the
  test harness changed to print *every* daemon's log rather than only the one being waited on.
  The founder's side is where the answer was, and no failure had ever shown it.

  What it showed: a joiner that learned its own admission and **never asked to be keyed in at
  all**. The request lived inside `Synced { accepted > 0 }`, nested under `learned > 0`, so it
  fired only on the sync that accepted new governance entries, and only if `ready` happened to
  succeed at that instant. Miss it once and there is no second chance — every later tick syncs,
  accepts nothing new, and skips the block. Starving the machine is what made that race lose:
  `ready` fails while the ledger advertisement has not landed, and on quiet loopback every entry
  arrives in one go, so the single opportunity was the one that failed.

  The ask now happens on the tick whenever this node is unkeyed, ready and has not asked yet.
  **Still exactly one request**, which is deliberate rather than timid: answering is not
  idempotent (O10), so this makes the single ask *reliable* without making asking *repeatable*.

  Pinned to two cores, `two_nodes` goes from six passing to eight; at four cores — the runner's
  width — all ten pass in 80s. **Two cores still fails two**, both now in the content path after
  keying rather than in keying, and that is recorded rather than tuned away.

  **The harness change is the durable part.** A two-node failure printed the log of the daemon
  that did not do the thing, and the reason is almost always in the other one — a founder
  refusing a request says so on *its* terminal, which the waiting side cannot see. That is how a
  stall reads as "nothing happened" from one side and cost two rounds of guessing.

- **2026-08-20** — **The Windows gate found a real bug, and it is not about Windows.** Four of
  six failures stop at the same line — `asked X to key us in`, then nothing for 135 seconds. On
  loopback that is not slow, it is stuck, and it is the one thing in the node loop that cannot
  recover on its own.

  **The request is sent once**, on the `Synced` event that first learns this node has been
  admitted, and that event need never recur. Everything else in the tick is re-asked every two
  seconds with a comment over it explaining why — a pull-based stack has no other way to learn
  that a peer changed its mind — and the key request is the one exception. A founder has
  ordinary reasons not to answer at that instant: answering appends a rotation, so it takes the
  store's append lock that every one-shot command also takes, and losing that race reports a
  degradation to *its* terminal and tells the asker nothing.

  **Retrying is the obvious fix and it is unsafe, which is worth knowing before somebody tries
  it again.** `answer_epoch_key` calls `add_member` unconditionally — there is no idempotence
  check — so a second request adds an existing member a second time and appends a second
  rotation, forking the log against the entry that admitted them. Implemented, and it voided a
  member's grant: a keyed member could no longer post, in a test that had passed for days. The
  retry is reverted and the finding kept.

  So the node **says it is stalled** rather than hiding it, after sixty seconds, naming why
  nothing will retry and what to do. The real fix is upstream, where the group is: answering a
  request from a member already in the group must re-deliver rather than re-add. Recorded as
  **O10**.

  This is why the Windows gate earns its cost even though the bug is not Windows-specific. A
  loaded runner loses a race a quiet development machine wins every time, and the two-machine
  test over a relay — the actual next milestone — is a far worse place to meet it than CI.

- **2026-08-20** — **Tripling the timeouts doubled the failures, which ruled out the diagnosis
  it was meant to fix.** Six tests failed where three had, and the suite took 335s where it took
  125. More time producing more failures is not what a machine that merely needs more time looks
  like.

  **The measurement that reframes it:** pinned to four cores, this suite passes on Linux in 83
  seconds; on a four-core Windows runner it failed six tests in 335. *Same width, different
  answer* — so parallelism was never the axis, and scaling patience by core count addressed the
  wrong variable. It stays, because a genuinely narrow machine does want it, but it was not the
  fix.

  What Windows charges that Linux does not is **per operation**. Defender inspects an executable
  on every launch and every file written beneath the tree; this suite launches a **76 MB debug
  binary about a hundred times** and its daemons write many small files per tick. Both workflows
  now exclude the workspace, the cargo directory and the temp directory, tolerating failure
  because a runner image with Defender already off should not fail the gate for refusing to
  turn it off twice.

  Ports and home directories were checked for collisions first and there are none — a previous
  entry records a test that did collide, so it was the obvious suspect and it is worth saying it
  was eliminated rather than never considered.

  **If this is not enough, the next lever is serialising the daemon-heavy suite on Windows
  rather than waiting longer**, and the one after that is the backfill test's thirty separate
  `kols post` launches, which exist to cross a seal threshold and could cross it with fewer,
  longer messages.

- **2026-08-20** — **The first Windows CI run failed three tests, and none of them was a
  Windows bug.** `two_nodes` timed out waiting for a joiner to backfill — on Windows only,
  which reads like a platform difference and is the wrong conclusion. **Pinning the same suite
  to two cores on Linux reproduces it exactly.**

  Every deadline in these tests is wall-clock and was tuned on a 24-core box, which makes them
  an assumption about the machine rather than about the software. The suite runs its tests in
  parallel and each spawns two or three daemons that sign, verify and encrypt, so a four-core
  runner does not run the same work slightly slower — it runs ten tests' worth against a sixth
  of the cores. The failing daemons were making steady progress and simply had less machine
  than the numbers assumed.

  So `tests/common::patience` scales a timeout by how much machine there is: at or above twelve
  cores nothing changes, below it the shortfall is the multiplier, bounded at eight because past
  that a hang should be reported as one rather than waited out. `KOLS_TEST_PATIENCE` overrides
  it, since `available_parallelism` reports cores rather than idleness and a loaded laptop looks
  nothing like an idle one. Applied inside `wait_for` rather than at each call site, and to the
  other five test files carrying the same assumption.

  **Verified at the width that failed**: all thirteen `two_nodes` tests pass pinned to four
  cores, in 83s. Two cores still fails one, and that is recorded rather than tuned away — it is
  half the narrowest runner anybody is proposing to use, and raising the bound far enough to
  cover it would trade a hang detector for a coffee break.

  Two notes for whoever touches this next. The helper's own tests live in `tests/patience.rs`
  rather than beside it, because `common` is compiled into every integration binary that
  declares it and unit tests there would run six times and report a count that lies. And
  `cargo fmt` reformats source files this repo has never run it over, so it was reverted off
  everything this change did not otherwise touch — a formatting sweep is its own commit, not a
  rider on a fix.

- **2026-08-20** — **All three repos are public, and CI needs no secret because of it.** The
  first workflow run failed at `actions/checkout` with a 404 on `distributed-intranet`, which
  reads as "that repository is gone" and means "this token cannot see it" — GitHub answers 404
  rather than 403 for a private repository so its existence is not disclosed. The fallback I
  wrote caused it: `secrets.PROTOCOL_REPO_TOKEN || github.token` is right for going public and,
  while private, quietly turned a missing secret into an unintelligible error two steps later.
  Same defect as the one fixed in `kols-cli::secret` the same morning, reintroduced in a
  workflow hours after writing about it.

  A preflight step now probes the API before checkout and fails with the cause named. **Two
  bugs came out of running that step rather than reading it**, which is the whole argument for
  the exercise. The heredoc terminator lands at column 0 only after YAML strips the block
  indentation — true, and worth confirming. And an **empty `Authorization` header is a bad
  credential rather than no credential**: GitHub answers 401 to it even for a public
  repository, so the check would have failed *after* the repos went public, which is the one
  scenario it existed to survive. The header is omitted when there is no token, and the
  post-public path is now the tested one: public repo, no secret, 200, exit 0.

  With the repos public the fallback is the live path and `PROTOCOL_REPO_TOKEN` is not needed.
  It stays supported, because visibility is a decision somebody may revisit and this keeps that
  a repository setting rather than an edit to every workflow.

  History was scanned for keys and credentials before this entry was written, since publishing
  a repository publishes every commit in it. Nothing found.

- **2026-08-20** — **Windows builds move to GitHub Actions, and the repo goes back to being
  OS-neutral.** Cross-compiling from the dev container was the fastest way to get an `.exe`
  into somebody's hands and the wrong place for it to live: it put mingw and a Windows Rust
  target into a Linux development image, and it made the only copy of a binary something that
  existed in one container.

  **The GNU build also cost something at the far end**, which is the argument for a Windows
  runner rather than a tidier cross-compile. An MSVC build links the WebView2 loader in; a
  mingw build *imports* `WebView2Loader.dll`, as a plain import rather than a delay import, so
  the window would not start at all unless that DLL travelled beside it. Building where the
  thing runs removes the whole problem instead of documenting it.

  `gate.yml` runs the tests and clippy on **Linux and Windows** for every push, and that half
  matters more than the release: `kols-cli::secret` restricts a seed differently per platform,
  and both bugs the first Windows run found were invisible from this container. A gate that
  only ever ran where development happens would ship them again.

  **The one thing that depends on the repos being private is written so it stops mattering.**
  CI checks out both repos as siblings, because the client depends on the protocol by path,
  and the default token reaches one repository only — so a PAT in `PROTOCOL_REPO_TOKEN` is
  needed while `distributed-intranet` is private. Both workflows resolve
  `secrets.PROTOCOL_REPO_TOKEN || github.token`, so going public means deleting a secret
  rather than editing a workflow.

  Also untracked `crates/kols-app/gen/schemas`, which Tauri regenerates on every build. Worth
  recording because it is the opposite of what the filenames suggest: `windows-schema.json` is
  regenerated by a **Linux** build, so these were never per-machine artefacts — just build
  output that produced a diff whenever a different machine built.

  **Written blind, and the first run is the test.** Nothing here has executed a workflow.
  Two mistakes were caught by reading rather than running — `cargo tauri` is a different
  distribution of the CLI from the `tauri` the npm package installs, and
  `--no-bundle-fail-on-warning` is a flag I invented — which is a reason to expect more.

- **2026-08-20** — **The window brings somebody in, so no step of the flow needs a terminal.**
  A founder could create a network in the window, run a node and never invite anybody, which
  is not a client you can hand to somebody else — it is a client plus an instruction to go and
  find one. `create_invite`, `waiting` and `admit` cross the boundary the same way everything
  else does, and the rail grows a doorway that is shown only to a holder of `approve-node`.

  **Seeing who is waiting needs the same capability as admitting them**, which is why it is one
  section and one flag rather than two. The waiting room is a local read rather than a command,
  for the reason `kols waiting` is: it is live state in the running node, which writes it down
  for anything else to read, so it is stale by construction and the interface says so where it
  shows it.

  `parse_identity` moved into the library rather than being copied into the shell. Two parsers
  for the same 32 bytes is how two front ends end up disagreeing about what is valid, and this
  is the third thing to make that trip — creating a network and serving one went first.

  **A first-launch bug, reported from Windows and fixed here.** The picker rendered correctly
  and the channel screen sat *underneath* it, reachable by scrolling. `hidden` is an attribute
  and the browser's `[hidden] { display: none }` is the weakest rule there is, so
  `.app { display: grid }` beat it. `[hidden] { display: none !important }` is the fix, and the
  `!important` is right exactly here: hiding is not a style choice a later rule may reasonably
  override, and a user theme (`design/09` §6) must not be able to reveal a screen this client
  decided you are not on. Every other `hidden` toggle in the interface had the same latent bug.

  Unguarded, and said rather than implied: nothing tests that a hidden element is invisible,
  because that needs a browser and this repo has no harness for one. The data path behind all
  three new commands is tested; what they look like is not.

- **2026-08-20** — **O8 closed on a real machine, and the window cross-compiles after all.**
  `kols.exe` created a network on NTFS and its seed's Security tab shows one account and
  nothing else — no inherited `SYSTEM`, no `Administrators`, which is exactly what the
  protected DACL is for and the one thing no build here could prove. Both halves of O8 are now
  observed rather than argued: the refusal from a filesystem that has no permissions to set,
  and the success on one that does.

  **`kols-desktop` was recorded as needing a Windows runner, and that was wrong.** Tauri on
  Windows is widely held to want MSVC, so the claim went in unexamined — twice, into `STATUS`
  and into the Dockerfile. What actually stopped the build was a **missing `icons/icon.ico`**,
  which `tauri-build` needs for a Windows resource file. An `.ico` generated from the 512×512
  PNG already in the repo, and the whole stack — tao, wry, webview2-com — links against mingw.
  A received opinion is not a finding, and this one cost nothing to check and was wrong.

  **What the GNU target changes about the product, which matters for handing it to somebody.**
  An MSVC build links the WebView2 loader statically; a GNU build **imports
  `WebView2Loader.dll`**, and it is a plain import rather than a delay import, so the process
  will not start at all without that DLL beside it. It is Microsoft's redistributable from the
  `Microsoft.Web.WebView2` NuGet package, it is not vendored in this repo, and packaging owes
  it a real answer — Tauri's own bundler does this on a Windows host, which nothing here is.

  Neither binary has been *run* as a window yet. Building is not running, which is the lesson
  of the entry above this one.

- **2026-08-20** — **The first Windows run failed, and both reasons were worth having.** It
  said `kols: Incorrect function. (os error 1)` — which names neither the file it was writing,
  nor what it was attempting, nor the one thing that would have fixed it.

  **The refusal was correct.** `SetNamedSecurityInfoW` returns `ERROR_INVALID_FUNCTION` on a
  filesystem that has no Windows permissions to set, and the run was from a `\\wsl$\` path,
  which is one. So the code did exactly what it was built to do — decline to write a seed it
  could not protect — and then failed at the only other thing it owed, which was saying so.
  A refusal nobody can act on costs more than the check that produced it saves. The message
  now names the file, the call, and the fix.

  **Underneath it was a real bug, and the reason it was silent is the interesting part.**
  `Store::default_root` read `$HOME`, which is normally unset on Windows — so it did not fail,
  it fell back to `"."` and put the store in the *current directory*. A client whose store
  follows the shell around is one that appears to lose a network whenever it is run from
  somewhere else, and nothing would ever have said why. Home is now `USERPROFILE` first on
  Windows and `HOME` first elsewhere, each falling back to the other, because Git Bash sets
  `HOME` on Windows and is common.

  **Set-but-empty is not an answer**, which `or_else` gets wrong: an empty variable would win
  the fallback and then be discarded, throwing away a good second candidate. That is a pure
  function with four tests rather than a line nobody can reach.

  Neither bug was reachable from this container — one needs a Windows filesystem and the other
  needs Windows environment variables — which is the argument for running the binary rather
  than admiring the build. 181 → 186 tests, clippy clean on both targets.

- **2026-08-20** — **`kols` builds for Windows, and the seed it writes there is restricted
  rather than inherited.** O8 was the gate on this and it is now written: `write_private`
  moved out of `store.rs` into `kols-cli::secret`, which restricts a secret to this user on
  every platform it supports and **refuses** on any it does not.

  **The ordering changed with it, and that is the half worth keeping.** It used to write the
  bytes and then chmod them, which leaves the secret on disk under the directory's
  permissions for a moment — and the moment is not the problem, a crash inside it is, because
  the file is still there afterwards. It now creates the file empty, restricts it, and only
  then writes. Restricting nothing costs the same as restricting a seed.

  On Windows the fix is a **protected** DACL, and the word is load-bearing: an unprotected one
  still inherits what its directory hands down, and inheritance is the whole defect — a seed
  in a profile directory picks up whatever that directory grants. Refusing when the DACL
  cannot be applied is the same call `design/00` §2's fail-closed principle makes everywhere
  else: a seed written where somebody else can read it is worse than a seed not written, since
  the second is an error a user sees and the first is one nobody ever does.

  **What is verified and what is not, stated because the gap is the whole risk.** The Windows
  path compiles for `x86_64-pc-windows-gnu` and `kols.exe` links — a real PE32+ binary. It has
  never run. Cross-compilation proves the calls exist with the shapes assumed here and proves
  nothing about the ACL that results, so O8 stays open with its remaining half named rather
  than being closed on a build.

  **Two things this shook out.** Clippy on the Windows target found a warning the Linux run
  cannot see, which means the gate is now two runs rather than one whenever `secret.rs`
  changes. And the confidence in a clean first compile of hand-written FFI was worth checking
  rather than enjoying: a deliberate type error inside the Windows branch proved it was being
  compiled and not quietly skipped by a `cfg`.

  The toolchain went into `.devcontainer/Dockerfile` rather than into this container by hand,
  which is `design/07` §2 S3's own lesson — the environment every claim depends on was once
  the one thing nothing recorded. 178 → 181 tests, clippy clean on both targets.

- **2026-08-20** — **The window creates a network, joins one by invite, and runs a node for
  it.** Three landings, one shape: everything the terminal could do that the window could not,
  because the code lived in a place only a terminal could call.

  **Creating** moved out of `kols init` into `kols_cli::workspace`, called by both front ends.
  That matters more than it sounds, because genesis has three requirements that are each silent
  when missed — `chat-log` on the content-type allowlist, the chat vocabulary registered, and
  `everyone` granted what a member needs — so a second copy in the shell would have looked right
  and failed at the first post. A workspace is a directory of networks, which is forced by the
  same thing that forces a node per network: `keypair_for` derives the libp2p keypair from the
  per-network identity, so networks cannot share a peer id without correlating identities Core
  §1.2 keeps unlinkable. It tolerates a `$KOLS_HOME` that is *itself* a store, because that is
  what `kols --home` means and still does.

  **The node** moved into the shell by giving `serve` a sink: a terminal prints its events, the
  window forwards them to the webview, and the loop knows about neither. The interface re-reads
  on every event rather than patching the screen — `design/05` §3's third property in its
  smallest form, since a record that arrived over gossip is also inside the segment that follows
  and a consumer that appended what it was handed would show every message twice.

  Only one process may run a node per network, and the store now enforces it: the MLS group is
  live state, and two nodes would each advance it without seeing the other, after which whichever
  saved last decides the network's key — with no symptom at the moment it happens. **The claim
  expires rather than only releasing on drop**, which turned out to be the load-bearing half: a
  window is closed by the window manager, which runs no destructors, so a claim released only on
  `Drop` would leak on the *normal* way this application ends. A pid check was the obvious
  alternative and is worse — liveness is a different answer on every platform and a reused pid
  looks alive while belonging to somebody else. Claiming also waits a stale claim out rather than
  refusing on sight, which two of the two-node tests found by restarting a daemon.

  **Joining** got the same treatment as serving, for the same reason. The picker offers it
  *before* creating, because those are not equally likely: somebody opening this client for the
  first time is usually holding an invite. Landing in a waiting room is reported as the success
  it is — under explicit intake an invite buys a connection and an identity and nothing else
  until a member admits you, so the window says so and shows the identity to be admitted rather
  than an empty network, which is what that state looks like when nothing explains it.

  172 → 178 tests. Neither the picker nor the join button can be pressed from a test, so the
  paths behind them are covered directly instead.

- **2026-08-20** — **Seeds are per network, and a password will wrap them rather than derive
  them.** `design/02` §6.3 said first run generates one master seed with a backup phrase. The
  implementation had always done something else — fresh entropy per network — and on reflection
  the implementation is right, so five documents changed instead of the code. The reasoning is
  the one per-network derivation already rests on: a master seed is the single object whose
  compromise correlates every membership at once, and the object a member would be told to write
  down. Recorded as **D28**.

  §6.3 now also says what restoring means, because "restore" promises more than it delivers. A
  seed derives an identity; it is not data and it is not a network. A phrase alone restores
  nothing, since a network's id cannot be derived from it — coming back needs the phrase, the
  network id and an address to reach the network at. Given those three, a member returns as *the
  same member*, and even their own messages come back, because their author log was published as
  content other members hold. What does not come back is named too, including the one that
  actually bites: the list of which networks they belonged to, which lives in the client's
  workspace and in no seed.

  And the shape of credentials is settled before anything builds them, because the tempting
  version is specifically wrong. Deriving a seed from a password and the network id needs no
  storage and is a **brainwallet with a verification oracle**: the network id is public,
  travelling in every invite, and member ids are in the governance log, so a guessed password
  produces a candidate identity checkable offline against a value the network publishes. A
  random seed *wrapped* under a password-derived key has nothing public to check against. Both
  credentials and backup are deferred deliberately and now sit in `design/00`'s roadmap as a
  release gate rather than a phase item.

- **2026-08-20** — **A network designates its relays, and §5.5 was optimistic about needing
  them.** Core §5.5 called bootstrap dependency "temporary per node" — a node caches peers on
  first join and reconnects without the bootstrap "as long as at least one previously-known peer
  is reachable". That clause assumes a reachable peer, and two members behind residential NAT
  are not reachable by each other. For an all-NAT network with no member relay, a bootstrap relay
  is a **standing dependency**, and the spec now says so.

  Underneath it was a structural gap. A hosted relay is **not a member** of the network it
  serves: `RelayNode` runs a restricted behaviour set and does not speak the ledger protocols, so
  it can never advertise itself the way a `relay_bootstrap_willing` member does. Nothing carried
  a newly deployed relay to members who had already joined — their invite is spent, their cache
  names a relay that may be gone. `NetworkPolicy.bootstrap_relays` is that carrier: replayed, and
  changed by `define-policy`. §5.5 now sets out all four carriers and why none replaces the
  others — invite, policy, local cache, ledger.

  Client side: `kols init --relay`, `kols relay list/set`, and `kols serve` reserving a circuit
  and recording it, so an invite carries the one address that works from another network.
  **`kols invite` refuses without a relay**, which is the honest ordering: a network needs one
  before it can invite anybody.

  **A wrong call of mine, corrected, and recorded because the misreading is easy to repeat.** A
  reservation the relay logged as *granted* left the member with no circuit listener, and I read
  that as a transport bug. It is not. A relay promotes listen addresses to external addresses
  only when they are **not loopback** — correct, since 127.0.0.1 is useless to another host — and
  libp2p builds a reservation's address list from external addresses alone. So a loopback relay
  grants every reservation, logs that it did, reports healthy, and hands back nothing. My test
  relay was on loopback.

  Two things worth keeping came out of it. `crates/kols-cli/tests/relay.rs` asserts what nothing
  asserted: that a reservation ends in a *usable* circuit. The protocol's own relay tests count
  grants on the relay side and its wildcard tests filter circuit addresses out to compare source
  ports, so "granted" was covered and "reachable" was not. And `kols serve` now names loopback as
  the likely cause when a circuit does not arrive, because every other vantage point says it
  worked.

  One real bug in the new code alongside it: `serve` appended this node's peer id to every listen
  address, and a circuit address already ends in one — producing `/p2p-circuit/p2p/<id>/p2p/<id>`,
  which nothing dials.

  171 → 172 tests here, 647 → 649 upstream.

- **2026-08-20** — **Invites: one string instead of three hex exchanges, and a protocol gap
  that nothing had hit.** Adding a second person used to mean `attach` with a 64-character
  network id, copying the joiner's 64-character identity back, `admit`, and then being told a
  multiaddr by hand. Now it is `kols invite` on one side and `kols join <that>` on the other.

  **The protocol could not serialize an invite.** Core §5.6 defines it as a credential carried
  out of band — pasted into a message, put behind a link — and the only place its bytes
  appeared was inside a `JoinRequest`, which is the *far end* of that journey. You could issue
  one and have no way to give it to anybody. Nothing had noticed because nothing had yet tried
  to invite a person. `encode_invite`/`decode_invite` landed upstream under their own domain
  tag, with the spec now saying an implementation owes this, since the omission was in the
  specification as much as in the code.

  **Decoding establishes framing and nothing else**, which is tested directly: a tampered
  invite still decodes and fails later at `validate`, where "does this issuer hold
  `approve-node` *now*" can actually be answered. A decoder that verified the signature would
  invite the reading that decoding had already settled something.

  **An invite needs an address and only a running node knows one.** So the daemon writes down
  what it is reachable on and `kols invite` reads that, refusing to mint rather than producing
  a credential that connects to nothing. In reverse, whoever redeems one keeps the addresses it
  carried — so `kols serve` needs no `--peer` afterwards, which was the last piece of manual
  address-passing in the flow.

  **The URI is a container, not a format.** The bytes are the protocol's; the scheme and the
  unpadded base32 are this client's, picked so an invite survives being pasted into a chat
  message and copied back out — tested against leading whitespace, a stripped scheme, wrapped
  lines and lower-casing, because an invite that only works when typed carefully has failed at
  its one job. A truncated one is refused where it was truncated rather than decoding into
  something that fails a signature check somewhere else.

  The waiting room needed the same treatment as the addresses: it is live state in the running
  node, so the daemon writes down who is in it and `kols waiting` reads that — stale by
  construction, and said so where it is shown.

  163 → 171 tests here, 644 → 647 upstream. Both gates green.

- **2026-08-20** — **Display names, and a design document that was wrong about where they
  live.** `design/02` §7 put a member's display name in the mutable pointer they own, next to
  their avatar and status. That works for everything except being unique — a pointer is
  single-writer by construction, so it says what I call myself and has no ordering relative to
  what anybody else published. Two members claiming one name produces two nodes that disagree
  about who is who, with nothing to settle it. **Uniqueness needs a total order, and the log is
  what has one.** Third time this project has reached that conclusion: App Hosting §4.3 for app
  names, D4 for channels, now this.

  So a claim is a `chat` application entry — E2's generic door, no platform change — and the
  avatar and status stay in the pointer, because neither needs ordering.

  **The payload carries no identity, which is the security property rather than an economy.**
  A claim binds whoever authored the entry, whom the protocol already verified, so claiming a
  name for somebody else is unsayable rather than refused. That is what lets `chat:set-name` be
  ordinary and sit on `everyone` at genesis without widening anything.

  **The interesting half is normalization**, spec 07 §3.9.1: NFKC, whitespace trimmed and
  collapsed, lower-cased, invisible code points refused outright rather than stripped — because
  stripping would let two claims that look identical produce one holder, quietly. Every step is
  pinned because two nodes normalizing differently would disagree about what is a duplicate,
  which is a consensus bug wearing the clothes of a display concern.

  **Confusables are deliberately not folded**, and the residual risk is written down rather
  than hidden: `alice` and a Cyrillic lookalike are distinct keys and both may be held. The
  tables are large, they collide names across scripts with every right to exist, and two nodes
  on different table versions would fork. So the obligation moves to interfaces — spec 07 §8
  now requires a name be rendered alongside enough of its holder's identity to tell two apart,
  and both the CLI and the window do.

  **A name is never released, including when its holder leaves.** History renders by author id
  with names resolved at display time, so an inherited name silently relabels somebody else's
  past messages: every line honestly attributed while the conversation becomes a lie.

  Two corrections to the spec came from implementing it. It asked for full case folding, which
  is the better tool and is not available identically everywhere — lowercasing is, and a rule
  everybody applies the same way beats a better rule applied two ways. And it claimed a
  determinism it cannot have: the character categories and the normalization are both defined
  against a Unicode version, so two nodes on different versions can disagree about a name built
  from newly assigned code points. Bounded, rare, resolved by upgrading — and the sharpest
  argument for refusing confusables, whose tables move far more.

  One conflict inside §3.9.1 surfaced only by running it: a tab is a control character, so step
  1 refuses it, while step 3 says whitespace is collapsed. Refusing is the consistent answer and
  the spec now says so — silently collapsing a character the claimant cannot see is exactly what
  step 1 exists to prevent.

  139 → 163 tests, clippy clean.

- **2026-08-20** — **A window, and a build that got smaller while gaining one.** The first
  interface slice: `kols-app` is a Tauri v2 shell holding one `Executor`, `kols-ui` is HTML,
  CSS and one script holding no keys, no sockets and no files. It lists channels, renders one,
  posts to it, and hides the controls this member cannot use — `design/09` §4's first two
  questions and §5's permission-gated chrome.

  **The shell converts rather than deriving, and that is the decision worth keeping.** The
  domain's records have exactly one serialization and it is normative: spec 07 §3's canonical
  encoding, hand-written because a record's id is the hash of those bytes. Putting `Serialize`
  on the same types would have created a second serialization beside the first, and what that
  invites is not hypothetical — somebody eventually sends the convenient one over a wire and
  finds ids no longer match. So `kols-app` owns view types, and `kols-api` has no `serde`
  dependency at all. The webview also never *builds* a command: it names an intent with plain
  arguments and the shell constructs it, which is one fewer place a front end can hand the core
  a shape it did not expect.

  **Adding Tauri made `target/` 7.0 GB, so the profile changed and it is now 2.4 GB.**
  Dependencies get no debug information at all — `[profile.dev.package."*"] debug = false` —
  which is the same cut this workspace already made once for its own crates, made again where
  it now costs the most: webkit, wry, tao, gtk and their bindings dwarf the code. Our crates
  keep line tables, so a backtrace still points at a file and line in this repo, which is what
  a failing test is read through. The build is *smaller than before Tauri arrived*, on a clean
  rebuild with everything green.

  **What could not be verified, stated rather than implied.** The data path is tested and the
  process starts, but the window was not looked at: the X display that S3 proved a Tauri window
  on was dead by the time this landed, and the Wayland path it does run on cannot be
  screenshotted from here. So the layout is unreviewed, and `design/09` §7's first open
  question — the navigation shape — is answered by a first guess rather than a decision.

  139 tests, clippy clean.

- **2026-08-20** — **The event half: the boundary now carries both directions, and the
  vocabulary was written from what the daemon already said.** `design/05` §3 sketches an
  `Event` enum with nine variants. What landed has six, and the difference is the method: the
  daemon has been reporting for weeks — records learned off a head segment, records recovered
  from behind it, a record arriving live, governance entries, an epoch rotation, a member
  keyed in, a handful of degradations it carries on through — and every one of those is now an
  event with something producing it. Nothing was added because a sketch listed it.

  **Two things are deliberately not events.** This node's transport — which addresses it
  listens on, which peers it is connected to — is a fact about the machine rather than about
  the network's content, and a sandboxed build would not be told it at all (App Hosting §3.2's
  "no ambient host access"). And the startup report, which is what this node *is* when it comes
  up rather than something that happened while it ran. Both keep printing; neither crosses.

  **Property 3 is the consumer's, and it comes down to one word.** Events are idempotent and
  re-deliverable not because the emitter is careful but because it cannot be: the live path may
  be lossy, and a record pushed over gossip is *also* inside the segment that follows it. So
  the obligation is to **merge, never append** — a records payload goes through `ChannelView`,
  which is a function of the record set and deduplicates by record id. Five tests hold a
  consumer to it, and the one worth naming is `a_record_that_arrives_live_and_again_in_a_segment_is_one_message`:
  that is the normal delivery pattern, not an edge case, so a consumer that appended would show
  every message twice, every time.

  `Arrival` carries how a record got here — live, off the head, or backfilled with how many
  sealed segments the walk reached. It exists for notification and progress and never for
  ordering, which is asserted directly, because it is the thing a client is most likely to get
  wrong: order is computed from the merged set (`design/01` §4), so the same record renders
  identically whichever way it came.

  The refactor is behaviour-preserving by construction: the two-node tests assert on the
  daemon's exact wording, so `render` reproducing it is what says the change moved code and not
  behaviour. 129 → 134 tests, clippy clean.

- **2026-08-20** — **The executor: one submit path, and a reader-side hole found by building
  it.** `authorize` returned an `Authorized` and each caller then took it apart and did the
  work inline — a gate with no dispatcher behind it, and a sequence a second caller would have
  copied slightly differently. `Executor::submit` is now the one way in: authorize, then run.
  The `Authorized` never leaves the module, because `run` is what requires one and nothing
  else can produce one, so the check is not something a future caller can be *asked* to
  remember. It returns typed `Outcome`s and prints nothing — an executor that printed is one
  no interface could reuse, which is the whole reason there is a boundary.

  **The finding is a pin.** `design/02` §2.2 puts pinning under `chat:moderate`, and the
  boundary requires it — but `ChannelView` admitted a `Pin` record under `chat:post`, like any
  ordinary record. So a modified client holding only posting rights could pin, and every
  conformant reader would have honoured it. **A check the writer makes and the reader does not
  is a check that does not exist**, and this one was decorative from the moment the boundary
  started requiring it. The reader now asks for moderation authority, through a new
  `Authority::may_moderate_now` — deliberately separate from `may_moderate_at`, because the two
  answer different questions: a redaction cites the governance head its author observed and
  keeps standing when that author is later demoted, while a pin cites nothing and should stop
  holding when its author's authority does. An existing test had to change with it, which is
  the honest signal that the behaviour did.

  **Two of §6's debts closed because the executor holds a store and the boundary deliberately
  does not.** An edit or withdrawal aimed at somebody else's message is refused before a record
  is signed, rather than after every reader has ignored it — structurally it was never possible
  to *succeed*, since nobody writes into another author's log, so what this adds is being told.
  And the rate ceiling is enforced over the author's own HLC readings, which is what makes it
  the same verdict on every node: a user typing too fast is told they are, and reader-side
  refusal stays the backstop against a modified client rather than the primary experience.

  **`kols-cli` is a library with a binary on top.** The executor was worth testing without
  spawning a process — a test that had to run `kols` to reach it would be testing argument
  parsing at the same time, and vague about which half failed. The binary is now argument
  parsing and rendering, which is also the shape the desktop client needs: a different front
  end over the same submit path rather than a second copy of it.

  **Every record kind the design describes now runs.** Edits, withdrawals, reactions and pins,
  plus channel rename, topic, slowmode and archive — with `read` printing message ids, because
  every command that acts on a message needs one and a user who cannot see it cannot act. Ten
  new tests drive them through the real binary. 117 → 129, clippy clean.

- **2026-08-19** — **A documentation pass, and it found three stale claims rather than
  none.** The point of reading all of it after a landing is that some of it is wrong, and it
  was: `design/02` announced E11 as outstanding and told a reader the capability registry
  matches names exactly, so a parametrized capability needs an entry per scope — the exact
  problem E11 landed to remove, described in the present tense two rounds after it was gone.
  `design/01` §3.2 still flagged derived pointer ids as a protocol change to make, when E3
  was withdrawn because `PointerId::from_bytes` was already public. And §7 still called
  gossipsub "the recommendation" after E4 shipped it. A design document that describes a
  solved problem as open is worse than one that says nothing, because somebody will plan
  around it.

  **§6 is new: what this client owes, and why each thing is outstanding.** Seven items — the
  executor behind `kols-api`'s gate, the event half and the idempotence property with it, the
  two checks `authorize` deliberately does not make, the commands with no code behind them,
  E12's client half, `may_moderate_at` ignoring the head it is given, and `kols-store` not
  existing. None blocks anything else, which is exactly why they need writing down: a debt
  nobody records is one somebody rediscovers as a surprise. Every entry names where it was
  incurred, so the reason survives without the person who had it.

  Two decisions from the boundary work joined the register in `design/00` §3, since both are
  architectural rather than incidental: **D25**, that authorization is a type rather than a
  call, so an executor cannot receive a command nobody checked; and **D26**, that a command's
  consent class follows the tier of the capability it needs rather than how consequential the
  action looks.

  Also refreshed: `design/00`'s roadmap, which listed P1 as future work while it was under
  way; `design/07`'s status, which said P1 had not started and is now marked complete in its
  own terms, with a line saying its job is finished and `STATUS.md` carries P1 — a build plan
  kept as a running status is a second one to disagree with this file; `design/05` §3 and its
  test table; `design/09` §5, which claimed the enforcement half as future when it is built;
  and the README, which described three crates and 98 tests.

- **2026-08-19** — **`kols-api`: the boundary exists, and the CLI is the first thing to
  cross it.** `design/05` §3 fixes three properties and says retrofitting any of them is
  expensive. Two are now held; the third has nothing to hold yet.

  **`Authorized` is the shape that makes the check unavoidable.** It wraps a `Command`, has
  no public constructor and no public field, so the only way to hold one is to have passed
  `authorize` — an executor takes an `Authorized` rather than a `Command`, and skipping the
  gate stops being a thing a reviewer has to notice and starts being a thing that does not
  compile. That claim is a `compile_fail` doctest rather than a sentence. It is the same
  structural move the protocol repo made with the media relay's guard, where `authorize` is
  the only way to learn a frame's recipients, and for the same reason: a check a caller can
  route around eventually is.

  **`Sensitivity` is derived from the capability vocabulary, not from judgement.** App
  Hosting §3.3 requires a platform prompt before any signed action on the user's behalf, so
  the line that matters is whether a command signs. Above that line the class follows the
  *tier* `design/02` §2.2 assigns — which is why `Pin` is `Governs` and `SendMessage` is not,
  despite pinning being the smaller act: pinning needs `chat:moderate`, which is
  governance-tier. A drift test resolves every command's verb against
  `capabilities::VERBS`, so re-tiering a verb and forgetting this classification fails there
  rather than in a consent prompt that silently stopped appearing.

  **One small thing was missing from `kols-core` and is now there.** Creating a channel can
  only ever be authorized at category or network scope, because the channel's id is minted by
  the entry that creates it and no grant could name it beforehand. `holds_in_scope` is
  `holds` with its first step removed — separate rather than reached by passing a placeholder
  channel id, so a caller cannot invent an id to ask about and have the answer quietly depend
  on it.

  **The CLI crosses the boundary for `channel create`, `post` and `read`**, and the checks
  those used to open-code are gone rather than duplicated: `may_post`, the message ceiling
  and the network-profile test all live in one place now, and `network::require_server` was
  deleted because the boundary answers it. The binary-driving tests pass unchanged, which is
  the only evidence worth having that the seam holds — it is the same argument `kols` itself
  was written for.

  **What is deliberately not in this crate.** No `Event`: the sync engine that would emit one
  is `kols serve`, whose records do not cross this boundary yet, and an event vocabulary
  written before anything emits one is a contract with no implementation to keep it honest.
  No DM, search, voice or stage commands — each has a line in `design/05` §3 and no code
  behind it, and adding the variants now would put a claim in a type nothing could serve.
  Two checks are named in `authorize`'s own docs as *not* done there, with where they are
  done instead: that an edit targets a message you wrote (a fact about the record set, caught
  on read by `ChannelView`), and the message rate ceiling (computed over the author's own
  HLCs, so also the store). A check that looks complete and is not is worse than one that
  says what it does not cover.

  98 → 117 tests, clippy clean.

- **2026-08-19** — **S3 done: the environment builds and runs a Tauri app, and the list of
  what it needed was wrong in two places.** Installing the packages is not the interesting
  part; finding out what the packages actually are is. `design/07` §2 named
  `libappindicator3-dev`, which **does not exist in Debian 12** — the tray library it ships is
  the Ayatana fork — so the environment as specified could not have been built by anyone
  following it. And the display arrives by a different route than assumed: there is no
  `/mnt/wslg` inside the container, because WSLg runs on the *host*; what reaches the
  container is VS Code's own forwarding of both an X server and a Wayland socket, and both are
  live.

  **Confirmation had to be a running window, because nothing weaker distinguishes "the
  packages are installed" from "a GUI works here".** A scaffolded Tauri v2 app compiled and
  linked against the system webview in 44 seconds, and an 800×600 window mapped on `$DISPLAY`
  within a second with its WebKit child process alongside. Built in a scratch directory, not
  in this repo — `kols-app` gets created when it has code, not to hold a smoke test.

  One consequence worth keeping for whoever writes a test that looks for a window: GTK takes
  the Wayland socket when `WAYLAND_DISPLAY` is set, and a Wayland window is invisible to
  `xwininfo`. The app is fine either way; a *script* asserting on a window needs
  `GDK_BACKEND=x11`, or it will conclude nothing opened.

  **A papercut found on the way, worth having now rather than at P1**: the image carried only
  the C locale while `LANG` arrived set to `en_US.UTF-8`, so every GTK process started with
  "Locale not supported by C library" and fell back. That is a poor footing for a client whose
  entire payload is other people's text, and it was one generated locale away from fixed.

  All of it landed in `.devcontainer/` rather than in this shell's history: the Dockerfile
  carries the packages, Node 24 LTS from NodeSource (Debian ships 18, past end of life) and
  the locale; `devcontainer.json` gains `ko-ls/Cargo.toml` in `rust-analyzer.linkedProjects`,
  which had named only the protocol workspace and left half the tree unanalysed, and its
  `postCreateCommand` now checks node, npm and `pkg-config --modversion webkit2gtk-4.1` —
  the dependency whose absence otherwise surfaces as a Rust link error.

  **That config now lives in this repo, at `.devcontainer/`, and is tracked.** It sat at the
  workspace root, which is in neither git repo — so the environment every claim above depends
  on was the one thing nothing recorded. It is the client that needs Tauri and this repo's
  `design/07` that owns S3, so this is where it belongs. Because the client builds against
  its sibling by path dependency, the container still has to see both: `workspaceMount` binds
  the *parent* of the two repos and `workspaceFolder` lands at `/workspaces/ko-ls`, so the
  folder you open is now the `ko-ls` repo and the tree you land in is unchanged.

  **Two build caches, not one.** `ko-ls/target` had been sitting on the bind mount while the
  protocol's was in a named volume — the exact case the volumes exist to avoid — so
  `ko-ls-client-target` joins them. The first build after this recompiles from scratch, and
  the 2.1 GB already in `ko-ls/target` on the host is shadowed rather than removed; it wants
  deleting from outside the container.

  **`distributed-intranet/.devcontainer/` is deleted.** It called itself `dclone-dev`,
  predated the NAT harness and the clippy half of the gate, mounted no docker socket, and was
  referenced by nothing. One config, in one place, that is actually the one being used.

  **The Dockerfile was then built from scratch rather than trusted**, because everything
  above had been proven against a container patched by hand, and "a rebuild has these" is a
  different claim from "this machine has these". A clean `docker build` of it comes up with
  Node 24.19.0, webkit2gtk 4.1, JavaScriptCore, libsoup 3, Ayatana appindicator, patchelf and
  a resolving `en_US.UTF-8` — so the image reproduces the environment rather than recording
  what was done to one instance of it.

  No client code changed: 98 tests here and clippy silent, both unchanged.

- **2026-08-19** — **E12 landed, and landed narrower than it was written.** `design/06` §12
  asked for tiered node liveness — hot, warm and cold, with a warm node holding a relay
  reservation without a full behaviour set. What went into the protocol is **Core §5.1.1:
  peer discovery is optional**, and nothing else. `MemberBehaviour`'s `kad` and `mdns` are
  `Toggle`d and `MemberNode::with_discovery(.., Discovery::Off)` builds a node without them,
  keeping everything else — it still listens, dials, is dialable, relays, hole-punches,
  gossips and serves every request-response protocol.

  **The split is the finding, not an omission.** The behaviour set is the platform's because
  a consuming client cannot assemble a partial one. The tiering is not: whether a node exists
  at this moment, and whether it holds a reservation while nothing is happening, is a decision
  made over time by whoever is holding the nodes, and a specification has no view on it.
  Asking for it in a spec would have put client policy in the platform. Hot, warm and cold
  stay in `design/09` §2, built on reservations and dialability, both of which already existed.

  **How absence is reported is the part that needed care.** `find_providers` and
  `enumerate_collection` return `Option` now, and `None` means *there was no query to run* —
  returning a query id that never resolves would be indistinguishable from content that
  genuinely has no holders, which is the confusion `set_dht_server_mode` already exists to
  prevent. Announcing is a no-op rather than an error.

  **One consequence surfaced in review and is now specified.** The routing table is also the
  address book, so a node without discovery dials by address and never by peer id alone, and
  caching an address against a peer is a no-op there. A pairwise network pays nothing for it —
  addresses arrive with membership — but it constrains any other use.

  Protocol repo: 644 tests, clippy clean, `tests/discovery_off.rs` new. Client: 98 tests,
  unchanged and still green against it. The client half — asking for `Discovery::Off` on a
  conversation-profile network — is owed and blocks nothing.

- **2026-08-19** — **`design/09` written: the interface, before any interface code.** `05`
  fixed the client's architecture and stopped before anything about what the interface looks
  like; eight design documents held two UX commitments between them, both incidental to
  sections about something else. That gap is now a document.

  **Three findings came out of writing it, and two became protocol extensions.**

  *One node per network is forced, not chosen.* `keypair_for` derives the libp2p keypair from
  the per-network identity, so the tempting optimisation — one swarm across several networks —
  would mean one peer id across them and would correlate identities Core §1.2 keeps
  unlinkable. The resource-saving version of the switcher is the one that breaks the security
  model, so it is written down as rejected rather than left to be rediscovered as a
  performance idea. Since a DM *is* a network (`03` §4.3), the node count is
  `servers + conversations` — hence **E12**, tiered liveness, which is P1 and blocks nothing
  else.

  *The wake-up ping does not need to exist.* Keeping a connection open for a quiet DM is
  impossible anyway — Core §5.3 caps a relayed circuit at 120 seconds deliberately, because a
  relay assists connection establishment rather than carrying traffic. But a *reservation* is
  metered separately and is long-lived, so being dialable is the primitive, and **the dial is
  the wake signal**: an inbound stream wakes the handler, with no extra round trip and no new
  message type. This was very nearly specified as its own mechanism.

  *Two people starting a DM must never provision a relay.* The shared network is already the
  rendezvous — `03` §4.3 carries the invite over a stream inside it, with a common-ownership
  proof — so the DM connection can be bootstrapped over the connection that already exists
  (**E13**). Relaying DM traffic through a shared-network node also works and is worse: it
  tells a third party who is talking to whom, where address exchange tells nobody anything
  they did not know. It stays as the fallback, and Core §5.3's correction says a stateless
  bootstrap relay "carries bytes and never inspects a join at all", so that fallback needs no
  protocol change. E13 carries one hard gate: addresses for network X are disclosed only to a
  member of X, or it becomes an oracle for enumerating a user's other identities.

  On theming, the security question resolved cleanly rather than being traded away. CSS can
  exfiltrate — attribute selectors plus any URL-loading property — but it has exactly one way
  to do it, a network request, and `url()`, `@import` and `@font-face src` are the complete
  set. Under a CSP permitting no remote origins, arbitrary user CSS *cannot* phone home. What
  CSP does not solve is spoofing, so security-critical surfaces render as native dialogs
  outside the themeable DOM: a theme may make the client unrecognisable and can never fake a
  signature prompt.

- **2026-08-19** — **E11 landed: a registry entry can cover a namespace.** An extension
  capability's tier came from a registry matching names *exactly*, and chat capabilities are
  parametrized by scope — so `chat:post:<channel>` needed an entry per channel, added by a
  policy change. Creating a channel with a permission override meant amending network
  policy, and the registry grew with the channel count forever. A registration ending in
  `:` now covers the namespace beneath it, longest match winning (Core §2.2.1).

  **The separator requirement is the part `design/06` §11 left open, and it mattered.**
  Plain prefix matching would let a registration for `chat:post` also cover
  `chat:postmortem` — a different capability that merely starts with the same letters,
  silently inheriting a tier nobody chose for it. Requiring a namespace to end at a
  separator means it can only cover names genuinely within it, and it is also what keeps
  every existing exact registration exact.

  **Removing the workaround showed it had never actually worked.** `design/06` §11 described
  it as registering "a scope's names when that scope is created", and nothing in the client
  ever did that: `for_category` and `for_channel` existed and were called only by tests. So
  `chat:<verb>:<channel>` was in no registry anybody wrote, a per-channel grant was refused
  at replay, and the per-channel override `design/02` §4 describes could not be used at all.
  That is now a test rather than a discovery waiting to happen.

  The drift test between what the client registers and what it declares got stronger on the
  way through. It could previously only check the network-wide `:*` name, because scoped
  names were supposedly registered elsewhere; it now resolves *every* form a declaration can
  take against the real registry, including a scope invented after genesis.

- **2026-08-19** — **A key per segment, and a freshness bound on the live path.** The two
  gaps the sealing work left open, closed.

  **Per-segment keys were forced, not chosen.** `MutablePointer::update` carries
  `dek_commitment` forward unchanged — deliberately, since Storage §1.2 fixes a DEK for its
  object's lifetime — so a pointer commits to one key for its entire life, and every
  segment sharing a pointer shares a key. A key that opens the newest message then opens
  the oldest, and retention can only ever forget a whole log. There is also no way to smuggle
  a key in beside it: `PointerRequest::Fetch` returns wrappings only alongside a pointer
  record that exists, so a wrapping for a pointer nobody published never syncs. A
  separately-forgettable key therefore needs a pointer of its own, and that settles the
  design: `author_segment_pointer(channel, author, sequence)`, one per segment.

  Sealing now starts a new segment **and** a new key, and the sealed segment keeps the key
  it was written under — nothing is re-encrypted. Re-keying it would move every CID in it,
  forcing every reader holding it to refetch the whole object, and would leave the
  superseded ciphertext readable under a key nothing retires, which is the exact thing this
  makes possible to avoid.

  The cost is one indirection on the read side. `author_log_pointer` — the derivation a
  reader computes from public information alone — no longer names the messages; it names a
  head index, an otherwise empty segment whose `sequence` says which segment is current.
  Reusing `Segment` for it meant no new type, encoding or content type. Its pointer version
  **is** the sequence, and that is not cosmetic: same-version pointer records are settled by
  lower record hash, so an index republished at version zero would lose that coin-flip
  against the copy peers hold about half the time, and newer history would never be found.

  Two things fell out of building it. Reading past a retention boundary is deliberately
  indistinguishable from history that has not arrived — a missing wrapping, a wrapping under
  an epoch this node has not caught up to, and a segment retired last year all look the
  same, and a client claiming "this was deleted" would assert what it cannot know. And
  because a retired boundary never becomes readable, a walk would have re-fetched,
  re-decrypted and re-verified every signature behind it on every tick forever; the store
  now keeps each segment's chain link, so a re-walk costs file reads and no crypto.

  **The live path's retry is now bounded.** It exists to shave latency off records being
  written now, but an unbounded retry made its retry set everything the node ever wrote, so
  an author's whole history went out over gossipsub the instant anybody subscribed. §6.1
  says nothing may *depend* on that path; it does not license the path to substitute for the
  durable one. Records outside the window are retired from the set rather than skipped, so a
  tick stops rescanning history to decide against it again.

  One test bug worth recording: the new live-path test reused ports another test already
  had, so two daemons shared a log file and one could not bind — which surfaced as a daemon
  whose log went *empty* after previously containing output, not as a bind error.

- **2026-08-19** — **Segments seal, and readers walk back through them.** Backfill was
  supposed to be the reader half of a model already in place. It was not: `AuthorLog::seal`
  existed and the daemon never called it, so every author log was a single ever-growing
  segment. That has a consequence worth stating plainly, because it inverts the reason for
  doing the work — **a reader already saw all of an author's history**, since the one
  segment held it. The gap was not scrollback. It was that opening a channel cost the whole
  conversation rather than a screenful, which is exactly what `design/01` §5's
  "head segment only" exists to prevent.

  So sealing landed first, on §3.1's two thresholds, and the walk after it. Sealing needs
  **no persisted state**, which is the part worth keeping: boundaries are a pure function
  of the record sequence, so a node that restarts and replays its store re-derives the same
  seals and republishes the identical chain. That only holds because age is measured across
  a segment's own records rather than against the clock — "older than a day *now*" would
  seal somewhere new on every restart and publish a chain competing with the one readers
  already hold.

  Two bugs, both found by the test rather than by reading. The walk marked a segment
  absorbed as soon as its records were stored, and stopped at the first marked segment — so
  a reader took exactly one hop of history and then stopped, permanently, while *reporting
  a successful backfill*. A mark now means "this and everything behind it", which can only
  be set once the chain bottoms out. And the test itself was passing for the wrong reason
  until `--no-live` existed: an author retries a record that failed to publish, so its
  entire backlog goes out over gossipsub the moment a peer subscribes, and Bob was learning
  all thirty messages live. That retry is still unbounded — spec 07 §6.1 says nothing may
  *depend* on the live path, not that it may substitute for the durable one — and it is
  logged in §1 rather than fixed here.

  `kols serve --no-live` also closes a MUST: §6.1 requires conformance be testable with
  gossip disabled, and the test that claimed to do that had been arranging for the two
  daemons never to overlap, which held only while there was a single record to race on.

- **2026-08-19** — **E4 landed: records arrive live as well as durably.** Gossipsub joins
  `MemberBehaviour` (Core §5.1) as the one broadcast primitive in a stack that is otherwise
  pull-based — and the exception proves the rule, since everything else carries state a
  partitioned node must be able to obtain *late*, which a broadcast cannot provide. A live
  payload is the opposite case: one nobody needs to receive at all.

  Three configuration choices are load-bearing rather than incidental. **Signing is off** —
  a record already carries its own signature over its own canonical bytes, so signing them
  again with the transport keypair would leave a receiver with two authorities for "who
  wrote this" and no rule for choosing; `gossip_behaviour` takes no keypair, so the absence
  is visible in the signature. **Message ids are content hashes**, not the default sender
  and sequence, because the same record legitimately arrives twice — once live, once in a
  segment — and deduplication has to agree with the consumer's own content addressing.
  **The transport validates nothing**: it does not know what a payload means, and half a
  check would be worse than none, since a caller would read it as done.

  The payload is sealed under spec 07 §5.2's channel content key, derived from the epoch
  and bound to both channel and rotation — so it cannot be read by a non-member sharing the
  mesh, cannot be relayed into another channel, and survives publication either side of a
  rotation because it carries the `rotation_ref` its sender used.

  **Three things this shook out, and the third was my own test being wrong.** A record was
  marked broadcast even when the publish failed for want of a subscriber, so a message
  posted moments before a peer subscribed never went live at all — the one case the path
  exists for. Landing it then broke two passing tests, which was the useful finding: they
  waited on the durable path's "learned 1 record", and a record arriving live is *already
  stored* by the time the durable absorb runs, so that line correctly never printed. Both
  paths now report in the same words with the path named, because two vocabularies for one
  event make "did this arrive" depend on which way it came.

  The third: the new test waited for the record to reach the peer *live* and kept failing —
  and the path was fine. On loopback the durable path is milliseconds too, so which arrives
  first is a race, and §6.1 promises only that the record arrives. The test was demanding
  something the design explicitly declines to offer. It now asserts what is actually
  promised: the record demonstrably goes out live (a publish only succeeds once somebody is
  subscribed, so the daemon reports that separately), it arrives, and it lands **exactly
  once** however many paths carried it.

  The gossip-disabled case §6.1 requires be testable is covered by never overlapping the
  two nodes: Alice writes and stops before Bob runs, so nothing can reach him live and
  everything he ends up with came through the durable path alone.

  **A third finding, and the worst of the three: the daemon was starving its own CLI.**
  `exclude_removed_members` took the append lock on *every tick* — thirty acquisitions a
  minute, each held across a full log read — while one-shot commands need the same lock to
  append at all. `kols admit` timed out waiting for it. The check now runs *before* locking
  and only locks when there is genuinely a member to exclude, which is almost never. The
  lock guards appends; reading never needed permission, and treating it as though it did
  made the daemon and its own commands compete for a resource neither of them was really
  using.
- **2026-08-19** — **Retention landed as two windows, and a catch-up bug was found asking
  what the default should be.** The question was what to set for retiring superseded epoch
  keys, with the worry that somebody absent for thirty days would come back locked out.
  Checking that premise found it was not the failure mode — every rotation carries an MLS
  commit, so an absent member derives the keys it missed by replaying them (Core §3.3), and
  the log never shrinks. **But the daemon never called `apply_pending_rotations`**, so
  nobody caught up on anything. It stayed invisible because an object keeps its DEK for
  life: an absent node could still read appends to logs it already knew, and only a *new*
  object under an epoch it never derived would fail — as content that fetched perfectly and
  would not open. Fixed, with a three-node test where a member is offline while somebody
  joins and posts.

  That also settled the shape of the setting. Retiring keys is not a knob: a key is
  droppable once nothing inside the retention window is still wrapped under it, which the
  re-wrap-on-read path already arranges. A separate key-lifetime setting could contradict
  the retention window — keep content a year, drop its key at six months — and make
  retained content silently unreadable.

  So it is **two content windows, not one**: `chat:retain-messages-days` and
  `chat:retain-attachments-days`. A message is capped at 8 KiB with a capped rate, so a
  million of them is a few gigabytes network-wide; one attachment may be 25 MiB, ten to a
  message. A network bounding what it spends on other members' disks means the attachments,
  and a single window would charge it the scrollback too.

  **Both default to `Forever`, which revises `design/01` §8's earlier rolling-window
  default.** The argument is asymmetry rather than taste: retention can be switched on
  whenever a network wants it, and content already allowed to go dark cannot come back — so
  a network that never thinks about the setting should keep its history. Zero, negative and
  absurd values all read as `Forever` for the same reason. A log is judged on its *newest*
  record: one somebody is still writing to is live however far back it reaches.

  Attachments have a window and nothing yet to apply it to, since the CLI does not carry
  attachments — the policy is readable and honest about that rather than pretending. 78 → 83.
- **2026-08-19** — **Made "try every epoch key" stop growing without bound.** Raised as a
  scaling worry and it was a fair one, though the proposed fix — re-encrypting old content
  under a new key — is the one thing this design must never do: a per-object DEK is fixed
  for the object's lifetime (Storage §1.2), which is what makes chunk encryption
  deterministic, which is what makes delta-fetch work. Re-encrypting re-chunks everything
  and destroys the 1,556-of-176,115 property the segment model exists for.

  What gets refreshed is the **wrapping** — a 48-byte record, one AEAD seal — which Storage
  §5.3 already specifies and `design/05` §4 already lists as daemon maintenance nobody had
  written. `design/01` §8's retention is the same lever inverted: content that stops being
  re-wrapped goes dark on its own.

  **Measured before changing anything**, at ~720ns per unwrap attempt: 0.72ms to scan a
  thousand keys, 3.6ms for five thousand. Survivable per unwrap; the real cost was that
  `absorb_segments` called `epoch_keys()` *inside* a channels × members loop, and that reads
  and AEAD-opens every stored key from disk — a thousand directory scans per two-second
  tick on a network that had rotated a thousand times, dwarfing the crypto it was there to
  serve. A wrapping that opened under no held key paid the full scan forever, every tick,
  never converging.

  Three changes, all correctness-preserving: keys come back **current-first**, so a
  refreshed wrapping opens on the first attempt; a wrapping opened under an older key is
  **re-wrapped under the current one**; and a DEK learned from somebody else's wrapping is
  **remembered**, so the scan happens once per object rather than every tick. Foreign DEKs
  are checked against the pointer's own commitment, so a stale cache — the author sealed
  that object and started another — is discarded rather than used to fail at decryption.

  **Retiring old keys is deliberately not done here.** Dropping a key makes anything still
  wrapped under it unreadable forever, so it is `design/01` §8's retention decision rather
  than a cleanup to do quietly. Next. 77 → 78 tests.
- **2026-08-19** — **`kols revoke` closes the revocation path, and found two bugs doing it.**
  The command writes the membership removal; the daemon rotates the epoch to exclude them.
  The split is not convenience — rotating needs the live MLS group only the daemon holds, and
  Core §3.3 requires the removal to be logged *first* anyway, because a rotation minted while
  somebody is still a member produces a key they remain entitled to and §3.1 says a key
  cannot be un-known afterwards. `revoke` says so in its own output rather than implying the
  job is finished when it returns.

  **The first working revocation deleted a channel**, and the cause is worth keeping. The
  store has two writers — one-shot commands and the daemon — and each parented its entry on
  the head *it* last saw. `channel create` and the daemon's admission rotation landed on the
  same parent, forking the log; fork-choice picked the rotation branch and voided the
  channel. That is the protocol working exactly as specified, and a disaster in a client that
  never noticed. There is now an append lock (an atomic `create_dir`), held across
  read-head-then-write by every writer, and the daemon adopts whatever the store gained
  *inside* the lock before appending so its parent is the real head.

  **The second bug only a rotation could reveal**: reading a DEK unwrapped it with the
  *current* epoch key, but a wrapping is made under whichever epoch was current when it was
  written. The moment anything rotated, a node could not open its own content — Alice could
  not post to her own channel after removing somebody. It now tries every held key and
  re-wraps under the current one, which is what Storage §5.3 means by any current member
  re-wrapping on rotation.

  Both were reachable only once rotation existed, which is why they surfaced today rather
  than when the daemon was written. 75 → 77 tests.
- **2026-08-19** — **MLS group state survives a restart, and the spec says it must.** The
  last confidentiality gap, and it was a gap in the *specification* as much as in the code:
  §3 asked members to rotate, welcome and revoke without ever saying that the state those
  need has to outlive the process holding it. An implementation reading §3 alone keeps it in
  memory, which is what every MLS library makes easy, and the result is survivable in a test
  and not in a deployment. **Core §3.3.1** now states the obligation and its three
  properties: a restored member derives the same key, can still advance the epoch, and fails
  rather than half-loading.

  `GroupSession::save`/`restore` in `intranet-epoch`, `save_epoch_group`/`restore_epoch_group`
  on `MemberNode`, and `kols serve` restoring its group on startup and re-saving whenever the
  group advances. A founder who restarts can now key somebody in — before this, they kept
  their epoch keys, could still read, and could never welcome anybody again, with no symptom
  until the next person tried to join.

  Two implementation notes worth keeping. openmls gates `MemoryStorage::serialize` behind its
  `test-utils` feature and says so in a comment, so the save path reads the storage's public
  `values` map directly rather than turning a test-only feature on in a shipping build. And
  the signature key pair goes *into* that storage via `SignatureKeyPair::store` rather than
  travelling as its own field, because openmls's accessor for the private half is likewise
  test-only — `restore` reads it back out with `read`.

  **The blob is secret** — the group's secret tree and the member's signature private key,
  jointly enough to impersonate them and read the network — so it is sealed at rest under the
  same seed-derived key as the epoch keys, and the spec section says plainly that a client
  writing it in the clear has given away the network. Six conformance tests upstream, and one
  in the client that restarts a founder and has them key in somebody new who then reads
  pre-restart content. Protocol 626 → 632.
- **2026-08-19** — **Two nodes hold a conversation.** `kols serve` is a real node: it
  listens, syncs the governance log, advertises capacity, publishes this member's segments,
  pulls other members' pointers, fetches their segments and stores the records. `attach`
  bootstraps a store from nothing but a network id, and `admit` is the explicit-intake half
  of `design/02` §6.2. A joiner now goes from knowing one hex string to reading messages
  written before they existed, with nothing shared out of band.

  **The daemon owns the MLS group, and that is why it exists.** `GroupSession` keeps live
  state in an in-memory provider, so keying somebody in needs a process that has not exited
  since the group was made — which a one-shot command can never be. `init` therefore no
  longer creates the group; the founder's first `serve` does. The cost is stated in `init`'s
  own output: serve before posting.

  **Four bugs, each invisible to every existing test**, which is the argument for testing
  the binaries rather than the libraries:

  - A joiner could not advertise capacity before syncing, and could not sync without
    advertising. Startup work is best-effort now and retried after every governance sync,
    because "not a member yet" is a state to sync out of rather than fail on.
  - A fetch was requested once. It needs **two rounds** — the manifest, then the chunks it
    names — so remembering "already asked" left every segment permanently half-fetched.
  - Only the newest epoch key was kept. Adding a member *rotates* the epoch, so content
    written before a joiner arrived is wrapped under an earlier key: it fetched perfectly
    and decrypted never. The store now holds the whole keyring, and unwrapping tries each.
  - `read` on a fetched segment must take the DEK from the pointer's **wrapping**, not from
    the local store — the local path mints a DEK for objects this node owns and would have
    produced a key that opens nothing.

  **One design consequence worth knowing before it surprises somebody:** `post` now requires
  `serve` to have run, because only the daemon can mint the first epoch key. Every test that
  posts starts a node first, which is what a user does anyway.

  **A fifth bug, found by fixing the fourth's leftovers**, and it is the one worth
  remembering: the daemon re-asked for the governance log and for pointers on every tick,
  but never for the **capability ledger**. Source selection drops a holder that has not
  advertised capacity, as not having volunteered — and a joiner advertises only once
  admitted, which is *after* the ledger exchange that ran when it connected. So a joiner
  stayed permanently unrankable as a source: its pointer arrived, its DEK wrapping arrived,
  and the chunk itself never did. The crate's own guidance says a fetch that mysteriously
  finds nothing is usually exactly this, and it was. The tick now re-asks for all three.

  With that, a reply travels between two live daemons with nothing restarted, which the
  second two-node test now asserts. 72 → 74 tests.
- **2026-08-19** — **Content keying moved onto the real path.** Raised as a concern, and it
  was a fair one: the CLI's first cut derived its DEK from the network id, which is public
  in every meaningful sense — it is in every invite, every address and every log entry — so
  the only thing keeping a non-member from reading a segment they had obtained was honest
  nodes declining to serve them. A serving policy standing in for cryptography.

  It now does what Storage §5 specifies. Every author log gets a **random** DEK; what
  persists is the DEK **wrapped under the network's epoch key**, which is exported from a
  real `GroupSession` created at `init` rather than derived from anything public. The epoch
  key is sealed at rest under a key derived from the master seed, because
  `EpochKey::expose_for_delivery` states outright that storing it unsealed defeats the
  guarantee the module exists to provide. Secrets are written `0600` and a test asserts it.

  **What is still missing is rotation, and it is a real gap rather than a rounding error.**
  Core §3.3 advances the epoch on every membership change, and that is what stops a removed
  member reading anything published afterwards. Advancing it needs live MLS state, and
  `GroupSession` holds an in-memory openmls provider with no persistence — so a process that
  exits cannot rotate. A removed member keeps the key, which is precisely the naive scheme
  Core §3.2 rejects. Two ways out, both real: `intranet-epoch` growing a persistent storage
  provider, or a long-running node here holding the group in memory. The second is the same
  daemon the wire half needs, so they likely land together.

  Three tests pin what changed: two networks share no key material and no DEK; secrets are
  unreadable to other users; and a store whose epoch key has gone refuses rather than
  minting a fresh one, since silently re-keying would produce a node writing content nobody
  else can read while looking like it works. 69 → 72.
- **2026-08-19** — **`kols` exists, and the project is runnable for the first time.**
  Chosen over E4 deliberately: gossip is an optimization of a path that already works
  (`design/01` §7 — "a client with gossip entirely disabled is slower and completely
  correct"), while nothing had ever composed genesis, permission resolution, channel
  entries, author logs and rendering into one path. That seam had a bug in it the same day
  it was written, which is the argument in miniature.

  `init`, `whoami`, `channel create`, `channel list`, `post`, `read` — persisted between
  invocations, replaying a real governance log each time rather than caching state.
  Seven tests drive the actual binary rather than the library, because what is under test
  is that separate processes agree through the store, which an in-process test would
  share its way past.

  Three things worth carrying:

  - **Genesis has three requirements and each is silent when missed.** `chat-log` must be
    on the content-type allowlist, the chat vocabulary must be registered, and `everyone`
    needs `publish:chat-log` alongside `chat:post`. Miss any one and the network looks fine
    until the first post is refused by the author's own node. `kols-cli::network::genesis`
    is now the one place that gets it right.
  - **The CLI's DEK started as a stand-in and was replaced the same day.** The first cut
    derived it from the network id — which travels in every invite, address and log entry —
    so anyone who ever saw the id could decrypt any segment they obtained. It now does what
    Storage §5 actually specifies: a random DEK per author log, **wrapped under an epoch key
    exported from a real MLS group**, with only the wrapping persisted and the epoch key
    itself sealed at rest under a seed-derived key. `EpochKey::expose_for_delivery` says
    plainly that storing it unsealed defeats the guarantee, so it is not stored unsealed.
  - **A network name is not a policy value.** Spec 07 defines no key for one, so the CLI
    keeps a local label rather than inventing vocabulary the normative document lacks —
    which is how two clients end up disagreeing about what a network is called.

  **What it cannot do: reach another node.** `kols-net` publishes and fetches between two
  live nodes in tests, and nothing in the binary drives it. That is the next task, not a
  gap being glossed. 62 → 69 tests.
- **2026-08-19** — **Chat channel entries landed, and spec 07 gained the bytes they
  needed.** E2 put channel structure in one generic application entry rather than four
  chat-shaped variants; `kols-core::channel` is the chat side of that. All four kinds —
  definition, update, membership, rotation — encode as `chat`-namespace payloads under a
  new `intranet.chat-channel-entry.v1` tag, with the header mirroring a record's so the
  reasoning transfers: no `network_id`, because the channel id already derives from it.

  **A contradiction in the normative spec had to be settled first, and the user chose.**
  §1.3 said a channel definition must declare `chat:manage-channel`, while §4.1 listed
  `chat:create-channel` as Ordinary and then never used it anywhere. Resolved the way
  `design/02` §2.2 always had it: **creating is ordinary, changing is governance**, because
  a definition grants nobody access to anything — a new private channel has an empty roster
  until a membership entry adds someone, and that entry is the governance-tier one. The tier
  follows what an action can widen, not how consequential it sounds. Spec 07 §1.3 corrected,
  §3.8 written to carry the mapping normatively.

  **Both obligations E2 moved onto readers are now unavoidable rather than optional.**
  `ChannelEntry::read` is the only way to get an entry out of a log body, and it runs the
  capability check and the profile check on the way through; decoding bytes directly is
  still possible but is named `decode_payload` and yields only a value. The writing side
  declares its capability *from the entry's own kind*, so a client cannot publish one
  declaring something else — the same structural move as `authorize` in the protocol's
  media limiter, and for the same reason: a check a caller can route around eventually is.

  One decision worth keeping: an unallocated discriminant in a channel entry is **refused**,
  the opposite of the rule for record kinds. An unknown record is retained, counted and not
  rendered (`design/08` §9); an unknown channel entry carries structure, and a reader that
  skipped it would hold different channel state from one that understood it.

  Frozen vectors included, since §3.8 is normative now and a change to one is a wire break
  rather than a value to re-bless.

  **A bug found immediately afterwards, by wiring it to a real governance log.** The entry
  declared a *fixed* capability per kind, which made channel creation work for Founders and
  nobody else: an extension capability resolves by exact name, so a member holding
  `chat:create-channel:*` — the grant `capabilities::network_scoped()` exists to register,
  and the whole reason the verb is Ordinary — was refused, because the entry declared a
  channel-scoped name they did not hold and which nobody *could* have registered in advance,
  the channel id not existing until the entry creating it does. The declaration is now
  chosen from what the author actually holds, narrowest first, and refuses up front when
  they hold nothing rather than producing a log entry every node rejects.

  The lesson is worth more than the fix: **what an entry declares has to be a name its
  author was really granted, not the one that best describes the action.** The encoding
  tests could not have caught it — they build values by hand, which is right for a wire
  contract and blind to whether the protocol accepts what this client produces.
  `channel_governance.rs` closes that, replaying a real log end to end. 37 → 62 in this
  repo.
- **2026-08-19** — **A relay can now refuse, and its advertisement binds.** Found by asking
  what actually stops a volunteer being asked for more than it offered: nothing did. A
  node's `bandwidth_cap` was read by every node *except* the one that declared it — it
  steered other members' relay and source selection while the volunteer enforced nothing,
  so a user could set a limit, watch the client ignore it, and have no way to tell. That was
  survivable while a relay forwarded one envelope per envelope received. Fan-out made it a
  multiplier, which is why this follows E5 rather than standing alone.

  Specified in Real-Time §2.2.2 as a requirement to *have* ceilings without prescribing
  values, and implemented as `intranet-transport::media_limits`: concurrent calls,
  participants per call, and sustained bytes forwarded — charged for what **leaves** the
  node, since charging the inbound size under-meters by exactly the fan-out factor the
  ceiling exists to bound. Kept out of network policy deliberately: a ceiling describes one
  node's hardware, so a network able to set it could compel a member to spend bandwidth it
  never offered, which inverts Core §4.3's opt-in.

  Two things the shape had to get right. `MediaRelayGuard` owns the participant sets, so
  `authorize` is the only way to learn a frame's recipients and it charges in the same call
  — the same structural answer `relay_limits` uses against a limiter that computes a verdict
  and never enforces it. And a fan-out that does not fit is refused **whole**, because
  serving some participants and not others turns a bandwidth ceiling into silent one-sided
  call degradation, which is worse to experience and harder to diagnose than a refusal.

  Thirteen tests: eleven unit over the charging arithmetic, refill curve, a backwards clock
  and a carrier that is also a participant, plus two over live nodes. `relay_call` now
  returns `Result`, and refusing is ordinary — the call renegotiates onto another relay
  exactly as it would if this one went offline, so the client shows nothing.

  One bug caught in this change's own code, by writing the test to exercise the real path
  rather than the convenient one: changing a node's limits replaced the guard wholesale and
  silently dropped every call it was carrying, which would have hung up on everyone with no
  explanation at the far end. Limits now change in place — a lowered ceiling stops this node
  taking new work without retracting agreement already given, and a raised one grants no
  allowance, since allowance is earned from elapsed time and a config change that minted it
  would be a rate limit anyone could step around by toggling a setting.
- **2026-08-19** — **E5 landed, pulled forward from P3.** The relay reduced nobody's upload:
  `MediaEnvelope` carried one `to`, so a sender in an n-party relayed call emitted n−1
  envelopes and the relay added a hop to each. It now carries `Recipient::{One,
  Participants}` — one envelope in, n−1 out — specified in Real-Time §2.2.1 with the
  implementation, ten live-node tests and four encoding tests. Three things the proposal
  had not worked out, each found while building it: the fan-out form deliberately carries
  **no recipient list**, so it is stricter than the form it replaces rather than looser;
  the relay must **readdress every forwarded copy**, or a participant that also relays
  would fan the same envelope out again; and the relay must **bind the claimed sender to
  the connection**, a check that never existed on this path and that fan-out turns from one
  stray frame into n−1 sends at the relay's expense. The wire break is versioned rather
  than smuggled — the envelope's domain tag is now `intranet.wire.call-media.v2`, so a v1
  envelope fails to decode instead of parsing its recipient out of the wrong bytes.

  The gotcha `design/05` §4 carries about `next_swarm_event` draining its buffer on entry
  turned out to be live rather than theoretical, and caught this change on the way through:
  a relay that is itself a participant buffers its own copy of a frame, and in a call whose
  only other participant is the sender there is nothing to forward alongside it — so the
  buffered event waited for unrelated traffic that on a quiet call never comes. It is
  returned directly in that case, with a test whose deadline is deliberately tight, since a
  generous one passes under both behaviours and pins nothing.

  The client's advice to stay in mesh (`design/04` §3.1) is withdrawn.
- **2026-08-19** — **Protocol repo test count refreshed.** Its README claimed 591 in two
  places, which was true when written and went stale when E9 and E2 landed — each added
  conformance tests and neither updated the figure. Measured rather than inferred this
  time — **613 passing, clippy clean** — and worth recording how, because the first two
  attempts disagreed: `cargo test --workspace` piped into `grep` **undercounts**,
  because cargo interleaves the output of concurrently running suites and a result line
  that lands mid-line no longer matches `^test result` — two piped runs of identical source
  reported 571 and 612. Redirect to a file and count there. The tree at `HEAD` was 604, so
  591 was thirteen tests stale, which is about what E9 and E2 added between them.
- **2026-08-19** — **E2 landed, but generically.** Four chat-shaped entry variants would have
  repeated the mistake E9 avoided, so the log gained one application entry instead:
  namespace, kind, declared capability, opaque payload. Two claims weakened honestly as a
  result — the protocol enforces the capability an entry *declares* rather than the right
  one, and rejecting channel entries in a conversation network is now every conformant
  reader's job rather than the protocol's. Application entries also do **not** count toward
  branch length, reversing this design's earlier reasoning, because the metric cannot
  resolve a capability's tier. A variant-discriminant collision found on the way is now
  guarded by a round-trip test over every entry body, including distinct action hashes.
- **2026-08-19** — **E9 landed in the protocol repo.** `NetworkPolicy` now carries namespaced
  application-layer values it stores, orders, encodes and hash-covers without interpreting —
  the same division `extension_capabilities` already used. Specified in Core §2.6.2, six
  conformance tests, both repos' gates green. `kols-core::policy` reads the chat settings
  out of it, including the network profile, with defaults applying when a key is absent.
- **2026-08-19** — **Conformance obligation 6 met.** The encoding now runs on a big-endian
  target: `scripts/cross-check.sh` cross-compiles `kols-core` to s390x and runs the suite
  under qemu, where all 30 tests pass including the frozen vectors. Byte order was correct
  by construction before this — no native-endian conversion, no transmute, no unsafe
  anywhere in the path — but that was an argument, and this replaces it with a run.
- **2026-08-19** — **F3 done: design set reviewed and bumped to v1.0.** The pass earned its
  keep: `08` stopped being normative, since its content moved upstream in F2 and two
  normative descriptions of one wire format is the drift this project criticises elsewhere.
  It also caught the set claiming no implementation existed, a roadmap listing P0 as future
  work, a scale section calling two now-measured numbers guesses, and a test plan with no
  column for what had actually been written. Four new decisions recorded (D22–D24 plus the
  precedence rule).
- **2026-08-19** — **F2 done: spec 07 written and committed upstream.** The first
  application-layer spec, carrying the channel and record model, the normative encoding,
  the capability vocabulary, the keying tiers, and §7's five platform amendments. Written
  from a working implementation rather than ahead of one, so its two most consequential
  rules — the count-free record list and per-device HLC strictness — arrive with the
  measurements that produced them. Protocol repo stays green: 591 tests, clippy clean.
- **2026-08-19** — **P0 closed: the wire half works.** `kols-net` publishes a segment
  (store, announce per chunk, accept pointer) and reassembles one from fetched chunks. Two
  live `MemberNode`s: 120 messages cross the wire and render identically on both sides, and
  a reader holding the previous version refetches **1 chunk of 3** after an append. Three
  documented gotchas were all real — Kademlia client mode had to be turned off, ledger
  advertisement had to precede the fetch, and the manifest needs its own round before the
  chunks it names.
- **2026-08-19** — **P0 criterion 5 met.** `AuthorLog::rebase` implements the content-merge
  semantics Storage §2.2 deliberately left to the application layer: union by record id,
  republish at the next version, nothing dropped. Two findings recorded in the design —
  HLC strictness is per **(author, device)** rather than per author, since two devices
  cannot coordinate a shared counter without a lock; and rebase cost depends on where the
  loser's records sort, cheap when they follow the winner's and expensive when they
  interleave.
- **2026-08-19** — **P0 criteria 1, 3 and 4 met at the merge layer.** `ChannelView` renders
  as a pure function of the admitted record set: 40 permutations, reversal, duplicate
  delivery and a two-sided partition heal all converge on identical output. Reader-side
  refusal covers non-members, forged signatures and wrong-channel records. Honest scope:
  this proves the *merge*, not the wire — `kols-net` still owes the byte-transfer half.
- **2026-08-19** — **E11 found:** the extension-capability registry matches names exactly,
  but every chat permission is parametrized by scope, so each scope would need its own
  policy entry. Namespace registration proposed in `design/06` §11; `kols-core::capabilities`
  carries the workaround meanwhile. `AuthorLog` publishes
  segments through `intranet-storage`; the byte-level assertion showed 51,405 of 176,123
  bytes moving per appended message. Cause: `Enc::seq`'s count prefix sits at the head of
  the encoding, so every append changed the first chunk. Removing the count — the record
  list runs to end of input, each record already length-prefixed — brings it to **1,556 of
  176,115, one new chunk of eight**. `design/08` §6 and `design/01` §3.1 updated.

- **2026-08-19** — `kols-core` encoding implemented against `design/08`: records, segments,
  HLC, derived identifiers. 13 tests green (round-trip, injectivity, domain separation, id
  stability, rate-class, bounds), clippy clean. Test vectors frozen — a change to one is a
  wire break, not a value to re-bless.
- **2026-08-19** — E3 withdrawn: deriving pointer ids needs no protocol change, so **P0
  requires no change to `distributed-intranet` at all**. That repo remains untouched.
- **2026-08-19** — S1: client repo created, design docs moved in, `kols-core` scaffolded.
- **2026-08-19** — F1 done: canonical record encoding pinned in `design/08-record-encoding.md`.
- **2026-08-19** — Design set `00`–`07` drafted and reviewed across four passes; 21 decisions recorded.
