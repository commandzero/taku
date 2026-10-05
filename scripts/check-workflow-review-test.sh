#!/usr/bin/env bash
set -euo pipefail
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir "$stage/repository" "$stage/bin"
cd "$stage/repository"
git init -q
git config user.name 'Workflow review fixture'
git config user.email 'fixture@example.invalid'
printf 'base\n' > README.md
git add .
git commit -qm base
export BASE_REF
git_base=$(git rev-parse HEAD)
BASE_REF=$git_base
mkdir -p scripts
printf 'untrusted replacement must never run\n' > scripts/check-workflow-review.sh
git add .
git commit -qm 'change protected gate'
export PR_REPOSITORY=example/repository PR_NUMBER=1 PR_AUTHOR=author PR_HEAD_SHA
PR_HEAD_SHA=$(git rev-parse HEAD)
export REVIEW_FIXTURE="$stage/reviews.json" PERMISSION_FIXTURE="$stage/permission.json"
cat > "$stage/bin/gh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
case "${*: -1}" in
  */reviews) cat "$REVIEW_FIXTURE" ;;
  */permission) cat "$PERMISSION_FIXTURE" ;;
  *) echo 'Unexpected API endpoint' >&2; exit 1 ;;
esac
SH
chmod +x "$stage/bin/gh"
export PATH="$stage/bin:$PATH"
review() {
  tq -n -o json -c --arg head "$1" --arg state "$2" --arg login "$3" --arg type "$4" --argjson id "$5" \
    "{id: \$id, commit_id: \$head, state: \$state, submitted_at: \"2026-10-05T00:00:00Z\", user: {login: \$login, type: \$type}}"
}
check() {
  local name=$1 expected=$2 reviews=$3 permission=$4 status=0
  printf '%s\n' "$reviews" > "$REVIEW_FIXTURE"
  printf '{"permission":"%s"}\n' "$permission" > "$PERMISSION_FIXTURE"
  bash "$script_dir/check-workflow-review.sh" > "$stage/output" 2>&1 || status=$?
  if [ "$status" -ne 0 ]; then status=1; fi
  if [ "$status" != "$expected" ]; then
    cat "$stage/output" >&2
    printf 'Review case %s: expected %s, got %s\n' "$name" "$expected" "$status" >&2
    exit 1
  fi
  printf 'PASS %s\n' "$name"
}
approved=$(review "$PR_HEAD_SHA" APPROVED maintainer User 1)
check current-head 0 "[[$approved]]" write
check old-head 1 "[[$(review "$git_base" APPROVED maintainer User 1)]]" write
check no-approval 1 '[[]]' write
check read-only-reviewer 1 "[[$approved]]" read
check self-approval 1 "[[$(review "$PR_HEAD_SHA" APPROVED author User 1)]]" admin
check bot-approval 1 "[[$(review "$PR_HEAD_SHA" APPROVED automation Bot 1)]]" write
check dismissed 1 "[[$approved,$(review "$PR_HEAD_SHA" DISMISSED maintainer User 2)]]" write
check changes-requested 1 "[[$approved,$(review "$PR_HEAD_SHA" CHANGES_REQUESTED maintainer User 2)]]" write
check later-comment 0 "[[$approved,$(review "$PR_HEAD_SHA" COMMENTED maintainer User 2)]]" write
check paginated-reapproval 0 "[[$(review "$PR_HEAD_SHA" CHANGES_REQUESTED maintainer User 1)],[$(review "$PR_HEAD_SHA" APPROVED maintainer User 2)]]" maintain
check malformed-review-data 1 'not JSON' write
# A new head cannot inherit an approval of the preceding commit.
printf 'next\n' >> scripts/check-workflow-review.sh
git add .
git commit -qm next
PR_HEAD_SHA=$(git rev-parse HEAD)
check new-head 1 "[[$approved]]" write
# Ordinary changes need no integrity approval and must not call the API.
BASE_REF=$PR_HEAD_SHA
printf 'ordinary\n' >> README.md
git add .
git commit -qm ordinary
PR_HEAD_SHA=$(git rev-parse HEAD)
check ordinary-change 0 'not JSON' read
for input in scripts/license-notices.sh scripts/license-notices.hbs about.toml; do
  BASE_REF=$PR_HEAD_SHA
  printf 'protected notice input\n' > "$input"
  git add .
  git commit -qm 'change notice input'
  PR_HEAD_SHA=$(git rev-parse HEAD)
  check "$input" 1 '[[]]' write
done
