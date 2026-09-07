#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

case "$(okf --version)" in
  'okf 0.2.7 '*) ;;
  *) echo 'Install okf 0.2.7: cargo install okf --version 0.2.7 --locked' >&2; exit 1 ;;
esac
okf validate docs
okf lint docs
okf links --check docs

# Generate indexes in a copy; validation never rewrites authored files.
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
cp -R docs "$staging/docs"
okf index "$staging/docs"
diff -ru docs "$staging/docs" || {
  echo 'Documentation indexes are stale. Run: okf index docs' >&2
  exit 1
}
