#!/usr/bin/env bash
#
# Build and install the Vault desktop app — the everyday "run the thing I'm
# working on" path. No arguments builds a real .app, installs it to
# /Applications/_vibe_coded replacing the last one, and launches it.
#
#   ./build.sh              package the .app and install it, the default
#   ./build.sh dev          dev run with a hot-reload window
#   ./build.sh check        run every check: the JS and Rust unit tests (fast, no network)
#   ./build.sh check net    also run the live tests that fetch real channels
#   ./build.sh test [net]   the same thing under its old name
#   ./build.sh clean        drop build output
#
# Vault is a Tauri app: `npm run tauri build` compiles the Rust side and the
# web bundle together and drops Vault.app under src-tauri/target/release/bundle.
# There is no deploy target — this is a local tool, not a hosted one.
#
set -euo pipefail

# Run from the repo root regardless of where this is called from.
cd "$(dirname "$0")"

APP_NAME=Vault
INSTALL_DIR=/Applications/_vibe_coded
INSTALLED_APP="${INSTALL_DIR}/${APP_NAME}.app"
BUILT_APP="src-tauri/target/release/bundle/macos/${APP_NAME}.app"

# cargo lives under ~/.cargo/bin, which a GUI-launched or fresh shell may not
# have on PATH. Source rustup's env if it's there so the build works either way.
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

die() { echo "$*" >&2; exit 1; }

# yt-dlp is the whole point of the app; a build without it produces something
# that opens and can't do anything.
require_ytdlp() {
  command -v yt-dlp >/dev/null 2>&1 && return
  [ -x "$HOME/.local/bin/yt-dlp" ] && return
  [ -x /opt/homebrew/bin/yt-dlp ] && return
  cat >&2 <<'EOF'
yt-dlp is not installed, so Vault would build but not be able to download
anything. Install it with either:

  brew install yt-dlp
  uv tool install yt-dlp
EOF
  exit 1
}

# The player is built on libmpv, and the Rust build links it from Homebrew.
require_mpv() {
  brew --prefix mpv >/dev/null 2>&1 && [ -d "$(brew --prefix mpv)/lib" ] && return
  die "mpv is not installed, and Vault's player is built on it. Install it with:

  brew install mpv"
}

[ -d node_modules ] || { echo "Installing dependencies…"; npm install; }

# Kill a running copy before swapping the bundle underneath it. Replacing a live
# .app is what leaves a stale process running while every rebuild "succeeds".
kill_running() {
  local pids
  pids=$(pgrep -f "${INSTALLED_APP}/Contents/MacOS/" 2>/dev/null) || true
  [ -z "$pids" ] && return
  echo "Replacing the Vault already running (pid ${pids//$'\n'/, }) …"
  # shellcheck disable=SC2086
  kill $pids 2>/dev/null || true
  # Return as soon as it's gone. Written as an explicit `if` rather than
  # `pgrep … || return`, because a bare `return` reports the status of the last
  # command — pgrep's failure — and under `set -e` that aborts the whole build
  # right at the point the kill succeeded, leaving nothing installed.
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if ! pgrep -f "${INSTALLED_APP}/Contents/MacOS/" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.2
  done
  # shellcheck disable=SC2086
  kill -9 $pids 2>/dev/null || true
  sleep 0.3
}

# macOS 26 repaints an old-format .icns as glass, which lets the tile's red
# show through the white V and turns it pink. An icon in Apple's layered format
# is drawn as its settings say, and src-tauri/Vault.icon sets the V opaque. The
# .icns Tauri bundles stays in the app for older macOS. This runs before the
# signing below, since it changes the bundle.
layered_icon() {
  local out=src-tauri/target/layered-icon
  if ! xcrun --find actool >/dev/null 2>&1; then
    echo "Xcode's actool wasn't found, so the app keeps its old-format icon and macOS 26 will draw its V pink. Install Xcode 26 or later, run 'sudo xcode-select -s /Applications/Xcode.app', then re-run ./build.sh." >&2
    return 0
  fi
  rm -rf "$out" && mkdir -p "$out"
  xcrun actool src-tauri/Vault.icon --compile "$out" --app-icon Vault \
    --platform macosx --minimum-deployment-target 26.0 \
    --output-partial-info-plist "$out/partial.plist" >/dev/null \
    || die "actool couldn't compile src-tauri/Vault.icon. Run the actool command in build.sh's layered_icon by hand to see its errors; icon.json or Assets/v.svg is likely malformed."
  [ -f "$out/Assets.car" ] || die "actool ran but wrote no Assets.car, which needs Xcode 26 or later. Check 'xcodebuild -version', update Xcode, then re-run ./build.sh."
  cp "$out/Assets.car" "${INSTALLED_APP}/Contents/Resources/Assets.car"
  /usr/libexec/PlistBuddy -c "Add :CFBundleIconName string Vault" "${INSTALLED_APP}/Contents/Info.plist" 2>/dev/null \
    || /usr/libexec/PlistBuddy -c "Set :CFBundleIconName Vault" "${INSTALLED_APP}/Contents/Info.plist"
}

