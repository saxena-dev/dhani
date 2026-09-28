#!/usr/bin/env bash
# Local quality gate: the same checks as the required CI job (.github/workflows/ci.yml).
#
#   scripts/verify.sh           feature rows on the stable toolchain, plus lint, docs, the
#                               compiled-only live lane, packaging and dependency-graph checks
#   scripts/verify.sh --msrv    additionally checks every feature row on the MSRV toolchain
#                               (rustup toolchain 1.88) and runs the all-features tests on it;
#                               CI runs every row's tests on both toolchains
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

# The package ships no vendored, test or local-only file, and the packaged crate builds out of
# the tree in every feature row. --allow-dirty lets this run before a commit; CI packages a
# clean checkout. The crate is written to a fresh directory, so a stale one is never unpacked.
pkg_dir=""
unpacked=""
trap 'rm -rf "$pkg_dir" "$unpacked"' EXIT
package_check() {
  local list; list=$(cargo package --locked --allow-dirty --list)
  if grep -E '^(vendor/|tests/|\.ignore/)|DhanHQ-py/' <<<"$list"; then
    echo "the package must not contain vendored, test or local-only files" >&2
    exit 1
  fi
  echo "+ package excludes vendor/, tests/ and .ignore/"
  pkg_dir=$(mktemp -d)
  unpacked=$(mktemp -d)
  run cargo package --locked --allow-dirty --no-verify --target-dir "$pkg_dir"
  local version; version=$(cargo pkgid | sed 's/.*[#@]//')
  tar -xzf "$pkg_dir/package/dhani-$version.crate" -C "$unpacked"
  local target_dir="$PWD/target/package-build"
  for row in "${ROWS[@]}"; do
    # shellcheck disable=SC2046
    (cd "$unpacked/dhani-$version" && run cargo build --locked --target-dir "$target_dir" $(row_args "$row"))
  done
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
  # Locally, compiling each narrower row (all targets, so tests and benches too) proves it
  # builds on the MSRV; running the tests once with every feature covers MSRV behaviour. This
  # keeps the MSRV pass to a fraction of a full second matrix; CI still runs all of it.
  for row in "${ROWS[@]}"; do
    [ "$row" = all ] && continue
    # shellcheck disable=SC2046
    run cargo "+$MSRV" check --locked --all-targets $(row_args "$row")
  done
  run cargo "+$MSRV" test --locked --all-features
fi
RUSTDOCFLAGS=-Dwarnings run cargo doc --locked --all-features --no-deps
run cargo test --locked --doc --all-features
# The live lane needs Dhan credentials: compile it, never run it.
run cargo test --locked --features live-tests --test live --no-run
package_check
forbid_deps decoder tokio reqwest tungstenite tokio-tungstenite
forbid_deps rest tungstenite tokio-tungstenite
forbid_deps feed reqwest
echo "verify: all checks passed"
