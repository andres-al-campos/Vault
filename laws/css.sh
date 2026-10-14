#!/usr/bin/env bash
#
# Two ways the stylesheet has failed without a sound:
#
# - `font: 600 12.5px inherit` is invalid (a CSS-wide keyword can't stand in
#   for one part of a shorthand), so the browser drops the whole line. Fourteen
#   controls were drawn in the default button font this way, their sizes and
#   weights never applied.
# - A class's base rule written in two places. When the filter row's wrapper
#   was given `.bar { display: contents; }`, the Downloads panel's progress
#   bars, already called .bar, turned into tall green blocks.
#
set -uo pipefail
cd "$(dirname "$0")/.."

fail=0
say() { echo "✗ css: $*" >&2; fail=1; }

bad=$(grep -nE '(^|[ ;{])font: *[^;]*[^-a-z](inherit|initial|unset|revert)\b' src/styles.css || true)
[ -z "$bad" ] || say "a font shorthand in src/styles.css names a CSS-wide keyword among its values, so the browser ignores the line:
$bad
Write it as longhands: font-weight: 600; font-size: 12.5px; font-family: inherit;"

twice=$(grep -oE '^\.[a-zA-Z][a-zA-Z0-9_-]* \{' src/styles.css | sort | uniq -d || true)
[ -z "$twice" ] || say "src/styles.css starts a rule for these classes in more than one place:
$twice
If they are one thing, merge the rules. If they are two things that share a name, rename one."

[ "$fail" -eq 0 ] && echo "✓ css: every font line applies, and each class's base rule is in one place"
exit "$fail"
