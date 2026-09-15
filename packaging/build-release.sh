#!/usr/bin/env bash
# Build the release artifacts: the tarball the install script downloads and the
# -bin package fetches, plus the checksums both verify against.
#
#   packaging/build-release.sh          build for this machine's architecture
#
# Everything downstream — the AUR package, the one-line installer, the website
# link — points at what this produces, so it is the single place the contents
# of a release are decided.

set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' overlay/Cargo.toml | head -1)"
ARCH="$(uname -m)"
NAME="nimbus-wayland-${VERSION}-${ARCH}"
OUT="$ROOT/dist"

echo "Building nimbus-wayland ${VERSION} for ${ARCH}"

# --frozen so a release can never quietly pick up a different dependency than
# the one the lockfile records and the tests ran against.
( cd overlay && cargo build --frozen --release )
( cd overlay && cargo test --frozen --release >/dev/null )

rm -rf "$OUT/$NAME"
mkdir -p "$OUT/$NAME/omarchy"

install -m755 overlay/target/release/nimbus-wayland "$OUT/$NAME/nimbus-wayland"
install -m644 packaging/nimbus-wayland.service      "$OUT/$NAME/nimbus-wayland.service"
install -m644 config.example.json                   "$OUT/$NAME/config.example.json"
install -m644 LICENSE                               "$OUT/$NAME/LICENSE"
install -m644 README.md                             "$OUT/$NAME/README.md"
cp -r omarchy/nimbus.ring                           "$OUT/$NAME/omarchy/"

strip "$OUT/$NAME/nimbus-wayland" 2>/dev/null || true

# One versioned directory at the top of the archive, so unpacking by hand does
# not spray files into the current directory. PKGBUILD-bin and install.sh both
# cd into it; change one and change all three.
( cd "$OUT" && tar -czf "${NAME}.tar.gz" "$NAME" )
rm -rf "$OUT/$NAME"

# One SHA256SUMS covering every architecture in the release, appended rather
# than overwritten so a second architecture built later joins the same file.
( cd "$OUT" && sha256sum "${NAME}.tar.gz" >> SHA256SUMS.new \
  && sort -u -k2 SHA256SUMS.new > SHA256SUMS && rm -f SHA256SUMS.new )

echo
echo "  dist/${NAME}.tar.gz  ($(du -h "$OUT/${NAME}.tar.gz" | cut -f1))"
echo "  dist/SHA256SUMS"
echo
echo "Next:"
echo "  gh release create v${VERSION} dist/${NAME}.tar.gz dist/SHA256SUMS \\"
echo "    --title 'Nimbus ${VERSION}' --notes 'What changed'"
echo "  sha256sum dist/${NAME}.tar.gz   # then paste into packaging/PKGBUILD-bin"
