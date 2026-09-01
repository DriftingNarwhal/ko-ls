# The Two-Machine Test

**Purpose:** two laptops on entirely separate networks, one on a mobile hotspot, talking
through a bootstrap relay and no VPS.

**Status: run, and it passed.** Two of the author's own laptops, one on a mobile hotspot, and
then a third person on a third network — connection worked across all three, survived close and
reopen, reconnected, and every chat function worked. `STATUS.md` §1 carries what it established
and the defects it found. So this is a **runbook** now rather than a plan: the path below is the
one that was walked, corrected where walking it proved it wrong.

It is still worth keeping separate from the README, because most of it is the relay deployment
and the failure modes — neither of which belongs in a project's front page, and both of which
are what somebody standing at a stalled join actually needs.

**The window does all of it.** No terminal, at any step — including making the relay's identity,
which used to need one. Deploying the relay happens in a browser. The `kols` binaries in the
release are a development tool (`design/00` D30) and nothing here uses them.

**Run it in order.** The ordering is not presentation: the relay needs the network id before it
will boot, and the network needs a relay before it can invite anybody. That circularity is the
reason the steps are numbered.

Throughout: **A** is the founder's machine, **B** is the joiner on the hotspot, and the relay is
neither — it is a routable address both can reach.

---

## 1. Get the window onto both machines

**<https://github.com/DriftingNarwhal/ko-ls/releases>** — take the newest.

| Machine | Take |
|---|---|
| Windows | `windows-x64-ko-ls_<version>_x64-setup.exe`, or `windows-x64-kols-desktop.exe` to run without installing |
| MacBook | `macos-arm64-ko-ls_<version>_aarch64.dmg` — the `.dmg`, and not the portable binary beside it |

Both platforms object to an unsigned download the first time, in ways that read as a broken
build.

**macOS**: drag the app out of the `.dmg`, then **right-click → Open** rather than
double-clicking. If macOS says the app is **damaged and can't be opened**, that is quarantine
plus a build older than v0.3.4 — clear it with
`xattr -dr com.apple.quarantine /Applications/ko-ls.app`, or take a newer build, which is ad-hoc
signed and gets the ordinary "unidentified developer" prompt instead.

**Windows**: SmartScreen warns about the installer. The portable `.exe` avoids it entirely.

**There is no macOS equivalent of that portable route, and the release publishes something that
looks like one.** `macos-arm64-kols-desktop` is a bare executable, and Finder runs one of those
by opening a Terminal window alongside it — which is what Finder does with any Unix executable
outside a bundle, not a fault in the build. Nothing can change it: macOS has no counterpart to
the subsystem flag that keeps a console off `kols-desktop.exe`. On macOS the application is
`ko-ls.app`, out of the `.dmg`.

Neither is a symptom: notarising and code-signing need paid certificates this project does not
have.

Ignore the `kols` binaries in the release. They are a development tool (`design/00` D30) and no
step here uses one.

**One state directory per machine, and it is chosen for you.** The window uses `~/.kols`
(`%USERPROFILE%\.kols` on Windows) because an application launched from Finder or the Start menu
inherits no environment you set in a shell. The seed written there is the only copy of that
identity and there is no recovery service.

## 2. Create the network — **A**

Open the window. With no networks it opens on the picker.

Under **or make one**, give it a name — *the workshop* — and **leave the relay field empty**.
There is nothing to put in it yet: the relay does not exist, and it will not start until it
knows this network's id.

**create**. The window opens on the new network with you as its sole founder.

## 3. Make the relay's identity — **A**

**settings** — the button at the top of the left rail, beside *networks* — then **network**,
then *set the relay*.

**generate a relay identity**, then **copy**. That is a BIP-39 phrase and it is the relay's
private key — anyone holding it can answer as this relay. It is shown once and stored nowhere,
so paste it somewhere you can reach in the next few minutes.

It is **not your identity**. Yours never leaves the machine and has no interface at all.

Directly below it is this network's id. **copy** that too — it is `RELAY_NETWORK`, and DI-Relay
refuses to start without it.

## 4. Deploy the relay

