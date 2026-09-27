#!/usr/bin/env bash
# Local quality gate: the same checks as the required CI job (.github/workflows/ci.yml).
#
#   scripts/verify.sh           feature rows on the stable toolchain, plus lint, docs and
#                               dependency-graph checks
#   scripts/verify.sh --msrv    additionally runs the feature rows on the MSRV toolchain
#                               (rustup toolchain 1.88); CI always runs both
#
# Exits non-zero on the first failure. Never fetches or hashes DhanHQ documentation and never
# runs the live test lane.
set -euo pipefail
cd "$(dirname "$0")/.."

MSRV=1.88
msrv=0
case "${1:-}" in
  "") ;;
  --msrv) msrv=1 ;;
  *) echo "usage: scripts/verify.sh [--msrv]" >&2; exit 2 ;;
esac

# The six feature rows; "all" stands for --all-features.
ROWS=("" "decoder" "rest" "feed" "rest,instruments" "all")

run() {
  echo "+ $*"
  "$@"
}

row_args() {
  case "$1" in
    all) echo "--all-features" ;;
    "") echo "--no-default-features" ;;
    *) echo "--no-default-features --features $1" ;;
  esac
}

test_rows() { # [+toolchain]
  for row in "${ROWS[@]}"; do
    # shellcheck disable=SC2046 # row_args output is deliberately word-split
    run cargo "$@" test --locked $(row_args "$row")
  done
}

# Crate names in the normal-dependency graph of one feature selection.
dep_names() {
  cargo tree --locked -e normal --prefix none --format '{p}' "$@" | awk '{print $1}' | sort -u
}

forbid_deps() { # <row> <crate>...
  local row=$1; shift
  # shellcheck disable=SC2046
  local names; names=$(dep_names $(row_args "$row"))
  for crate in "$@"; do
    if grep -qx "$crate" <<<"$names"; then
      echo "dependency graph for row '$row' must not contain $crate" >&2
      exit 1
    fi
  done
  echo "+ row '$row' excludes: $*"
}

# CI runs `git submodule update --init vendor/DhanHQ-py`; locally the checkout is only checked, because
# updating would silently move a submodule that was deliberately moved off the pin.
if [ ! -e vendor/DhanHQ-py/.git ]; then
  echo "vendor/DhanHQ-py is not checked out; run: git submodule update --init vendor/DhanHQ-py" >&2
  exit 1
fi
run cargo fmt --all -- --check
run cargo clippy --locked --all-targets --all-features -- -D warnings
test_rows
if [ "$msrv" = 1 ]; then
  test_rows "+$MSRV"
fi
RUSTDOCFLAGS=-Dwarnings run cargo doc --locked --all-features --no-deps
run cargo test --locked --doc --all-features
forbid_deps decoder tokio reqwest tungstenite tokio-tungstenite
forbid_deps rest tungstenite tokio-tungstenite
forbid_deps feed reqwest
echo "verify: all checks passed"
