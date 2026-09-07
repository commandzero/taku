#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

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
  rustup run "$toolchain" rustfmt --edition 2024 --check scripts/check-openspec.rs
  rustup run "$toolchain" cargo clippy --workspace --all-targets --locked -- -D warnings
  rustup run "$toolchain" cargo test --workspace --locked
  staging=$(mktemp -d)
  trap 'rm -rf "$staging"' EXIT
  rustup run "$toolchain" rustc --edition 2024 -D warnings --test scripts/check-openspec.rs -o "$staging/tests"
  "$staging/tests"
fi
if [[ "$mode" == all || "$mode" == docs ]]; then
  bash scripts/check-docs.sh
  bash scripts/check-openspec.sh
fi
if [[ "$mode" == msrv ]]; then
  minimum=$(awk -F '"' '/^rust-version = / { print $2 }' Cargo.toml)
  rustup run "$minimum.0" cargo test --workspace --locked
fi
