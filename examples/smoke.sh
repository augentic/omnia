#!/bin/bash
# Smoke-test every example against its README contract: start each host
# binary, drive the documented requests/argv, and assert status codes and
# exit codes. Assumes guests and hosts are already built — run via
# `cargo make smoke`, which builds both first.
#
# Expected skips: `identity` fails fast without IDENTITY_* credentials, and
# the http-proxy origin routes need outbound internet.
set -u

ROOT="$(git rev-parse --show-toplevel)" || exit 1
cd "$ROOT" || exit 1
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
export RUST_LOG="${RUST_LOG:-info,opentelemetry_sdk=off}"

LOG="$(mktemp -d -t omnia-smoke)"
RESULTS="$LOG/results.txt"
: > "$RESULTS"

BIN="$CARGO_TARGET_DIR/debug/examples"
WASM="$CARGO_TARGET_DIR/wasm32-wasip2/debug/examples"

if nc -z localhost 8080 2>/dev/null; then
  echo "error: port 8080 is already in use; stop that server first" >&2
  exit 1
fi
for b in http keyvalue blobstore vault sql docstore otel http-proxy messaging \
  config websocket mcp http-routing model cli cli-static \
  guest-link guest-link-dynamic guest-link-register identity; do
  if [ ! -x "$BIN/$b" ]; then
    echo "error: $BIN/$b missing — run \`cargo make smoke\` to build first" >&2
    exit 1
  fi
done

SERVER_PID=""

note() { echo "$1" | tee -a "$RESULTS"; }

# check <example> <label> <expected_status> <curl args...>
check() {
  local ex=$1 label=$2 expect=$3; shift 3
  local body="$LOG/$ex.$label.body"
  local code
  code=$(curl -s --max-time 20 -o "$body" -w '%{http_code}' "$@")
  if [ "$code" = "$expect" ]; then
    note "PASS $ex/$label ($code)"
  else
    note "FAIL $ex/$label (got $code want $expect) body=$(head -c 200 "$body")"
  fi
}

start_server() { # start_server <name> <cmd...>
  local name=$1; shift
  "$@" > "$LOG/$name.log" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 120); do
    nc -z localhost 8080 2>/dev/null && return 0
    kill -0 "$SERVER_PID" 2>/dev/null || { note "FAIL $name/startup (process died)"; return 1; }
    sleep 0.5
  done
  note "FAIL $name/startup (port 8080 never opened)"
  return 1
}

stop_server() {
  if [ -n "$SERVER_PID" ]; then
    kill "$SERVER_PID" 2>/dev/null
    wait "$SERVER_PID" 2>/dev/null
    SERVER_PID=""
  fi
  for _ in $(seq 1 40); do
    nc -z localhost 8080 2>/dev/null || return 0
    sleep 0.5
  done
  note "WARN port 8080 still open after stop"
}

# ---------- http ----------
if start_server http "$BIN/http" run "$WASM/http_wasm.wasm"; then
  check http post 200 -H 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
  check http get 200 http://localhost:8080
fi
stop_server

# ---------- keyvalue ----------
if start_server keyvalue "$BIN/keyvalue" run "$WASM/keyvalue_wasm.wasm"; then
  check keyvalue post 200 -H 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
fi
stop_server

# ---------- blobstore ----------
if start_server blobstore "$BIN/blobstore" run "$WASM/blobstore_wasm.wasm"; then
  check blobstore post 200 -H 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
fi
stop_server

# ---------- vault ----------
if start_server vault "$BIN/vault" run "$WASM/vault_wasm.wasm"; then
  check vault post 200 -H 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
fi
stop_server

# ---------- sql ----------
if start_server sql "$BIN/sql" run "$WASM/sql_wasm.wasm"; then
  check sql create-agency 200 -X POST http://localhost:8080/agencies -H 'Content-Type: application/json' \
    -d '{"agency_id":1,"name":"Ritchies Transport","url":"https://ritchies.co.nz","timezone":"Pacific/Auckland"}'
  check sql list-agencies 200 http://localhost:8080/agencies
  check sql patch-agency 200 -X PATCH http://localhost:8080/agencies/1 -H 'Content-Type: application/json' \
    -d '{"name":"Ritchies Transport Agency","timezone":"Pacific/Auckland"}'
  check sql create-feed 200 -X POST http://localhost:8080/agencies/1/feeds -H 'Content-Type: application/json' \
    -d '{"feed_id":1,"description":"Bus routes and schedules"}'
  check sql list-feeds 200 http://localhost:8080/feeds
  check sql delete-feed 200 -X DELETE http://localhost:8080/feeds/1
fi
stop_server

