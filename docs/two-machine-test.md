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

## 1. Get `kols` onto both machines

Build it where there is a toolchain, or take what CI produces —
`.github/workflows/release.yml` builds Windows and Apple Silicon macOS on a `v*` tag or a
manual dispatch.

```bash
cargo build -p kols-cli      # target/debug/kols
```

Give each machine its own state directory:

```bash
# A, macOS/Linux
export KOLS_HOME=~/kols-alice
export PATH="$PWD/target/debug:$PATH"

# B, macOS/Linux
export KOLS_HOME=~/kols-bob

# Windows PowerShell, either machine
$env:KOLS_HOME = "C:\kols\alice"
```

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

It does not exercise the desktop window, which is built for Windows and macOS and has never been
run as a window on either (`STATUS.md` §0). It does not exercise private channels, direct
messages, voice or search, none of which exist. And it says nothing about behaviour over days —
retention, key retirement and segment sealing thresholds are all unmeasured beyond a single
spike.
