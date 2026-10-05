#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "$0")/.."
tag=${1:?Usage: scripts/release-homebrew.sh TAG ASSET_DIRECTORY|--release OUTPUT.rb}
source=${2:?Missing asset directory or --release}
output=${3:?Missing formula output path}
[ "$#" -eq 3 ] || { echo 'Usage: scripts/release-homebrew.sh TAG ASSET_DIRECTORY|--release OUTPUT.rb' >&2; exit 1; }
[[ "$tag" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-rc\.(0|[1-9][0-9]*))?$ ]] || {
  echo "Invalid release tag: $tag" >&2; exit 1;
}
version=${tag#v}
[ ! -e "$output" ] || { echo "Refusing to replace existing formula: $output" >&2; exit 1; }

stage=$(mktemp -d)
remote=false
trap 'rm -rf "$stage"' EXIT
if [ "$source" = --release ]; then
  remote=true
  # Missing or private GitHub releases fail closed; no formula with invented URLs.
  source="$stage/downloads"
  mkdir "$source"
  for target in aarch64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    archive="taku-$tag-$target.tar.gz"
    url="https://github.com/commandzero/taku/releases/download/$tag/$archive"
    curl --fail --location --silent --show-error "$url" --output "$source/$archive"
    curl --fail --location --silent --show-error "$url.sha256" --output "$source/$archive.sha256"
  done
else
  [ -d "$source" ] || { echo "Asset directory does not exist: $source" >&2; exit 1; }
  source=$(CDPATH='' cd -- "$source" && pwd)
fi

asset_url() {
  if [ "$remote" = true ]; then
    printf 'https://github.com/commandzero/taku/releases/download/%s/%s' "$tag" "$1"
  else
    printf 'file://%s/%s' "$source" "$1"
  fi
}

release_commit=

for target in aarch64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
  archive="taku-$tag-$target.tar.gz"
  [ -f "$source/$archive" ] || continue
  [ -f "$source/$archive.sha256" ] || { echo "Missing checksum: $archive" >&2; exit 1; }
  expected=$(cd "$source" && shasum -a 256 "$archive")
  [ "$(cat "$source/$archive.sha256")" = "$expected" ] || {
    echo "Checksum sidecar mismatch: $archive" >&2; exit 1;
  }
  (cd "$source" && shasum -a 256 -c "$archive.sha256")
  tar -tzf "$source/$archive" > "$stage/members"
  printf '%s\n' taku LICENSE NOTICES.md BUILD-INFO.txt | sort > "$stage/expected"
  sort "$stage/members" > "$stage/actual"
  diff -u "$stage/expected" "$stage/actual"
  commit=$(tar -xOzf "$source/$archive" BUILD-INFO.txt | awk -v tag="$tag" -v target="$target" '
    $0 == "tag=" tag { tag_ok = 1 }
    $0 == "target=" target { target_ok = 1 }
    $0 == "features=default" { features_ok = 1 }
    /^commit=[0-9a-f]+$/ { commit = substr($0, 8) }
    END { if (tag_ok && target_ok && features_ok && commit != "") print commit; else exit 1 }
  ') || { echo "Release provenance mismatch: $archive" >&2; exit 1; }
  if [ -n "$release_commit" ] && [ "$commit" != "$release_commit" ]; then
    echo "Mixed source commits in release archives: $archive" >&2; exit 1
  fi
  release_commit=$commit
  mkdir "$stage/$target"
  tar -xzf "$source/$archive" -C "$stage/$target"
  if [ ! -x "$stage/$target/taku" ] || [ ! -s "$stage/$target/LICENSE" ] || [ ! -s "$stage/$target/NOTICES.md" ]; then
    echo "Missing executable, license, or notices: $archive" >&2; exit 1;
  fi
done

if [ "$remote" = true ]; then
  tagged_commit=$(git rev-parse --verify "$tag^{commit}") || {
    echo "Fetch the reviewed release tag before generating the public formula: $tag" >&2; exit 1;
  }
  [ "$release_commit" = "$tagged_commit" ] || {
    echo "Release archive commit differs from reviewed tag: $tag" >&2; exit 1;
  }
fi

# A single locally built archive can exercise a reviewable macOS-arm formula
# against file:// URLs before publication; it never alters the public tap.
mac_archive="taku-$tag-aarch64-apple-darwin.tar.gz"
[ -f "$source/$mac_archive" ] || { echo "Missing required macOS arm64 archive: $mac_archive" >&2; exit 1; }
mkdir -p "$(dirname -- "$output")"
temporary=$(mktemp "$(dirname -- "$output")/.taku-formula.XXXXXX")
trap 'rm -rf "$stage"; rm -f "$temporary"' EXIT
{
  printf 'class Taku < Formula\n'
  printf '  desc "Git-versioned control for configured remote resources"\n'
  printf '  homepage "https://github.com/commandzero/taku"\n'
  printf '  version "%s"\n' "$version"
  printf '  license "Apache-2.0"\n\n'
  printf '  on_macos do\n'
  printf '    depends_on arch: :arm64\n'
  printf '    on_arm do\n'
  printf '      url "%s"\n' "$(asset_url "$mac_archive")"
  printf '      sha256 "%s"\n' "$(shasum -a 256 "$source/$mac_archive" | cut -d ' ' -f 1)"
  printf '    end\n  end\n'
  linux_count=0
  for target in aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
    archive="taku-$tag-$target.tar.gz"
    [ -f "$source/$archive" ] || continue
    if [ "$linux_count" -eq 0 ]; then printf '\n  on_linux do\n'; fi
    case "$target" in
      aarch64-unknown-linux-gnu) printf '    on_arm do\n' ;;
      x86_64-unknown-linux-gnu) printf '    on_intel do\n' ;;
    esac
    printf '      url "%s"\n' "$(asset_url "$archive")"
    printf '      sha256 "%s"\n' "$(shasum -a 256 "$source/$archive" | cut -d ' ' -f 1)"
    printf '    end\n'
    linux_count=$((linux_count + 1))
  done
  if [ "$linux_count" -gt 0 ]; then printf '  end\n'; fi
  printf '\n  def install\n    bin.install "taku"\n    pkgshare.install "LICENSE", "NOTICES.md", "BUILD-INFO.txt"\n  end\n\n'
  printf '  test do\n'
  printf '    assert_match version.to_s, shell_output("#{bin}/taku --version")\n'
  printf '    system "git", "init", "-q", testpath\n'
  printf '    system bin/"taku", "--project", testpath, "--non-interactive",\n'
  printf '           "init", "--layout", "single", "--environment", "dev"\n'
  printf '    system bin/"taku", "--project", testpath, "--non-interactive", "install", "elasticsearch"\n'
  printf '    assert_match "valid: true", shell_output("#{bin}/taku --project #{testpath} --non-interactive validate")\n'
  printf '  end\nend\n'
} > "$temporary"
chmod 644 "$temporary"
mv "$temporary" "$output"
printf 'Review generated formula before proposing a tap change: %s\n' "$output"
