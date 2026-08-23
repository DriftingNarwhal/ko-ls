This is `ko-ls`, a Discord-shaped chat client where a "server" is a network — its own
governance log, membership, epoch key chain and DHT namespace. It is the consuming
implementation of the Distributed Intranet protocol's seventh spec.

**Start with [`STATUS.md`](STATUS.md).** It is a map: where the work stands, what is owed,
and which document owns each subject. It deliberately holds nothing that lives anywhere
else, so read it to find the right document rather than to learn how something works.

**`../distributed-intranet/specs/07-chat-application-spec.md` is normative** where it and
the `design/` set overlap — encoding, entry payloads, the capability vocabulary, the policy
limits. The design set owns client design, rationale and sequencing; where the two disagree,
the spec is right and the design set is stale. `design/06` is the ledger of what the
protocol still owes, and a needed protocol change is recorded there rather than assumed
into existence.

The protocol crates are a **path dependency** on the sibling checkout, so a change there can
break this and its own `cargo test` will not say so. Both repos' gates must pass.

Build, test, and the three traps that make a red suite lie are in
[`CONTRIBUTING.md`](CONTRIBUTING.md). Read them before believing a failure.

## Invariants that are easy to break

Each of these is load-bearing and none is enforced by anything that will shout at you.

- **`kols-core` is I/O-free and deterministic.** Merge ordering, permission resolution,
  encoding and retention must produce identical answers on every node. No libp2p types, no
  clock read, no filesystem — pure functions over explicit inputs, which is how that stays
  testable.
- **Everything crosses `kols-api`, and the gate returns a value nothing else can build.**
  `authorize` hands back an `Authorized` with no public constructor, and the executor takes
  one of those rather than a `Command` — so being handed something nobody checked does not
  compile. A `compile_fail` doctest runs that claim. Do not add a constructor, and do not
  add a second path into the executor.
- **Records have exactly one serialization and it is normative.** Spec 07 §3, hand-written,
  because a record's id is the hash of those bytes. Never derive `Serialize` on a domain
  type: a second serialization beside the first eventually goes over a wire and ids stop
  matching. `kols-app` converts to its own view types instead, which is why `kols-api` has
  no `serde` dependency at all — keep it that way.
- **The webview never builds a `Command`.** It names an intent with plain arguments and the
  shell constructs the command, so a front end cannot hand the core a shape it did not
  expect.
- **Merge by record id; never append.** A record arriving over gossip is *also* inside the
  segment that follows it, so duplicate delivery is the normal case rather than an edge
  one. Render from the merged projection through `ChannelView`. A consumer that appended
  what it was handed shows every message twice, every time.
- **Frozen test vectors are the contract.** A change that alters one is a wire break to be
  recognised and versioned, never a value to re-bless because a test went red.
  `crates/kols-core/tests/encoding.rs`.
- **A command's consent class follows the tier of the capability it needs**, not how
  consequential the action looks — so pinning prompts harder than posting. A drift test
  resolves every command's verb against the vocabulary's own tier table.
- **Hiding a control is presentation, never enforcement.** Chrome is gated on permission for
  the user's sake; the core re-checks on receipt regardless. A hidden button looks like a
  check and is not one.
- **Key material types implement no `Debug` and no serialization, deliberately.** Use their
  `fingerprint()` methods for logging and tests rather than deriving around it.
- **A crate is created when there is code for it.** An empty crate is a claim that something
  exists. `kols-store` and `kols-media` are named in `design/05` §2 and do not exist.

The one `unsafe` in this workspace is the Windows DACL path in `kols-node::secret`, which is
FFI and is documented where it sits. Everything else is safe Rust; keep it that way.

## Two habits this project has paid for

**Reproduce before reasoning.** Several long-lived bugs here were misdiagnosed from reading
and found in one run — a keying race invisible at full width and reliable at two cores, a
"flaky" suite that was orphaned processes holding ports. Check the machine is clean before
believing a reproduction.

**Say what is actually true.** Where a guarantee is weaker than a user would expect —
deletion hides rather than un-sends, retention is not deletion, unlinkability is a property
of keys and not of traffic — the interface and the documents state the weaker thing. An
honest limit is a feature of this project, not an omission to tidy away.
