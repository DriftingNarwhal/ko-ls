# ko-ls

A Discord-shaped chat client — text now, voice and video later — built on the
[Distributed Intranet protocol](../distributed-intranet) as its networking, identity,
storage and real-time backend.

A "server" is a **network**: its own governance log, membership, epoch key chain and DHT
namespace. A role is a governance group, a permission is a capability, and a direct message
is its own two-person network rather than a channel in somebody else's. There is no
operator above the members.

**Working name.** `ko-ls` is a placeholder; nothing in the design depends on it.

---

## Start here

| If you want | Read |
|---|---|
| Where the work stands and what is next | [`STATUS.md`](STATUS.md) |
| Why anything is the way it is | [`design/00-overview.md`](design/00-overview.md) |
| The normative wire contract | `../distributed-intranet/specs/07-chat-application-spec.md` |

The design set (`design/00`–`09`) owns client design, rationale and sequencing.
**Spec 07 in the protocol repo is normative** where the two overlap.

## Layout

```
crates/kols-core   records, canonical encoding, merge ordering, permissions, chat policy,
                   channel structure as chat-namespace governance payloads
crates/kols-net    publishing a channel over the transport, and reading one back
crates/kols-api    the boundary — commands and their gate going in, outcomes and events out
crates/kols-cli    the executor, the node loop, the workspace, and `kols` — the terminal client
crates/kols-app    `kols-desktop` — the Tauri shell, holding a workspace and running a node
crates/kols-ui     the interface: HTML, CSS and one script, holding no keys and no sockets
design/            00-08 at v1.0, 09 the interface
.devcontainer/     the environment for this repo and the protocol beside it
scripts/           cross-check.sh — runs the encoding on a big-endian target
```

`kols-core` is I/O-free and deterministic on purpose: merge ordering, encoding and
permission resolution must produce identical answers on every node, and pure functions over
explicit inputs are how that stays testable.

`kols-api` is the only way into any of it. Every command names its target, the gate resolves
permission by replaying the governance log, and what it hands back — an `Authorized` — has no
public constructor, so an executor cannot be handed a command nobody checked. The terminal
client crosses that boundary exactly as a UI would, which is the only reason to believe it
works.

## Building

Requires Rust 1.85+ (edition 2024) and a checkout of `distributed-intranet` as a sibling
directory — the protocol crates are path dependencies while the extensions it needs are
still landing. The dev container in `.devcontainer/` has the whole toolchain if you would
rather not assemble one.

```bash
cargo test                                   # 181 tests; the two-node ones spawn real nodes on loopback
cargo clippy --workspace --all-targets       # must stay clean
./scripts/cross-check.sh                     # big-endian verification, see below
```

`.devcontainer/` builds all of it, plus the protocol workspace beside it and the Tauri
toolchain the desktop client will need. Open **this repo's folder** in it — the container
mounts the parent so both repos are visible, which is what the path dependency needs.

`cross-check.sh` is not part of the default gate. It needs `qemu-user-static`,
`gcc-s390x-linux-gnu` and the `s390x-unknown-linux-gnu` Rust target, and takes about a
minute. Run it when the encoding, its test vectors, or anything in the hashing and signing
path changes — the frozen vectors were produced on a little-endian machine, so running them
only there proves they are self-consistent rather than host-independent.

## Trying it

```bash
cargo build -p kols-cli
alias kols="$PWD/target/debug/kols"

# A network needs a relay before it can invite anybody: two people behind home
# routers cannot reach each other directly (Core §5.5). Run one, on a routable
# address — never loopback, or it grants circuits that carry no address.
cargo run -p intranet-harness --manifest-path ../distributed-intranet/Cargo.toml -- \
    relay --seed 1 --network 42 --listen /ip4/0.0.0.0/tcp/4001

KOLS_HOME=/tmp/alice kols init "the workshop" --relay /ip4/<host>/tcp/4001/p2p/<peer-id>
KOLS_HOME=/tmp/alice kols serve &                 # keys it, reserves a circuit
KOLS_HOME=/tmp/alice kols name alice
KOLS_HOME=/tmp/alice kols channel create general
KOLS_HOME=/tmp/alice kols post general "hello"
KOLS_HOME=/tmp/alice kols read general            # prints message ids
KOLS_HOME=/tmp/alice kols react general <id> +1
```

`serve` must run before posting: it holds the network's MLS group, which is live state no
one-shot command can keep. Then somebody joins, holding one string and nothing else:

