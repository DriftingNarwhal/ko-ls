# Finalization and P0 Build Plan

**Document status:** v1.0 — F1, F2, S1 and P0 complete; F3 done; P1 not started
**Depends on:** `00`–`06`
**Consumed by:** whoever starts building

---

## 0. Where Things Stand

The design set is complete at the architectural level: every mechanism has a home, every
protocol change is enumerated with acceptance criteria, and no architectural question is
carried open. What is *not* done is the layer below architecture — byte formats,
repository layout, and a development environment that can build a Tauri app.

This document lists exactly that, in the order it has to happen.

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

### F2 — Write the seventh specification document

`distributed-intranet/specs/07-chat-application-spec.md`, consuming the platform the way
App Hosting and Search do. It carries: the channel and record model (`01`), the extension
capability vocabulary and its tiers (`02` §2.2), the keying tiers (`03`), the record
encoding (F1), and the wire formats for E4's gossip payload and E10's DM invite.

Not optional bookkeeping: the repository's central claim is that the specs are
authoritative and the code implements them. Adding entry types, a capability vocabulary
and two wire protocols without writing them down would make that claim false, and the
README says so in its own words.

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

### S1 — Repository layout

The client is its own git repository. Suggested shape, with the protocol as a **path
dependency** during development so protocol changes and client changes can be made and
tested together:

```
/workspaces/ko-ls/
├── distributed-intranet/     existing repo — protocol, specs, harness
└── ko-ls/                    new repo — the client
    ├── design/               these documents (moved from chat-app/design/)
    └── crates/               kols-core, kols-store, kols-net, kols-media, kols-api, kols-app
```

Switch the dependency to a git tag once the protocol changes have landed and stabilised.

### S2 — Protocol changes land on `main`, one at a time

The protocol repo is clean and on `main` at `DriftingNarwhal/distributed-intranet`. Each
extension lands as its own change carrying **spec text, implementation and tests
together**, with `cargo test --workspace` and `cargo clippy --workspace --all-targets`
both green — the repo's existing gate. Not a long-lived feature branch: these are small,
independent, and each one improves the protocol for any consumer.

Order for P1: **E9** (app policy map, small) → **E2** (channel entry variants) →
**E4** (gossipsub). E3 was dropped from this list — deriving pointer ids needs no
protocol change, since `PointerId::from_bytes` is already public (`06` §3). **P0 therefore
requires no protocol change at all**, which is a better position than the plan assumed.

### S3 — Development environment

Present and working: Rust 1.97.1, cargo, clippy, Docker (for the NAT harness).

Missing, and needed before the Tauri shell can build:

- **Node.js** — for the UI toolchain. Not needed for P0, which is CLI-only.
- **webkit2gtk 4.1 + Tauri system dependencies** — `libwebkit2gtk-4.1-dev`,
  `libappindicator3-dev`, `librsvg2-dev`, `patchelf`, `build-essential`. Also CLI-free
  for P0.
- **A display path for GUI on WSL2** — WSLg handles this on Windows 11; worth confirming
  early rather than discovering it at P1.

None of this blocks P0. All of it blocks P1.

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
