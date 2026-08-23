# Finalization and P0 Build Plan

**Document status:** v1.1 — **complete.** Every one-off item here is done: F2 and S1 now carry the done markers they had earned elsewhere, and S2 is named as the standing practice it is rather than an item left unfinished. P1 is under way and tracked in `STATUS.md`, not here
**Depends on:** `00`–`06`
**Consumed by:** whoever starts building

---

## 0. Where Things Stand

The design set is complete at the architectural level: every mechanism has a home, every
protocol change is enumerated with acceptance criteria, and no architectural question is
carried open. What was *not* done when this was written is the layer below architecture —
byte formats, repository layout, and a development environment that can build a Tauri app.

This document lists exactly that, in the order it has to happen. All of it is now done;
what each item turned out to cost is recorded in its own section.

---

## 1. Finalize the Documents (Before Any Code)

### F1 — Pin the canonical encoding of chat records ✅ **done**

`01` §3.3 specifies record *shapes*. It does not specify their *bytes*, and it must,
because `message_id = H(canonical record bytes)`. Every reference to a message — replies,
reactions, redactions, pins — is that hash, so two implementations disagreeing by a byte
would produce two different ids for the same message and silently fail to reference each
other.

The protocol already establishes the pattern and the reasoning: encoding is hand-written
per type (`intranet_crypto::Enc`, domain-separated), deliberately not derived, because
determinism here is load-bearing and a derive macro's output is not a stable contract.
Chat records get the same treatment.

**Delivered as [`08-record-encoding.md`](08-record-encoding.md)**, normative: domain tag
registry, the common record header, kind discriminants grouped by rate class, what the
signature covers, how the record id is formed, the segment container, every derived
identifier, the channel content key derivation, evolution rules for unknown kinds, and the
six conformance tests required before P0 is done.

Three things it settled that were not obvious going in, each recorded there with its
reasoning: the discriminant range encodes the **rate class**, so a node that does not
understand a future record kind still counts it identically for rate limiting; the
`device` field is present from v1 despite v1 shipping single-device, because adding an
authenticated field later costs a second encoding forever; and an unknown record kind is
**retained, counted and not rendered** rather than rejected or dropped, since both
alternatives are worse.

It folds into spec 07 (F2) as written.

### F2 — Write the seventh specification document ✅ **done**

`distributed-intranet/specs/07-chat-application-spec.md`, consuming the platform the way
App Hosting and Search do. It carries: the channel and record model (`01`), the extension
capability vocabulary and its tiers (`02` §2.2), the keying tiers (`03`), the record
encoding (F1), and the wire formats for E4's gossip payload and E10's DM invite.

Not optional bookkeeping: the repository's central claim is that the specs are
authoritative and the code implements them. Adding entry types, a capability vocabulary
and two wire protocols without writing them down would make that claim false, and the
README says so in its own words.

**Delivered** as `distributed-intranet/specs/07-chat-application-spec.md`, committed to the
protocol repository. It is normative where it and this design set overlap, and it is the
document that took F1's encoding over — which is why `08` stopped being normative in F3.

### F3 — Mark the design set reviewed ✅ **done**

Bumped `00`–`08` to v1.0 after a review pass against the working P0 implementation, which
did shake something out, as expected: **`08` stopped being normative.** Its content moved
upstream in F2, and leaving two normative descriptions of one wire format in place would
have been precisely the drift this project criticises elsewhere. `08` is now the client's
conformance obligations and module map; Chat Application Spec §3 is the contract.

The review also caught the design set claiming no implementation existed, a roadmap
listing P0 as future work, and a scale section calling every threshold a guess when two of
them had since been measured.

---

## 2. Set Up (Parallel With Finalization)

### S1 — Repository layout ✅ **done**

The client is its own git repository. Suggested shape, with the protocol as a **path
dependency** during development so protocol changes and client changes can be made and
tested together:

```
/workspaces/ko-ls/
├── distributed-intranet/     existing repo — protocol, specs, harness
└── ko-ls/                    new repo — the client
    ├── .devcontainer/        the environment for both workspaces (S3)
    ├── design/               these documents (moved from chat-app/design/)
    └── crates/               kols-core, kols-store, kols-net, kols-media, kols-api, kols-app
```

Switch the dependency to a git tag once the protocol changes have landed and stabilised.

**The dev container config lives in the client repo and is tracked there**, because the
client is what needs a Tauri toolchain and this document is what owns S3. Since the path
dependency above means a container seeing only this repo would be missing half the build,
it mounts the *parent* of the two and lands at `/workspaces/ko-ls` — the tree drawn here.
So the folder to open is `ko-ls`, and the tree you land in is the one above. The protocol
repo carried a second, older config until S3; two configs for one workspace is the same
drift this project objects to elsewhere, so there is now one.

### S2 — Protocol changes land on `main`, one at a time — **a standing practice, not a task**

The protocol repo is clean and on `main` at `DriftingNarwhal/distributed-intranet`. Each
extension lands as its own change carrying **spec text, implementation and tests
together**, with `cargo test --workspace` and `cargo clippy --workspace --all-targets`
both green — the repo's existing gate. Not a long-lived feature branch: these are small,
independent, and each one improves the protocol for any consumer.

Order for P1: **E9** (app policy map, small) → **E2** (channel entry variants) →
**E4** (gossipsub). E3 was dropped from this list — deriving pointer ids needs no
protocol change, since `PointerId::from_bytes` is already public (`06` §3). **P0 therefore
requires no protocol change at all**, which is a better position than the plan assumed.

