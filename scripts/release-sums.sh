#!/usr/bin/env bash
# The SHA256SUMS of a release: downloads the seven installers the release holds, writes their
# sha256 and uploads the list as one more file, so whoever downloads an installer can check it
# against the list. Run it once the dmg is attached (release-macos.yml does that on GitHub) and
# before publishing the draft; run it again after a file of the release changes.
#
#   scripts/release-sums.sh vX.Y.Z             download, write SHA256SUMS, upload it
#   scripts/release-sums.sh vX.Y.Z --dry-run   the same, without the upload
#
# The release can be a draft or published. It refuses a release that does not hold exactly the
# names build.yml gives the installers. When dist/<version>/ (what release-local.sh built) is
# there, each file in it is compared with the one the release holds. The list lands in
# target/release-sums/<tag>/ (git-ignored).
set -eu
cd "$(git rev-parse --show-toplevel)"

die() { echo "release-sums: $*" >&2; exit 1; }
warn() { echo "release-sums: warning: $*" >&2; }

tag=""
dry=0
for arg in "$@"; do
  case "$arg" in
    --dry-run) dry=1 ;;
    -*) die "unknown argument: $arg" ;;
    *) [ -z "$tag" ] || die "one tag only: $tag and $arg"; tag="$arg" ;;
  esac
done
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "usage: scripts/release-sums.sh vX.Y.Z [--dry-run]"
version=${tag#v}

gh auth status > /dev/null 2>&1 || die "gh is not signed in: gh auth login"

# The names CI gives the installers (build.yml), as release-local.sh reads them.
want=$(grep -o 'dist/takeshot-\$VERSION-[A-Za-z0-9_.-]*' .github/workflows/build.yml |
  sed "s|^dist/takeshot-\\\$VERSION-|takeshot-$version-|" | sort -u)
[ -n "$want" ] || die "found no installer names in .github/workflows/build.yml"

work="target/release-sums/$tag"
rm -rf "$work"
mkdir -p "$work"
# takeshot-* leaves out a SHA256SUMS the release already has.
gh release download "$tag" --dir "$work" --pattern 'takeshot-*' || die "could not download the files of $tag"
have=$(cd "$work" && ls | sort)
[ "$have" = "$want" ] ||
  die "$tag does not hold the installers build.yml names (is the dmg attached yet?):"$'\n'"$(diff <(echo "$want") <(echo "$have") || true)"

mapfile -t files < <(cd "$work" && ls | LC_ALL=C sort)
(cd "$work" && sha256sum --text "${files[@]}" > SHA256SUMS)
cat "$work/SHA256SUMS"

if [ -d "dist/$version" ]; then
  for built in "dist/$version"/takeshot-*; do
    name=${built##*/}
    if cmp -s "$built" "$work/$name"; then
      echo "same as the file built here: $name"
    else
      warn "$name differs from dist/$version/$name (built again since the upload?)"
    fi
  done
fi

if [ "$dry" = 1 ]; then
  echo "dry run: SHA256SUMS not uploaded ($work/SHA256SUMS)"
  exit 0
fi
gh release upload "$tag" "$work/SHA256SUMS" --clobber
echo "SHA256SUMS is on $tag."
