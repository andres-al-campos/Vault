#!/usr/bin/env bash
#
# Steady buttons: the player bar is redrawn four times a second by the clock,
# and a button whose icon is replaced between press and release drops the
# click. renderTransport goes through setIcon, which only swaps a changed icon.
#
set -uo pipefail
cd "$(dirname "$0")/.."

body=$(awk '/^function renderTransport\(/,/^}/' src/main.js)
hits=$(printf '%s\n' "$body" | grep -n 'innerHTML' || true)
if [ -n "$hits" ]; then
  echo "✗ steady buttons: renderTransport in src/main.js sets innerHTML:
$hits
Swapping a button's content on every clock tick loses clicks. Use setIcon(el, icon), which only replaces it when it changes." >&2
  exit 1
fi
echo "✓ steady buttons: the player bar never replaces an icon that hasn't changed"
