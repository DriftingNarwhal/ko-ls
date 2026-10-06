#!/usr/bin/env bash
# Keep the workspace's build output inside its disk budget.
#
# This container's disk is its host's, and the person whose machine it is has
# set a limit for build output across the three repositories: 12 GB, counted in
# decimal gigabytes as they said it (`KOLS_DISK_BUDGET_GB`). Cargo never deletes
# anything on its own. Every distinct build (lib, test, clippy, rust-analyzer,
# each toolchain, each change to Cargo.lock) leaves artifacts behind that no
# later build reads, so without this the `target/` directories grow until
# somebody notices. On 2026-10-05 they had reached 53 GB, 27 GB of it stale
# incremental-compilation state.
#
# Two cadences, each due by a timestamp rather than a clock, so a container that
# was stopped when one fell due catches up the next time it runs:
#
#   daily   `cargo sweep` for toolchains no longer installed and for dependency
#           artifacts unused for a week; then incremental state for any crate
#           not compiled in three days, which cargo-sweep never touches and which
#           is most of what goes stale; then one-off trees (`target/release`,
#           `target/<triple>`) nothing has written to for a day. If the total is
#           still over budget after that, all incremental state goes, since it is
#           only rebuild speed. Nothing a current build needs is deleted.
#   weekly  `cargo clean` in every repository. A full rebuild costs minutes;
#           what it reclaims is everything the sweep cannot tell is dead.
#
# Run with no arguments to do whatever is due, `--now` to sweep regardless, or
# `--clean` to clean regardless. `--status` only reports. `--loop` does what is
# due every hour; the dev container starts it (`postStartCommand`). Safe to run
# by hand at any time.
#
# It does not start work while cargo or rustc is running in this container, and
# checks again before each repository. It cannot see a build that starts in the
# middle of one repository's sweep, or one run on the host — `DI-Relay/target`
# sits on the bind mount, where a host-side build could be using it.
#
# Needs `cargo-sweep` 0.8 (installed by the Dockerfile).

set -uo pipefail

ROOT=${KOLS_WORKSPACE:-/workspaces/ko-ls}
BUDGET_GB=${KOLS_DISK_BUDGET_GB:-12}
# On the bind mount rather than in the container's home, so rebuilding the
# container keeps the schedule instead of restarting the weekly clock. The
# workspace repository ignores `.cache/`.
STATE=${KOLS_DISK_STATE:-$ROOT/.cache/keep-targets-small}
LOG=$STATE/log

REPOS=(ko-ls distributed-intranet DI-Relay)

mkdir -p "$STATE"
# A state directory seen for the first time starts the weekly clock rather than
# cleaning at once: the target/ volumes outlive a container, and a rebuild is no
# reason to throw them away. The daily sweep costs nothing it needs and runs now.
[ -e "$STATE/cleaned" ] || touch "$STATE/cleaned"

say() { printf '%s %s\n' "$(date '+%F %T')" "$*" | tee -a "$LOG"; }

# Due when the stamp is missing or older than the given number of minutes.
due() { [ -z "$(find "$STATE/$1" -mmin "-$2" 2>/dev/null)" ]; }

busy() { pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null; }

targets() { for r in "${REPOS[@]}"; do [ -d "$ROOT/$r/target" ] && echo "$ROOT/$r/target"; done; }

# Decimal gigabytes, one decimal place.
usage_gb() {
    local dirs
    mapfile -t dirs < <(targets)
    [ ${#dirs[@]} -gt 0 ] || { echo 0.0; return; }
    du -sb "${dirs[@]}" 2>/dev/null | awk '{b += $1} END {printf "%.1f", b / 1e9}'
}

over() { awk -v u="$(usage_gb)" -v b="$BUDGET_GB" 'BEGIN {exit !(u > b)}'; }

report() {
    say "build output: $(usage_gb) GB of ${BUDGET_GB} GB budget"
    over && say "OVER BUDGET — run '$0 --clean'; if that is not enough, the owner decides"
}

# Runs a command with its output in the log, and says so when it fails.
logged() {
    "$@" >>"$LOG" 2>&1 || { say "failed: $*"; return 1; }
}

sweep() {
    command -v cargo-sweep >/dev/null || { say "cargo-sweep is not installed; not sweeping"; return 1; }
    for r in "${REPOS[@]}"; do
        local t=$ROOT/$r
        [ -d "$t/target" ] || continue
        busy && { say "cargo started; leaving $r and the rest for the next run"; return 1; }
        logged cargo sweep --installed "$t"
        logged cargo sweep --time 7 "$t"
        # rustc rewrites a crate's incremental directory on every compile, so its
        # mtime is when that crate was last built this way.
        find "$t/target" -mindepth 3 -maxdepth 3 -path '*/incremental/*' -type d -mtime +2 \
            -exec rm -rf {} + 2>>"$LOG"
        # One-off trees: deleted once nothing in them has been written for a day.
        local d
        for d in "$t"/target/*/; do
            case $(basename "$d") in debug | tmp | flycheck*) continue ;; esac
            [ -z "$(find "$d" -mmin -1440 -print -quit 2>/dev/null)" ] && rm -rf "$d"
        done
    done
    if over; then
        say "still over budget after sweeping; dropping all incremental state (rebuild speed only)"
        find $(targets) -mindepth 2 -maxdepth 2 -name incremental -type d -exec rm -rf {} + 2>>"$LOG"
    fi
    # Test scratch, unless a process still names it.
    local s
    for s in /tmp/kols-*; do
        [ -e "$s" ] && [ -n "$(find "$s" -maxdepth 0 -mtime +1)" ] || continue
        # `pgrep -f` never matches itself, where grep over every cmdline would
        # always find its own; environments are where KOLS_HOME would name it.
        pgrep -f -- "$s" >/dev/null && continue
        grep -qsF "$s" /proc/[0-9]*/environ && continue
        rm -rf "$s"
    done
    touch "$STATE/swept"
    say "swept"
}

clean() {
    local r
    for r in "${REPOS[@]}"; do
        busy && { say "cargo started; leaving $r and the rest for the next run"; return 1; }
        # The two volume mount points cannot be removed themselves, so cargo
        # reports EBUSY after emptying them; what was reclaimed is judged by the
        # size afterwards, not by that.
        [ -f "$ROOT/$r/Cargo.toml" ] && cargo clean --manifest-path "$ROOT/$r/Cargo.toml" >>"$LOG" 2>&1
    done
    touch "$STATE/cleaned" "$STATE/swept"
    say "cleaned"
}

case ${1:-} in
    --loop)
        # One loop per container. A pidfile rather than `pkill -f`, which would
        # match the shell that launched this as readily as the loop it meant —
        # and checked for being *this* loop, since a pid recorded before a
        # restart may since have been given to anything.
        if [ -s "$STATE/loop.pid" ]; then
            old=$(cat "$STATE/loop.pid")
            if [ "$old" != $$ ] && grep -qs keep-targets-small "/proc/$old/cmdline"; then
                exit 0
            fi
        fi
        echo $$ >"$STATE/loop.pid"
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
tail -n 1000 "$LOG" >"$LOG.tmp" 2>/dev/null && mv "$LOG.tmp" "$LOG"
