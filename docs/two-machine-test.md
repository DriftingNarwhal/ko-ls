# The Two-Machine Test

**Purpose:** two laptops on entirely separate networks, one on a mobile hotspot, talking
through a bootstrap relay and no VPS. Everything below is built; none of it has crossed a real
network.

**Status:** this document exists because that test has not been run yet. When it has, and the
path it describes is either routine or wrong, delete it or fold what survived into the README —
a runbook kept past its usefulness is another thing to disagree with `STATUS.md`.

**Run it in order.** The ordering is not presentation: the relay needs the network id before it
will boot, and the network needs a relay before it can invite anybody. That circularity is the
reason the steps are numbered.

Throughout: **A** is the founder's machine, **B** is the joiner on the hotspot, and the relay is
neither — it is a routable address both can reach.

---

## Terminal or window

**Every step below can now be done in the window instead**, since O12 and O13 closed on
2026-08-21 and relay setup was the last thing that needed a terminal. The steps are written as
`kols` commands anyway, and that is a deliberate choice rather than a limitation:

- **The terminal says more when it fails.** `kols serve` prints the relay's standing, every
  governance entry it learns and every key request it answers. The window shows the same
  information in less of it.
- **The window has never been launched.** Not on Windows, not on macOS, not once
  (`STATUS.md` §0). Running the test through it means two unproven things at the same time, and
  a failure that could be either.

So: **run it in the terminal the first time**, then repeat it in the window, where a failure is
unambiguously the window's. If you would rather start in the window, this is the mapping:

| Step | In the window |
|---|---|
| 2 — create | **or make one** on the picker. Leave the relay field empty |
| 3 — the relay's `RELAY_NETWORK` | **relay** in the sidebar → *set the relay* → **copy** |
| 4 — designate it | Paste the address into the same panel → **designate**, then reopen the network from **networks** — a relay is dialled when a node starts |
| 5 — serve | Nothing. The window runs a node for whichever network is open |
| 6 — name, channel, post | The composer asks for a name before it lets you post; **+** beside *channels* |
| 7 — invite | **invite** under *people*, then **copy** |
| 8 — join | Paste into **join a network** on B's picker |
| 9 — admit | The waiting list under *people*, **admit** |
| 11 — confirm | Both messages render in both windows |

Step 3 itself — deploying on Railway — is a web dashboard either way, and the relay's identity
phrase comes from `intranet-harness`, which is a protocol-repo tool with no window at all.

---

## 1. Get `kols` onto both machines

Take a published build. `.github/workflows/release.yml` builds Windows x64 and Apple Silicon
macOS on a `v*` tag, and the tag is what publishes them — a manual dispatch builds the same
binaries and leaves them as artifacts, which download as a **zip** and lose the executable bit
on the way. Releases attach each file on its own:

**<https://github.com/DriftingNarwhal/ko-ls/releases>**

| Take | For |
|---|---|
| `windows-x64-kols.exe`, `macos-arm64-kols` | **This runbook.** Every step below is a `kols` command |
| `…-setup.exe`, `….dmg` | The window. Optional here, and it installs *only* the window |
| `…-kols-desktop.exe`, `macos-arm64-kols-desktop` | The window, portable — no installer |

**The installer does not give you `kols`.** It installs the desktop application, and nothing
below is reachable from it. Download the CLI separately even if you install.

Both platforms object to an unsigned download the first time, in ways that read as a broken
build:

```bash
# macOS — the release asset is prefixed, and arrives quarantined and non-executable
mv macos-arm64-kols kols
chmod +x kols
xattr -d com.apple.quarantine kols      # else: "cannot be opened because the developer…"
```

On Windows, rename `windows-x64-kols.exe` to `kols.exe` and put it somewhere on `PATH`.
SmartScreen warns about the installer and leaves the portable `.exe` files alone. There is no
signing certificate on this project, so both objections are expected rather than symptoms.

Building instead, where there is a toolchain:

```bash
cargo build -p kols-cli      # target/debug/kols
export PATH="$PWD/target/debug:$PATH"
```

Then give each machine its own state directory:

```bash
# A, macOS/Linux
export KOLS_HOME=~/kols-alice

# B, macOS/Linux
export KOLS_HOME=~/kols-bob

# Windows PowerShell, either machine
$env:KOLS_HOME = "C:\kols\alice"
```

