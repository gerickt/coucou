#!/bin/sh
# Runs the plugin specs under the Luau CLI with a mocked Noctalia host.
# Each spec is concatenated with the mocks and its entry script, since the
# host gives every entry a single global environment. sprite.luau and frames/
# manifest are copied next to it so the entries' require("./sprite") resolves.
# Usage: mochi/tests/run.sh   (needs `luau` in PATH, or LUAU=/path/to/luau)
set -eu
LUAU=${LUAU:-luau}
here=$(cd "$(dirname "$0")" && pwd)
plugin=$(dirname "$here")
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cp "$plugin/sprite.luau" "$tmp/sprite.luau"
mkdir "$tmp/frames"
cp "$plugin/frames/manifest.luau" "$tmp/frames/manifest.luau"
status=0
for entry in service desktop bar panel agents; do
  ui="$here/mock_ui.luau"
  [ "$entry" = service ] && ui=/dev/null
  cat "$here/mock_host.luau" "$here/mock_json.luau" "$ui" "$plugin/$entry.luau" "$here/${entry}_spec.luau" > "$tmp/$entry.luau"
  if "$LUAU" "$tmp/$entry.luau" > "$tmp/$entry.out" 2>&1; then
    tail -n 1 "$tmp/$entry.out"
  else
    echo "FAILED: $entry"; cat "$tmp/$entry.out"; status=1
  fi
done
exit $status