# ---------- docstore ----------
if start_server docstore "$BIN/docstore" run "$WASM/docstore_wasm.wasm"; then
  check docstore create1 200 -X POST http://localhost:8080/stops -H 'Content-Type: application/json' \
    -d '{"id":"stop-001","stop_name":"Britomart Transport Centre","stop_lat":-36.8442,"stop_lon":174.7676,"zone_id":"zone-1"}'
  check docstore create2 200 -X POST http://localhost:8080/stops -H 'Content-Type: application/json' \
    -d '{"id":"stop-002","stop_name":"Newmarket Station","stop_lat":-36.8690,"stop_lon":174.7779,"zone_id":"zone-1"}'
  check docstore create3 200 -X POST http://localhost:8080/stops -H 'Content-Type: application/json' \
    -d '{"id":"stop-003","stop_name":"Albany Station","stop_lat":-36.7275,"stop_lon":174.6986,"zone_id":"zone-3"}'
  check docstore get 200 http://localhost:8080/stops/stop-001
  check docstore put 200 -X PUT http://localhost:8080/stops/stop-001 -H 'Content-Type: application/json' \
    -d '{"stop_name":"Britomart","stop_lat":-36.8442,"stop_lon":174.7676,"zone_id":"zone-1"}'
  check docstore query-all 200 http://localhost:8080/stops
  check docstore query-text 200 'http://localhost:8080/stops?q=Station'
  check docstore query-zone 200 'http://localhost:8080/stops?zone=zone-1'
  check docstore query-lat 200 'http://localhost:8080/stops?min_lat=-36.90&max_lat=-36.80'
  check docstore query-limit 200 'http://localhost:8080/stops?limit=2'
  check docstore delete 200 -X DELETE http://localhost:8080/stops/stop-003
fi
stop_server

# ---------- otel ----------
if start_server otel "$BIN/otel" run "$WASM/otel_wasm.wasm"; then
  check otel post 200 -H 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
fi
stop_server

# ---------- http-proxy (origin routes need outbound internet) ----------
if start_server http-proxy "$BIN/http-proxy" run "$WASM/http_proxy_wasm.wasm"; then
  check http-proxy cache1 200 http://localhost:8080/cache
  check http-proxy cache2 200 http://localhost:8080/cache
  # The origin routes proxy jsonplaceholder.cypress.io; probe it directly and
  # skip (rather than fail) when there is no outbound internet.
  if curl -s --max-time 10 -o /dev/null https://jsonplaceholder.cypress.io/posts/1; then
    check http-proxy origin-sm 200 http://localhost:8080/origin-sm
  else
    note "SKIP http-proxy/origin-sm (no outbound internet)"
  fi
fi
stop_server

# ---------- messaging ----------
if start_server messaging "$BIN/messaging" run "$WASM/messaging_wasm.wasm"; then
  check messaging pub-sub 200 -H 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080/pub-sub
fi
stop_server

# ---------- config ----------
if start_server config "$BIN/config" run "$WASM/config_wasm.wasm"; then
  check config get 200 http://localhost:8080
fi
stop_server

# ---------- websocket ----------
if start_server websocket "$BIN/websocket" run "$WASM/websocket_wasm.wasm"; then
  check websocket post 200 -H 'Content-Type: application/json' -d '{"text":"hello"}' http://localhost:8080
fi
stop_server

# ---------- mcp ----------
if start_server mcp "$BIN/mcp" run "$WASM/mcp_wasm.wasm"; then
  check mcp tools-list 200 -X POST http://localhost:8080/mcp/docs -H 'content-type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
  check mcp tools-call 200 -X POST http://localhost:8080/mcp/docs -H 'content-type: application/json' \
    -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_doc","arguments":{"name":"overview"}}}'
fi
stop_server

# ---------- http-routing (manifest compiled in) ----------
if start_server http-routing "$BIN/http-routing" run; then
  check http-routing route-a 200 http://localhost:8080/a
  check http-routing route-b 200 http://localhost:8080/b
  check http-routing route-c-404 404 http://localhost:8080/c
  if grep -q 'guest a' "$LOG/http-routing.route-a.body"; then
    note "PASS http-routing/route-a-body"
  else
    note "FAIL http-routing/route-a-body body=$(cat "$LOG/http-routing.route-a.body")"
  fi
fi
stop_server

# ---------- model (direct command) ----------
"$BIN/model" > "$LOG/model.log" 2>&1
rc=$?
[ $rc -eq 0 ] && note "PASS model/run (exit 0)" || note "FAIL model/run (exit $rc)"

# ---------- cli ----------
"$BIN/cli" run "$WASM/cli_wasm.wasm" -- greet Ada > "$LOG/cli-greet.log" 2>&1
rc=$?
if [ $rc -eq 0 ] && grep -qi 'ada' "$LOG/cli-greet.log"; then
  note "PASS cli/greet"
