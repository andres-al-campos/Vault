#!/usr/bin/env bash
#
# The feature map stays in step with itself: every row in dev/features/README.md
# has its file, every feature file has a row, and every `./build.sh <arm>` a
# file names is an arm build.sh has.
#
set -uo pipefail
cd "$(dirname "$0")/.."

fail=0
say() { echo "✗ feature map: $*" >&2; fail=1; }

rows=$(grep -oE '\]\([a-z0-9-]+\.md\)' dev/features/README.md | sed -E 's/^\]\((.*)\)$/\1/' | sort -u)
[ -n "$rows" ] || say "dev/features/README.md lists no features. Add a row per feature file."

for f in $rows; do
  [ -f "dev/features/$f" ] || say "dev/features/README.md lists $f but dev/features/$f doesn't exist. Write the file or drop the row."
done

for path in dev/features/*.md; do
  f=$(basename "$path")
  [ "$f" = README.md ] && continue
  echo "$rows" | grep -qx "$f" || say "dev/features/$f has no row in dev/features/README.md. Add one."
done

for path in dev/features/*.md; do
  for arm in $(grep -oE '\./build\.sh [a-z]+' "$path" | awk '{print $2}' | sort -u); do
    grep -qE "^[[:space:]]*([a-z|]*\|)?${arm}(\|[a-z|]*)?\)" build.sh ||
      say "$path names \`./build.sh $arm\`, which build.sh doesn't have. Fix the command or add the arm."
  done
done

[ "$fail" -eq 0 ] && echo "✓ feature map: every row has a file, every file a row, every command exists"
exit "$fail"
