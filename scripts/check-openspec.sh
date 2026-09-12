#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd "$(dirname "$0")" && pwd)
repo_root=${REPO_ROOT:-$(cd "$script_dir/.." && pwd)}
cd "$repo_root"
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
gate_source=${OPENSPEC_GATE_SOURCE:-$repo_root/scripts/check-openspec.rs}
rustup run "$toolchain" rustc --edition 2024 -D warnings "$gate_source" -o "$staging/check-openspec"
"$staging/check-openspec"
