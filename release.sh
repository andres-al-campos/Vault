#!/usr/bin/env bash
#
# Cut a GitHub release of Vault: package the .app fresh with ./build.sh package,
# zip it, and publish it under a tag named for the version in
# src-tauri/tauri.conf.json. Bump the version there (and in package.json and
# src-tauri/Cargo.toml), commit, push, then run this.
#
set -euo pipefail
cd "$(dirname "$0")"

die() { echo "$*" >&2; exit 1; }

VERSION=$(node -p "require('./src-tauri/tauri.conf.json').version")
TAG="v${VERSION}"

# The three version fields describe one app; a release from a mismatch would
# tag one number and report another in About.
NPM_VERSION=$(node -p "require('./package.json').version")
CARGO_VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' src-tauri/Cargo.toml | head -1)
[ "$NPM_VERSION" = "$VERSION" ] && [ "$CARGO_VERSION" = "$VERSION" ] \
  || die "Versions disagree: tauri.conf.json $VERSION, package.json $NPM_VERSION, Cargo.toml $CARGO_VERSION. Set all three to the same version, commit, push, then re-run ./release.sh."

[ -z "$(git status --porcelain)" ] \
  || die "There are uncommitted changes, so the release wouldn't match what's on GitHub. Commit and push them, then re-run ./release.sh."

git fetch -q origin
[ "$(git rev-parse HEAD)" = "$(git rev-parse '@{u}')" ] \
  || die "HEAD isn't what's on origin. Push (or pull) first, then re-run ./release.sh."

if git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null || gh release view "$TAG" >/dev/null 2>&1; then
  die "${TAG} is already released. Bump the version in src-tauri/tauri.conf.json, package.json and src-tauri/Cargo.toml, commit, push, then re-run ./release.sh."
fi

./build.sh package

APP=src-tauri/target/release/bundle/macos/Vault.app
ZIP="src-tauri/target/release/bundle/Vault-${VERSION}-macos-$(uname -m).zip"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"

NOTES=$(cat <<'EOF'
For Macs with Apple Silicon. Vault drives tools it doesn't bundle, so install them first:

```sh
brew install yt-dlp ffmpeg mpv
```

The app is ad-hoc signed, not notarized, so macOS won't open a downloaded copy until its quarantine flag is cleared. After unzipping:

```sh
xattr -dr com.apple.quarantine Vault.app
```
EOF
)

gh release create "$TAG" "$ZIP" --target "$(git rev-parse HEAD)" \
  --title "Vault ${VERSION}" --notes "$NOTES" --generate-notes

echo "Released ${TAG}: $(gh release view "$TAG" --json url -q .url)"
