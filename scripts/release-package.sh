#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "$0")/.."
tag=${1:?Usage: scripts/release-package.sh TAG TARGET DESTINATION}
target=${2:?Missing target triple}
destination=${3:?Missing destination directory}
[ "$#" -eq 3 ] || { echo 'Usage: scripts/release-package.sh TAG TARGET DESTINATION' >&2; exit 1; }
case "$target" in
  aarch64-apple-darwin|x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu) ;;
  *) echo "Unselected native release target: $target" >&2; exit 1 ;;
esac

package_id=$(rustup run 1.97.1 cargo pkgid --locked -p taku)
version=${package_id##*#}
version=${version##*@}
[ -n "$version" ] && [[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-rc\.(0|[1-9][0-9]*))?$ ]] || {
  echo "Invalid workspace version: $version" >&2; exit 1;
}
[ "$tag" = "v$version" ] || { echo "Tag must be v$version" >&2; exit 1; }
[ "$(git rev-parse --verify "$tag^{commit}")" = "$(git rev-parse HEAD)" ] || {
  echo 'Tag must point to checked-out commit' >&2; exit 1;
}
[ -z "$(git status --porcelain --untracked-files=normal)" ] || {
  echo 'Packaging requires a clean, tagged source checkout' >&2; exit 1;
}
awk -v prefix="## [$version] - " '
  index($0, prefix) == 1 && $0 ~ /[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]$/ { found = 1 }
  END { exit !found }
' CHANGELOG.md || { echo "Missing dated CHANGELOG.md heading for $version" >&2; exit 1; }
compiler=$(rustup run 1.97.1 rustc --version)
host=$(rustup run 1.97.1 rustc -vV | awk '/^host:/ { print $2 }')
[ "$host" = "$target" ] || { echo 'Packaging and smoke require a native release host' >&2; exit 1; }

mkdir -p "$destination"
destination=$(CDPATH='' cd -- "$destination" && pwd)
archive="taku-$tag-$target.tar.gz"
[ ! -e "$destination/$archive" ] && [ ! -e "$destination/$archive.sha256" ] || {
  echo 'Refusing to replace existing release artifacts' >&2; exit 1;
}
bash scripts/license-notices.sh --check
rustup run 1.97.1 cargo build --release --locked --target "$target" -p taku
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir "$stage/archive" "$stage/extracted"
cp "target/$target/release/taku" "$stage/archive/taku"
chmod 755 "$stage/archive/taku"
cp LICENCE.md "$stage/archive/LICENSE"
cp NOTICES.md "$stage/archive/NOTICES.md"
{
  printf 'tag=%s\ncommit=%s\ncompiler=%s\ntarget=%s\nfeatures=default\n' \
    "$tag" "$(git rev-parse HEAD)" "$compiler" "$target"
  printf 'build_host=%s\n' "$(uname -sr)"
  printf '%s\n' 'os_abi_floor=not certified; record tested hosts and ABI in release notes'
} > "$stage/archive/BUILD-INFO.txt"
COPYFILE_DISABLE=1 tar -czf "$stage/$archive" -C "$stage/archive" taku LICENSE NOTICES.md BUILD-INFO.txt
tar -xzf "$stage/$archive" -C "$stage/extracted"
[ "$("$stage/extracted/taku" --version)" = "taku $version" ] || {
  echo 'Extracted binary version mismatch' >&2; exit 1;
}
cmp LICENCE.md "$stage/extracted/LICENSE"
cmp NOTICES.md "$stage/extracted/NOTICES.md"
# No HTTP target or user project is touched by the extracted offline smoke.
project="$stage/project"
mkdir "$project"
git -C "$project" init -q
"$stage/extracted/taku" --project "$project" --non-interactive init --layout single --environment dev > "$stage/init.json"
"$stage/extracted/taku" --project "$project" --non-interactive install elasticsearch > "$stage/install.json"
"$stage/extracted/taku" --project "$project" --non-interactive validate > "$stage/validate.json"
( cd "$stage" && shasum -a 256 "$archive" > "$archive.sha256" && shasum -a 256 -c "$archive.sha256" )
mv "$stage/$archive" "$stage/$archive.sha256" "$destination/"
printf 'Verified native release archive: %s\n' "$destination/$archive"
