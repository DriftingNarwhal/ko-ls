# Contributing

## Licensing of contributions

This project is **licensed under the GNU Affero General Public License v3.0 (`LICENSE`)**. By contributing you agree your contribution is licensed
under those same terms.

## The licence split, and the one constraint that binds both repositories

Three licences are in play across the two repositories, and the combination is deliberate
rather than incidental:

| What | Licence | Why |
|---|---|---|
| This client, and `DI-Relay` | **AGPL-3.0-only** | The application is not meant to be enclosed |
| The `intranet-*` protocol crates | **MPL-2.0** | The platform *is* meant to be built on by other implementations |
| `specs/` in the protocol repo | **CC BY 4.0** | A software licence is the wrong instrument for prose an implementer will quote |

All are © DriftingNarwhal.

**The constraint: MPL files in the protocol repo must never carry Exhibit B.** MPL §3.3 —
which permits covered software to be combined into a larger work under a Secondary Licence,
AGPL-3.0 being one — is the *only* reason an AGPL client may link those crates. Marking a
protocol source file "Incompatible With Secondary Licenses" would silently make this whole
workspace undistributable, and nothing in either build would say so.

**`DI-Relay` is AGPL for the reason the client is**, and the Affero clause is the half that
does the work: a relay is a service by definition, so a licence triggering only on
distribution would never trigger at all. It does not reach the network served — a relay holds
no state, so running one places no licence condition on anybody's client or content.

**Anything depending on the protocol by tag must use `v1.0.1` or later.** `v1.0.0` predates
the licence, so building against it is building against code that is still all rights
reserved. A licence on `main` does not reach a tag.

## The gate

`KOLS_PASSWORD=<anything> cargo test --workspace`, `cargo clippy --workspace --all-targets`
and `cargo fmt --all --check` must all be clean before a change lands. A run that skipped
clippy because the toolchain lacked it has checked only part of the gate and should say so.

**Formatting is rustfmt's defaults, with no configuration.** Run `cargo fmt --all` before
committing. It also formats the protocol crates, which this workspace reaches by path, and the
protocol's own gate checks them too.

**The password is not optional and is not a convenience.** Seeds are wrapped under an
account key (`design/02` §6.3), and there is deliberately **no unwrapped path** — not even
for tests, because a code path that read a seed without a secret would be a second security
posture, and D30 keeps the terminal a test harness rather than a second interface. So the
suite unlocks exactly as the window does and differs only in where the password comes from.
Any value works; the harness provisions an account at deliberately cheap Argon2 parameters,
since a test account protects nothing and paying 64 MiB per process to protect it would make
the daemon suite (O20) fragile for a second reason.

**Use `--no-fail-fast` when a run goes red.** `cargo test` stops at the first failing test
*binary*, so a single early failure hides every target after it — which reads as though the
change broke far less than it did.

**Run the tests here, not in CI.** CI builds for Windows and macOS, which this container
cannot do; it does not gate. A failure reproduced locally arrives with complete output in
the minute it appears, which is the difference between diagnosing a fault and copy-pasting
log fragments at one.

**The front end has no gate, and one thing to run by hand instead.** `crates/kols-ui` is
some four and a half thousand lines of JavaScript, across a script per window (`app.js`,
`workspace.js`, `conversation.js`), that nothing asks questions of. `drive.mjs` beside it
loads the real pages and their scripts into `jsdom` with a stubbed `invoke`, walks the
interface, and reports:

```
cd crates/kols-ui && npm install jsdom && node drive.mjs
```

It is not wired into the gate — adding a second toolchain to `cargo test` is a decision
nobody has made — and it applies no CSS, so it says nothing about layout. Run it after
changing any of the three scripts. Its first half hour found two bugs that reading had not.