`--home <path>` does the same thing per-command, which is easier than exporting when two
terminals on one machine need different identities.

The seed written there is the only copy of that identity and there is no recovery service, so
for a test point it somewhere disposable.

## 2. Create the network — **A**

```bash
kols init "the workshop"
```

It prints the network id and, because no relay is designated yet, says so:

```
created the workshop
  network   3f9a…            ← copy all 64 characters
  you       14544277
  state     /home/you/kols-alice

No relay designated. Two members behind NAT cannot reach each other
without one, and `kols invite` will refuse until this network has one
```

That network id is `RELAY_NETWORK` in the next step.

## 3. Deploy the relay

A relay introduces two peers and gets out of the way — circuits are capped at 120 seconds and
8 MB by design (Core §5.3), so this is connection setup rather than bandwidth, and it fits in a
free tier. `DI-Relay`'s own README is the authority; what follows is the short form.

Generate an identity for it, from the protocol checkout:

```bash
cargo run -p intranet-harness -- identity new
```

That prints a BIP-39 phrase. It is the relay's identity — treat it like an SSH private key,
because anyone holding it can impersonate this relay to the network.

Then on Railway:

1. **New project → Deploy from GitHub repo →** `DI-Relay`. The first build failing before
   variables are set is expected.
2. **Variables:** `RELAY_PHRASE` (quoted), `RELAY_NETWORK` (the id from step 2), `PORT=8080`.
3. **Settings → Networking → Generate Domain**, and **Add TCP Proxy** on port `4001`.
4. Check `/health` for `{"status":"ready", …}`, then read `/peer-id`.

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

## 4. Designate it — **A**

```bash
kols relay set /dns4/monorail.proxy.rlwy.net/tcp/54321/p2p/12D3KooWKiD4…
kols relay list
```

A governance action needing `define-policy`, which the founder holds. Every member learns the
relay by replaying the log, which is what carries a newly deployed relay to people who already
joined — their invite is spent and their cache may name a relay that is gone (Core §5.5).

## 5. Start the founder's node — **A, terminal 1**

```bash
kols serve
```

`serve` holds the terminal, so leave it and open a second one. It also holds the network's MLS
group, which is live state no one-shot command can keep — which is why nothing can be posted or
keyed before it runs.

The line that matters:

```
  relay     reserved a circuit on /dns4/monorail.proxy…
```

