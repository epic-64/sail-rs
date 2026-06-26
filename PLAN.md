# Plan: Mobile (touch) controls for sail-rs

> **Status (in progress).** Core controls landed and compiling (native + wasm):
> `src/touch.rs` (pointer/tap/drag/hit-test layer, mouse-driven for desktop
> testing, dormant until a real touch or `SAIL_TOUCH=1`), `src/touch_ui.rs`
> (on-screen sailing HUD + menu nav cluster), wired into `main.rs`,
> `port_view.rs`, `pause_menu.rs`, and the log paging. Web shell gets
> `touch-action: none`.
>
> **Tap-to-activate on the port board: DONE.** `render` now records each
> tappable region (tab / row / market chip) into a `RefCell<Vec<HitRect>>` as it
> draws — geometry stays where it's drawn, no duplicated layout — and
> `handle_input` hit-tests them next frame, setting focus (+column) and calling
> the same `activate()` the keyboard uses. One tap on a chip or job row commits.
> The board shows only a ✕ cast-off button; the pause menu and captain's log keep
> the d-pad nav cluster.
>
> **UI scale pass: DONE.** `ui::scale()` derives a factor from the screen's short
> edge (720 px baseline, clamped 0.85–3.0), with `ui::px(v)` and `ui::fs_*()` the
> shared design tokens. Every parchment UI now scales through them — so the board,
> captain's log, pause menu and the sailing HUD read properly from a phone up to a
> 4K display (`high_dpi` reports physical pixels, so an unscaled 4K board was a tiny
> island of 15 px text). Specifically:
>   - `port_view::style` tokens became scaled fns (`pad()`, `chip_h()`, `panel_max_w()`…);
>   - the captain's log & pause menu (which predated the no-bare-literals rule) had
>     every size/offset routed through `px()`/`fs_*()`, and the pause menu now pulls
>     the shared palette from `ui` instead of its own copies;
>   - the HUD (purse line, debuff badges, salvage/race toasts, minimap cap) scales too;
>   - panel size caps (`panel_max_w/h`, the log/menu caps, the minimap) scale, so big
>     screens get a big board rather than a small one marooned in the middle.
>
> **Still deferred:** seed entry on mobile needs a soft keyboard (noted in §9).


**Target:** the existing WASM / itch.io build, played in a mobile browser
(landscape phones & tablets). Native mobile is out of scope for now, but the
touch layer is written platform-agnostically so a future `cargo-apk` build gets
it for free.

**Steering feel:** an analog drag wheel / thumb-pad (lower-left), rudder
proportional to horizontal drag, recentres on release.

**Guiding principle:** *additive, not a rewrite.* Touch emits the **same verbs**
the keyboard already emits. All game logic (`sailing.rs`, `game_state.rs`,
`port_view.rs::activate`, etc.) is untouched. Keyboard play on desktop is
byte-for-byte unchanged — the touch HUD only appears once a touch is seen.

---

## 0. Constraints learned from the codebase

- Input today is 100% keyboard; **no** `mouse_position`, `is_mouse_button`, or
  `touches()` anywhere. So there is no input abstraction to hook into yet — we
  add one.
- macroquad exposes `touches() -> Vec<Touch>` (`Touch { id, phase, position }`,
  phase ∈ Started/Stationary/Moved/Ended/Cancelled). It works on the WASM canvas
  and native mobile. **No portable tilt/haptics API** → not used.
- Layout already reads `screen_width()/screen_height()` each frame and scales the
  big panels, but **type sizes & paddings are fixed px** (`ui.rs` tokens,
  `port_view.rs` `mod style`). Phones need a UI scale factor — this is the only
  non-mechanical part.
- The port overlay's rule "no bare pixel literals, everything is a named token"
  means scaling has a single choke point. Good.
- Determinism rule does **not** apply here — no RNG draws are involved.

---

## 1. New module: `src/touch.rs` — the input layer

A single source of truth for touch state, ticked once per frame.

```text
pub struct TouchState {
    active: bool,            // has any touch ever happened? gates the HUD
    pointers: Vec<Pointer>,  // live touches this frame (id, start, pos, phase)
    taps: Vec<Vec2>,         // positions of touches that ENDED this frame w/o drag
    ...
}
```

Responsibilities:

- `update()` — pull `touches()`, diff against last frame, classify each pointer
  as **tap** (down→up, small travel, short time) vs **drag** (moved past a
  threshold), maintain a per-id start position.