```bash
KOLS_HOME=/tmp/alice kols invite                  # prints intranet-chat://join/…
KOLS_HOME=/tmp/bob   kols join <that>             # dials the relay, lands in the waiting room
KOLS_HOME=/tmp/alice kols waiting                 # who is asking
KOLS_HOME=/tmp/alice kols admit <their-identity>
KOLS_HOME=/tmp/bob   kols serve                   # no --peer: the invite carried the address
KOLS_HOME=/tmp/bob   kols read general
```

The joiner syncs the governance log, asks to be keyed in, receives the epoch keys
including the historical ones, fetches the other author's segments and renders a merged
view — everything over the real transport, nothing shared out of band but the network id.

State lands in `$KOLS_HOME`, else `~/.kols`. The seed written there is the only copy of
your identity and there is no recovery service, so point `--home` somewhere disposable
while you are poking at it.

## The window

```bash
cargo build -p kols-app
KOLS_HOME=/tmp/alice ./target/debug/kols-desktop
```

That is the whole launch. No `DISPLAY` or `GDK_BACKEND` to set: inside the dev container GTK
finds the Wayland socket VS Code forwards, and on a desktop it finds whatever is there. Run it
from a terminal you can leave occupied — it holds the terminal until you close the window.

It opens the network at `$KOLS_HOME` and shows what that node's store already holds: the
channels replay knows about, and the messages in one of them. Posting works, because writing a
record is a local act.

**It runs no node**, so it will not show you another member's messages and will not update
while you watch. `kols serve` is what fetches and what hears gossip; the window reads what the
store has. A shell that ran its own node would look identical and be a different piece of
work — [`STATUS.md`](STATUS.md) §6 carries it.

## What exists

Text chat's foundations, proven end to end between two live nodes: canonical record
encoding with frozen vectors, per-author segment logs on the storage layer, merge and
render, permission resolution, channel structure as governance-log payloads, and
publish/fetch over the real transport. `kols` drives all of that from a terminal on one
node — create a network, define a channel, post, read.

Two `kols` installs now reach each other end to end: admission, epoch-key delivery,
pointer sync, segment fetch and a merged view across authors, with messages travelling both
ways between live nodes.

Keying is whole: group state survives a restart, so a founder can still admit and key in
new members after one, and the epoch rotates on both add and remove.

`kols revoke` drives a real removal: the membership entry from the command, the epoch
rotation from the daemon, in that order because the protocol requires it. Records also
travel **live** over gossipsub as they are written — sealed under the channel's content key
— while remaining fully carried by the durable path, so a node with the live path silent is
slower and completely correct. `kols serve --no-live` turns gossip off outright, which is
how that claim is actually tested rather than asserted.

Author logs **seal** into chained segments once they cross a size or age threshold, and a
joiner walks the chain backwards to read history older than the current head — so opening a
channel stays a bounded fetch however long the conversation gets. Each segment carries its
own key, which is what lets a network's retention window drop *old* history and keep the
rest: an author stops re-wrapping what has aged out, and Storage's rule that content with
no live wrapping goes dark does the rest.

Every command the client runs crosses `kols-api`, the boundary `design/05` §3 specifies —
permission resolved by replaying the log, never by trusting that the interface only offered
buttons you were allowed to press, and a consent class on each command derived from the tier
its capability carries rather than from how consequential it looks.

Behind that boundary is one executor. It authorizes, then runs — and because the value it
requires can only come from the gate, there is no path to it that skipped the check. Every
record kind the design describes goes through it: messages, edits, withdrawals, reactions and
pins, plus channel rename, topic, slowmode and archive.

Events come back the other way, and the vocabulary was written from what the node already
reported rather than guessed ahead of it. They are idempotent by construction: a record pushed
over gossip is also inside the segment that follows, so a consumer merges by record id rather
than appending — which makes the ordinary case of hearing something twice a non-event.

The window does the same, without a terminal: it creates a network or joins one by invite,
runs a node for it, and updates as records arrive. `kols-desktop`, and [the launch
instructions](#the-window) below.

What does not exist yet: no private-channel keying, no voice, no search, no presence, and no
credentials — seeds sit on disk in the clear with no way to back them up, which is fine while
you are testing between machines you own and is a release gate before anything else.
[`STATUS.md`](STATUS.md) is the honest inventory, its §0 is where to start, and its §6 is the
list of what this client owes and why.

## The one number worth knowing

Appending a message to a full segment moves **1,556 bytes of 176,115** locally, and a
reader holding the previous version refetches **one chunk of three** across the wire. That
delta property is what makes a chat log affordable on a storage layer built for documents,
and the whole segment model exists to preserve it — it is asserted on bytes actually moved,
in `crates/kols-core/tests/author_log.rs` and `crates/kols-net/tests/two_nodes.rs`.
