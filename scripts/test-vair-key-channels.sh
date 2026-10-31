#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
pg_bin="$(pg_config --bindir)"
test_dir="$(mktemp -d "${TMPDIR:-/tmp}/vair-key-channel-tests.XXXXXX")"
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
"$pg_bin/initdb" -D "$test_dir/data" -U key_channel_test -A trust --no-locale >/dev/null
"$pg_bin/pg_ctl" -D "$test_dir/data" -l "$test_dir/postgres.log" \
  -o "-h 127.0.0.1 -p $test_port -k $test_dir" -w start >/dev/null

export DATABASE_URL="postgres://key_channel_test@127.0.0.1:$test_port/postgres"
export MASTER_KEY="key-channel-regression-test-only-master-key"
cargo test --locked vair_key_channels::db_tests -- --ignored --test-threads=1
cargo test --locked key_share_links::db_tests -- --ignored --test-threads=1
