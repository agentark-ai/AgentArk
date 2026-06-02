#!/usr/bin/env bash
set -u

MODE="${1:-check}"
case "$MODE" in
  check|clippy|build|test) ;;
  *)
    echo "Usage: ./build.sh [check|clippy|build|test]" >&2
    exit 2
    ;;
esac

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$REPO_ROOT"

if pgrep -x cargo >/dev/null 2>&1 ||
  pgrep -x rustc >/dev/null 2>&1 ||
  pgrep -x rustdoc >/dev/null 2>&1 ||
  pgrep -x clippy-driver >/dev/null 2>&1; then
  echo "Rust build process already running; aborting to preserve the single-instance build rule." >&2
  exit 75
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found on PATH. Install Rust before running this script." >&2
  exit 127
fi

LOG_DIR="$REPO_ROOT/.build-logs"
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
LOG_PATH="$LOG_DIR/agentark-$MODE.log"
mkdir -p "$LOG_DIR" "$TARGET_DIR"

export CARGO_BUILD_JOBS=1
export CARGO_TARGET_DIR="$TARGET_DIR"

case "$MODE" in
  check) CARGO_ARGS=(check --locked) ;;
  clippy) CARGO_ARGS=(clippy --locked --all-targets --all-features) ;;
  build) CARGO_ARGS=(build --locked) ;;
  test) CARGO_ARGS=(test --locked --all-targets) ;;
esac

{
  printf '[%s] cargo %s\n' "$(date -u +"%Y-%m-%dT%H:%M:%SZ")" "${CARGO_ARGS[*]}"
  printf 'CARGO_BUILD_JOBS=%s\n' "$CARGO_BUILD_JOBS"
  printf 'CARGO_TARGET_DIR=%s\n' "$CARGO_TARGET_DIR"
} >"$LOG_PATH"

set +e
cargo "${CARGO_ARGS[@]}" 2>&1 | tee -a "$LOG_PATH"
STATUS=${PIPESTATUS[0]}
set -e

if [ "$STATUS" -ne 0 ]; then
  echo "build.sh $MODE failed with exit code $STATUS. See $LOG_PATH" >&2
  exit "$STATUS"
fi

echo "build.sh $MODE completed successfully. Log: $LOG_PATH"
