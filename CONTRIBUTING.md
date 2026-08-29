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

`cargo test --workspace` and `cargo clippy --workspace --all-targets` must both be clean
before a change lands. A run that skipped clippy because the toolchain lacked it has
checked half the gate and should say so.

**Run the tests here, not in CI.** CI builds for Windows and macOS, which this container
cannot do; it does not gate. A failure reproduced locally arrives with complete output in
the minute it appears, which is the difference between diagnosing a fault and copy-pasting
log fragments at one.

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
  full width and reproducible in one run at two cores.

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
- **Kill orphaned daemons before believing a red suite**: `pkill -f 'target/debug/kols'`.
  `Daemon::drop` kills its child, and `Drop` does not run when the *harness itself* is killed
  — a Ctrl-C, a timeout, a `pkill` on cargo. The orphans keep listening on ports 45101–45162
  indefinitely, and every later run fails to bind. It presents as two or three tests failing
  in ways that read as distributed-systems faults, and it is not one.

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
  `pgrep -f 'debug/kol[s] --home'` both empty, `ss -ltn | grep 451` empty, and `/tmp/kols-*`
  removed.

  Worth doing before *any* starved run, since that is exactly where a leftover port presents
  as the race the starving was supposed to find.

**Clean up afterwards.** This container's storage is the host's, and a day of cross-compiles
took `target/` to 11 GB.
