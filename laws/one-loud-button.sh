#!/usr/bin/env bash
#
# One loud thing: the white fill (--on, class btn-on) belongs to a single
# button, so that button stays the main thing on the screen. It once sat on
# Subscribe, Download all, Open videos and both chosen filters at once, and
# with everything loud nothing was.
#
set -uo pipefail
cd "$(dirname "$0")/.."

fail=0
say() { echo "✗ one loud button: $*" >&2; fail=1; }

uses=$(grep -o 'btn-on' index.html src/main.js | wc -l | tr -d ' ')
[ "$uses" -eq 1 ] || say "the class btn-on is on $uses elements in index.html and src/main.js; it belongs on one. Give the others btn-primary (the grey fill) or btn-low (the tint)."

fills=$(grep -n 'background: *var(--on)' src/styles.css | grep -v '^[0-9]*:\.btn-on ' || true)
[ -z "$fills" ] || say "src/styles.css fills something other than .btn-on with --on:
$fills
Use --mid or --low there."

aliases=$(grep -n -- '--[a-z-]*: *var(--on)' src/styles.css || true)
[ -z "$aliases" ] || say "src/styles.css gives --on another name, which lets the white fill spread unseen:
$aliases
Point that variable at --mid or --low."

[ "$fail" -eq 0 ] && echo "✓ one loud button: the white fill is on one button only"
exit "$fail"
