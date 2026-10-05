#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd "$(dirname "$0")" && pwd)
repo_root=${REPO_ROOT:-$(cd "$script_dir/.." && pwd)}
cd "$repo_root"

# Modes keep docs-only PRs cheap while sharing exactly the same local commands.
mode=${1:-all}
toolchain=$(awk -F '"' '/^channel = / { print $2 }' rust-toolchain.toml)
case "$mode" in
  all|code|docs|msrv) ;;
  *) echo 'Usage: bash scripts/preflight.sh [all|code|docs|msrv]' >&2; exit 2 ;;
esac

if [[ "$mode" == all || "$mode" == code ]]; then
  rustup run "$toolchain" cargo fmt --all -- --check
  shellcheck scripts/*.sh
  actionlint
  for script in scripts/*.sh; do bash -n "$script"; done
  gate_source=${OPENSPEC_GATE_SOURCE:-$repo_root/scripts/check-openspec.rs}
  rustup run "$toolchain" rustfmt --edition 2024 --check "$gate_source"
  bash "$script_dir/license-notices.sh" --check
  bash "$script_dir/check-workflow-review-test.sh"
  rustup run "$toolchain" cargo clippy --workspace --all-targets --locked -- -D warnings
  rustup run "$toolchain" cargo test --workspace --locked
  if [[ ${SKIP_GATE_TESTS:-0} != 1 ]]; then
    staging=$(mktemp -d)
    trap 'rm -rf "$staging"' EXIT
    gate_source=${OPENSPEC_GATE_SOURCE:-$repo_root/scripts/check-openspec.rs}
    rustup run "$toolchain" rustc --edition 2024 -D warnings --test "$gate_source" -o "$staging/tests"
    "$staging/tests"
  fi
fi
if [[ "$mode" == all || "$mode" == docs ]]; then
  bash "$script_dir/check-docs.sh"
  bash "$script_dir/check-openspec.sh"
fi
if [[ "$mode" == msrv ]]; then
  minimum=$(awk -F '"' '/^rust-version = / { print $2 }' Cargo.toml)
  rustup run "$minimum.0" cargo test --workspace --locked
fi