else
  note "FAIL cli/greet (exit $rc) $(head -c 200 "$LOG/cli-greet.log")"
fi
"$BIN/cli" run "$WASM/cli_wasm.wasm" -- fail 42 > "$LOG/cli-fail42.log" 2>&1
rc=$?
[ $rc -eq 42 ] && note "PASS cli/fail-42 (exit 42)" || note "FAIL cli/fail-42 (exit $rc, want 42)"
"$BIN/cli" run "$WASM/cli_wasm.wasm" -- bogus > "$LOG/cli-bogus.log" 2>&1
rc=$?
[ $rc -eq 2 ] && note "PASS cli/bogus (exit 2)" || note "FAIL cli/bogus (exit $rc, want 2)"
"$BIN/cli" run "$WASM/cli_wasm.wasm" -- fail > "$LOG/cli-fail.log" 2>&1
rc=$?
[ $rc -eq 1 ] && note "PASS cli/fail (exit 1)" || note "FAIL cli/fail (exit $rc, want 1)"

# ---------- cli-static (direct command, no `run` grammar) ----------
"$BIN/cli-static" greet Ada > "$LOG/cli-static-greet.log" 2>&1
rc=$?
if [ $rc -eq 0 ] && grep -qi 'ada' "$LOG/cli-static-greet.log"; then
  note "PASS cli-static/greet"
else
  note "FAIL cli-static/greet (exit $rc) $(head -c 200 "$LOG/cli-static-greet.log")"
fi
"$BIN/cli-static" add 2 40 > "$LOG/cli-static-add.log" 2>&1
rc=$?
[ $rc -eq 0 ] && note "PASS cli-static/add (exit 0)" || note "FAIL cli-static/add (exit $rc)"
"$BIN/cli-static" fail 42 > "$LOG/cli-static-fail42.log" 2>&1
rc=$?
[ $rc -eq 42 ] && note "PASS cli-static/fail-42 (exit 42)" || note "FAIL cli-static/fail-42 (exit $rc, want 42)"

# ---------- guest-link trio ----------
# The host either runs the link demo to completion or stays up; both are
# healthy as long as the log is clean.
"$BIN/guest-link" run > "$LOG/guest-link.log" 2>&1 &
GL_PID=$!
sleep 8
if kill -0 $GL_PID 2>/dev/null; then
  if grep -qiE 'error|panic' "$LOG/guest-link.log"; then
    note "FAIL guest-link/run (errors in log)"
  else
    note "PASS guest-link/run (host up, clean log)"
  fi
  kill $GL_PID 2>/dev/null; wait $GL_PID 2>/dev/null
else
  wait $GL_PID; rc=$?
  [ $rc -eq 0 ] && note "PASS guest-link/run (exited 0)" || note "FAIL guest-link/run (exit $rc)"
fi

"$BIN/guest-link-dynamic" > "$LOG/guest-link-dynamic.log" 2>&1
rc=$?
[ $rc -eq 0 ] && note "PASS guest-link-dynamic (exit 0)" || note "FAIL guest-link-dynamic (exit $rc)"

"$BIN/guest-link-register" > "$LOG/guest-link-register.log" 2>&1
rc=$?
[ $rc -eq 0 ] && note "PASS guest-link-register (exit 0)" || note "FAIL guest-link-register (exit $rc)"

# ---------- identity (expected fail-fast without credentials) ----------
# Backend connection is checked ~10s into startup; allow up to 60s.
"$BIN/identity" run "$WASM/identity_wasm.wasm" > "$LOG/identity.log" 2>&1 &
ID_PID=$!
for _ in $(seq 1 60); do
  kill -0 $ID_PID 2>/dev/null || break
  sleep 1
done
if kill -0 $ID_PID 2>/dev/null; then
  note "FAIL identity/run (still running without credentials; expected fail-fast)"
  kill $ID_PID 2>/dev/null; wait $ID_PID 2>/dev/null
else
  wait $ID_PID; rc=$?
  if grep -qi 'IDENTITY_' "$LOG/identity.log"; then
    note "SKIP identity (fail-fast on missing IDENTITY_* vars, exit $rc — expected)"
  else
    note "FAIL identity (exit $rc without the expected missing-vars message)"
  fi
fi

echo
echo "===== SUMMARY ====="
pass=$(grep -c '^PASS' "$RESULTS")
fail=$(grep -c '^FAIL' "$RESULTS")
echo "pass: $pass"
echo "fail: $fail"
grep '^FAIL\|^WARN\|^SKIP' "$RESULTS"
echo "logs: $LOG"
[ "$fail" -eq 0 ]
