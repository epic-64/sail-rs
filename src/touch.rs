//! Touch / pointer input — the mobile control layer.
//!
//! The game proper is keyboard-driven (see `main.rs`); this module turns finger
//! taps, holds and drags into the **same verbs** the keyboard already emits, so
//! no game logic changes. The on-screen controls themselves (the steering wheel,
//! the sail buttons, the menu nav cluster) live in [`crate::touch_ui`]; this is
//! just the raw pointer bookkeeping and the hit-tests they query.
//!
//! The mouse is folded in as one synthetic pointer, so the controls can also be
//! driven with a mouse. Every hit-test is gated on [`TouchState::active`], which
//! follows the *last input device*: a touch or mouse click shows the controls and
//! makes them respond; a key press or a gamepad button hides them again (the
//! `SAIL_TOUCH` env var forces them on). So a keyboard/gamepad-only player sees
//! nothing, and a player who switches between mouse and a button device sees the
//! controls come and go to match. [`crate::device`] tracks the finer keyboard-vs-
//! gamepad distinction for on-screen keybind hints once the overlay is down.
//!
//! Screen-space maths here use macroquad's glam `Vec2`/`Rect` (pixels), *not* the
//! game's `geometry::Vec2` — there's deliberately no `use` of the latter, so the
//! prelude glob wins (unlike the world maths elsewhere, which shadows it).

use macroquad::prelude::*;
use std::collections::HashMap;

/// A pointer that travels more than this many pixels is a drag, never a tap.
const TAP_MAX_TRAVEL: f32 = 18.0;
/// A pointer held longer than this many seconds is a press/drag, never a tap.
const TAP_MAX_TIME: f32 = 0.45;
/// Synthetic id for the mouse, kept clear of the real touch id space.
const MOUSE_ID: u64 = u64::MAX;

/// One live finger (or the mouse), tracked across frames so we can tell a quick
/// tap from a press-and-hold or a drag.
struct Pointer {
    start: Vec2, // where it went down (the wheel captures by this)
    pos: Vec2,   // where it is now
    age: f32,    // seconds held
    moved: bool, // ever travelled past the tap threshold
    seen: bool,  // present this frame? (reaps pointers that vanish with no Ended)
}

/// All pointer state for a frame: who's down, what tapped, which finger owns the
/// steering wheel. Built once per frame by [`TouchState::update`].
pub struct TouchState {
    pointers: HashMap<u64, Pointer>,
    taps: Vec<Vec2>,        // pointers that *ended* this frame as a clean tap
    wheel_id: Option<u64>,  // the finger currently working the steering wheel
    pointer_active: bool,   // last input was a touch / mouse click (show the controls)
    force: bool,
}

impl Default for TouchState {
    fn default() -> Self {
        Self::new()
    }
}

impl TouchState {
    pub fn new() -> TouchState {
        // Force the touch HUD on for desktop testing via `SAIL_TOUCH=1` (native
        // only; the web build relies on real touches flipping it on).
        #[cfg(not(target_arch = "wasm32"))]
        let force = std::env::var_os("SAIL_TOUCH").is_some();
        #[cfg(target_arch = "wasm32")]
        let force = false;
        TouchState {
            pointers: HashMap::new(),
            taps: Vec::new(),
            wheel_id: None,
            pointer_active: false,
            force,
        }
    }

