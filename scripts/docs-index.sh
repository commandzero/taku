#!/usr/bin/env bash
set -euo pipefail

# OKF 0.2.7 generates unordered entries and has no list-style option. Normalize
# only its generated indexes to the team's adopted numbered-list convention.
bundle=${1:-docs}
okf index "$bundle"
while IFS= read -r -d '' index; do
  awk '
    /^#/ { number = 0 }
    /^\* / { sub(/^\* /, ++number ". ") }
    { print }
  ' "$index" > "$index.numbered"
  mv "$index.numbered" "$index"
done < <(find "$bundle" -type f -name index.md -print0)