case "${1:-}" in
  dev)
    exec npm run tauri dev
    ;;

  check|test)
    require_mpv
    ./laws/feature-map.sh
    ./laws/one-loud-button.sh
    ./laws/css.sh
    # The frontend's pure modules run under node's own test runner. The glob is
    # quoted so node expands it; a bare directory argument is read as a module.
    node --test 'src/*.test.js'
    # Plain `check` runs the fast unit tests. The live network tests really do
    # fetch whole channels (the members-only check pulls all 7k+ LTT videos),
    # so they are behind an explicit argument rather than in the default path.
    if [ "${2:-}" = "net" ]; then
      (cd src-tauri && cargo test -- --include-ignored --nocapture)
    else
      (cd src-tauri && cargo test)
    fi
    # Stop here. Falling out of the case runs the default path below, which is
    # how `./build.sh test` used to end by installing and launching the app.
    exit 0
    ;;

  clean)
    rm -rf dist src-tauri/target
    echo "Removed dist/ and src-tauri/target/."
    exit 0
    ;;

  '') ;;  # the default: package and install, handled below

  *)
    die "Unknown argument: $1
Usage: ./build.sh [dev|check [net]|clean]   (no argument packages and installs)"
    ;;
esac

require_ytdlp
require_mpv

echo "Building ${APP_NAME}.app …"
# --bundles app rather than the config default, so this never wanders into
# producing a .dmg it has nowhere to put.
npm run tauri build -- --bundles app

[ -d "$BUILT_APP" ] || die "The build finished but ${BUILT_APP} isn't there.
Re-run without --bundles app and check the packaging output above."

mkdir -p "$INSTALL_DIR"
echo "Installing to ${INSTALLED_APP} …"
kill_running
rm -rf "$INSTALLED_APP"
cp -R "$BUILT_APP" "$INSTALLED_APP"

layered_icon

# Ad-hoc sign so Gatekeeper lets a locally built, unsigned app launch, and drop
# any quarantine flag the copy inherited. Local use only — not a distributable
# signature.
codesign --force --deep --sign - "$INSTALLED_APP" >/dev/null 2>&1 || true
xattr -dr com.apple.quarantine "$INSTALLED_APP" 2>/dev/null || true

echo "Launching ${APP_NAME} …"
# LaunchServices treats a bundle id that is still quitting as "open" and
# silently no-ops `open`, which is how a rebuild can finish with no window.
for _ in 1 2 3 4 5 6 7 8 9 10; do
  pgrep -f "${INSTALLED_APP}/Contents/MacOS/" >/dev/null 2>&1 || break
  sleep 0.2
done
open -n "$INSTALLED_APP"

# Confirm it actually came up; a bare `open` can fail quietly.
for _ in 1 2 3 4 5 6 7 8 9 10; do
  pgrep -f "${INSTALLED_APP}/Contents/MacOS/" >/dev/null 2>&1 && break
  sleep 0.3
done
if ! pgrep -f "${INSTALLED_APP}/Contents/MacOS/" >/dev/null 2>&1; then
  echo "${APP_NAME} was installed but didn't launch. Open it from ${INSTALLED_APP} or re-run ./build.sh." >&2
  exit 1
fi

echo ""
echo "${APP_NAME} is installed and running."
echo "  app  →  ${INSTALLED_APP}"
echo "  downloads  →  per channel, chosen on first download"