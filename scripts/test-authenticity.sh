#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
pg_bin="$(pg_config --bindir)"
test_dir="$(mktemp -d "${TMPDIR:-/tmp}/vair-authenticity-tests.XXXXXX")"
cleanup() {
  [[ -n "${redis_pid:-}" ]] && kill "$redis_pid" >/dev/null 2>&1 || true
  "$pg_bin/pg_ctl" -D "$test_dir/data" -m immediate stop >/dev/null 2>&1 || true
  rm -rf "$test_dir"
}
trap cleanup EXIT

test_port="$(python3 - <<'PY'
import socket
with socket.socket() as s:
    s.bind(('127.0.0.1', 0))
    print(s.getsockname()[1])
PY
)"
"$pg_bin/initdb" -D "$test_dir/data" -U authenticity_test -A trust --no-locale >/dev/null
"$pg_bin/pg_ctl" -D "$test_dir/data" -l "$test_dir/postgres.log" \
  -o "-h 127.0.0.1 -p $test_port -k $test_dir" -w start >/dev/null

# Public checks keep the key in Redis while they run.
redis_port="$(python3 - <<'PY'
import socket
with socket.socket() as s:
    s.bind(('127.0.0.1', 0))
    print(s.getsockname()[1])
PY
)"
redis-server --port "$redis_port" --bind 127.0.0.1 --save '' --appendonly no >"$test_dir/redis.log" 2>&1 &
redis_pid=$!

DATABASE_URL="postgres://authenticity_test@127.0.0.1:$test_port/postgres" \
REDIS_URL="redis://127.0.0.1:$redis_port/" \
MASTER_KEY="authenticity-regression-test-only-master-key" \
  cargo test --locked authenticity::db_tests -- --ignored --test-threads=1
