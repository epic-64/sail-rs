#!/usr/bin/env bash
# Build the WASM/HTML web version into dist/.
#
#   ./build-web.sh          # build + serve at http://127.0.0.1:8080
#   ./build-web.sh --build  # build only (no server)
#   ./build-web.sh --zip    # build + package an itch.io-ready zip (no server)
#
# All assets (img/sounds) are baked into the binary via include_bytes!, so the
# dist/ folder is self-contained: index.html + mq_js_bundle.js + sail-rs.wasm.
set -euo pipefail
cd "$(dirname "$0")"

echo ">> cargo build --profile web --target wasm32-unknown-unknown"
cargo build --profile web --target wasm32-unknown-unknown

mkdir -p dist
WASM="target/wasm32-unknown-unknown/web/sail-rs.wasm"

# Post-optimize the wasm with binaryen's wasm-opt if it's available (PATH or the
# vendored copy under .tools/). This shrinks the code section further; the wasm
# uses bulk-memory ops, so that feature must be enabled for the validator.
WASM_OPT=$(command -v wasm-opt || true)
[ -z "$WASM_OPT" ] && WASM_OPT=$(ls .tools/binaryen-*/bin/wasm-opt.exe 2>/dev/null | sort -V | tail -1 || true)
if [ -n "$WASM_OPT" ]; then
  echo ">> wasm-opt -Oz ($("$WASM_OPT" --version))"
  "$WASM_OPT" -Oz --enable-bulk-memory --enable-bulk-memory-opt \
    --enable-nontrapping-float-to-int --enable-mutable-globals \
    "$WASM" -o dist/sail-rs.wasm
else
  echo ">> wasm-opt not found — shipping the unoptimized wasm"
  cp "$WASM" dist/sail-rs.wasm
fi
# Web shell + icon. The HTML shell is source (web/); everything in dist/ is
# generated, so dist/ is disposable build output (gitignored).
cp web/index.html dist/index.html
cp assets/favicon.svg dist/favicon.svg 2>/dev/null || true

# Assemble the JS loader from the EXACT crate sources in the local cargo
# registry, so the bundle's protocol/plugin versions always match the compiled
# wasm. (The macroquad master bundle drifts ahead of the pinned miniquad and
# also ships sapp_jsutils, which this project doesn't use.) The bundle is just
# miniquad's gl.js (graphics + plugin host) + quad-snd's audio.js (the
# macroquad_audio plugin). Re-run with the bundle deleted to regenerate.
if [ ! -f dist/mq_js_bundle.js ]; then
  echo ">> assembling mq_js_bundle.js from crate sources"
  REG="$HOME/.cargo/registry/src"
  GL=$(find "$REG" -path "*/miniquad-*/js/gl.js" | sort -V | tail -1)
  AUDIO=$(find "$REG" -path "*/quad-snd-*/js/audio.js" | sort -V | tail -1)
  if [ -z "$GL" ] || [ -z "$AUDIO" ]; then
    echo "!! could not find gl.js/audio.js in $REG — run a native build first" >&2
    exit 1
  fi
  { cat "$GL"; echo; cat "$AUDIO"; } > dist/mq_js_bundle.js
fi

echo ">> dist/ ready ($(du -h dist/sail-rs.wasm | cut -f1) wasm)"

case "${1:-}" in
  --build)
    ;; # build only; nothing more to do
  --zip)
    # itch.io wants index.html at the ROOT of the zip (not inside a folder), so
    # we archive the CONTENTS of dist/, not the dist/ directory itself. No `zip`
    # binary ships with git-bash on Windows, so use PowerShell's Compress-Archive.
    OUT="sail-rs-web.zip"
    rm -f "$OUT"
    echo ">> packaging $OUT (itch.io-ready: index.html at root)"
    powershell -NoProfile -Command "Compress-Archive -Path 'dist/*' -DestinationPath '$OUT' -Force"
    echo ">> done -> $OUT ($(du -h "$OUT" | cut -f1))"
    echo "   Upload to itch.io, tick 'This file will be played in the browser',"
    echo "   and set the viewport (e.g. 1280x720) in the embed options."
    ;;
  *)
    echo ">> serving http://127.0.0.1:8080  (Ctrl-C to stop)"
    if command -v basic-http-server >/dev/null 2>&1; then
      exec basic-http-server -a 127.0.0.1:8080 dist
    elif command -v python3 >/dev/null 2>&1; then
      exec python3 -m http.server 8080 --directory dist
    elif command -v python >/dev/null 2>&1; then
      exec python -m http.server 8080 --directory dist
    else
      echo "!! no web server found (install basic-http-server or python3)" >&2
      exit 1
    fi
    ;;
esac
