#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
pattern='^(feat|fix|docs|refactor|perf|test|build|ci|chore|revert)(\([a-zA-Z0-9._/-]+\))?!?: .+'
if [[ ! ${PR_TITLE:-} =~ $pattern ]]; then
  echo 'PR title must be a Conventional Commit, for example: fix(cli): preserve resource IDs' >&2
  exit 1
fi
: "${PR_BODY:?Set PR_BODY with an OpenSpec association line}"
: "${BASE_REF:?Set BASE_REF to the PR target commit or branch}"
bash scripts/check-openspec.sh
