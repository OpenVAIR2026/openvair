#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
pg_bin="$(pg_config --bindir)"
test_dir="$(mktemp -d "${TMPDIR:-/tmp}/vair-quality-tests.XXXXXX")"
cleanup() {
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
"$pg_bin/initdb" -D "$test_dir/data" -U quality_test -A trust --no-locale >/dev/null
"$pg_bin/pg_ctl" -D "$test_dir/data" -l "$test_dir/postgres.log" \
  -o "-h 127.0.0.1 -p $test_port -k $test_dir" -w start >/dev/null

DATABASE_URL="postgres://quality_test@127.0.0.1:$test_port/postgres" \
MASTER_KEY="quality-regression-test-only-master-key" \
  cargo test --locked monitor::db_tests -- --ignored --test-threads=1
