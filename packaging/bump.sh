#!/usr/bin/env bash
# Point the Homebrew formula and the AUR PKGBUILD at release <version>, using the checksums of the tarballs that
# packaging/package.sh made (or that were downloaded from the GitHub Release into target/package/):
#   packaging/bump.sh 0.2.0
# Edits packaging/homebrew/lenz.rb and packaging/aur/PKGBUILD in place; publishing them stays a manual step.
set -euo pipefail
cd "$(dirname "$0")/.."
version=${1:?usage: packaging/bump.sh <version>}
sum() {
  local f="target/package/lenz-$version-$1.tar.gz.sha256"
  if [ -f "$f" ]; then cut -d' ' -f1 "$f"; else echo "MISSING-$1"; fi
}
linux_x86=$(sum x86_64-unknown-linux-gnu)
linux_arm=$(sum aarch64-unknown-linux-gnu)
mac_arm=$(sum aarch64-apple-darwin)
sed -i -E \
  -e "s/^  version \".*\"/  version \"$version\"/" \
  -e "/aarch64-apple-darwin/{n;s/sha256 \".*\"/sha256 \"$mac_arm\"/}" \
  -e "/x86_64-unknown-linux-gnu/{n;s/sha256 \".*\"/sha256 \"$linux_x86\"/}" \
  -e "/aarch64-unknown-linux-gnu/{n;s/sha256 \".*\"/sha256 \"$linux_arm\"/}" \
  packaging/homebrew/lenz.rb
sed -i -E \
  -e "s/^pkgver=.*/pkgver=$version/" \
  -e "s/^pkgrel=.*/pkgrel=1/" \
  -e "s/^sha256sums_x86_64=.*/sha256sums_x86_64=('$linux_x86')/" \
  -e "s/^sha256sums_aarch64=.*/sha256sums_aarch64=('$linux_arm')/" \
  packaging/aur/PKGBUILD
grep -n 'version\|sha256' packaging/homebrew/lenz.rb
grep -n 'pkgver\|sha256' packaging/aur/PKGBUILD
if grep -q MISSING packaging/homebrew/lenz.rb packaging/aur/PKGBUILD; then
  echo "note: some targets have no tarball in target/package/ yet (MISSING-<target>)" >&2
fi