**If it says the relay granted no usable circuit, stop here.** The relay accepted the
reservation and handed back no address — almost always the loopback case above, or a relay with
no routable listen address of its own (`RELAY_PUBLIC_ADDR` in DI-Relay's configuration). Nothing
downstream can work without a circuit.

Only one process may run a node per store, so if the desktop window has this network open,
`kols serve` is refused and the reverse. A claim expires 30 seconds after its last heartbeat.

## 6. Set the network up — **A, terminal 2**

```bash
kols name alice
kols channel create general
kols post general "first thing said on this network"
kols read general
```

## 7. Mint the invite — **A, terminal 2**

```bash
kols invite --uses 1 --hours 24
```

An invite needs an address and only a running node knows one, so the daemon writes down what it
is reachable on and `invite` reads that — refusing to mint rather than producing a credential
that connects to nothing.

Send the `intranet-chat://join/…` string to **B** however you like. It survives being pasted
into a chat message and copied back out, and carries the network id, the relay and this node's
addresses — nothing about the network's contents.

## 8. Redeem it — **B**

```bash
kols join intranet-chat://join/mfsw44dvorxxq5dfoj2xg2lom5qs4…
```

**Landing in the waiting room is success, not failure.** Under explicit intake an invite buys a
connection and an identity and nothing else until a member admits you; a waiting-room identity
is served no content, no metadata and no bandwidth, by design (Core §2.4, Storage §5.4).

## 9. Admit them — **A, terminal 2**

```bash
kols waiting
kols admit <their-identity-hex>
```

The waiting room is live state in the running node, which writes it down for anything else to
read — so it is stale by construction and refreshes as the daemon sees things.

## 10. Start the joiner's node — **B, terminal 1**

```bash
kols serve
```

No `--peer` is needed: the addresses the invite carried were kept when it was redeemed. Expect,
in roughly this order:

```
relay     reserved a circuit on /dns4/monorail.proxy…
asked 14544277 to key us in
learned 4 governance entries
epoch     held
learned 1 record
```

**If keying takes a moment, that is now fine.** The joiner re-asks every 30 seconds until it is
keyed (Core §3.5.1). A founder has ordinary reasons not to answer the instant a request lands,
and until E14 landed a missed answer stranded the member permanently. After 60 seconds unkeyed
the node says so and names where to look — and the reason will be on **A's** terminal, not B's.

Then, in a second terminal on **B**:

```bash
kols name bob
kols read general
kols post general "reached you from the hotspot"
```

## 11. Confirm it travels both ways — **A, terminal 2**

```bash
kols read general
```

The test passes when both authors render on both machines:

```
general
  alice  first thing said on this network   [7c1f…]
  bob    reached you from the hotspot       [a904…]
```

Then exercise the other record kinds, since each takes a different path through the merge:
`kols react general <id> +1`, `kols edit`, `kols delete`, and several posts from both sides to
confirm both terminals agree on the order.

## 12. Take it back off both machines

There is no server, so **nothing is deleted anywhere else when you delete it here.** These
stores *are* the network — remove them from both machines and it is gone, with no account left
behind and nobody to ask. The seed is the identity and there is no recovery service, so this is
irreversible by design rather than by omission. That is the intended end state for a test.

### The state each machine holds

Everything `kols` writes lives under the store root, so one directory per identity covers it —
`seed`, `network`, `label`, `entries/`, `channels/`, `group`, `addresses`, `peers`, `relays`
and the `lock`/`serving` files a running node uses.

```bash
# macOS/Linux — whichever you exported in step 1
rm -rf ~/kols-alice ~/kols-bob
```

```powershell
# Windows
Remove-Item -Recurse -Force C:\kols\alice
```

**Check `~/.kols` as well, on both machines.** That is the default when `KOLS_HOME` is unset,
and it is easy to have created one without meaning to — a terminal opened before the export, or
any `kols` command run from a fresh shell. On Windows it is `%USERPROFILE%\.kols`.

```bash
ls -la ~/.kols 2>/dev/null && echo "^ this exists too"
```

### If you opened the window

**The window almost certainly used a different directory than your test did.** It resolves the
same `$KOLS_HOME`-else-`~/.kols` rule, but an application launched from Finder or the Start menu
does not inherit a `KOLS_HOME` you exported in a terminal — so it will have made its own
`~/.kols` regardless of which store the CLI steps used. Delete that too.

The webview keeps its own data under the bundle identifier `dev.kols.desktop`, separately from
anything ko-ls writes. Nothing secret is in it, but it is what is left after an uninstall:

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
always on the other machine — a founder refusing a request says so on *its* terminal, which the
joiner cannot see. That has cost this project two rounds of guessing before, and the harness was
changed to print every daemon's log for exactly this reason.

- **Both daemons' full output**, not just the one that appeared stuck. Highest-value item here
  and the one most often missed.
- **`kols relay list` on both machines.** Shows what replay designates and what each node
  cached. A node whose cache names a relay that is gone behaves differently from one that never
  had a relay.
- **The relay's `/health` and `/peer-id`.** A peer id that moved means `RELAY_PHRASE` is missing
  or was edited, and every address referencing the old one is stale.
- **Whether `relay reserved a circuit on …` appeared**, on each machine. Its absence is the most
  common single cause and is reported clearly enough that guessing is unnecessary.

## What this test does and does not establish

It establishes that the path works across real networks: relay reachability, admission,
epoch-key delivery, pointer sync, segment fetch and a merged view between two nodes that have
never been on the same machine.

It does not exercise the desktop window. That is now a separate and much cheaper check than it
was — v0.1.0 publishes an installer for both platforms — but the window has still never been
launched as a window on either (`STATUS.md` §0), so opening it once is worth doing on its own
rather than folding into this test, where a failure would be ambiguous between the two. It does not exercise private channels, direct
messages, voice or search, none of which exist. And it says nothing about behaviour over days —
retention, key retirement and segment sealing thresholds are all unmeasured beyond a single
spike.
