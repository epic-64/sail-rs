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
| `shared/{GameState,Goods,Trade,Upgrades,Hull}.scala` | `src/game_state.rs` | voyage state + port economy (markets, trade, upgrades, repair, missions) |
| `shared/Mission.scala` (`Mission`,`Missions`) | `src/mission.rs` | haulage contracts: board generation, accept/deliver/abandon |
| `shared/Race.scala` (`Race`) | `src/race.rs`, `src/rival_render.rs` | wager races: offers, accept/win/lose/withdraw, rival helm + billboard |
| `shared/Flotsam.scala` (`Flotsam`,`FlotsamField`) | `src/flotsam.rs`, `src/flotsam_render.rs` | drifting salvage: weighted kinds, collect-near, replenish + low-poly billboards |
| `client/PortView.scala` + docking (`SailingView`) | `src/port_view.rs` | docking handshake + Market/Contracts/Shipyard overlay |
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
  `ship_motion()` (heave/pitch/roll/yaw — only heave used so far). Wavelengths
  **stretch with the sea state** (`wavelength_stretch`), so a building sea rolls in
  long ridges instead of taller chop.
- **Weather** — `weather.rs`: faithful `shared.Weather` ladder (Calm→Storm), drifting
  to an adjacent scenario every ~130 s, but **biased toward calm** (`CALM_BIAS`) so
  fair seas dominate (≈¾ of the time fair, ~⅒ in squall/storm). `WeatherState` eases
  the `sea` (wave height + deck roll) and sky `gloom` it drives, and derives the
  storm/`fury` blend from the eased gloom — so the sea builds/lays and the sky
  greys/clears smoothly across a drift. **Q/E** nudge it calmer/stormier (dev aid).
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
  draft, and luffs (a travelling flog) when starved of wind. The chosen sail notch
  (None/Half/Full) is **eased visually** (`ShipRenderer::set`/`SET_EASE`) so the
  canvas furls/unfurls smoothly instead of teleporting; the physics throttle still
  steps at once. Rebuilt as geometry, *not* the original's `deck*.png` + CSS
  `perspective()`/`rotateY` transforms.
- **Bow spray** — `spray.rs`: screen-space foam particles flung up off the stem and
  the two bow shoulders, drawn over the deck so they sit in front of the bow. A small
  xorshift RNG jitters each droplet; they launch up/outward, arc under gravity and
  fade. Emission strengthens with **speed** (the standing bow wave) and **bursts**
  on a wave slam — frontal (the bow's downward heave rate, `SLAM_REF`) or side (a
  hard, fast roll throws extra off the lee shoulder). A new feature, not in the original.
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
- **Captain's log** — `captains_log.rs`: a parchment book flipped open with **L**
  and paged with the **arrow keys** (no mouse for the original's nav arrows; the
  helm stays on A/D so the captain can hold course mid-read). Three two-page
  spreads, faithful to the original's content (the DOM/CSS 3D page-flip theatrics
  dropped): **0** "Course & Conditions" live readouts (speed, heading+compass,
  sail, wind quarter, point of sail, weather, time) beside **The Chart** (the
  parchment minimap, captioned with the local waters); **1** "The Vessel"
  (gold, hull % inked by condition, food, max speed, sail haul, overload penalty)
  beside "The Hold" (laden fraction + fill bar with the overload notch, and the
  cargo/contract manifest); **2** "Bearings" (each contract's mark, the race mark
  + VMG, and the nearest shipyard — name & distance) beside "Performance" (FPS +
  frame time). A footer shows spread dots and the page/close hints.
