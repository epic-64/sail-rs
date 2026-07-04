---
name: game-screenshot
description: Take a screenshot of the running sail-rs game to visually verify a rendering or UI change on William's Windows PC. Use whenever a change needs to be seen in the real game (rig, hull, ocean, HUD, port boards), not just compiled or unit-tested.
---

# Screenshotting the sail-rs game

The game dumps its own frames; never use OS window capture (see "What does not
work" below).

## The reliable path

1. **Back up the save first.** Launching the game plays William's *live* save,
   and autosave persists everything within ~15 s. The save lives beside the
   exe: `target\release\sailrs_save.sav` (key names in `save.rs`). Copy it to
   the scratchpad before the first launch; restore it afterwards if the run
   changed anything he would care about.
2. **Check nothing is running.** `Get-Process sail-rs`: if a process exists,
   William may be playing (HUD speed changing between shots). Don't kill it in
   that case, and don't build (the linker fails on the locked exe with os
   error 5); use `cargo check` instead and wait.
3. **Launch with the screenshot hook** (native-only, bottom of the main loop
   in `main.rs`):

   ```powershell
   $env:SAIL_SHOT_DIR = "<scratchpad dir>"
   Start-Process -FilePath "target\release\sail-rs.exe" -WorkingDirectory "<repo root>"
   ```

   The game writes `<dir>\shot.png` every ~2 s. Wait a few seconds, then Read
   the PNG.
4. **Keep runs short and kill the process between looks.**
   `Get-Process sail-rs | Stop-Process -Force -Confirm:$false` as soon as the
   shot is on disk. A focused instance left alive keeps simulating: on
   2026-07-05 a few leisurely capture runs advanced an active race by ~1 km
   and wore 2% hull before anyone noticed. Check the *first* shot for an
   active race or voyage banner and be extra brisk if there is one.

## Focus semantics

- **Focused** (Start-Process usually focuses it): simulation runs, hull wears,
  ship sails on, autosave fires. This is why runs must be short.
- **Unfocused**: simulation pauses but rendering continues, dimmed, with a
  "Click here to bring game into focus" overlay across mid-screen. Fine for
  static geometry; the overlay text and dimming will be in the shot.

## Inspecting the shot

- Crop and brighten with System.Drawing + a ColorMatrix (scale RGB; ~1.2x for
  dusk, ~3x for moonless night). Brightening the whole frame 3x in daylight
  washes everything out; crop tight and stay mild when the scene is lit.
- The helm camera cuts the masthead off the top of the screen by design; to
  judge the upper rig, look at another ship instead (rival or trader through
  `rival_render`), which shows the whole rig in miniature.

## Sending input without stealing focus

`PostMessage(hwnd, WM_KEYDOWN/WM_KEYUP, vk, ...)` on
`(Get-Process sail-rs).MainWindowHandle` has worked (e.g. holding C to look
astern), but at least one unfocused attempt (H to hide the HUD) never
registered. If a key press must land, expect to briefly focus the window,
which runs the simulation: back up the save and keep it short. Avoid keys
that alter game state.

## What does not work

- `PrintWindow` (any flag): pure black. The game runs borderless fullscreen
  and DWM puts it in independent flip. Windowed mode didn't help.
- `Graphics.CopyFromScreen`: grabs whatever is frontmost (once captured a
  private browser window), and forcing the game frontmost steals William's
  focus while he is using the machine.

## Cheaper than the game

For the world-map beast art (`map_whale.rs` / `map_kraken.rs` /
`map_wave.rs`), skip the game: `cargo run --release --example whale_preview`
dumps a PNG to `$env:PREVIEW_OUT` and exits after 4 frames.
