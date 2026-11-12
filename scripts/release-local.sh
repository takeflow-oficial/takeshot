#!/usr/bin/env bash
# The installers of a release, built on this machine instead of on the release workflow's runners:
# Windows here, Linux in the CI image (scripts/linux-packages.sh). They go to the release of the tag,
# a draft this script creates when there is none, and it starts the release-macos workflow, which
# builds the dmg (macOS cannot be built here) and attaches it to the same draft. Nothing is published
# and no tag is pushed: a maintainer publishes the draft (`gh release edit vX.Y.Z --draft=false`),
# and that creates the tag on the commit the draft points to.
#
#   scripts/release-local.sh vX.Y.Z             check, draft release, macOS dmg, build, upload
#   scripts/release-local.sh vX.Y.Z --dry-run   build and list what would go up; no network at all
#   scripts/release-local.sh vX.Y.Z --no-macos  do not start the release-macos workflow
#   --skip-check (only with --dry-run)          skip scripts/check.sh, already green on this commit
#
# It releases HEAD, which must be on origin/main with the version of Cargo.toml equal to the tag and
# a clean tree. The files go to dist/<version>/ (git-ignored), named as build.yml names them; logs of
# each step to target/release-local/logs. Unsigned, like the CI's (it refuses if SIGN_WINDOWS is on).
set -eu
cd "$(git rev-parse --show-toplevel)"

die() { echo "release-local: $*" >&2; exit 1; }
warn() { echo "release-local: warning: $*" >&2; }
step() { echo "== $*"; }

tag=""
dry=0
skip_check=0
macos=1
for arg in "$@"; do
  case "$arg" in
    --dry-run) dry=1 ;;
    --skip-check) skip_check=1 ;;
    --no-macos) macos=0 ;;
    -*) die "unknown argument: $arg" ;;
    *) [ -z "$tag" ] || die "one tag only: $tag and $arg"; tag="$arg" ;;
  esac
done
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "usage: scripts/release-local.sh vX.Y.Z [--dry-run] [--no-macos] [--skip-check]"
[ "$skip_check" = 0 ] || [ "$dry" = 1 ] || die "--skip-check is for --dry-run only: a real release always runs scripts/check.sh"