**A green suite here can still be wrong about Windows, and once was.** `v0.13.2` opened a
window from a synchronous `#[tauri::command]`, which runs inline in the IPC handler — on
Windows the webview's message-loop thread, where `WebviewWindowBuilder::build()` deadlocks
(Tauri's *Known issues*, wry#583). It shipped: a blank white window that could not be closed,
and a tray whose quit did nothing. On Linux the identical code is correct, because WebKitGTK
creates its webview synchronously on the calling thread, so nothing here could have failed.

So when a shell behaviour depends on the platform, ask the second question as well as the
first — not only *is this asserted against the real configuration* (`design/05` §1), but
*could that assertion be true here and false where it ships*. Where it could,
the property gets checked as a rule about the source, which holds everywhere:
`crates/kols-app/tests/window_creation.rs` is that, and the way to write one is to revert the
fix and watch it fail, because a guard that passes on the broken code is not a guard.

Three things about the suite that are not obvious, each learned by getting them wrong:

- **`cargo clippy` on the Windows target is a second run**, `cargo clippy -p kols-node
  --target x86_64-pc-windows-gnu`. It catches warnings the Linux run cannot see, so it is
  worth making whenever `crates/kols-node/src/secret.rs` changes.
- **The daemon tests read `kols serve`'s stdout, so those strings are a contract.**
  `two_nodes.rs` waits on `listening`, `keyed into this network` and `learned 1 record` by
  substring. They are produced through `kols_node::Report` rather than by `println!` —
  `kols` passes a report that prints and the window passes `quiet`, because a
  GUI-subsystem process on Windows has no stdout and Rust panics on the write rather than
  dropping it. Changing one of those lines is changing what the suite waits for; changing
  *where* they go is how the window avoids crashing on Windows. Neither is a free edit.
- **Run the daemon tests starved as well as fast**: `taskset -c 0,1 cargo test -p kols-node
  --test two_nodes`. These spawn two or three processes that sign and encrypt, and a wide
  machine wins every race a narrow one loses — a keying bug latent for weeks was invisible at
  full width and reproducible in one run at two cores. `three_nodes` spawns three or four
  apiece and is heavier again; its deadlines are correspondingly longer, and the comment at
  the top of that file says why.

  **The whole suite starved is currently unreliable, and that is a fact about the suite rather
  than about your change.** Run together at two cores, one or two tests time out in
  `wait_for` — usually `a_node_offline_across_a_rotation_catches_up_and_can_still_read`,
  sometimes `a_joiner_walks_back_through_sealed_segments_to_read_the_start`. Measured on
  2026-08-29: at `main` with nothing else applied, two of eleven failed; the same run with a
  branch applied failed one. Each passes on its own starved — 113 s for the first — so what
  is being hit is contention between eleven tests' daemons over two cores, not the property
  under test.

  So a starved failure is a **signal to isolate, not a red suite**. Re-run the named test
  alone before believing it, and if you need to attribute a failure, run the same suite at
  `main` — that comparison is the only thing that separates a regression from this. Worth
  fixing eventually: the deadlines come from `tests/common::patience`, which scales with
  available parallelism but evidently not with how many daemons are competing for it.

  **Measured again on 2026-08-30, on a 24-core box, and the picture is worse than the
  paragraph above.** `a_node_offline_across_a_rotation_catches_up_and_can_still_read` now
  fails on *every* full-workspace run here and passes alone in 50 s. Not intermittent, and
  not starvation in the two-core sense — `factor()` returns 1 at this width, so no deadline
  is scaled at all, while fourteen tests' worth of signing and encrypting daemons are up at
  once. The suite has outgrown "twelve is the width this was written on and comfortable at".

  **Measured again across four full-width runs on 2026-08-31 and 2026-09-01: three clean, one
  failed.** So it is intermittent rather than deterministic, and the paragraph above saying it
  fails on *every* full-workspace run described a run of bad luck rather than a property.
  **That is not a fix and must not be read as one.** An intermittent failure is the same
  failure, and one clean run proves nothing — which is exactly what makes it expensive: it will
  go quiet for a day and come back on the run where you have just changed something.

  **The one run that failed is worth reading for how it was attributed, because it looked
  like a regression.** It came directly after a change to `intranet-governance` — a dependency
  of everything here — so "the protocol change broke a live-wire test" was the obvious
  reading. Four measurements settled it and none of them was reasoning: the named test passed
  alone in 77 s; the full suite with the protocol change reverted passed 306; the full suite
  with it applied passed 306 on the next run; and the change itself is inert for this client,
  which never writes the entry it affects. **Run the comparison even when you are sure.** The
  cost is ten minutes and it is the difference between "the flake" and "the flake, probably".

  **Attribute before believing it, and attribute by removing things.** Adding `three_nodes`
  looked like the cause — it holds four daemons up for two minutes beside `two_nodes`, and
  its arrival coincided with this going deterministic. It is not: moving that one file out of
  `tests/` and re-running the whole workspace reproduces the same single failure. Two full
  runs with it and one without, same test each time. That check took ten minutes and was the
  difference between a true statement and a plausible one.

  **Adding daemon tests still makes this worse for every other test.** `three_nodes`'s own
  three tests did time out at full width when first added, in the very tests they had just
  introduced — which reads exactly like the new feature being broken, and was not. They take
  a process-wide mutex now so only one runs at a time; that fixed them and left the failure
  above untouched, which is what identified it as separate. If you add a file like this,
  serialize it, budget its deadlines against the whole suite rather than a run of that file,
  and re-run the full gate before believing either result.
- **Kill orphaned daemons before believing a red suite**: the harness first, then the daemons,
  with a bracketed character in every pattern for the reason below —
  `for p in $(pgrep -f 'deps/two_node[s]-') $(pgrep -f 'deps/three_node[s]-'); do kill -9 $p; done`,
  then the same over `pgrep -f 'debug/kol[s] --home'`.
  `Daemon::drop` kills its child, and `Drop` does not run when the *harness itself* is killed
  — a Ctrl-C, a timeout, a `pkill` on cargo. The orphans keep listening on their fixed
  ports — 45101–45172 (`two_nodes`), 45301–45334 (`three_nodes`), and the 452xx, 454xx and
  455xx of `flow`, `names` and `invites` — indefinitely, and every later run fails to bind.
  It presents as two or three tests failing in ways that read as distributed-systems faults,
  and it is not one.

  **Two ways that cleanup silently does nothing**, both found by watching it fail — and the
  first one twice in one session, the second time *after* it had been written down here.

  **`pkill -f` matches the shell running it.** `-f` tests the whole command line, and the
  command line of the shell executing that `pkill` contains the pattern, so it kills its own
  parent and dies before reaching anything. It presents as an unexplained exit code beside a
  cleanup that appears to have run. This is a property of `-f` rather than of any particular
  pattern, which is the part worth carrying: bracketing one name (`'target/debug/[k]ols'`) and
  then writing the next one plainly (`'two_nodes-'`) reproduces it exactly. Bracket a
  character in **every** such pattern, or avoid `pkill` altogether —
  `for p in $(pgrep -f 'deps/two_node[s]-'); do kill -9 $p; done`.

  **Killing the daemons is not enough.** The **test harness** itself
  (`target/debug/deps/two_nodes-*`) survives and immediately spawns replacements, so the port
  list looks busy again a second later and the PIDs are new each time you look — which reads
  as a kill that is not working. Kill the harness first, then the daemons.

  Then check rather than assume: `pgrep -f 'deps/two_node[s]-'` and
  `pgrep -f 'debug/kol[s] --home'` both empty, `ss -ltn | grep -E ':45[1-5][0-9]{2}'` empty, and `/tmp/kols-*`
  removed.

  Worth doing before *any* starved run, since that is exactly where a leftover port presents
  as the race the starving was supposed to find.

**Clean up afterwards, and stay inside the budget.** This container's disk is its host's — a
person's own machine — and **build output across the three repositories (this one,
`distributed-intranet` and `DI-Relay`) stays near or below 12 GB**. That limit is the host
owner's, not a property of the volume, so free space there is no reason to exceed it; raising
it is a conversation to have first. `cargo` never reclaims anything on its own, so it is kept
there on a schedule rather than by memory: `scripts/keep-targets-small.sh` runs `cargo sweep`
daily and `cargo clean` weekly across all three, and the dev container starts it hourly
(`--loop`). Run it by hand with `--status` to see where things stand, `--now` to sweep, or
`--clean`.

What one fresh gate build occupies, measured on 2026-10-05: this repository 4.6 GB,
`distributed-intranet` 5.6 GB, `DI-Relay` 1.0 GB — 11.2 GB together, which is why the budget
has little slack and why the script caps each at its share. Both main repos build the same
dependency tree, so their `target/` directories largely duplicate each other and neither can
be shared away; the number is the sum.

- **Dependencies carry no debug information** (`[profile.dev.package."*"]` in all three
  workspaces), and our own crates only line tables. It halved `DI-Relay`'s build and is most
  of why the three fit at all.
- **`target/debug/incremental`** is the largest thing that is pure rebuild speed, and the
  first to go stale: every distinct build (test, clippy, rust-analyzer, each toolchain) keeps
  its own, and none is ever collected. It reached 27 GB before the schedule existed.
- **`target/release`** is only ever built here to measure (`tests/cost.rs`), where a debug
  build is about forty times slower and answers the wrong question. Delete it when the
  measuring is done; the script removes any one-off tree a day later regardless.