- Hit-testing helpers used by every screen:
  - `tapped_in(rect) -> bool` — a tap landed in this rect this frame.
  - `held_in(rect) -> Option<Pointer>` — a finger is currently down in this rect
    (for the wheel and look-astern hold).
  - `drag_x_norm(pointer, rect) -> f32` — horizontal drag within `[-1, 1]` for
    the steering wheel.
- `is_touch_active() -> bool` — true once any touch is seen; flips the HUD on.
  Set back toward keyboard if a key is pressed (track "last input device") so a
  desktop with a touchscreen still behaves like desktop until tapped.

Time base: use the existing per-frame `dt` for tap-duration thresholds (the
codebase already forbids `Date::now`-style calls in deterministic paths; we just
accumulate `dt`).

Note: macroquad reports touch positions in physical pixels with `high_dpi`;
convert to the same logical space as `screen_width()/height()` so rects match.
Verify against drawn rects during implementation.

---

## 2. Input abstraction (thin shims, keyboard OR touch)

Introduce small helper fns so call sites stop reading `KeyCode` directly. Each
returns keyboard result `||` touch result.

In `main.rs` (sailing context), replace direct key reads with:

- `helm_turn(touch, log_open) -> f32`
  - keyboard: existing `read_turn` (±1)
  - touch: analog wheel value in `[-1, 1]` from `drag_x_norm`
- `sail_up_pressed(touch)` / `sail_down_pressed(touch)` — key edge `||` wheel-
  button tap.
- `dock_pressed(touch)` — `Space` `||` anchor-button tap.
- `look_astern_held(touch)` — `is_key_down(C)` `||` `held_in(eye_btn)`.
- `log_toggle_pressed`, `pause_pressed` — key `||` corner-button tap.

For menus, give `PortScreen`, `PauseMenu`, and the log paging a `&TouchState`
argument and add touch branches (section 4).

This keeps `read_turn` etc. intact; we wrap them.

---

## 3. Sailing HUD — on-screen helm (`src/touch_hud.rs` or fold into `touch.rs`)

Drawn at end of the sailing-frame render, **only when `is_touch_active()`**.
All rects are computed from `screen_width()/height()` so they track rotation.
All controls are translucent so they don't fight the seascape.

Layout (landscape):

```
[⚓ dock]                                   [☰ pause]
                                            [📖 log]
                                            [👁 astern]
 ╭──────╮                              ▲  sail up
 │ wheel │  (drag L/R)                 ▼  sail down
 ╰──────╯
```

- **Steering wheel** (lower-left): draw a ring + hub. On `held_in(wheel_rect)`,
  read `drag_x_norm` → `turn`. Render the hub offset/rotated by the drag for
  feedback. Release → `turn` eases to 0 (matches keyed release).
- **Sail ▲ / ▼** (lower-right): tap steps the notch via existing
  `sail_mode` logic (keep the `sail_up()/sail_down()` sounds and the "only step
  on press" semantics — a tap is one step).
- **Anchor / dock** (top-left): rendered **enabled only** when
  `harbor.update_dockable` reports a dock in range; tap → `try_dock`.
- **Log 📖** (right): tap toggles the captain's log.
- **Astern 👁** (right): press-and-hold maps to the look-back view flip.
- **Pause ☰** (top-right): tap opens the pause menu.

Icons: draw with primitives / existing font glyphs to avoid new asset loading on
the web (keep the WebGL1, no-extra-texture profile from `window_conf`).

---

## 4. Menus — make existing buttons tappable (no logic rewrite)

The board/log/pause already compute element rects at draw time. Strategy:
**have the render pass also record focusable rects** into a small frame-local
list (e.g. `Vec<(FocusId, Rect)>`), then in `handle_input` add a touch branch:
tap inside a rect ⇒ set `focus`/`tab`/`column` to that element **and** call the
same `activate()` the keyboard's Enter calls.

- **Port board** (`port_view.rs`):
  - Tap a tab → switch tab.
  - Tap a Market row's **Buy / Fill / Dump / Sell** cell → set `column` + row and
    activate directly (touch skips the left/right column-cursor dance).
  - Tap Repair/Upgrade/Contract/Delivery/Race buttons → activate.
  - Tap a "Set sail" affordance (or the existing pause/back) → close board.
  - Existing red constraint-flash feedback is reused unchanged.
- **Captain's log** (`captains_log.rs` + `main.rs` paging): swipe left/right (or
  tap on/off-edge nav zones) to page; tap a spread button (e.g. caulk) to press
  it; tap outside / a close affordance to shut it.
