# sail-rs

A first-person age-of-sail trading game — sail a low-poly ship across a procedurally
generated archipelago, trade goods between ports, run haulage contracts, wager on
races, and scoop salvage from the swell. A Rust + [macroquad](https://github.com/not-fl3/macroquad)
port of an earlier ScalaJS/HTML5 game, rewritten for a smooth native renderer.

See [`PLAN.md`](PLAN.md) for the architecture map and feature status.

## Requirements

- **Rust** (stable, edition 2024 — tested on 1.87). Install via [rustup](https://rustup.rs).
- A desktop OS with OpenGL (Windows/macOS/Linux). On Linux you may need the usual
  ALSA/X11/Wayland dev packages macroquad documents.
- For the **web build** only: the `wasm32-unknown-unknown` target
  (`rustup target add wasm32-unknown-unknown`).

## Quick start (native)

```sh
cargo run --release
```

Use `--release` — the dense wave mesh and per-frame work want optimizations for a
smooth frame rate. A debug `cargo run` works but is choppier.

### Common dev commands

| Command | What it does |
|---|---|
| `cargo run --release` | Build and run the game (recommended). |
| `cargo run` | Debug run — faster to compile, lower FPS. |
| `cargo build --release` | Build the native binary without running. |
| `cargo test` | Run the unit tests (RNG/world/sailing/missions/races/… — no window needed). |
| `cargo check` | Fast type-check without producing a binary. |
| `cargo clippy` | Lint (if installed). |

## Controls

| Key | Action |
|---|---|
| **W / S** | Deploy / furl sail (None → Half → Full). Set it and she keeps going. |
| **A / D** | Helm (steer). |
| **Space** | Dock at a port in range (sails furled, bow pointed at it). |
| **C** (hold) | Look astern over the wake (helm unchanged). |
| **H** | Hide / show the HUD (corner readout, chart, controls). |
| **Q / E** | Nudge the weather calmer / stormier (it also auto-drifts). |
| **T / Y** | Advance / rewind the time of day. |
| **`[` / `]`** | Back / veer the wind (dev aid for feeling the points of sail). |
| **L** | Open / close the captain's log (**← / →** turn pages while open). |
| **Esc** | Close the log / open the pause menu / quit. |

Bloom and 4× MSAA are toggled in the pause menu's **Options** (native only — both show as
"Not supported" on the web; see below).
| *In port* | **Arrows** move cursor · **Tab** switch board · **Enter** trade · **Esc** set sail. |

## Building for the web / itch.io

The whole game compiles to a single self-contained WebAssembly module — all art and
audio are baked in (`include_bytes!`), so there are no side files to host. One script
does everything:

```sh
./build-web.sh          # build + serve at http://127.0.0.1:8080
./build-web.sh --build  # build only (writes dist/)
./build-web.sh --zip    # build + package an itch.io-ready sail-rs-web.zip
```

### One-time setup

```sh
rustup target add wasm32-unknown-unknown
```

Two **optional** tools make the download smaller. The script auto-detects them on
`PATH` or under `.tools/`, and degrades gracefully (with a warning) if they're absent:

- **[wasm-opt](https://github.com/WebAssembly/binaryen)** (binaryen) — shrinks the
  compiled code. Without it the wasm just ships unoptimized.
- **[ffmpeg](https://ffmpeg.org)** — re-encodes the embedded audio to a lower bitrate
  for the web build (~5.3 MB → ~2.5 MB). Without it the web build embeds the
  full-quality clips (larger wasm) and prints a `cargo:warning`. The audio re-encode
  happens automatically in `build.rs` at build time; nothing is committed.

To vendor them locally, drop the release builds under `.tools/` (e.g.
`.tools/binaryen-version_130/bin/wasm-opt.exe`,
`.tools/ffmpeg-*/bin/ffmpeg.exe`). `.tools/` is gitignored.

### Ship it to itch.io

1. `./build-web.sh --zip` → produces **`sail-rs-web.zip`** (~2.9 MB, `index.html` at
   the zip root, as itch requires).
2. On itch.io: create/edit a project, set **Kind of project → HTML**.
3. Upload `sail-rs-web.zip` and tick **"This file will be played in the browser."**
4. Under **Embed options**, set the viewport to **1280 × 720** and enable the
   **Fullscreen button**. "Click to launch" is recommended — the click gives the
   browser the user gesture it needs to start audio cleanly.

### How the web build works (under the hood)

- **`cargo build --profile web`** — a size-tuned profile (LTO, one codegen unit,
  `panic = "abort"`, strip) separate from native `release`, so native keeps its
  speed-tuned settings.
- **`mq_js_bundle.js`** is assembled from the *exact* crate sources in your cargo
  registry (miniquad's `gl.js` + quad-snd's `audio.js`) so the JS protocol always
  matches the compiled wasm. (Don't grab macroquad's `master` bundle — it drifts
  ahead of the pinned miniquad.)
- **WebGL compatibility:** the web build uses a plain **WebGL1** context with **no
  MSAA** and **no bloom**. Those rely on WebGL2-only calls (`blitFramebuffer`/
  `readBuffer`/`drawBuffers`) that aren't granted in every browser/iframe (notably
  inside itch's embed, and behind some privacy extensions). Native is unaffected —
  it keeps 4× MSAA and bloom. If a player still sees a `getContext`/WebGL error on
  itch, it's almost always an ad/privacy/canvas-fingerprint blocker; testing in an
  incognito window with extensions disabled rules it out.

## Project layout

```
src/            game + renderer modules (see PLAN.md for the full map)
assets/         img/ + sounds/ (baked into the binary), favicon
web/            web shell source (index.html)
build.rs        build-time web-audio re-encode (wasm target only)
build-web.sh    build / serve / package the web version
dist/           generated web build output (gitignored, disposable)
PLAN.md         architecture map + porting status
```

## License

TODO.