A relay introduces two peers and gets out of the way — circuits are capped at 60 seconds and
256 KB by design (Core §5.3), so this is connection setup rather than bandwidth, and it fits in
a free tier. **Deploy from `main`, not from a tag.** The ceilings live in the protocol crate
`DI-Relay` pins, so a relay built before 2026-08-22 still enforces the old 120 seconds and
8 MB — which is a relay that would carry a conversation rather than refusing to.
`DI-Relay`'s own README is the authority; this is the short form. All of it happens in a
browser.

1. **New project → Deploy from GitHub repo →** `DI-Relay`. The first build failing before
   variables are set is expected.
2. **Variables:** `RELAY_PHRASE` (the phrase from step 3, in quotes), `RELAY_NETWORK` (the id
   from step 3), `PORT=8080`.
3. **Settings → Networking → Generate Domain**, and **Add TCP Proxy** on port `4001`.
4. Open `/health` and expect `{"status":"ready", …}`, then `/peer-id`.
5. **Add one more variable, now that the proxy has a host and port:**
   `RELAY_PUBLIC_ADDR=/dns4/<proxy-host>/tcp/<proxy-port>` — **no `/p2p/…` on the end**. Then
   redeploy.

**Step 5 is the one that is skipped, and skipping it fails in the least helpful way.** The relay
listens on a private container address, so what it announces is unreachable. It still accepts
reservations, still reports healthy, and hands clients an address list they reject — so
everything looks fine except that no circuit is ever granted. `RELAY_PUBLIC_ADDR` is what tells
it where it actually is. It is harmless when unnecessary.

**Both parts of step 3 are required and it is easy to do only the first.** Without the domain
the health check fails and Railway restarts forever. Without the TCP proxy the service looks
perfectly healthy and no peer can reach it.

Assemble the address from the **proxy** host and port plus the peer id:

```
/dns4/monorail.proxy.rlwy.net/tcp/54321/p2p/12D3KooWKiD4…
                              ↑ the proxy's port, not 4001
```

Read the peer id from the HTTPS endpoint rather than trusting whatever answers on the TCP port.
That is why the two are exposed separately.

> **Never bind a relay to loopback.** It grants every reservation, logs that it did, reports
> healthy — and hands back nothing, because a relay only promotes non-loopback addresses to
> external ones and libp2p builds a circuit address from external addresses alone. Everything
> downstream then fails for the wrong reason. This has cost real debugging time.

## 5. Designate it — **A**

Paste that address into the same **relay** panel — settings → network — and press
**designate**.

The address is checked before it becomes policy — one naming no peer id is refused here, because
this entry is replayed by every member and a relay nothing can verify is worse than none.

The node then restarts itself onto the new relay. Watch the line above the panel:

| It says | Meaning |
|---|---|
| **reserved a circuit on …** | Working. Continue |
| **asking the relay for a circuit…** | Working on it. Settles within about 20 seconds |
| **designated, and no circuit was granted** | **Stop here** — and see step 4.5. The relay answered and handed back nothing usable, which is nearly always a missing `RELAY_PUBLIC_ADDR`. The panel says so, with the value to set |
| **none designated** | The designation did not take. Check for a refusal under the form |

Nothing downstream can work without a circuit, so do not carry on past the second row.

## 6. Set the network up — **A**

Press **+** beside *channels* and name one `general`.

The composer asks for a name before it will let you post — claim `alice`. Names are unique here
and bound permanently, including after you leave, so nobody inherits yours and relabels what you
wrote.

Then post something.

## 7. Mint the invite — **A**

**invite**, at the top of the left rail, then **mint an invite** and **copy**.

The button is shown only to a holder of `approve-node`, which is the same capability admitting
somebody needs — so a member who could not let anybody in is not shown a door at all.

An invite needs an address and only a running node knows one, so this carries what your node is
currently reachable on. **Keep this window open** — it is what answers when B redeems it.

Send the `intranet-chat://join/…` string to **B** however you like. It survives being pasted
into a chat message and copied back out, and carries the network, the relay and this node's
addresses — nothing about the network's contents.

## 8. Redeem it — **B**

Open the window on the MacBook. Paste the invite into **join a network**.

**Landing in the waiting room is success, not failure.** Under explicit intake an invite buys a
connection and an identity and nothing else until a member admits you; a waiting-room identity
is served no content, no metadata and no bandwidth, by design (Core §2.4, Storage §5.4). The
window says so rather than showing you an empty network.

