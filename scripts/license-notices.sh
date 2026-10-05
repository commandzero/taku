#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "$0")" && pwd)
cd "$script_dir/.."
output=${1:-NOTICES.md}
[ "$#" -le 1 ] || { echo 'Usage: scripts/license-notices.sh [--check|OUTPUT]' >&2; exit 2; }
[ "$(cargo-about --version)" = 'cargo-about 0.8.4' ] || {
  echo 'Install cargo-about 0.8.4: rustup run 1.97.1 cargo install cargo-about --version 0.8.4 --locked' >&2
  exit 1
}
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
raw="$stage/raw"
temporary="$stage/NOTICES.md"
rustup run 1.97.1 cargo-about generate --locked --fail \
  "$script_dir/license-notices.hbs" --output-file "$raw"
# cargo-about --fail still permits synthesized SPDX text. Do not distribute it
# instead of a dependency's actual source license and attribution.
if grep -Fq 'ERROR: unsourced license ' "$raw"; then
  echo 'Missing source license text; review cargo-about workarounds or hash-verified clarifications in about.toml.' >&2
  exit 1
fi
# Number the authored crate references, never the imported fenced license text.
awk '
  /^```/ { fenced = !fenced; references = 0 }
  !fenced && /^Used by:$/ { references = 1; number = 0 }
  !fenced && references && /^- / { $0 = ++number ". " substr($0, 3) }
  { print }
' "$raw" > "$temporary"
if [ "$output" = --check ]; then
  cmp NOTICES.md "$temporary" || {
    echo 'Dependency notices are stale. Run: bash scripts/license-notices.sh' >&2
    exit 1
  }
else
  chmod 644 "$temporary"
  mv "$temporary" "$output"
fi
