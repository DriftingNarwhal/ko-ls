# The Two-Machine Test

**Purpose:** two laptops on entirely separate networks, one on a mobile hotspot, talking
through a bootstrap relay and no VPS. Everything below is built; none of it has crossed a real
network.

**Status:** this document exists because that test has not been run yet. When it has, and the
path it describes is either routine or wrong, delete it or fold what survived into the README —
a runbook kept past its usefulness is another thing to disagree with `STATUS.md`.

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
| MacBook | `macos-arm64-ko-ls_<version>_aarch64.dmg` |

Both platforms object to an unsigned download the first time, in ways that read as a broken
build. On macOS the `.dmg` is refused by Gatekeeper: **right-click → Open**, once, rather than
double-clicking. On Windows SmartScreen warns about the installer; the portable `.exe` avoids it
entirely. There is no signing certificate on this project, so both are expected.

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

In the sidebar, **relay** → *set the relay*.

**generate a relay identity**, then **copy**. That is a BIP-39 phrase and it is the relay's
private key — anyone holding it can answer as this relay. It is shown once and stored nowhere,
so paste it somewhere you can reach in the next few minutes.

It is **not your identity**. Yours never leaves the machine and has no interface at all.

Directly below it is this network's id. **copy** that too — it is `RELAY_NETWORK`, and DI-Relay
refuses to start without it.

## 4. Deploy the relay

A relay introduces two peers and gets out of the way — circuits are capped at 120 seconds and
8 MB by design (Core §5.3), so this is connection setup rather than bandwidth, and it fits in a
free tier. `DI-Relay`'s own README is the authority; this is the short form. All of it happens
in a browser.

1. **New project → Deploy from GitHub repo →** `DI-Relay`. The first build failing before
   variables are set is expected.
2. **Variables:** `RELAY_PHRASE` (the phrase from step 3, in quotes), `RELAY_NETWORK` (the id
   from step 3), `PORT=8080`.
3. **Settings → Networking → Generate Domain**, and **Add TCP Proxy** on port `4001`.
4. Open `/health` and expect `{"status":"ready", …}`, then `/peer-id`.

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

Paste that address into the same **relay** panel and press **designate**.

The address is checked before it becomes policy — one naming no peer id is refused here, because
this entry is replayed by every member and a relay nothing can verify is worse than none.

The node then restarts itself onto the new relay. Watch the line above the panel:

| It says | Meaning |
|---|---|
| **reserved a circuit on …** | Working. Continue |
| **designated, but none of them granted a circuit** | **Stop here.** The relay accepted the reservation and handed back no address — almost always the loopback case above, or a relay with no routable listen address (`RELAY_PUBLIC_ADDR`) |
| **none designated** | The designation did not take. Check for a refusal under the form |

Nothing downstream can work without a circuit, so do not carry on past the second row.

## 6. Set the network up — **A**

Press **+** beside *channels* and name one `general`.

The composer asks for a name before it will let you post — claim `alice`. Names are unique here
and bound permanently, including after you leave, so nobody inherits yours and relabels what you
wrote.

Then post something.

## 7. Mint the invite — **A**

Under *people*, press **invite**, then **copy**.

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

Under *people*, the waiting list fills. Press **admit**.

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

- **The relay panel**, in the sidebar. It says which of four states that node is in — a circuit
  reserved, none designated, designated and none usable, or not yet reported — and lists what
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

**The window's honest limit:** it can tell you that nobody answered, and not why. The reason
lives on the other machine, so a screenshot of B's window is worth little without one of A's.
If you exhaust the above, the development binary prints considerably more — `kols serve` narrates
every governance entry, key request and answer — but it is a debugging escalation, not part of
this test.

## What this test does and does not establish

It establishes that the path works across real networks: relay reachability, admission,
epoch-key delivery, pointer sync, segment fetch and a merged view between two nodes that have
never been on the same machine.

**It tests two unproven things at once, and that is worth going in knowing.** The window has
been opened once and rendered, but nothing was wired to it then (`STATUS.md` §0), so every path
through it is unexercised — as is the network path across real machines. A failure could be
either. When something does not work, the question to ask first is whether the *other* window
shows the state you expect: if A's says B was admitted and keyed, and B's shows nothing, the
fault is between them rather than in the interface.

It does not exercise private channels, direct messages, voice or search, none of which exist.
And it says nothing about behaviour over days — retention, key retirement and segment sealing
thresholds are all unmeasured beyond a single spike.
