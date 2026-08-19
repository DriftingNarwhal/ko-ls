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
crates/kols-cli    `kols` — the terminal client, and the first runnable thing here
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
cargo test                                   # 72 tests, no network or services needed
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
cargo run -p kols-cli -- init "the workshop"
cargo run -p kols-cli -- channel create general --topic "anything at all"
cargo run -p kols-cli -- post general "first message"
cargo run -p kols-cli -- read general
```

State lands in `$KOLS_HOME`, else `~/.kols`. The seed written there is the only copy of
your identity and there is no recovery service, so point `--home` somewhere disposable
while you are poking at it.

## What exists

Text chat's foundations, proven end to end between two live nodes: canonical record
encoding with frozen vectors, per-author segment logs on the storage layer, merge and
render, permission resolution, channel structure as governance-log payloads, and
publish/fetch over the real transport. `kols` drives all of that from a terminal on one
node — create a network, define a channel, post, read.

What does not exist yet: the CLI's **wire half**, so two `kols` installs cannot yet reach
each other (`kols-net` does this in tests; the CLI does not drive it). No user interface,
no private-channel keying, no voice, no search, no live gossip path.
[`STATUS.md`](STATUS.md) is the honest inventory.

## The one number worth knowing

Appending a message to a full segment moves **1,556 bytes of 176,115** locally, and a
reader holding the previous version refetches **one chunk of three** across the wire. That
delta property is what makes a chat log affordable on a storage layer built for documents,
and the whole segment model exists to preserve it — it is asserted on bytes actually moved,
in `crates/kols-core/tests/author_log.rs` and `crates/kols-net/tests/two_nodes.rs`.