## 9. Admit them — **A**

The count on the **invite** button fills. Open it, and press **admit** beside whoever is
waiting.

The waiting room is live state in the running node, so it is stale by construction and refreshes
as the daemon sees things.

## 10. Watch B come in — **B**

Nothing to press. Redeeming the invite already opened the network and started its node.

**If keying takes a moment, that is fine.** The joiner re-asks every 30 seconds until it is
keyed (Core §3.5.1), and until E14 landed a missed answer stranded the member permanently. Until
then the sidebar says *waiting to be keyed in* — which is an ordinary place to be, not a fault.

Then claim a name — `bob` — and post something.

## 11. Confirm it travels both ways — **both**

Both messages should render in both windows, with both authors named.

Then exercise the other record kinds, since each takes a different path through the merge:

- **react** — hover a message, **+1**. Click the chip again to take it back; a chip you hold is
  outlined
- **edit** — hover your own message, **edit**
- **withdraw** — hover your own message, **withdraw**. It is *hidden, never unsent*: anybody who
  already has it keeps the bytes, and the window says which happened
- **pin** — founder only here
- **several posts from both sides**, to confirm both windows agree on the order

Order is computed from the merged set rather than from arrival (`design/01` §4), so both windows
must agree regardless of who posted when.

## 12. Take it back off both machines

**Leave first, if you want the other machine to know.** With the network open, the picker's
button reads **leave** rather than **forget**: it publishes a departure — one entry per group
you are in — and only then deletes the store. Forgetting a network that is *not* open still
works and still deletes everything, but no node is running to tell anybody, and the interface
says so rather than reporting the same success. The order cannot be reversed: the departure is
signed by the seed the deletion destroys.

There is no server, so **nothing is deleted anywhere else when you delete it here.** These
stores *are* the network — remove them from both machines and it is gone, with no account left
behind and nobody to ask. The seed is the identity and there is no recovery service, so this is
irreversible by design rather than by omission. That is the intended end state for a test.

### The state each machine holds

One directory per machine, and the window chose it: **`~/.kols`**, or `%USERPROFILE%\.kols` on
Windows. Everything is under it — `seed`, `network`, `label`, `entries/`, `channels/`, `group`,
`addresses`, `peers`, `relays`, and the `lock`/`serving` files a running node uses.

**Close the window first.** A running node holds a claim on the store and rewrites the heartbeat
as it goes.

- **macOS** — Finder, **Go → Go to Folder**, type `~/.kols`, delete the folder.
- **Windows** — Explorer, paste `%USERPROFILE%\.kols` in the address bar, delete the folder.

Or, if you would rather:

```bash
rm -rf ~/.kols                                    # macOS
```

```powershell
Remove-Item -Recurse -Force "$env:USERPROFILE\.kols"   # Windows
```

Nothing else on the machine holds network state. If you also ran the development binaries at
some point, they default to the same place and take `$KOLS_HOME` when it is set — worth a look
only if you know you set one.

### What the application itself leaves

Separate from anything ko-ls writes, and it survives an uninstall. Nothing secret is in it.

```bash
# macOS — any of these that exist
rm -rf ~/Library/Application\ Support/dev.kols.desktop \
       ~/Library/Caches/dev.kols.desktop \
       ~/Library/WebKit/dev.kols.desktop \
       ~/Library/Saved\ Application\ State/dev.kols.desktop.savedState
rm -rf /Applications/ko-ls.app          # if you installed from the .dmg
```

```powershell
# Windows — uninstall "ko-ls" from Settings > Apps if you ran the installer, then
Remove-Item -Recurse -Force "$env:LOCALAPPDATA\dev.kols.desktop"
```

### The relay, which is the one thing that costs money

**Delete the Railway project.** A relay left deployed keeps running, keeps its public address
and keeps consuming whatever the free tier allows — and the identity in `RELAY_PHRASE` stays
live, so anyone holding that phrase can still stand up something that answers as this relay.
Deleting the project settles both.

Then discard the phrase itself wherever you put it, and remember that any invite you pasted
into a chat still names the relay's address and this network's id. Invites expire on their own
(24 hours by default in step 7) and a spent one grants nothing, but the text is worth deleting
from wherever you sent it.