- **Pause menu** (`pause_menu.rs`): tap a row to select+activate; tap the volume
  slider track (or −/＋ tap zones) to nudge; Back/Resume row tappable. World-seed
  field: provide on-screen ＋/− digit buttons or trigger the browser soft
  keyboard (decide during impl — soft keyboard is simplest if reachable).

Add a `&TouchState` param to each `handle_input`; keyboard paths stay first and
unchanged.

---

## 5. Readability / UI scale (the one substantive change)

Add a global UI scale derived from screen size & DPI so text is legible on a
phone:

- Compute `ui_scale = (min(w, h) / REF_SHORT_EDGE)` clamped to a sane band
  (e.g. 0.85–1.6), where `REF_SHORT_EDGE ≈ 720`.
- Route the `ui.rs` type tokens (`FS_TITLE`, `FS_HEADING`, `FS_BODY`,
  `FS_SMALL`, `FS_CHIP`, `line_h`) and the `port_view.rs` `mod style` spacing
  tokens through `ui_scale`. Because these are already named tokens with no bare
  literals at call sites, this is a localized change.
- Enforce a **minimum touch-target size** (~44 logical px) for HUD/menu hit
  rects independent of glyph size.

On desktop `ui_scale ≈ 1.0`, so nothing visibly changes there.

---

## 6. Web/build specifics

- macroquad's `touches()` already works on the WASM canvas — no JS shim needed
  for input.
- Ensure the canvas fills the viewport and handles orientation/resizing: confirm
  `index.html` / the itch wrapper sets viewport meta
  (`width=device-width, initial-scale=1, user-scalable=no`) and a full-bleed
  canvas; add CSS `touch-action: none` so the browser doesn't hijack drags as
  scroll/zoom. (Check the existing web shell; patch if missing.)
- Keep the WebGL1 / no-MSAA / no-extra-texture profile from `window_conf` for
  broad mobile-GPU compatibility.
- `fullscreen: true` + `high_dpi: true` stay as-is.

---

## 7. File-by-file change list

| File | Change |
|---|---|
| `src/touch.rs` *(new)* | `TouchState`, per-frame `update()`, tap/drag classification, hit-test helpers, `is_touch_active`. |
| `src/touch_hud.rs` *(new, or fold into touch.rs)* | Draw + hit-test the sailing HUD (wheel, sail ▲▼, dock, log, astern, pause). |
| `src/main.rs` | Construct/`update()` `TouchState` each frame; swap direct key reads in the sailing block for the shims (`helm_turn`, `sail_*_pressed`, `dock_pressed`, `look_astern_held`, `log_toggle_pressed`, `pause_pressed`); draw the HUD when touch-active; pass `&TouchState` into menu `handle_input`s. |
| `src/port_view.rs` | Record focusable rects during `render`; add touch branch in `handle_input` (tap-to-activate, direct Buy/Sell cells). |
| `src/captains_log.rs` + log paging in `main.rs` | Swipe-to-page + tap buttons. |
| `src/pause_menu.rs` | Tap rows / slider / seed entry. |
| `src/ui.rs` | `ui_scale()` + scaled type tokens / `line_h`. |
| `src/port_view.rs` `mod style` | Route spacing tokens through `ui_scale`. |
| web shell (`index.html` / itch wrapper) | viewport meta + `touch-action: none` + full-bleed canvas, if not already present. |

---

## 8. Suggested build order

1. `touch.rs` skeleton + `is_touch_active` + tap/drag/hit-test, log to screen to
   verify coordinate space matches drawn rects.
2. Sailing HUD: steering wheel first (analog `turn`), then sail ▲▼, then dock /
   log / astern / pause. Playable end-to-end on a phone after this.
3. `ui_scale` readability pass.
4. Port board touch (highest menu value).
5. Pause menu + captain's log touch.
6. Web shell viewport/`touch-action` polish + on-device test (Chrome DevTools
   device emulation, then a real phone via the itch/WASM build).

## 9. Risks / open questions

- **Coordinate space**: confirm `touches()` position units vs logical
  `screen_*` under `high_dpi` early (step 1) — everything keys off this.
- **World-seed text entry on mobile**: on-screen digit buttons vs browser soft
  keyboard — decide in step 5 (soft keyboard preferred if it focuses reliably).
- **Wheel vs. simultaneous button tap**: ensure multi-touch (steer + tap a
  button) is handled — `touches()` returns all live pointers, so the wheel reads
  its own pointer id while taps are matched per-id.
- Verify itch's cross-origin iframe doesn't swallow touch events (the existing
  WebGL1 workaround note suggests their embed is finicky).
