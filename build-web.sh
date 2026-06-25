#!/usr/bin/env bash
# Build the WASM/HTML web version into dist/ and serve it.
#
#   ./build-web.sh          # build + serve at http://127.0.0.1:8080
#   ./build-web.sh --build  # build only (no server)
#
# All assets (img/sounds) are baked into the binary via include_bytes!, so the
# dist/ folder is self-contained: index.html + mq_js_bundle.js + sail-rs.wasm.
set -euo pipefail
cd "$(dirname "$0")"

echo ">> cargo build --release --target wasm32-unknown-unknown"
cargo build --release --target wasm32-unknown-unknown

mkdir -p dist
cp target/wasm32-unknown-unknown/release/sail-rs.wasm dist/sail-rs.wasm
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

if [ "${1:-}" != "--build" ]; then
  echo ">> serving http://127.0.0.1:8080  (Ctrl-C to stop)"
  if command -v basic-http-server >/dev/null 2>&1; then
    exec basic-http-server -a 127.0.0.1:8080 dist
  else
    exec python -m http.server 8080 --directory dist
  fi
fi
