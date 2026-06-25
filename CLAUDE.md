read PLAN.md

## Cargo vs. hold (`game_state.rs`)

Two distinct quantities — don't conflate them:

- **`cargo` / `cargo_used()`** — ordinary, sellable goods only (the `cargo` array
  summed). This is what the Market buys and sells.
- **`mission_hold()`** — units of reserved mission cargo in transit: contract goods
  that occupy space but **cannot be sold** until delivered (or abandoned).
- **`hold_used()`** = `cargo_used()` + `mission_hold()` — the *total* space in use.
- **`hold_free()`** = `hold_capacity` − `hold_used()`.

**Gotcha:** the ship's weight — what drives `upgrades::speed_scale` (top-speed
penalty) and `upgrades::overload_penalty` (the "Overladen" HUD badge) — is the
whole laden hold, so it must be computed from **`hold_used()`**, *not*
`cargo_used()`. Using `cargo_used()` makes mission cargo weightless and silently
drops the overload debuff (a bug fixed in `main.rs`; `captains_log.rs` was already
correct). The race rival passes `0` (it sails an empty hold) — that's intentional.