    /// Pull this frame's pointers (real touches + the mouse) and classify taps.
    /// Call once per frame, before anything reads the hit-tests. `pad_pressed` is
    /// this frame's [`crate::pad::Pad::any_pressed`], folded in so a controller
    /// button hides the overlay exactly like a keyboard key does.
    pub fn update(&mut self, dt: f32, pad_pressed: bool) {
        self.taps.clear();
        for p in self.pointers.values_mut() {
            p.seen = false;
        }

        // Gather raw down / ended events: real touches first…
        let mut downs: Vec<(u64, Vec2)> = Vec::new();
        let mut ends: Vec<(u64, Vec2)> = Vec::new();
        let mut pointer_event = false; // any touch / mouse activity this frame
        for t in touches() {
            pointer_event = true;
            match t.phase {
                TouchPhase::Ended | TouchPhase::Cancelled => ends.push((t.id, t.position)),
                _ => downs.push((t.id, t.position)),
            }
        }
        // …then the mouse, as one synthetic pointer.
        let (mx, my) = mouse_position();
        let m = vec2(mx, my);
        if is_mouse_button_released(MouseButton::Left) {
            ends.push((MOUSE_ID, m));
            pointer_event = true;
        } else if is_mouse_button_down(MouseButton::Left) {
            downs.push((MOUSE_ID, m));
            pointer_event = true;
        }

        // The on-screen controls follow the *last* input device: a touch or mouse
        // click shows them; any key press or gamepad button hides them again. (On
        // mobile there are no keys or pads, so once touched they stay; `force`
        // keeps them on for testing.)
        if pointer_event {
            self.pointer_active = true;
        } else if !get_keys_pressed().is_empty() || pad_pressed {
            self.pointer_active = false;
        }

        for (id, pos) in downs {
            let p = self.pointers.entry(id).or_insert(Pointer {
                start: pos,
                pos,
                age: 0.0,
                moved: false,
                seen: true,
            });
            p.seen = true;
            p.age += dt;
            p.pos = pos;
            if p.start.distance(pos) > TAP_MAX_TRAVEL {
                p.moved = true;
            }
        }
        for (id, pos) in ends {
            if let Some(mut p) = self.pointers.remove(&id) {
                p.pos = pos;
                if !p.moved && p.age <= TAP_MAX_TIME && p.start.distance(pos) <= TAP_MAX_TRAVEL {
                    self.taps.push(pos);
                }
            }
        }
        // Reap any pointer that vanished without an Ended event (not a tap).
        self.pointers.retain(|_, p| p.seen);
        // Drop the wheel grab if its finger is gone.
        if self.wheel_id.is_some_and(|id| !self.pointers.contains_key(&id)) {
            self.wheel_id = None;
        }
    }

    /// Should the on-screen controls show (and respond)? True while the last input
    /// was a touch or mouse click; a key press turns it back off. `SAIL_TOUCH` forces
    /// it on. A keyboard-only player never sees the controls.
    pub fn active(&self) -> bool {
        self.pointer_active || self.force
    }

    /// A clean tap landed inside `r` this frame.
    pub fn tapped_in(&self, r: Rect) -> bool {
        self.active() && self.taps.iter().any(|&p| r.contains(p))
    }

    /// The position of a clean tap inside `r` this frame, if any — for controls that
    /// care *where* in the rect they were hit (e.g. a slider track).
    pub fn tap_pos_in(&self, r: Rect) -> Option<Vec2> {
        if !self.active() {
            return None;
        }
        self.taps.iter().copied().find(|&p| r.contains(p))
    }

    /// A finger is currently held down with its position inside `r` (for press-and-
    /// hold controls like glancing astern).
    pub fn held_in(&self, r: Rect) -> bool {
        self.active() && self.pointers.values().any(|p| r.contains(p.pos))
    }

    /// The steering wheel's value in `[-1, 1]` from the finger that grabbed it, or
    /// `None` when nobody's steering. A finger that goes down inside `wheel` is
    /// captured and keeps steering even if it drags outside (a virtual tiller); the
    /// value is its horizontal offset from the wheel's centre.
    pub fn steering(&mut self, wheel: Rect) -> Option<f32> {
        if !self.active() {
            return None;
        }
        if self.wheel_id.is_none() {
            if let Some(id) = self
                .pointers
                .iter()
                .find(|(_, p)| wheel.contains(p.start))
                .map(|(&id, _)| id)
            {
                self.wheel_id = Some(id);
            }
        }
        let id = self.wheel_id?;
        let p = self.pointers.get(&id)?;
        let cx = wheel.x + wheel.w * 0.5;
        let half = wheel.w * 0.5;
        Some(((p.pos.x - cx) / half).clamp(-1.0, 1.0))
    }
}
