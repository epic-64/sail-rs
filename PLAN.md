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
| `shared/Islands.scala` (`IsleFeatures`) | `src/isle_features.rs` | per-island scenery scatter |
| `client/OceanRenderer.scala` | `src/ocean_renderer.rs` | wave mesh + island depth-interleave |
| `client/IslandFloorRenderer.scala` + billboards | `src/islands_render.rs` | low-poly islands + features |
| `client/SailingView.scala` (sky/camera/loop) | `src/main.rs` | scene, input, loop (partial) |
| `client/SailingView.scala` (deck/rig/sail) | `src/ship_render.rs` | foreground deck + articulating rig |
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
- **HUD**: speed, heading/compass, wind indicator, sail trim, hull bar, gold, cargo.
- **Minimap** (`client/MinimapRenderer.scala`): local cluster radar with islands/ports.
- **Audio**: wire the copied `assets/sounds/*` — sailing music, calm/storm ambience,
  sail flap, win / game-over / transition stings (via macroquad audio).

### Gameplay systems (logic exists in `shared/`, needs porting + UI)
- **GameState** (`GameState.scala`): gold, cargo, hold capacity, location, persisted
  voyage state (separate from per-frame kinematics).
- **Ports & docking + trade** (`Goods.scala`, `Trade.scala`, `Market`, `client/PortView.scala`):
  dock within `dock_range`, buy/sell at deterministic per-port prices.
- **Upgrades** (`Upgrades.scala`): shipyard sail/cargo upgrades; laden-hull speed penalty.
- **Missions** (`Mission.scala`, `client/MissionMapView.scala`): haulage contracts,
  deposits, delivery, mission-bound cargo.
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
- Controls: **W/S** raise/lower sail (None/Half/Full) · **A/D** helm · **Q/E**
  sea-state · **G** storm · **T** daytime · **[ ]** back/veer the wind (dev aid for
  feeling the points of sail) · **Esc** quit.
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