- **Ports, docking & trade** — `game_state.rs` + `port_view.rs`: a faithful
  `GameState` (gold, cargo by good, hold capacity, hull, location), deterministic
  per-island `Market` prices (±45% jitter, same RNG draw order), and `Trade`
  (buy/fill/dump/sell). Sail in within a port's `dock_range` with the bow pointed
  at it and the sails struck, press **Space**, and a parchment board opens over
  the live sea (the world keeps running underneath). Two tabs: **Market**
  (buy/sell the eight goods — including **planks**, ordinary cargo that doubles as
  a field repair: caulk the hull at sea for +10 from the captain's log) and
  **Shipyard/Drydock** (mend the hull, and at
  shipyard ports buy hull/sail/cargo upgrades). Keyboard-driven (arrows + Tab + Enter,
  Esc sets sail). **Top speed comes from the hull tier alone** (24/29/34/39 kn);
  sails raise only haul tolerance and an overladen hold trims the hull's peak
  (`upgrades::top_speed(hull_level, sail_level, load)` → `sailing::step_with`), so a
  full hold crawls until the sails are upgraded. Purse + hold shown on the HUD.
- **Missions** — `mission.rs` + `port_view.rs` Contracts tab: a faithful
  `shared.Mission`/`Missions` port. Each port deterministically offers 3 haulage
  contracts (same RNG draw order: seed `world.seed ^ id*GOLDEN ^ 0x5f3759df`, per
  slot `pick(good)`/`pick(target)`/`between(5,15)`) targeting other ports *in the
  same cluster*; reward scales with goods value + haul distance, deposit is value
  +10% (closes the accept-abandon-sell arbitrage). Accepting pays the deposit and
  loads mission-bound cargo (occupies hold, can't be sold); the Contracts tab also
  lists deliveries owed at this port (return deposit + reward, free the hold) and
  the hold manifest of reserved cargo bound elsewhere (Abandon → keep goods as
  sellable cargo, forfeit deposit/reward). The port chart rings target islands.
- **Races** — `race.rs` + `port_view.rs` Racing tab + `rival_render.rs` + the race
  loop in `main.rs`: a faithful `shared.Race` port. A harbour offers a deterministic
  card of up to four rival ports (always the nearest and furthest, the rest filled
  from between) with a distance-fixed stake (`goldPerKm` + a quadratic
  `bonusPerKmSq`). The Racing tab uses the **same select-by-Enter flow as the
  contracts board**: none is chosen at first, each rival port is a row (highlighting
  one previews its leg on the chart), and Enter books it — charging the stake — then
  shows the armed race. Until the off the captain may **abandon it for a full
  refund** (no consequence). On setting sail the rival draws up alongside; heave to
  (sails struck, dead slow) within range and raise sail to fire the gun — only then
  does the rival sail, on a pristine copy of the player's own rig (its sail level,
  empty hold), beating for the mark but never into the wind's eye (`rivalHelm`/
  `layHeading`). First to within `finishMargin` of the mark wins (stake back
  doubled) or loses (stake forfeit), with a win/loss sting. A standings strip shows
  the gap; the rival is drawn as a low-poly sloop billboard riding the swell, and
  the mark is ringed in **red with an "R"** on every chart (contracts ring yellow
  with an "M").
- **Flotsam salvage** — `flotsam.rs` + `flotsam_render.rs` + the salvage sweep in
  `main.rs`: a faithful `shared.Flotsam` port. Crates, barrels and the rare
  strongbox drift on the swell (weighted `FlotsamKind::pick`, same draw order),
  topped up to `target` pieces in a spawn ring biased *ahead* of the bow (cubic
  bearing bias) and never on a shore; stale salvage astern of `cullDist` is
  forgotten. The captain scoops a piece by sailing within `REACH` of it
  (`collect_near`): its gold lands in the purse with a coin chime and a fading
  "+gold" toast naming the find. Each piece is a flat-shaded billboard (planked
  crate / hooped cask / brass-bound chest) riding the local wave and depth-sorted
  into the wave march like the rival, so nearer crests and islands occlude it.