**This item never completes, and that is not it being unfinished.** F1, F3, S1 and S3 were
each done once. S2 describes how every protocol change lands, so it holds for as long as
extensions are still landing. Which of them have is `06` §0's table, which is the one place
that state is kept.

### S3 — Development environment ✅ **done**

All three items landed in `.devcontainer/`, so a rebuilt container has them rather than
each machine acquiring them by hand:

- **Node.js 24 LTS**, from NodeSource. Debian 12 ships Node 18, which is past end of life.
- **webkit2gtk 4.1 and the GTK stack Tauri v2 links against** — `libwebkit2gtk-4.1-dev`,
  `libjavascriptcoregtk-4.1-dev`, `libsoup-3.0-dev`, `libayatana-appindicator3-dev`,
  `librsvg2-dev`, plus `patchelf`, `file` and `xdg-utils` for the bundler.
- **A display path**, confirmed by running rather than by argument.

**Two things this list had wrong, both found by running it.** `libappindicator3-dev` does
not exist in Debian 12 — the tray library it ships is the Ayatana fork,
`libayatana-appindicator3-dev` — so the environment as written could not have been built.
And the display arrives differently than assumed: there is no `/mnt/wslg` inside the
container, because WSLg is on the *host* and what reaches the container is VS Code's own
forwarding — an X server on `$DISPLAY` and a Wayland socket in `$XDG_RUNTIME_DIR`, both
live. GTK takes the Wayland one when `WAYLAND_DISPLAY` is set, which leaves nothing for
`xwininfo` to observe, so `GDK_BACKEND=x11` is what a script checks a window with.

**Confirming it meant building a Tauri v2 app and launching it**, which is the only form
of confirmation this item accepts: a scaffolded app compiled and linked against the system
webview in 44 seconds, and an 800×600 window mapped on the X display within a second, with
its WebKit child process alongside. Done in a scratch directory, not in this repo — crates
are created when there is code for them.

One thing worth having found now rather than at P1: the image carried only the C locale
while `LANG` arrived set to `en_US.UTF-8`, so GTK started with *"Locale not supported by C
library"* and fell back. A poor footing for a client whose entire payload is other
people's text, and one generated locale away from fixed.

None of this blocked P0. All of it blocked P1, and no longer does.

---

## 3. P0 — The Spike

**Purpose: falsify the assumption the whole design rests on**, which is that an author's
channel log can be an append-grown, segment-chained object on this storage layer at chat
message rates. Everything else in the design is arrangement; this is the load-bearing
claim, and it is cheapest to break now.

### Scope

Two nodes, one network, one public channel, text only, durable path only, CLI interface.

**In:** derived pointer ids (E3); record and segment encoding (F1); segment publish,
append and seal; pointer resolve and delta-fetch; merge ordering across two authors;
`publish:chat-log` permission checked against replayed governance state.

**Out, deliberately:** gossip live path, any UI, private channels, DMs, voice, search,
attachments, moderation, retention.

### Definition of done

1. Node A posts 100 messages across several sealed segments; node B renders all 100 in
   an order node A computes identically.
2. Appending one message to an open segment transfers **only the new tail chunks** —
   asserted against actual bytes moved, not assumed from the design. If this fails, the
   segment model needs rethinking and P0 has done its job.
3. Both nodes partition, both post, heal, and converge on a byte-identical rendered
   order — the property `05` §8 calls the correctness claim of the design.
4. An identity without `publish:chat-log` has its records refused by the *reader*, not
   merely by its own client.
5. Pointer version collision between two writers resolves by lower record hash, and the
   loser's records are re-published rather than dropped.

### Outcome

**All five met.** Two findings changed the design rather than the code: the record list
carries no count prefix (spec 07 §3.5), and HLC strictness is per device rather than per
author (spec 07 §2.6). Both were found by measurement and assertion, not review — which is the
argument for putting a byte-level test at the front of a plan rather than at the end.

Three things the protocol's own guidance warned about were all real, and all cost time
exactly where it said they would: Kademlia stays in client mode without a confirmed
external address, so provider lookups return nobody on loopback; a holder that has not
advertised capacity is dropped by source selection regardless of what the DHT says; and a
manifest needs its own fetch round before the chunks it names can be requested.

### What P0 measures, for tuning later

Bytes on the wire per message at steady state; segment seal latency; time to render a
cold channel; DHT provider-record churn per publish. These feed the thresholds `00` §4
says are currently guesses — segment size and age, backfill depth, gossip fanout.

---

## 4. Recommended Order

```
F1  pin record encoding           ✅ done — 08-record-encoding.md
                ↓
S1  create client repo            ✅ done
S2  land E9 (E3 not needed)      ─┐ can proceed in parallel
F2  write spec 07                ─┘
                ↓
P0  the spike, against real crates
                ↓
F3  design set → v1.0, revised by what P0 taught
                ↓
S3  environment for Tauri  →  E2, E4  →  P1
```

With F1 done, nothing blocks P0 but the repository itself. F2 should land before or
alongside P0 rather than after: writing the spec is what catches an encoding decision that
reads fine in isolation and wrong next to the protocol's own.

**All of it is now done, and this document's job is finished.** It existed to get from a
complete design to a first line of code, and the last thing in its way — S3's environment —
is built. P1's progress and what it still owes live in `STATUS.md`, which is updated in the
same change that moves work; a build plan kept as a running status would be a second one to
disagree with it.
