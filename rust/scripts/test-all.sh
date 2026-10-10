#!/usr/bin/env bash
#
# Runs the Rust test suite on this Mac and in a Linux Docker container at the
# same time, so most failures show up in minutes instead of after a CI run.
#
# Windows is not covered here. Only GitHub CI (rust-ci.yml, windows-latest)
# builds and tests the Windows target.
#
# Usage: rust/scripts/test-all.sh [-n REPEATS] [cargo test filter...]
#   -n REPEATS  run the suite this many times on each platform (default 1)
#   filter      passed through to `cargo test` as a test-name filter
#
# The Linux run needs a running Docker daemon; without one only macOS runs.
# Full logs are written to a temp folder, whose path is printed at the start.
# Exits non-zero if any repeat failed or did not finish.
set -euo pipefail

usage() {
  echo "Usage: $0 [-n REPEATS] [cargo test filter...]" >&2
  exit 2
}

REPEATS=1
while getopts ":n:h" opt; do
  case "$opt" in
    n) REPEATS="$OPTARG" ;;
    h) usage ;;
    *) echo "unknown option: -$OPTARG" >&2; usage ;;
  esac
done
shift $((OPTIND - 1))

if ! [[ "$REPEATS" =~ ^[1-9][0-9]*$ ]]; then
  echo "-n needs a positive whole number, got: $REPEATS" >&2
  exit 2
fi

RUST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FILTER_TEXT="${*:-all tests}"

if command -v cargo >/dev/null 2>&1; then
  CARGO=cargo
elif [ -x "$HOME/.cargo/bin/cargo" ]; then
  CARGO="$HOME/.cargo/bin/cargo"
else
  echo "cargo not found on PATH or in ~/.cargo/bin" >&2
  exit 2
fi

LOG_DIR="$(mktemp -d "${TMPDIR:-/tmp}/nicegit-test-all.XXXXXX")"
LINUX_CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/nicegit-linux"

# Runs inside the container: installs the system libraries, then runs the
# suite REPEATS times, printing one NICEGIT_RESULT line per repeat.
LINUX_SCRIPT='set -euo pipefail
repeats="$1"
shift
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq && apt-get install -y -qq libxkbcommon-dev libgtk-3-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libwayland-dev libssl-dev pkg-config git-lfs >/dev/null
git config --global init.defaultBranch main
cd /src
for ((i = 1; i <= repeats; i++)); do
  echo "=== repeat $i of $repeats ==="
  if cargo test --workspace -q "$@"; then
    echo "NICEGIT_RESULT repeat=$i status=pass"
  else
    echo "NICEGIT_RESULT repeat=$i status=fail"
  fi
done'

HAVE_LINUX=0
if ! command -v docker >/dev/null 2>&1; then
  echo "note: docker is not installed, so the Linux run is skipped (macOS only)."
elif ! docker info >/dev/null 2>&1; then
  echo "note: docker is not running, so the Linux run is skipped (macOS only). Start Docker to run both."
else
  HAVE_LINUX=1
fi

echo "Rust tests: $REPEATS repeat(s) per platform, filter: $FILTER_TEXT"
echo "Logs: $LOG_DIR"

run_macos() {
  (
    cd "$RUST_DIR"
    local i
    for ((i = 1; i <= REPEATS; i++)); do
      echo "=== repeat $i of $REPEATS ==="
      if "$CARGO" test --workspace -q "$@" 2>&1; then
        echo "NICEGIT_RESULT repeat=$i status=pass"
      else
        echo "NICEGIT_RESULT repeat=$i status=fail"
      fi
    done
  ) | tee "$LOG_DIR/macos.log" | sed -u 's/^/[macos] /'
}

run_linux() {
  mkdir -p "$LINUX_CACHE/target" "$LINUX_CACHE/registry"
  docker run --rm --cpus=2 \
    -v "$RUST_DIR:/src:ro" \
    -v "$LINUX_CACHE/target:/target" \
    -v "$LINUX_CACHE/registry:/usr/local/cargo/registry" \
    -e CARGO_TARGET_DIR=/target \
    rust:latest \
    bash -c "$LINUX_SCRIPT" nicegit-linux "$REPEATS" "$@" 2>&1 \
    | tee "$LOG_DIR/linux.log" | sed -u 's/^/[linux] /'
}

start=$(date +%s)

( run_macos "$@" || true; echo $(( $(date +%s) - start )) > "$LOG_DIR/macos.secs" ) &
mac_pid=$!

linux_pid=""
if [ "$HAVE_LINUX" = 1 ]; then
  ( run_linux "$@" || true; echo $(( $(date +%s) - start )) > "$LOG_DIR/linux.secs" ) &
  linux_pid=$!
fi

wait "$mac_pid" || true
if [ -n "$linux_pid" ]; then
  wait "$linux_pid" || true
fi

FAILED_ANY=0

# Prints pass/fail counts for one platform and, when something failed, the
# panic and FAILED lines that name the failing tests.
summarize() {
  local name="$1" log="$LOG_DIR/$1.log" secs="?" passed failed unfinished details
  if [ -f "$LOG_DIR/$name.secs" ]; then
    secs="$(cat "$LOG_DIR/$name.secs")s"
  fi
  passed=$(grep -c 'NICEGIT_RESULT .*status=pass' "$log" 2>/dev/null || true)
  failed=$(grep -c 'NICEGIT_RESULT .*status=fail' "$log" 2>/dev/null || true)
  passed=${passed:-0}
  failed=${failed:-0}
  unfinished=$((REPEATS - passed - failed))
  echo "[$name] $passed of $REPEATS repeat(s) passed, $failed failed, $unfinished did not finish (wall time $secs)"
  if [ $((failed + unfinished)) -gt 0 ]; then
    FAILED_ANY=1
    details="$(grep -E 'panicked at|FAILED|^error' "$log" | grep -v '^test result:' | sort -u || true)"
    if [ -n "$details" ]; then
      echo "$details" | sed "s/^/[$name]   /"
    else
      echo "[$name]   no test failure lines found; last lines of the log:"
      tail -n 5 "$log" 2>/dev/null | sed "s/^/[$name]   /" || true
    fi
  fi
}

echo
echo "==== summary ===="
summarize macos
if [ "$HAVE_LINUX" = 1 ]; then
  summarize linux
else
  echo "[linux] skipped (docker unavailable)"
fi
echo "total wall time: $(( $(date +%s) - start ))s"

if [ "$FAILED_ANY" = 0 ]; then
  echo "RESULT: all repeats passed"
  exit 0
fi
echo "RESULT: failures found, full logs in $LOG_DIR"
exit 1
