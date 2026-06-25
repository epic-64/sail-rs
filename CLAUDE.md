read PLAN.md

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
