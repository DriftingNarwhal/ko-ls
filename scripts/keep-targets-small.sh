#!/usr/bin/env bash
# Keep the workspace's build output inside its disk budget.
#
# This container's disk is the host's, and the person whose machine it is has
# set a budget for build output across the three repositories: about 12 GB,
# `KOLS_DISK_BUDGET_GB`. Cargo never deletes anything on its own — every
# distinct build (lib, test, clippy, rust-analyzer, each toolchain, each change
# to Cargo.lock) leaves artifacts behind that no later build reads — so without
# this the `target/` directories grow until somebody notices. On 2026-10-05
# they had reached 53 GB, 27 GB of it stale incremental-compilation state.
#
# Two cadences, each due by a timestamp rather than a clock, so a container that
# was stopped when one fell due catches up the next time it runs:
#
#   daily   `cargo sweep`: drop artifacts from toolchains no longer installed and
#           anything unused for a week, then cap each target/ at its share of
#           the budget, oldest first.
#   weekly  `cargo clean` in every repository. A full rebuild costs minutes;
#           what it reclaims is everything sweep cannot see is dead.
#
# Run with no arguments to do whatever is due, `--now` to sweep regardless, or
# `--clean` to clean regardless. `--status` only reports. `--loop` does what is
# due every hour, and is what the dev container starts (`postStartCommand`), so
# it also runs at every container start; safe to run by hand at any time. It never touches a target/ while cargo or rustc is
# running, since cleaning under a live build breaks it — it says so and leaves
# the work for the next run.
#
# Needs `cargo-sweep` (installed by the Dockerfile).

set -uo pipefail

ROOT=${KOLS_WORKSPACE:-/workspaces/ko-ls}
BUDGET_GB=${KOLS_DISK_BUDGET_GB:-12}
STATE=${XDG_STATE_HOME:-$HOME/.local/state}/keep-targets-small
LOG=$STATE/log

# Each repository's cap, in MiB (binary units throughout, as `du` and the
# report use: `cargo sweep` reads MB as decimal, and a 6000 MB cap once swept
# live artifacts out of a 5.6 GiB tree): what one fresh gate build (tests + clippy)
# occupied on 2026-10-05 — ko-ls 4.6 GB, distributed-intranet 5.6 GB, DI-Relay
# 0.97 GB, 11.2 GB together — with the rest of the budget spread over them, and
# scaled with it. A cap below a fresh build would make every sweep delete live
# artifacts and every gate rebuild them, so if a repository grows past its cap,
# that is a conversation about the budget rather than a reason to cut the cap.
share() {
    case $1 in
        ko-ls) echo $((BUDGET_GB * 5120 / 12)) ;;
        distributed-intranet) echo $((BUDGET_GB * 6144 / 12)) ;;
        DI-Relay) echo $((BUDGET_GB * 1024 / 12)) ;;
    esac
}

REPOS=(ko-ls distributed-intranet DI-Relay)

mkdir -p "$STATE"
# A fresh container starts the weekly clock rather than cleaning on its first
# run, since its target/ volumes outlive it and a rebuild is not a reason to
# throw them away. The daily sweep has no such cost and runs straight away.
[ -e "$STATE/cleaned" ] || touch "$STATE/cleaned"
say() { printf '%s %s\n' "$(date '+%F %T')" "$*" | tee -a "$LOG"; }

# Due when the stamp is missing or older than the given number of minutes.
due() { [ -z "$(find "$STATE/$1" -mmin "-$2" 2>/dev/null)" ]; }

busy() { pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null; }

usage_gb() {
    du -sk "${@/#/$ROOT/}" 2>/dev/null | awk '{k += $1} END {printf "%.1f", k / 1048576}'
}

targets() { for r in "${REPOS[@]}"; do [ -d "$ROOT/$r/target" ] && echo "$r/target"; done; }

report() {
    local used
    used=$(usage_gb $(targets))
    say "build output: ${used} GB of ${BUDGET_GB} GB budget"
    awk -v u="$used" -v b="$BUDGET_GB" 'BEGIN {exit !(u > b)}' &&
        say "OVER BUDGET: ${used} GB > ${BUDGET_GB} GB — run '$0 --clean', or raise the budget with the owner"
}

sweep() {
    command -v cargo-sweep >/dev/null || { say "cargo-sweep is not installed; skipping sweep"; return 1; }
    for r in "${REPOS[@]}"; do
        local t=$ROOT/$r
        [ -d "$t/target" ] || continue
        cargo sweep --installed "$t" >/dev/null 2>&1
        cargo sweep --time 7 "$t" >/dev/null 2>&1
        cargo sweep --maxsize "$(share "$r")MiB" "$t" >/dev/null 2>&1
        # Neither ever earns its space back: a one-off release or cross build.
        find "$t/target" -mindepth 1 -maxdepth 1 -type d \
            ! -name debug ! -name tmp ! -name 'flycheck*' -mtime +1 -exec rm -rf {} + 2>/dev/null
    done
    find /tmp -maxdepth 1 -name 'kols-*' -mtime +1 -exec rm -rf {} + 2>/dev/null
    touch "$STATE/swept"
    say "swept"
}

clean() {
    for r in "${REPOS[@]}"; do
        [ -f "$ROOT/$r/Cargo.toml" ] && cargo clean --manifest-path "$ROOT/$r/Cargo.toml" >/dev/null 2>&1
    done
    touch "$STATE/cleaned" "$STATE/swept"
    say "cleaned"
}

case ${1:-} in
    --loop)
        # One loop per container: a second start finds the first alive and
        # leaves. A pidfile rather than `pkill -f`, which would match the shell
        # that launched this as readily as the loop it meant.
        if [ -s "$STATE/loop.pid" ] && kill -0 "$(cat "$STATE/loop.pid")" 2>/dev/null; then
            exit 0
        fi
        echo $$ > "$STATE/loop.pid"
        while true; do
            "$0"
            sleep 3600
        done
        ;;
    --status) report; exit 0 ;;
    --now) busy && { say "cargo is running; not sweeping"; exit 0; }; sweep; report; exit 0 ;;
    --clean) busy && { say "cargo is running; not cleaning"; exit 0; }; clean; report; exit 0 ;;
    "") ;;
    *) echo "usage: $0 [--now | --clean | --status | --loop]" >&2; exit 2 ;;
esac

if busy; then
    say "cargo is running; leaving what is due for the next run"
    exit 0
fi
if due cleaned $((7 * 24 * 60)); then
    clean
elif due swept $((24 * 60)); then
    sweep
fi
report
# Keep the log from becoming the next thing to clean up.
tail -n 500 "$LOG" > "$LOG.tmp" 2>/dev/null && mv "$LOG.tmp" "$LOG"