- **Trader NPCs** — `trader.rs` + the trader march in `ocean_renderer.rs`: a small
  fleet of merchant craft (3 per cluster, only the *current* cluster simulated) that
  ply fixed circuits of 3–4 nearby ports, re-spawned when the captain crosses to new
  waters. Each helms itself with the rival's tacking logic (`race::rival_helm` —
  beating up to a port that lies upwind rather than stalling in irons) and grazes off
  shores like the player; on reaching a port it lies to for 60–90 s before the next
  leg. Drawn as the rival sloop billboard but flying a green pennant (vs the race
  rival's red), depth-sorted into the wave march, and dotted on the corner chart. A
  new feature, not in the original.
- **Saves** — `save.rs`: the voyage persists across sessions, on the desktop *and*
  in the browser (itch.io localStorage). A compact versioned `key=value` text holds
  only voyage state — the seed (the chart is regenerated from it), the `GameState`
  (gold/cargo/hull/missions/booked race/location), the ship's `Kinematics`, and the
  clock/sail-notch/wind quarter; transient scenery (traders, flotsam, weather) is
  reseeded fresh on load. The game autosaves every 15 s and on quit, and **continues
  the saved voyage automatically on launch** (its seed picks the chart, restoring a
  docked board or a mid-leg race rival). Storage backend: native writes a `.sav`
  beside the exe; the web build calls three `localStorage` shims imported from the JS
  loader (a `miniquad_add_plugin` in `web/index.html`) — no serde, no extra crates.
- **Assets** — all `img/*` and `sounds/*` copied into `assets/`.

## 🟡 Partial / diverged from original (intentional)

- **Daytime** — cycled manually with **T**, not on an automatic dawn→night clock.
- **HUD** — a single debug text line (state + controls), not the real HUD.
- **Islands look** — deliberately **not** the SVG billboards; rebuilt as low-poly
  geometry (user is not attached to the old look).

---

## ⬜ To do

Roughly in suggested build order; each is a milestone.

### Rendering / feel
- **Automatic daytime cycle** (`shared/Daytime.scala`): advance dawn→day→dusk→night.
- **HUD**: ✅ full stats live in the **captain's log** (`captains_log.rs`) — course,
  vessel (gold/hull/food/rig), hold manifest, bearings, performance — plus the
  corner chart and the gold/hold line on the debug HUD.
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
- **Upgrades** (`Upgrades.scala`): ✅ done — three orthogonal shipyard fittings:
  **hull** (top speed 24/29/34/39 kn + max hull points), **sails** (haul tolerance),
  **hold** (cargo slots); laden-hull speed penalty via `upgrades::top_speed` →
  `sailing::step_with`. Diverged from the original's sail-driven speed (see CLAUDE.md).
- **Drydock / hull repair** (`Hull.scala`): 🟡 repair UI works at every port, but no
  damage source yet — wire storm/starvation wear so the hull actually wants mending.
- **Races** (`Race.scala`): ✅ done — `race.rs` + `port_view.rs` Racing tab +
  `rival_render.rs` + the race loop in `main.rs`. Wager races vs a computer rival
  (rebuilt as a low-poly sloop billboard, not the `ship-*.svg` sprites) + a
  standings banner. Longer legs demand a higher **hull tier** to enter (and field a
  rival of that tier); the stake is the bare quadratic leg wager — a port-side
  extension, see CLAUDE.md.
- **Hull & rations** (`Hull.scala`): hull integrity worn by storms/starvation, slow
  when battered; food eaten per daytime.
- **Flotsam salvage** (`Flotsam.scala`): ✅ done — `flotsam.rs` + `flotsam_render.rs`.
  Drifting crates/barrels/chests scooped by sailing over them, rebuilt as low-poly
  billboards (not the `barrel/crate/chest.svg` sprites) + a coin chime and pickup toast.

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
  dock at a port in range (sails struck) · **C** (hold) look astern — spins the view
  180° over the wake, helm unchanged · **Q/E** nudge the weather calmer/stormier
  (it auto-drifts) · **T/Y** daytime forward/back · **[ ]** back/veer the wind (dev aid for feeling
  the points of sail) ·
  **L** open/close the captain's log (**←/→** turn its pages while open; on the
  Vessel spread **↑/↓** select and **Enter** presses the *Caulk hull* button — a +10
  field repair that spends a plank from the hold) · **Esc** close the log / quit. In
  port: arrows
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
- **Port overlay style** (`port_view.rs` `mod style`): every type size, spacing step,
  column split and symbol the port board draws is a named token in one `style` module —
  there are no bare pixel literals in the render functions (sizes derive from a tight
  `FS_*` ladder and a `UNIT`-based spacing grid; `line_h`/`row_h`/`tab_h` derive the
  rhythm; column positions are `*_X`/`*_R` width fractions). Transitions/routes use the
  `ARROW` token (`→`); inline separators are `·`. Reduce or restyle the whole board by
  editing the tokens, not the call sites.
- **Fonts** (`font.rs`): two embedded faces (`assets/fonts/`, `include_bytes!`)
  replace macroquad's default ProggyClean (no symbols, blurry when scaled): **DejaVu
  Sans** is the body face for *everything*, and **IM Fell English SC** (a 1600s
  press small-caps face) is used **only for headings** — page/board titles, port
  names, and section headers. Swapped via macroquad's global `set_default_font`, so
  the ~180 `draw_text`/`measure_text(.., None, ..)` call sites need no font argument.
  Sans is the standing default, **reset at the top of every frame** (load-bearing —
  drop it and a heading's face would bleed into the rest of the frame). A heading is
  drawn by wrapping its `draw_text`/`measure_text` in `font::heading(|| …)`, which
  flips to the display face and restores sans after. Headings ≠ table column labels,
  row labels, item titles, tabs, buttons or hints — those stay sans.
