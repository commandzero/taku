#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export OPENSPEC_TELEMETRY=0
[[ "$(openspec --version)" == 1.11.0 ]] || {
  echo 'Install @fission-ai/openspec 1.11.0 before running this check.' >&2
  exit 1
}
# Validate main specs only. Unrelated active changes do not gate this PR.
openspec validate --specs --strict
toolchain=$(awk -F '"' '/^channel = / { print $2 }' rust-toolchain.toml)
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
rustup run "$toolchain" rustc --edition 2024 -D warnings scripts/check-openspec.rs -o "$staging/check-openspec"
"$staging/check-openspec"
