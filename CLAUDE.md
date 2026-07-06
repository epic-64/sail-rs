A native **Rust + [macroquad](https://github.com/not-fl3/macroquad)** sailing game.
Run with `cargo run --release` (the dense wave mesh wants release for smooth FPS;
debug runs but is choppier). Tests: `cargo test`.

**On Linux, develop against the web build (`./build-web.sh`), not the native
desktop binary.** A local GL driver issue pins the desktop window to ~6 FPS, while
the exact same code runs at a smooth 60 FPS in the browser. `./build-web.sh` with
no args builds and serves at http://127.0.0.1:8080; use that to see and test
changes during development.

## Writing

- **No em dashes (`—`), ever.** This applies everywhere: code comments, user-facing
  strings, commit messages, and chat replies. Reach for any other punctuation instead:
  a colon (`A: B`), parentheses (`A (B)`), a semicolon (`A; B`), or just two sentences
  (`A. B.`). Existing em dashes in the codebase are legacy; do not add new ones, and
  prefer removing one when you touch a line that has it.
- **Comments shouldn't restate facts the code already pins down.** Don't bake a count,
  a list length, an exhaustive enumeration of cases, or a specific value into prose
  when the code right beside it is the real source of truth: "the five steps" rots the
  moment a sixth is added, where "the steps (see `steps`)" stays true forever. Describe
  *what a thing is for* and *why*, and point at the code for the *how many* / *which
  ones*. When you touch a comment that has hard-coded such a detail, fix it to refer to
  the code rather than duplicating it (the same housekeeping the em-dash rule asks for).

## Engine conventions / gotchas

- **Determinism is load-bearing.** World generation must be reproducible from a seed:
  **do not reorder RNG draws** in `world.rs` / `isle_features.rs` — it shifts every
  island. Tune ranges/values, not the draw sequence. `world.islands` is in id order
  (index == id); `features` is generated once and aligned by that index.
- Our `geometry::Vec2` (0=N, clockwise) **shadows glam's `Vec2`** — an explicit `use`
  wins over the macroquad prelude glob. World maths uses ours.
- **macroquad 2D drawing has no depth buffer.** Island/feature/billboard occlusion is
  solved by drawing each island **interleaved between wave bands by distance** in
  `OceanRenderer::render`.
- **Port overlay style** (`port_view.rs` `mod style`): every type size, spacing step,
  column split and symbol is a named token — there are **no bare pixel literals** in the
  render functions. Restyle by editing tokens, not call sites. The pixel tokens are
  **scaled functions** (`pad()`, `chip_h()`, `panel_max_w()`…), multiplied by
  [`ui::scale`]; only ratios/fractions (`CAP_RATIO`, `MKT_PRICE_R`) stay `const`. See
  **UI scale** below.
- **Fonts** (`font.rs`): two embedded faces via `set_default_font` — **DejaVu Sans** is
  the body face for everything; **IM Fell English SC** is for headings only (page/board
  titles, port names, section headers). Sans is **reset at the top of every frame**
  (load-bearing — drop it and a heading's face bleeds into the rest of the frame). Draw a
  heading by wrapping `draw_text`/`measure_text` in `font::heading(|| …)`. Table column
  labels, row labels, item titles, tabs, buttons and hints stay sans.
- **`font_size` is a glyph-cache key; macroquad never evicts.** Every distinct size
  rasterizes a fresh glyph set and re-uploads the whole atlas: a continuously varying
  size leaks memory every frame, and even a fixed new size hitches the frame that
  first draws it. So **text is drawn through `font::draw_text` / `font::measure_text`**
  (same signatures as macroquad's; import them explicitly and they shadow the prelude,
  like `geometry::Vec2` does): they quantize every size through `font::bucket`, which
  keeps the reachable size set finite, and `font::init` pre-warms it. Never call the
  prelude's text functions directly in new code; a draw that needs raw `TextParams`
  (rotation) must run its size through `font::bucket` itself (see the berth tags in
  `ship_render.rs`).

## Sound (`sound.rs`, `build.rs`)

Clips are baked into the binary with `include_bytes!` via the `snd!` macro. **Adding a
new sound is a two-place change, and forgetting the second place breaks only the web
build:**

- **Native** reads straight from `assets/sounds/<file>`.
- **Web (wasm)** reads from `OUT_DIR/sounds-web/<file>`, where `build.rs` stages a
  re-encoded (smaller) copy. **`build.rs` only stages files listed in its `CLIPS`
  table** — so every clip referenced by `snd!` *must* also be added to `CLIPS` (with its
  channel count + bitrate), or the wasm `include_bytes!` fails to resolve and the web
  build won't compile. Native builds don't touch `OUT_DIR`, so a missing `CLIPS` entry
  is invisible until you build for web.

When you add a clip: drop the mp3 in `assets/sounds/`, add a `snd!(...)` const + field +
`load_clip` + a play method in `sound.rs`, add it to the decode test array, **and add it
to `CLIPS` in `build.rs`.**

## UI scale (`ui.rs`)

Every parchment UI (port board, captain's log, pause menu) and the sailing HUD size
themselves off **one screen-derived scale**, so they read well from a phone up to a
4K display. The gotcha that forces this: with `high_dpi: true` (`window_conf`),
`screen_width()/height()` report **physical** pixels, so an unscaled board on a 4K
panel is a tiny island of 15 px text.

- **`ui::scale()`** — `min(screen_w, screen_h) / 720` clamped to `0.85..=3.0` (720 px
  is the design baseline; 1080p ≈ 1.5, 4K ≈ 3.0).
- **`ui::px(v)`** scales a design-space pixel length; **`ui::fs_title/heading/body/
  small/chip()`** scale the type ladder (they return `u16`). These are the shared
  design tokens — **all UI sizing goes through them; no bare pixel literals** in the
  parchment UIs or HUD. Ratios (a `CAP_RATIO`-style baseline drop), alphas, and
  layout *fractions* (`w * 0.86`) stay raw — only lengths/sizes are scaled.
- The shared parchment **palette** (`ink`/`dim_ink`/`parchment`/`parchment_edge`) and
  `format_dist` also live in `ui.rs`; `port_view`, `captains_log` and `pause_menu` all
  import them rather than keeping copies.
- **Panel size caps scale too** (`panel_max_w/h`, the log/menu caps, the minimap), so
  a big screen gets a big board rather than a small one marooned mid-screen.

## Input & on-screen controls (`touch.rs`, `touch_ui.rs`)

The game is keyboard-driven; touch/mouse are an **additive layer that emits the same
verbs**, so no game logic is touch-aware. Three input contexts: sailing, the port
board, and the menus (pause menu + captain's log).

- **`touch.rs` — `TouchState`** is the pointer layer, ticked once per frame at the top
  of the loop (`touch.update(dt)`). It folds real touches **and the mouse** (one
  synthetic pointer) into `pointers`, classifies a quick stationary release as a
  **tap**, and exposes the hit-tests every control queries: `tapped_in(rect)`,
  `tap_pos_in(rect)` (where in the rect), `held_in(rect)` (press-and-hold), and
  `steering(wheel)` (a finger that goes down in the wheel is captured as a virtual
  tiller, value = horizontal offset).
- **`active()` follows the last input device.** A touch or mouse click shows the
  on-screen controls and makes the hit-tests live; **any key press hides them again**
  (`get_keys_pressed()` non-empty). `SAIL_TOUCH=1` (native env var) forces them on for
  desktop testing. A keyboard-only player never sees them. Every hit-test returns
  nothing while `!active()`.
- **`touch_ui.rs`** owns the on-screen layouts + drawing (rects derived from screen
  size, so they track rotation; geometry is shared between the hit-test and the draw):
  the **sailing HUD** (`sail_hud`/`draw_sail_hud`: a drag steering wheel, sail ▲/▼,
  dock, and a pause/log/astern stack), and the **menu nav cluster** (`nav_cluster`/
  `draw_nav_cluster`: a d-pad + ✓/✕). `main.rs` draws the HUD while sailing and the
  cluster over any open menu, only when `touch.active()`.
- **Direct tapping of boards/menus uses retained hitboxes.** `render` records each
  tappable region into a `RefCell<Vec<…>>` *as it draws it* (so geometry lives in one
  place), and the next frame's `handle_input` hit-tests them — the one-frame lag is
  invisible for a static panel. The board (`port_view.rs`, `HitEffect`) records tabs,
  rows and market chips; **a row body selects (so the chart previews the leg), only an
  action chip commits** — taps resolve to the **smallest** matching rect so a chip
  nested in its row wins. The pause menu (`pause_menu.rs`, `Tap`) records its rows and
  the volume track the same way. Both *also* honour the nav cluster as a fallback, so
  d-pad and direct tapping coexist.
- **Web**: `web/index.html` sets `touch-action: none` on the canvas (else the browser
  eats drags as scroll/zoom and the wheel won't work). Touch positions match the
  `screen_*` space under `high_dpi`. *Not yet done:* on-mobile world-seed entry needs a
  soft keyboard.

## Cargo vs. hold (`game_state.rs`)

Two distinct quantities — don't conflate them:

- **`cargo` / `cargo_used()`** — ordinary, sellable goods only (the `cargo` array
  summed). This is what the Market buys and sells.
- **`mission_hold()`** — units of reserved mission cargo in transit: contract goods
  that occupy space but **cannot be sold** until delivered (or abandoned).
- **`hold_used()`** = `cargo_used()` + `mission_hold()` — the *total* space in use.
- **`hold_free()`** = `hold_capacity` − `hold_used()`.

**Gotcha:** the ship's weight — what drives `upgrades::top_speed` (the laden top
speed handed to the sailing engine) and `upgrades::overload_penalty` (the
"Overladen" HUD badge) — is the whole laden hold, so the `load` argument must be
**`hold_used()`**, *not* `cargo_used()`. Using `cargo_used()` makes mission cargo
weightless and silently drops the overload debuff (a bug fixed in `main.rs`;
`captains_log.rs` was already correct). The race rival passes `0` (it sails an
empty hold) — that's intentional.

## Progression: hull vs. sails vs. hold (`game_state.rs` `upgrades`)

Three orthogonal shipyard fittings — keep their effects separate:

- **Hull** (`hull_level`, tiers 0–`HULL_MAX_LEVEL`=3, shown Lv 1–4) — the **only**
  fitting that raises top speed: `peak_knots` = 24/29/34/39 kn (`+KNOTS_PER_HULL_LEVEL`
  each). It also raises **max hull points** (`hull::max_hull` = 180/240/300/360);
  because wear is a *fraction* of the bigger hull, a sturdier ship costs more to keep
  mended (intended higher upkeep, not a bug).
- **Sails** (`sail_level`) — raise **only** haul tolerance (`max_haul`), i.e. how much
  the hold can carry before `overload_penalty` trims the hull's peak speed. No speed,
  no hull points.
- **Hold** (`hold_capacity`) — cargo slots only.

`top_speed(hull_level, sail_level, load)` / `top_knots(...)` take all three: speed
from the hull tier, the penalty from sails-vs-load.

**Races & hull tier:** a race's `required_level` (0 = open) is set **by leg length**
(`race::required_level_for`, `HULL_REQ_KM` rungs). The harbour refuses a captain whose
`hull_level` is below it (`TradeError::HullTierTooLow`), and the rival sails a hull of
exactly that tier (`main.rs` passes `r.required_level` to `top_speed`). The stake is
the **bare** leg wager (`race::stake_for`) — no per-tier premium, since `stake_for` is
already quadratic in distance and the higher-tier legs are the longer ones.
