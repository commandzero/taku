#!/usr/bin/env bash
set -euo pipefail

: "${BASE_REF:?Missing trusted base revision}"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
git diff --no-renames --name-only -z "$BASE_REF...HEAD" > "$stage/paths"
protected=false
while IFS= read -r -d '' path; do
  case "$path" in
    .github/workflows/*|rust-toolchain.toml|scripts/preflight.sh|scripts/check-pr.sh|scripts/check-docs.sh|scripts/docs-index.sh|scripts/check-openspec.sh|scripts/check-openspec.rs|scripts/check-workflow-review*.sh|scripts/license-notices.sh|scripts/license-notices.hbs|about.toml)
      protected=true
      ;;
  esac
done < "$stage/paths"
if [ "$protected" = false ]; then
  echo 'No trusted workflow or validation-gate changes require approval.'
  exit 0
fi

: "${PR_REPOSITORY:?Missing PR repository}"
: "${PR_NUMBER:?Missing PR number}"
: "${PR_HEAD_SHA:?Missing PR head revision}"
: "${PR_AUTHOR:?Missing PR author}"
[ "$(git rev-parse HEAD)" = "$PR_HEAD_SHA" ] || {
  echo 'Checked-out revision differs from the review target.' >&2; exit 1;
}
gh api --paginate --slurp "repos/$PR_REPOSITORY/pulls/$PR_NUMBER/reviews" > "$stage/reviews.json"
# Comments do not revoke approval. A later request for changes or dismissal does.
# Only a human approval of this exact commit can satisfy the integrity gate.
tq -r --arg head "$PR_HEAD_SHA" --arg author "$PR_AUTHOR" "
  [.[][] | select(.state == \"APPROVED\" or .state == \"CHANGES_REQUESTED\" or .state == \"DISMISSED\")]
  | group_by(.user.login)[]
  | sort_by(.submitted_at, .id) | .[-1]
  | select(.state == \"APPROVED\" and .commit_id == \$head and .user.type == \"User\" and .user.login != \$author)
  | .user.login
" "$stage/reviews.json" > "$stage/reviewers"
while IFS= read -r reviewer; do
  gh api "repos/$PR_REPOSITORY/collaborators/$reviewer/permission" > "$stage/permission.json"
  permission=$(tq -r '.permission' "$stage/permission.json")
  case "$permission" in
    admin|maintain|write)
      printf 'Trusted gate changes approved for %s by maintainer %s.\n' "$PR_HEAD_SHA" "$reviewer"
      exit 0
      ;;
  esac
done < "$stage/reviewers"
echo 'Workflow and validation-gate changes require a maintainer APPROVED review of this exact head commit. After approval, rerun the failed PR contract job; a new commit requires new approval.' >&2
exit 1
