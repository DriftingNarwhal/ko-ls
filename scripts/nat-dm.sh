#!/usr/bin/env bash
# Two real installations holding a direct message across the Docker NAT matrix.
#
# The matrix (`../distributed-intranet/harness/`) proves two peers *connect*, and
# at which tier. It carries no application message, so it cannot say how long a
# message takes, or whether one arrives at all. This runs `kols-drive` — a whole
# installation, headless, doing what the window does — in two of the matrix's
# peer containers, has them meet in a shared network, start a conversation, and
# talk, and reports when each message landed on the other side.
#
#   scripts/nat-dm.sh <scenario> [messages]
#
#   1  same LAN, no NAT between them       peer-a  + peer-a2
#   3  two restricted-cone NATs            peer-a  + peer-b   (hole-punch expected)
#   4  restricted NAT against CGNAT, v4    peer-a  + peer-c   (no direct path, so no join:
#                                                             Core §5.2, and the expected end)
#
# The topology is DI's and is used unchanged; only the binary is ko-ls's, copied
# into the running peers. Build it first:
#
#   cargo build -p kols-node --bin kols-drive
#
# Logs land in $OUT (default: a fresh directory under /tmp), one per side, every
# line stamped with the wall clock in milliseconds. Both containers share the
# host's clock, so a record's lag is its arrival minus its author's stamp.
#
# KEEP=1 leaves the topology up afterwards, for poking at by hand. RELAY=both runs
# the relay on QUIC as well as TCP, which DI-Relay as deployed does not.

set -euo pipefail

SCENARIO="${1:?usage: nat-dm.sh <1|3|4> [messages]}"
MESSAGES="${2:-20}"
HERE="$(cd "$(dirname "$0")" && pwd)"
KOLS="$(cd "$HERE/.." && pwd)"
DI="$(cd "$KOLS/../distributed-intranet" && pwd)"
# An empty client config of its own: the dev container's points at VS Code's
# credential helper, which fails outside an attached editor ("error listing
# credentials") and takes every image pull with it. Nothing here needs a login.
export DOCKER_CONFIG="${DOCKER_CONFIG_DRIVE:-$(mktemp -d /tmp/kols-docker-config.XXXXXX)}"
[[ -f "$DOCKER_CONFIG/config.json" ]] || echo '{}' > "$DOCKER_CONFIG/config.json"
COMPOSE=(docker compose -f "$DI/harness/docker/compose.yml")
# RELAY=both keeps DI's relay as written, TCP and QUIC; the default is the
# deployed relay's shape (`nat-dm/relay-tcp-only.yml` says why it matters).
[[ "${RELAY:-tcp}" == tcp ]] && COMPOSE+=(-f "$HERE/nat-dm/relay-tcp-only.yml")
BIN="${BIN:-$KOLS/target/debug/kols-drive}"
OUT="${OUT:-$(mktemp -d /tmp/kols-nat-dm.XXXXXX)}"
TICK="${TICK:-60}"

case "$SCENARIO" in
  1) A=peer-a; B=peer-a2 ;;
  3) A=peer-a; B=peer-b ;;
  4) A=peer-a; B=peer-c ;;
  *) echo "unknown scenario '$SCENARIO' (expected 1, 3 or 4)" >&2; exit 2 ;;
esac

log() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
die() { printf '\033[31mFAIL\033[0m %s\n' "$*" >&2; exit 1; }

# A request to the installation running in a container.
drive() {
  local service="$1"; shift
  "${COMPOSE[@]}" exec -T "$service" /usr/local/bin/kols-drive --home /w "$@"
}

# Waits until a request's answer holds a needle as a whole word.
#   until_says <seconds> <needle> <service> <request...>
until_says() {
  local seconds="$1" needle="$2"; shift 2
  local deadline=$((SECONDS + seconds)) said=""
  while ((SECONDS < deadline)); do
    said="$(drive "$@" 2>/dev/null || true)"
    # A whole line, or a whole word within one: `keyed` must not match `not keyed`.
    if grep -qE "(^|[[:space:]/])${needle}($|[[:space:]/])" <<<"$said"; then
      printf '%s' "$said"; return 0
    fi
    sleep 1
  done
  return 1
}

cleanup() {
  for service in "$A" "$B"; do
    "${COMPOSE[@]}" exec -T "$service" sh -c 'cat /w.log' > "$OUT/$service.log" 2>/dev/null || true
  done
  [[ -z "${DOCKER_CONFIG_DRIVE:-}" ]] && rm -rf "$DOCKER_CONFIG"
  if [[ -z "${KEEP:-}" ]]; then
    "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  fi
  echo "logs: $OUT"
}

