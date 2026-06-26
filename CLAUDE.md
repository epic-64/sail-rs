A native **Rust + [macroquad](https://github.com/not-fl3/macroquad)** sailing game.
Run with `cargo run --release` (the dense wave mesh wants release for smooth FPS;
debug runs but is choppier). Tests: `cargo test`.

Please read PLAN.md for the current task.

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
  render functions. Restyle by editing tokens, not call sites.
- **Fonts** (`font.rs`): two embedded faces via `set_default_font` — **DejaVu Sans** is
  the body face for everything; **IM Fell English SC** is for headings only (page/board
  titles, port names, section headers). Sans is **reset at the top of every frame**
  (load-bearing — drop it and a heading's face bleeds into the rest of the frame). Draw a
  heading by wrapping `draw_text`/`measure_text` in `font::heading(|| …)`. Table column
  labels, row labels, item titles, tabs, buttons and hints stay sans.

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
