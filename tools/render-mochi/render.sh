#!/bin/sh
# Regenerates mochi/frames/ from the ORIGINAL Mochi engine.
#
#   tools/render-mochi/render.sh [--compare <contact.png>]
#
# 1. checks vendor/ is still byte-identical to the original files at 081a2ad
#    (when that commit is reachable),
# 2. render.mjs drives BotEngine in headless Chrome and saves raw PNG frames,
# 3. optimize.py palettes, dedupes and names them, and writes manifest.luau.
#
# Needs: node >= 23 (node:module stripTypeScriptTypes), Chrome or Chromium
# (CHROME=/path, default /usr/bin/google-chrome-stable), python3 with Pillow
# (built with libimagequant) and numpy. WORK=<dir> keeps the raw frames.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
out="$repo/mochi/frames"

if git -C "$repo" cat-file -e 081a2ad 2>/dev/null; then
  while read -r vendored original; do
    if ! git -C "$repo" show "081a2ad:$original" | cmp -s - "$here/vendor/$vendored"; then
      echo "vendor/$vendored differs from 081a2ad:$original" >&2
      exit 1
    fi
  done <<EOF
mochi/engine.ts windows/src/mochi/engine.ts
mochi/outfits.ts windows/src/mochi/outfits.ts
core/anim.ts windows/src/core/anim.ts
core/fonts.ts windows/src/core/fonts.ts
EOF
  echo "vendor: identical to 081a2ad"
fi

if [ -n "${WORK:-}" ]; then
  work=$WORK
  rm -rf "$work"
  mkdir -p "$work"
else
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT
fi

node --no-warnings "$here/render.mjs" "$work"
python3 -I "$here/optimize.py" "$work" "$out"
echo "frames: $(du -sh "$out" | cut -f1) in $out"

if [ "${1:-}" = "--compare" ]; then
  node --no-warnings "$here/render.mjs" --compare "$work" "$out" "$2"
  echo "contact sheet: $2"
fi
