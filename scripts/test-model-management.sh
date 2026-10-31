#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
pg_bin="$(pg_config --bindir)"
test_dir="$(mktemp -d "${TMPDIR:-/tmp}/vair-model-tests.XXXXXX")"
free_port() {
  python3 - <<'PY'
import socket
with socket.socket() as s:
    s.bind(('127.0.0.1', 0))
    print(s.getsockname()[1])
PY
}
test_port="$(free_port)"
redis_port="$(free_port)"
cleanup() {
  "$pg_bin/pg_ctl" -D "$test_dir/data" -m immediate stop >/dev/null 2>&1 || true
  redis-cli -p "$redis_port" shutdown nosave >/dev/null 2>&1 || true
  rm -rf "$test_dir"
}
trap cleanup EXIT

"$pg_bin/initdb" -D "$test_dir/data" -U pricing_test -A trust --no-locale >/dev/null
"$pg_bin/pg_ctl" -D "$test_dir/data" -l "$test_dir/postgres.log" \
  -o "-h 127.0.0.1 -p $test_port -k $test_dir" -w start >/dev/null
# Account security tests also need Redis for attempt limits.
redis-server --port "$redis_port" --bind 127.0.0.1 --save '' --appendonly no \
  --dir "$test_dir" --daemonize yes >/dev/null

DATABASE_URL="postgres://pricing_test@127.0.0.1:$test_port/postgres" \
MASTER_KEY="pricing-regression-test-only-master-key" \
TEST_REDIS_URL="redis://127.0.0.1:$redis_port/" \
  cargo test --locked -- --ignored --test-threads=1
