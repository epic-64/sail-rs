# sail-rs — Porting Plan

Porting the sailing game **`C:\Users\William\Downloads\sail-main\sail-main`** (Scala 3
cross-project: ScalaJS + Laminar client, Cask JVM server, shared game logic) to a
native **Rust + [macroquad](https://github.com/not-fl3/macroquad)** application.

**Why:** the original is an HTML/CSS/Canvas + ScalaJS app whose rendering
performance is poor. Rewriting in Rust/macroquad targets a smooth native renderer
while keeping the game's mechanics and feel.

**Guiding principles**
- Preserve **mechanics and feel**; the *look* may be improved (we are free to drop
  the SVG billboards, DOM/CSS rendering, etc.).
- Keep deterministic generation **bit-identical** where it drives the world, so a
  seed produces the same chart (RNG draw order is sacred — see `rng.rs`/`world.rs`).
- Match the native budget: denser meshes and per-frame work the HTML version
  couldn't afford.

---

## Architecture map (source → port)

| Original (Scala) | Port (Rust) | Notes |
|---|---|---|
| `shared/Geometry.scala` (`Vec2`) | `src/geometry.rs` | our `Vec2` (0=N, CW), `clamp`, `wrap_angle` |
| `shared/Rng.scala` (SplitMix64) | `src/rng.rs` | identical draw sequence |
| `shared/Ocean.scala` | `src/ocean.rs` | wave height field + `ship_motion` |
| `shared/Sailing.scala` (`Kinematics`,`Ship`,`Projection`) | `src/sailing.rs`, `src/projection.rs` | physics + camera constants |
| `shared/Daytime.scala` + sea/sky palettes | `src/palette.rs` | time-of-day colour |
| `shared/World.scala` (`WorldGen`) | `src/world.rs` | clusters/islands generation |
| `shared/{GameState,Goods,Trade,Upgrades,Hull}.scala` | `src/game_state.rs` | voyage state + port economy (markets, trade, upgrades, repair) |
| `client/PortView.scala` + docking (`SailingView`) | `src/port_view.rs` | docking handshake + Market/Shipyard overlay |
| `shared/Islands.scala` (`IsleFeatures`) | `src/isle_features.rs` | per-island scenery scatter |
| `client/OceanRenderer.scala` | `src/ocean_renderer.rs` | wave mesh + island depth-interleave |
| `client/IslandFloorRenderer.scala` + billboards | `src/islands_render.rs` | low-poly islands + features |
| `client/SailingView.scala` (sky/camera/loop) | `src/main.rs` | scene, input, loop (partial) |
| `client/SailingView.scala` (deck/rig/sail) | `src/ship_render.rs` | foreground deck + articulating rig |
| `client/MinimapRenderer.scala` | `src/minimap.rs` | local-cluster chart (HUD + parchment palettes) |
| `client/SailingView.scala` (logbook spread) | `src/captains_log.rs` | parchment stats panel + chart |
| `jvm/.../static/{img,sounds}` | `assets/` | copied; SVGs not yet used |
| `jvm/server/WebServer.scala` (Cask) | — | N/A for native, skip |

---

## Status legend
✅ done  ·  🟡 partial / diverged  ·  ⬜ not started

---

## ✅ Implemented

- **Math & RNG** — `geometry.rs`, `rng.rs`.
- **Ocean wave field** — `ocean.rs`: travelling-sine swells, `height()`,
  `ship_motion()` (heave/pitch/roll/yaw — only heave used so far).
- **Wave renderer** — `ocean_renderer.rs`: world-anchored faceted mesh, Blinn-Phong
  glitter, Fresnel sky, subsurface glow, whitecap foam, glassy-crest alpha, foam-fleck
  optical-flow streaks. Tuned for native: far-biased row distribution (chunky near,
  fine to the horizon), mesh reaches the true horizon, MSAA ×4.
- **Scene** — `main.rs`: three-stop sky gradient (per daytime, eases to storm), sun
  disc, far-water backdrop, horizon.
- **Camera + ship movement** — `sailing.rs`: `Kinematics`, `Helm`, `step` (drag,
  keel side-slip, rudder authority, yaw inertia).
- **Wind & sailing** — `sailing.rs`: faithful `Wind` (`factor` drive curve with the
  30° no-go zone, `floorDrive` jump, beam-reach peak, run ease-off; `random`/
  `favorable`; `PointOfSail` for the HUD). Sails are discrete notches (None/Half/
  Full) set with **W/S** — set once and the ship keeps going; **A/D** is the helm.
  The wind blows toward a bearing, opens *favorable* and backs/veers to a random
  quarter every 300 s, so making ground upwind forces tacking. The sail's render
  belly/luff now reads off the same `Wind::factor` curve (`wind_factor_rel`).
- **World generation** — `world.rs`: faithful `WorldGen` (5×5 clusters 42 km apart,
  5×5 jittered isles each, names/terrain/ports/shipyards). Positions match the
  original chart for a given seed.
- **Islands** — `islands_render.rs`: low-poly faceted landmass + floor disc, flat-shaded
  to match the waves. Terrain-dependent height (green/jungle flat, rocky/volcanic
  hills/cones), correct per-band wave occlusion via depth-interleave in the wave loop.
- **Island features** — `isle_features.rs` + `islands_render.rs`: deterministic scatter of
  trees, palms, bushes, rocks, ruins, and port structures (huts, watchtower, dock,
  flags) + shipwrecks, drawn as flat-shaded billboards standing on the mound surface.
- **Collision / grounding** — `sailing::resolve_grounding` (graze-and-slide off shores).
- **Ship foreground** — `ship_render.rs`: flat-shaded low-poly deck (planked floor,
  bulwarks, spinning wheel) + mast/yard and a square sail built from overlapping
  cloth panels. The whole assembly sways as a rigid body with the swell (heave/
  pitch/roll/yaw, deck-share split, roll/yaw low-passed), and the **rig
  articulates**: the yard braces about the mast, the sail bellies into a parabolic
  draft, and luffs (a travelling flog) when starved of wind. Rebuilt as geometry,
  *not* the original's `deck*.png` + CSS `perspective()`/`rotateY` transforms.
- **Camera ride + wind heel** — `main.rs`: the whole world (sky, sun, waves,
  islands) is drawn through a `Camera2D` that *rides* the swell — it tilts the
  horizon opposite the hull's lean (the sea stays level while the captain heels),
  drops/rises as the bow pitches, and swings as it yaws — while the deck leans
  *with* the lean, so the two read as one heeling ship. The lean is swell-roll plus
  a **wind heel**: the sails' press leans her away from the wind, hardest on a beam
  reach and nil before the wind or in irons (`HEEL_GAIN`, eased). **Pitch is a real
  fore-aft nod**, not a bob: the deck plane tilts about mid-deck and the rig rocks
  about its foot (masthead aft on bow-up), while the horizon travels to match.
  Tunables (`CAM_*`, `HEEL_GAIN`) sit by the loop; world-anchored fills are
  over-scanned so a rolled/pitched view never reveals background in the corners.
  (`Camera2D::from_display_rect` flips Y to the screen — `zoom.y` is flipped back.)
- **Minimap** — `minimap.rs`: faithful `MinimapRenderer` port. Frames the local
  cluster (the ship's current waters) north-up so its isles nearly fill a square
  chart: land dots (ports brighter), shipyard/contract rings, wind streaks with
  flow chevrons, and the ship a heading arrow clamped to the frame in open sea.
  Two ink schemes (`hud` glass / `parchment`). Drawn always-on top-right and inside
  the log. Wind streaks are Liang–Barsky-clipped to the frame (macroquad has no
  canvas clip).
- **Captain's log** — `captains_log.rs`: a parchment panel flipped open with **L**.
  A "Course & Conditions" page of live readouts (speed, heading+compass, sail,
  wind quarter, point of sail, weather, time) beside the chart spread (the
  parchment minimap) captioned with the local waters' name. The vessel/hold/
  bearings pages from the original wait on the GameState/trade/mission ports.
- **Ports, docking & trade** — `game_state.rs` + `port_view.rs`: a faithful
  `GameState` (gold, cargo by good, hold capacity, hull, location), deterministic
  per-island `Market` prices (±45% jitter, same RNG draw order), and `Trade`
  (buy/fill/dump/sell). Sail in within a port's `dock_range` with the bow pointed
  at it and the sails struck, press **Space**, and a parchment board opens over
  the live sea (the world keeps running underneath). Two tabs: **Market**
  (buy/sell the seven goods) and **Shipyard/Drydock** (mend the hull, and at
  shipyard ports buy sail/cargo upgrades). Keyboard-driven (arrows + Tab + Enter,
  Esc sets sail). The rig's **top speed now scales** with sail upgrades and the
  weight in the hold (`upgrades::speed_scale` → `sailing::step_scaled`), so a
  full hold crawls until the sails are upgraded. Purse + hold shown on the HUD.
- **Assets** — all `img/*` and `sounds/*` copied into `assets/`.

## 🟡 Partial / diverged from original (intentional)

- **Weather / sea-state** — `sea` and `storm` are **debug toggles** (Q/E, G), not the
  `Weather` calm→storm scenario system that drifts between adjacent states.
- **Daytime** — cycled manually with **T**, not on an automatic dawn→night clock.
- **HUD** — a single debug text line (state + controls), not the real HUD.
- **Islands look** — deliberately **not** the SVG billboards; rebuilt as low-poly
  geometry (user is not attached to the old look).

---

## ⬜ To do

Roughly in suggested build order; each is a milestone.

### Rendering / feel
- **Weather system** (`shared/Weather.scala`): auto-drift calm→storm along adjacent
  states, driving `sea`/`gloom`; wire into the existing storm-blend palette.
- **Automatic daytime cycle** (`shared/Daytime.scala`): advance dawn→day→dusk→night.
- **HUD**: ✅ live stats live in the **captain's log** (`captains_log.rs`) + the debug
  line. Still wanting hull bar, gold, cargo — those need the GameState port.
- **Minimap** (`client/MinimapRenderer.scala`): ✅ done — `minimap.rs`, always-on
  corner chart + the log's parchment chart.
- **Audio**: wire the copied `assets/sounds/*` — sailing music, calm/storm ambience,
  sail flap, win / game-over / transition stings (via macroquad audio).

### Gameplay systems (logic exists in `shared/`, needs porting + UI)
- **GameState** (`GameState.scala`): ✅ done — `game_state.rs` (gold, cargo, hold,
  hull, location). Persisted voyage state, separate from per-frame kinematics.
- **Ports & docking + trade** (`Goods.scala`, `Trade.scala`, `Market`, `client/PortView.scala`):
  ✅ done — `game_state.rs` + `port_view.rs`. Dock within `dock_range`; buy/sell at
  deterministic per-port prices.
- **Upgrades** (`Upgrades.scala`): ✅ done — shipyard sail/cargo upgrades wired into
  top speed; laden-hull speed penalty via `step_scaled`.
- **Drydock / hull repair** (`Hull.scala`): 🟡 repair UI works at every port, but no
  damage source yet — wire storm/starvation wear so the hull actually wants mending.
- **Missions** (`Mission.scala`, `client/MissionMapView.scala`): haulage contracts,
  deposits, delivery, mission-bound cargo. Port board has no Contracts tab yet.
- **Races** (`Race.scala`): wager races vs a computer rival; rival ship rendering
  (`ship-bow/stern.svg`) + banners.
- **Hull & rations** (`Hull.scala`): hull integrity worn by storms/starvation, slow
  when battered; food eaten per daytime.
- **Flotsam salvage** (`Flotsam.scala`): collectible crates/barrels/chests drifting on
  the swell, scooped by sailing over them (`barrel/crate/chest.svg`).

### Screens / flow
- **Title / splash** (`client/TitleView.scala`, `SplashRenderer.scala`).
- **Large map view** (`client/LargeMapView.scala`): zoomed-out world chart of clusters.
- **Game-over / win** flow.

### Explicitly out of scope (web-only)
- Cask web server, PWA manifest/service worker, Vite minify, itch build scripts.

---

## Build & run

- Run: `cargo run --release` (the dense wave mesh wants release for smooth FPS;
  debug runs but is choppier).
- Controls: **W/S** raise/lower sail (None/Half/Full) · **A/D** helm · **Space**
  dock at a port in range (sails struck) · **Q/E** sea-state · **G** storm · **T**
  daytime · **[ ]** back/veer the wind (dev aid for feeling the points of sail) ·
  **L** open/close the captain's log · **Esc** close the log / quit. In port: arrows
  move the cursor, **Tab** switches board, **Enter** trades, **Esc** sets sail.
- Tuning knobs live in `OceanRenderer::new` (mesh density, `row_bias`, `f_far`,
  `depth_far`) and `world.rs` (island radius/height by terrain).

## Conventions / gotchas
- Our `geometry::Vec2` shadows glam's `Vec2` (explicit `use` wins over the macroquad
  prelude glob); world maths uses ours.
- **Do not reorder RNG draws** in `world.rs` / `isle_features.rs` — it shifts every
  island. Tune ranges/values, not the sequence.
- `world.islands` is in id order (index == id); `features` is generated once and
  aligned by that index.
- macroquad 2D drawing has no depth buffer; island/feature occlusion is solved by
  drawing each island **interleaved between wave bands by distance** in
  `OceanRenderer::render`.