### The downloads, and the build tree

The release binaries and installers in `~/Downloads` on both machines, plus the mounted `.dmg`
volume if it is still attached. If you built rather than downloaded, `target/` is the large one
— a day of cross-compiles took it to 11 GB here.

```bash
cargo clean          # in the checkout, if you built locally
```

---

## If it stalls, capture this before changing anything

A two-node failure reads as "nothing happened" from the waiting side, and the reason is almost
always on the *other* machine. That has cost this project two rounds of guessing before.

Capture, from **both** machines:

- **The relay panel**, under settings → network. It says which of four states that node is in
  — a circuit reserved, none designated, designated and none usable, or not yet reported — and
  lists what
  replay designates above what this node cached, italicised where the two differ. A node whose
  cache names a relay that is gone behaves differently from one that never had a relay, and this
  is where you see which you have.
- **The red strip** under the messages. Everything that went wrong and was survivable lands
  there: a refused command, a key request that arrived before its sender was admitted, a payload
  that opened under no held epoch.
- **The sidebar's key line.** *Waiting to be keyed in* is an ordinary state and not a fault, but
  it is the difference between "B is not admitted" and "B is admitted and unkeyed", which have
  different causes.
- **The relay's `/health` and `/peer-id`**, from a browser. A peer id that moved means
  `RELAY_PHRASE` is missing or was edited, and every address naming the old one is stale.

**Absence of a circuit is the most common single cause**, and the panel reports it plainly
enough that guessing should not be necessary.

### A joiner that never reaches the relay at all

The tell is one-sided: **B says nobody answered, and B's peer id never appears in the relay's
log.** A peer id is only known after the Noise handshake, so its absence means the handshake
never completed — the join reached the relay and was refused, or the packets never arrived.

Distinguish those from B's machine before touching anything on A's, because A is not involved:

- **Load the relay's `/health` in a browser on B.** That is HTTPS on 443. If it loads, the host
  and DNS are fine and only the libp2p port is in question — which is the interesting case,
  since a hosted relay is usually reached on a high TCP port and plenty of networks allow 443
  and drop everything else outbound.
- **Try B on a phone hotspot.** If the join works there and not on its own network, the network
  is filtering and nothing in this project can change that. That is the fastest single test.

The refusal itself names the addresses it dialled and, since it started reporting them, why each
one failed — a refused connection, a name that would not resolve, and a port silently dropped
are three different sentences rather than one silence. If it says the connections were *still
outstanding*, nothing refused them, which is what dropping rather than rejecting looks like from
the joining end.

**The window's honest limit:** it can tell you that nobody answered, and not why. The reason
lives on the other machine, so a screenshot of B's window is worth little without one of A's.
If you exhaust the above, the development binary prints considerably more — `kols serve` narrates
every governance entry, key request and answer — but it is a debugging escalation, not part of
this test.

## What this test does and does not establish

It establishes that the path works across real networks: relay reachability, admission,
epoch-key delivery, pointer sync, segment fetch and a merged view between two nodes that have
never been on the same machine.

**It tested two unproven things at once, which is why the first run's failures took reading.**
The window and the network path were both unexercised, so a failure could have been either. That
is no longer the position — both have been walked end to end — but the diagnostic habit it
forced is the one to keep: when something does not work, ask first whether the *other* window shows the
state you expect. If A's says B was admitted and keyed and B's shows nothing, the fault is
between them rather than in the interface.

**What the first runs actually found was never in the chat path.** Every defect was a layer
below the interface, behaving correctly by its own lights and quietly removing a feature: an
empty Tauri ACL that had refused every node event for the life of the client, a native drag
handler that took the drag before the page saw it, a re-dial loop that counted the relay as a
connection and so never re-dialled after a laptop slept, and a founder's network name that was
never written to the log. `docs/log.md` carries each. Worth knowing before running it again:
the interface is not usually the thing that is wrong, and it is not usually able to say so.

It does not exercise private channels, direct messages, voice or search, none of which exist.
And it says nothing about behaviour over days — retention, key retirement and segment sealing
thresholds are all unmeasured beyond a single spike.