main() {
  [[ -x "$BIN" ]] || die "no $BIN — cargo build -p kols-node --bin kols-drive"
  trap cleanup EXIT

  log "topology up"
  # Down first, so no installation, relay reservation or conntrack entry from an
  # earlier run is still in the picture.
  "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  "${COMPOSE[@]}" up -d --build >/dev/null 2>&1
  for _ in $(seq 1 30); do
    "${COMPOSE[@]}" exec -T relay curl -fsS http://127.0.0.1:8080/health 2>/dev/null \
      | grep -q ready && break
    sleep 2
  done
  local relay_id
  relay_id="$("${COMPOSE[@]}" exec -T relay curl -fsS http://127.0.0.1:8080/peer-id \
    | sed 's/.*"peer_id":"\([^"]*\)".*/\1/')"
  [[ -n "$relay_id" ]] || die "relay has no peer id"
  local relay="/ip4/172.30.0.10/tcp/4001/p2p/$relay_id"

  log "installations up in $A and $B (tick ${TICK}s)"
  for service in "$A" "$B"; do
    "${COMPOSE[@]}" cp "$BIN" "$service:/usr/local/bin/kols-drive" >/dev/null
    "${COMPOSE[@]}" exec -d -e KOLS_PASSWORD=x -e RUST_LOG="${RUST_LOG:-}" "$service" \
      sh -c "exec /usr/local/bin/kols-drive --home /w run --tick-secs $TICK > /w.log 2>&1"
  done
  sleep 2

  log "$A creates the shared network"
  local net
  net="$(drive "$A" create shared "$relay")"
  until_says 60 p2p-circuit "$A" addresses "$net" >/dev/null \
    || die "$A never got a circuit address through the relay"
  local invite
  invite="$(drive "$A" invite "$net")"

  log "$B joins; $A admits"
  local joined who
  if ! joined="$(drive "$B" join "$invite" 2>&1)"; then
    # Scenario 4's expected end, not a failure: a pair that cannot hole-punch
    # and has no IPv6 path does not join, because a relay may carry the
    # negotiation and nothing else (Core §5.2).
    if [[ "$SCENARIO" == 4 && "$joined" == *"hole punch"* ]]; then
      printf '\033[32mPASS\033[0m %s\n' "no join, as Core §5.2 requires: $joined"
      exit 0
    fi
    die "$B could not join: $joined"
  fi
  who="$(sed -n 's/^waiting //p' <<<"$joined")"
  [[ -n "$who" ]] && drive "$A" admit "$net" "$who"
  until_says 120 keyed "$B" keyed "$net" >/dev/null \
    || die "$B was never keyed into the shared network"

  log "$A starts a conversation; $B accepts"
  local a_id b_id conversation started
  a_id="$(drive "$A" identity "$net")"
  b_id="$(drive "$B" identity "$net")"
  started=$SECONDS
  conversation="$(drive "$A" start "$net" "$b_id")"
  until_says 180 "$a_id" "$B" requests "$net" >/dev/null \
    || die "$B never saw the request"
  echo "request seen after $((SECONDS - started))s"
  drive "$B" accept "$net" "$a_id" >/dev/null
  for _ in $(seq 1 120); do
    [[ "$(drive "$B" keyed "$conversation" 2>/dev/null)" == keyed ]] && break
    sleep 1
  done
  [[ "$(drive "$B" keyed "$conversation")" == keyed ]] || die "$B never keyed into the conversation"
  echo "conversation keyed after $((SECONDS - started))s"

  log "$MESSAGES messages each way, a few seconds apart"
  for n in $(seq 1 "$MESSAGES"); do
    drive "$A" send "$conversation" "from-a $n" >/dev/null
    sleep "$((2 + RANDOM % 5))"
    drive "$B" send "$conversation" "from-b $n" >/dev/null
    sleep "$((2 + RANDOM % 5))"
  done
  sleep 20

  log "arrivals"
  for service in "$A" "$B"; do
    "${COMPOSE[@]}" exec -T "$service" sh -c 'cat /w.log' > "$OUT/$service.log"
  done
  report "$A" "$B" "$conversation"
  report "$B" "$A" "$conversation"
}

# Matches every record one side sent with the first time the other saw it.
#   report <sender> <receiver> <conversation>
report() {
  local from="$1" to="$2" short="${3:0:8}"
  awk -v from="$from" -v to="$to" '
    FNR == NR && $3 == "sent" { sent[$4] = $1; order[++n] = $4; next }
    FNR != NR && $3 == "event" && $4 == "record" {
      # One line may hold several records, separated by " ; ".
      line = $0; sub(/^[0-9]+ [^ ]+ event /, "", line)
      split(line, parts, / ; /)
      for (i in parts) {
        split(parts[i], w, " ")
        if (!(w[2] in seen)) { seen[w[2]] = $1; how[w[2]] = w[3] }
      }
    }
    END {
      printf "%s -> %s\n", from, to
      lost = 0
      for (i = 1; i <= n; i++) {
        id = order[i]
        if (id in seen) printf "  %s  %6d ms  %s\n", id, seen[id] - sent[id], how[id]
        else { printf "  %s  never arrived\n", id; lost++ }
      }
      printf "  %d sent, %d never arrived\n", n, lost
    }' "$OUT/$from.log" "$OUT/$to.log"
}

main "$@"
