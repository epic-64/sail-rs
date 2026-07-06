---
name: game-screenshot
description: Take a screenshot of the running sail-rs game to visually verify a rendering or UI change on William's Windows PC. Use whenever a change needs to be seen in the real game (rig, hull, ocean, HUD, port boards), not just compiled or unit-tested.
---

# Screenshotting the sail-rs game

The game dumps its own frames; never use OS window capture (see "What does not
work" below). Prefer a staged scene over William's live save.

## The preferred path: a staged scene (`SAIL_SCENE`)

With `SAIL_SCENE` set (native-only, parsed by `scene_spec` in `main.rs`), the
game starts a synthetic voyage instead of loading the save, and the storage
backend refuses all writes, so the real voyage cannot be disturbed. Combine
with `SAIL_SHOT_DIR` (the frame dump) and optionally `SAIL_WINDOWED=1`:

```powershell
$env:SAIL_SHOT_DIR = "<scratchpad dir>"
$env:SAIL_SCENE = "hull=3,cargo=14,tod=0.5,heading=180,wind=180,rival=170,rival_hull=3,hud=0"
Start-Process -FilePath "target\release\sail-rs.exe" -WorkingDirectory "<repo root>"
# wait a few seconds, Read <dir>\shot.png (written every ~2 s), then:
Get-Process sail-rs | Stop-Process -Force -Confirm:$false
```

Comma-separated `key=value` pairs, all optional (`SAIL_SCENE=1` alone is a
fresh start under full sail); the authoritative key list is the `scene_spec`
doc comment in `main.rs`. The useful ones: `hull` / `cargo` (ship tier, hold
units), `sail` (canvas notch, default full; `sail=0` parks the ship so the
framing holds still), `tod` (0.25 sunrise, 0.5 noon, 0.75 sunset), `heading` /
`wind` (bearings in degrees, 0 = north), `weather` (calm...storm by name),
`rival=M` + `rival_hull=N` (a rival ship M metres ahead, beam-on, of tier N),
`refit=N` (loop the shipyard rebuild animation between `hull` and tier N every
few seconds, so shots keep catching it mid-swap), `hud=0` (clean frame), `seed`.

Staging tips, learned the slow way:

- The scene starts the fresh-voyage position: bow at the home port isle.
  `heading=180` faces open water; the default heading keeps the isle in frame.
- Under full sail the ship advances (~13 m/s), so a rival dead ahead closes
  fast and the player's own canvas screens it. To inspect a rival's rig, use
  `sail=0` (stationary, bare yards in the foreground) and grab an early shot.
- A ship's yard braces to *her* wind: a rival with the wind astern shows her
  cloth edge-on to a viewer abeam. Pick `wind` for the ship you want to read
  (wind on her beam swings her yards toward a viewer she is beam-on to).
- Docking needs a Space press, so a furled scene inside dock range stays at
  sea; ignore the "Furl sail (S) to enter" toast.
- The focus call-to-action veil is auto-dismissed in scene mode, and while the
  window is unfocused the simulation pauses but rendering continues: an
  unfocused staged scene is a perfect freeze-frame.

## The fallback: the live save

Only when the thing to verify depends on William's actual voyage (his cargo,
an active race, a particular port). Then the session plays his real save:

1. **Back it up first**: `target\release\sailrs_save.sav` (key names in
   `save.rs`), copied to the scratchpad. Autosave persists within ~15 s of
   play; there is no undo once it fires.
2. **Check nothing is running** (`Get-Process sail-rs`). A live process may be
   William playing (HUD values changing between shots): don't kill it, don't
   `cargo build` (the linker fails on the locked exe with os error 5); use
   `cargo check` and wait.
3. While the window is focused the simulation runs: the ship sails on, hull
   wears, races advance. On 2026-07-05 a few leisurely capture runs advanced
   an active race by ~1 km. Keep runs to a few seconds and kill the process
   between looks.

## Inspecting the shot

- Crop and brighten with System.Drawing + a ColorMatrix (scale RGB; ~1.2x for
  dusk, ~3x for moonless night). Daylight needs no brightening; crop tight
  and zoom (NearestNeighbor) instead.
- The helm camera cuts the masthead off the top of the screen by design; to
  judge the upper rig, stage a rival (`rival=...`) and read the whole rig in
  miniature.

## Sending input without stealing focus

`PostMessage(hwnd, WM_KEYDOWN/WM_KEYUP, vk, ...)` on
`(Get-Process sail-rs).MainWindowHandle` has worked (e.g. holding C to look
astern), but at least one unfocused attempt (H to hide the HUD) never
registered; prefer staging via `SAIL_SCENE` keys over keypresses. If a key
must land, expect to briefly focus the window (which runs the simulation:
back up the save, keep it short) and avoid keys that alter game state.

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

For island scenery models (`feature_models.rs`), skip the sailing too:
`cargo run --release --example feature_preview` stands a lineup of
`FeatureKind` models on flat ground under the game's own projection and
lighting, dumps a PNG to `$env:PREVIEW_OUT`, and exits after 4 frames. Edit
`lineup` in the example to choose the kinds (repeat a kind to see another
hashed yaw). Finding a staged isle that shows a specific feature is luck;
the preview shows exactly the models you ask for.
