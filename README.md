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

The design set (`design/00`–`08`) owns client design, rationale and sequencing.
**Spec 07 in the protocol repo is normative** where the two overlap.

## Layout

```
crates/kols-core   records, canonical encoding, merge ordering, permissions, chat policy,
                   channel structure as chat-namespace governance payloads
crates/kols-net    publishing a channel over the transport, and reading one back
crates/kols-cli    `kols` — the terminal client and its node daemon
design/            00-08, all v1.0
scripts/           cross-check.sh — runs the encoding on a big-endian target
```

`kols-core` is I/O-free and deterministic on purpose: merge ordering, encoding and
permission resolution must produce identical answers on every node, and pure functions over
explicit inputs are how that stays testable.

## Building

Requires Rust 1.85+ (edition 2024) and a checkout of `distributed-intranet` as a sibling
directory — the protocol crates are path dependencies while the extensions it needs are
still landing.

```bash
cargo test                                   # 75 tests; the two-node ones spawn real nodes on loopback
cargo clippy --workspace --all-targets       # must stay clean
./scripts/cross-check.sh                     # big-endian verification, see below
```

`cross-check.sh` is not part of the default gate. It needs `qemu-user-static`,
`gcc-s390x-linux-gnu` and the `s390x-unknown-linux-gnu` Rust target, and takes about a
minute. Run it when the encoding, its test vectors, or anything in the hashing and signing
path changes — the frozen vectors were produced on a little-endian machine, so running them
only there proves they are self-consistent rather than host-independent.

## Trying it

```bash
cargo build -p kols-cli
alias kols="$PWD/target/debug/kols"

KOLS_HOME=/tmp/alice kols init "the workshop"     # prints the network id
KOLS_HOME=/tmp/alice kols serve &                 # keys the network, prints an address
KOLS_HOME=/tmp/alice kols channel create general
KOLS_HOME=/tmp/alice kols post general "hello"
```

`serve` must run before posting: it holds the network's MLS group, which is live state no
one-shot command can keep. Then, in another terminal, somebody joins:

```bash
KOLS_HOME=/tmp/bob kols attach <network-id>       # prints their identity
KOLS_HOME=/tmp/alice kols admit <their-identity>  # from the founder
KOLS_HOME=/tmp/bob kols serve --peer <alice's address>
KOLS_HOME=/tmp/bob kols read general
```

The joiner syncs the governance log, asks to be keyed in, receives the epoch keys
including the historical ones, fetches the other author's segments and renders a merged
view — everything over the real transport, nothing shared out of band but the network id.

State lands in `$KOLS_HOME`, else `~/.kols`. The seed written there is the only copy of
your identity and there is no recovery service, so point `--home` somewhere disposable
while you are poking at it.

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

What does not exist yet: no user interface, no private-channel keying, no voice, no search,
no live gossip path — and nothing in the CLI yet drives a *removal*, so the revocation path
is exercised upstream but not end to end. [`STATUS.md`](STATUS.md) is the honest inventory.

## The one number worth knowing

Appending a message to a full segment moves **1,556 bytes of 176,115** locally, and a
reader holding the previous version refetches **one chunk of three** across the wire. That
delta property is what makes a chat log affordable on a storage layer built for documents,
and the whole segment model exists to preserve it — it is asserted on bytes actually moved,
in `crates/kols-core/tests/author_log.rs` and `crates/kols-net/tests/two_nodes.rs`.