version=${tag#v}
sha=$(git rev-parse HEAD)
dist="dist/$version"
logs="$(pwd)/target/release-local/logs"
root=$(pwd -W 2> /dev/null || pwd)

# What a dry run only warns about, a real one refuses.
gate() { if [ "$dry" = 1 ]; then warn "$* (dry run: going on)"; else die "$*"; fi; }

# The names CI gives the files (build.yml), without the version: the dmg is the macOS workflow's.
windows_linux=(
  windows-x64-setup.exe
  windows-x64.msi
  windows-x64-portable.zip
  linux-x86_64.deb
  linux-x86_64.AppImage
  linux-x86_64.tar.gz
)
ci_names=$(grep -o 'dist/takeshot-\$VERSION-[A-Za-z0-9_.-]*' .github/workflows/build.yml |
  sed 's|^dist/takeshot-\$VERSION-||' | sort -u)
mine=$(printf '%s\n' "${windows_linux[@]}" macos-universal.dmg | sort -u)
[ "$ci_names" = "$mine" ] ||
  die "the names in this script differ from build.yml's:"$'\n'"$(diff <(echo "$mine") <(echo "$ci_names") || true)"

step "Checks on $tag at ${sha:0:7}"
[ "$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)" = "$version" ] ||
  die "Cargo.toml says $(grep -m1 '^version' Cargo.toml | cut -d'"' -f2), not $version: bump it in a pull request first"
[ -z "$(git status --porcelain)" ] || gate "the working tree is not clean (the Linux packages are built from the commit, Windows from the tree)"
if [ "$dry" = 0 ]; then
  git fetch --quiet origin main
else
  echo "dry run: origin/main as last fetched, no network"
fi
git merge-base --is-ancestor HEAD origin/main || gate "HEAD (${sha:0:7}) is not on origin/main"

packager_version=$(grep -m1 'PACKAGER_VERSION:' .github/workflows/build.yml | awk '{print $2}')
command -v cargo-packager > /dev/null && cargo packager --version | grep -q "$packager_version" ||
  die "cargo-packager $packager_version is missing: cargo install cargo-packager --version $packager_version --locked"
docker info > /dev/null 2>&1 || die "Docker is not running (the Linux packages are built in a container)"
if command -v 7z > /dev/null; then
  zip7=7z
elif [ -x "/c/Program Files/7-Zip/7z.exe" ]; then
  zip7="/c/Program Files/7-Zip/7z.exe"
else
  die "7-Zip is missing (the portable zip): install it or put 7z in the PATH"
fi

if [ "$dry" = 0 ]; then
  gh auth status > /dev/null 2>&1 || die "gh is not signed in: gh auth login"
  [ "$(gh variable get SIGN_WINDOWS 2> /dev/null || true)" != true ] ||
    die "SIGN_WINDOWS is on: CI signs the Windows files and this script does not; release with the release workflow"
  # Like release.yml: a version that is out already is not released again.
  if release=$(gh release view "$tag" --json isDraft,targetCommitish --jq '"\(.isDraft) \(.targetCommitish)"' 2> /dev/null); then
    read -r draft target <<< "$release"
    [ "$draft" = true ] || die "$tag is published already: bump version in Cargo.toml first"
    target_sha=$(git rev-parse --verify -q "$target^{commit}" || git rev-parse --verify -q "origin/$target^{commit}") ||
      die "the draft of $tag points to $target, which this checkout does not know"
    [ "$target_sha" = "$sha" ] ||
      die "the draft of $tag points to ${target_sha:0:7} and HEAD is ${sha:0:7}: delete the draft or check out that commit"
    have_release=1
  else
    git ls-remote --exit-code --tags origin "refs/tags/$tag" > /dev/null 2>&1 &&
      die "$tag exists on origin without a release: bump version in Cargo.toml first"
    have_release=0
  fi
fi

mkdir -p "$logs"
run() { # run <log name> <command...>: output to the log, which an error names
  local log="$logs/$1.log"
  shift
  "$@" > "$log" 2>&1 || die "failed, see $log"
}

if [ "$skip_check" = 1 ]; then
  warn "scripts/check.sh skipped"
else
  step "scripts/check.sh (host, then Linux and macOS clippy in the container)"
  scripts/check.sh || die "check failed: nothing was built or uploaded"
fi

if [ "$dry" = 0 ]; then
  if [ "$have_release" = 0 ]; then
    step "Draft release $tag (the tag is created when it is published)"
    gh release create "$tag" --draft --target "$sha" --title "Takeshot $version" --generate-notes
  fi
  if [ "$macos" = 1 ]; then
    step "release-macos: the dmg builds on GitHub while this PC builds the rest"
    gh workflow run release-macos.yml -f "tag=$tag" ||
      die "could not start release-macos (is the workflow on main?); the draft stays, try again or use --no-macos"
  fi
fi

rm -rf "$dist"
mkdir -p "$dist"
pick() { # pick <name> <glob>: copy the one file the glob matches
  local name=$1
  shift
  [ $# -eq 1 ] && [ -f "$1" ] || die "expected one file for $name, found: $*"
  cp "$1" "$dist/$name"
}

step "Windows: cargo build --release"
run windows-build cargo build --release
step "Windows: cargo packager (nsis, wix)"
rm -rf target/packages
run windows-packager cargo packager --release --formats nsis,wix
pick "takeshot-$version-windows-x64-setup.exe" target/packages/*-setup.exe
pick "takeshot-$version-windows-x64.msi" target/packages/*.msi
step "Windows: portable zip"
stage=target/release-local/portable
rm -rf "$stage"
mkdir -p "$stage"
cp target/release/takeshot.exe "$stage/Takeshot.exe"
cp LICENSE THIRD_PARTY_NOTICES.md "$stage/"
(cd "$stage" && run windows-zip "$zip7" a "$root/$dist/takeshot-$version-windows-x64-portable.zip" .)

step "Linux: scripts/linux-packages.sh ${sha:0:7}"
run linux-packages scripts/linux-packages.sh "$sha"
for ext in deb AppImage tar.gz; do
  pick "takeshot-$version-linux-x86_64.$ext" "target/packages-linux/takeshot-$version-linux-x86_64.$ext"
done

step "Names"
built=$(cd "$dist" && ls | sort)
want=$(printf 'takeshot-'"$version"'-%s\n' "${windows_linux[@]}" | sort)
[ "$built" = "$want" ] || die "dist/$version differs from what CI names:"$'\n'"$(diff <(echo "$want") <(echo "$built") || true)"
(cd "$dist" && ls -l && sha256sum *)

files=()
for suffix in "${windows_linux[@]}"; do files+=("$dist/takeshot-$version-$suffix"); done

if [ "$dry" = 1 ]; then
  step "Dry run: nothing touched GitHub. A real run would"
  echo "  create the draft $tag at ${sha:0:7} if there is none"
  [ "$macos" = 0 ] || echo "  start the release-macos workflow for $tag"
  echo "  upload (--clobber) to $tag:"
  printf '    %s\n' "${files[@]}"
  exit 0
fi

step "Upload to $tag"
gh release upload "$tag" "${files[@]}" --clobber
gh release view "$tag" --json url,isDraft,assets --jq '"\(.url) (draft: \(.isDraft))", (.assets[] | "  \(.name)  \(.size)")'

cat << EOF

The draft $tag is not published and the tag does not exist yet.
macOS: the dmg is attached by the release-macos run (gh run list --workflow release-macos.yml);
without it, run: gh workflow run release-macos.yml -f tag=$tag
When all seven files are there and you decide to release: gh release edit $tag --draft=false
EOF
