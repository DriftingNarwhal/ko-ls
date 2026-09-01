# ko-ls

**A chat client shaped like Discord, with nobody running it.** Servers, channels, roles and
direct messages — text now, voice and video later — carried entirely by the machines of the
people using them. No company owns a community here, no hosting bill keeps it alive, and
there is no operator who could read it, sell it, or switch it off.

Built on the
[Distributed Intranet protocol](https://github.com/DriftingNarwhal/distributed-intranet) as
its networking, identity, storage and real-time backend.

**Working name.** `ko-ls` is a placeholder; nothing in the design depends on it.

---

## What "no server" means here

The word *server* does two jobs in chat, and this project keeps one and drops the other. A
Discord "server" — a community, with its channels and its roles — is still here; it is called
a **network**. The machine in a datacentre that would hold it is gone.

In its place, the members' own nodes store and serve the network's content between them,
encrypted so that holding a copy is not the same as being able to read one, and how much disk
a network may use is each member's own choice. Membership, roles and permissions live in a
**governance log every member replays for themselves**, so *"may they post here?"* is a
question each node answers by computing, never by asking a peer or a service. A role is a
governance group, a permission is a capability, and a direct message is its own two-person
network rather than a channel in somebody else's — so a private conversation lands on the two
machines having it and on no others.

What that is actually worth:

- **There is nothing to shut down.** A network lasts as long as its members do. There is no
  account to suspend, no bill to stop paying, and no operator to serve an order on.
- **There is no operator above the members.** Every moderation and administration action is
  taken by somebody holding a capability, and recorded where every member can replay it.
- **Your identity here is not your identity there.** Each network gets its own keys, so no
  member of one can tell that your membership in another is also yours, unless you
  deliberately prove it to them. Stated precisely, because the difference matters: that holds
  at the key layer. Two identities connecting from one address are still correlatable by
  anyone positioned to see both, and the protocol says so rather than implying otherwise.

**And what it costs, stated here rather than discovered later:**

- **A network still needs one relay, and this is the honest exception.** Two people behind
  ordinary home routers cannot reach each other directly, so a network designates a relay — a
  member's own machine on a routable address, or a small hosted one. It helps peers connect
  and carries bytes; it holds no messages, reads nothing, and stores no state.
- **There is no support, and no recovery.** No password reset, no account restoration, no
  appeal above the network's own members. Losing your keys loses that identity.
- **Nothing is browsable.** Joining is by invite, always. There is no directory of networks
  and no global username, by construction rather than by omission.
- **Deleting hides a message; it does not un-send it.** Nothing here can retract bytes
  somebody already has, and the interface is required to say so rather than imply otherwise.

None of this is a plan. What is built, and what is not, is [below](#what-exists) and in
[`STATUS.md`](STATUS.md).

---

## Start here

| If you want | Read |
|---|---|
| Where the work stands and what is next | [`STATUS.md`](STATUS.md) |
| To actually run it across two machines | [`docs/two-machine-test.md`](docs/two-machine-test.md) |
| Why anything is the way it is | [`design/00-overview.md`](design/00-overview.md) |
| The normative wire contract | [`specs/07-chat-application-spec.md`](https://github.com/DriftingNarwhal/distributed-intranet/blob/main/specs/07-chat-application-spec.md) in the protocol repo |

The design set (`design/00`–`09`) owns client design, rationale and sequencing.
**Spec 07 in the protocol repo is normative** where the two overlap.

## Layout

```
crates/kols-core   records, canonical encoding, merge ordering, permissions, chat policy,
                   channel structure as chat-namespace governance payloads
crates/kols-net    publishing a channel over the transport, and reading one back
crates/kols-api    the boundary — commands and their gate going in, outcomes and events out
crates/kols-node   the executor, the node loop, the store, the workspace — what the window
                   depends on for everything except its own window. Also `kols`, a
                   development client over the same boundary (D30), not a product
crates/kols-app    `kols-desktop` — the Tauri shell, holding a workspace and running a node
crates/kols-ui     the interface: HTML, CSS and one script, holding no keys and no sockets
design/            00-08 the design set, 09 the interface
.devcontainer/     the environment for this repo and the protocol beside it
.github/           release.yml — Windows and macOS builds, on a tag or on demand
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
cargo test                                   # 306 tests; the two-node ones spawn real nodes on loopback
cargo clippy --workspace --all-targets       # must stay clean
./scripts/cross-check.sh                     # big-endian verification, see below
```

`.devcontainer/` builds all of it, plus the protocol workspace beside it and the Tauri toolchain
`kols-app` links against. Open **this repo's folder** in it — the container mounts the parent so
both repos are visible, which is what the path dependency needs.

**Windows and macOS binaries are built by CI, not here** — `.github/workflows/release.yml`, on a
`v*` tag or a manual dispatch. Building where the thing runs is not fastidiousness: a
cross-compiled Windows window *imports* `WebView2Loader.dll` rather than linking the loader in,
so it will not start unless that DLL travels beside it, and an MSVC build has no such tail.
Nothing runs on an ordinary push; the tests run here.

`cross-check.sh` is not part of the default gate. It needs `qemu-user-static`,
`gcc-s390x-linux-gnu` and the `s390x-unknown-linux-gnu` Rust target, and takes about a
minute. Run it when the encoding, its test vectors, or anything in the hashing and signing
path changes — the frozen vectors were produced on a little-endian machine, so running them
only there proves they are self-consistent rather than host-independent.

## Trying it

**Download a build.** [Releases](https://github.com/DriftingNarwhal/ko-ls/releases) carry an
installer and a portable binary for Windows and Apple Silicon macOS. `kols-desktop` is the
application; it needs no terminal for any step, including setting up a relay.

**The two platforms are not symmetric about the portable binary, and the names hide it.** On
Windows `kols-desktop.exe` runs on its own. On macOS the application is `ko-ls.app`, out of the
`.dmg` — the portable `kols-desktop` beside it is a bare executable, and Finder runs one of
those by opening Terminal. That is what Finder does with any Unix executable outside a bundle
and nothing in the build can change it: macOS has no equivalent of the subsystem flag that
keeps a console off the Windows build. The macOS portable binary is kept because the bundler
is allowed to fail without taking the build down with it, so it is a fallback rather than the
ordinary way in.

Everything below is the **development** path. `kols` is a tool for building this, not a way to
use it: it exists because a front end over `kols-api` could be driven before there was a window,
and it stays because it is the reference path when the window misbehaves. It owes the window no
feature parity and nobody is expected to chat from a command line.

```bash
cargo build -p kols-node
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

It opens on a picker — join a network with an invite, or create one — and then **runs a node
for whichever network you open**, so it fetches, hears gossip and updates while you watch. It
lists channels, renders one, posts to it, and, for a member holding `approve-node`, mints an
invite and admits whoever redeems it. No step of the flow needs a terminal any more.

Only one process may run a node per network, so `kols serve` on a store the window has open is
refused, and the other way round. The claim expires after six seconds without a heartbeat,
because a window is closed by the window manager and that runs no destructors — a crash costs a
pause rather than a stuck store.

What it does **not** do is presence. `design/09` §4's third question — who is here, and are
they around — has no answer, because nothing implements the ephemeral gossip it needs. What the
window shows instead is the narrower thing it can actually know: how many members this node is
connected to, and whether that is more than none. `design/09` §4.1 carries what is still owed
there, and why it is last.

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
rotation from the daemon, in that order because the protocol requires it.

**And a member can now leave**, which nothing here could express until the protocol gained
the entry for it (Core §2.5.1): a membership removal naming its own author needs no
capability, because it grants nothing and names nobody but its signer. `kols leave` writes
one per group; in the window, forgetting a network that is open publishes the departure
before it deletes anything — the order is fixed, since that entry is signed by the seed the
deletion destroys. It reports how many members were connected when the departure went out,
and never claims they received it: gossip acknowledges nothing.

Records also
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

The window does all of it without a terminal: it creates a network or joins one by invite, runs
a node for it, updates as records arrive, and brings the next person in — mint an invite, watch
the waiting room, admit. `kols-desktop`, and [the launch instructions](#the-window) above.

It builds for **Windows and macOS** as well as Linux, in CI rather than here, and a seed it
writes is restricted to the account that wrote it on every platform — a `chmod` on Unix, a
protected DACL on Windows — or it is not written at all. That last part is the design's
fail-closed rule applied where it is easy to skip: a secret written somewhere another account
can read it is worse than a secret not written, because the second is an error somebody sees.

What does not exist yet: no private-channel keying, no voice, no search, no presence, and no
credentials — seeds are unencrypted on disk with no way to back them up, which is fine while
you are testing between machines you own and is a release gate before anything else.
[`STATUS.md`](STATUS.md) is where to start: it says where the work stands and points at the
document that owns each part of it.

## The one number worth knowing

Appending a message to a full segment moves **1,556 bytes of 176,115** locally, and a
reader holding the previous version refetches **one chunk of three** across the wire. That
delta property is what makes a chat log affordable on a storage layer built for documents,
and the whole segment model exists to preserve it — it is asserted on bytes actually moved,
in `crates/kols-core/tests/author_log.rs` and `crates/kols-net/tests/two_nodes.rs`.

---

## Licence

**[GNU Affero General Public License v3.0](LICENSE)** — © DriftingNarwhal.

Free for anyone to use, study, modify and share, commercial use included. What the licence
requires in return is that it stays that way: **if you distribute this, or run a modified
version as a service other people use over a network, you must publish the complete
corresponding source under the same licence.** The Affero clause is the one that matters
here, because the obvious way to enclose a chat system is to host a modified copy rather
than to ship one.

Selling it is permitted, and the licence is what makes that unprofitable rather than
forbidden: whoever buys it receives the source and may give it away. That is the intended
outcome — nobody can take this, close it, and sell it as their own.

The protocol crates it links are [MPL-2.0](https://github.com/DriftingNarwhal/distributed-intranet),
which is deliberate and compatible: MPL §3.3 permits covered software to be combined into a
larger work under a Secondary Licence, and AGPL-3.0 is one.
