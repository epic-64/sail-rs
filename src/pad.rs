//! Gamepad input, the controller layer (native only).
//!
//! Like [`crate::touch`], this is an **additive** layer that emits the *same verbs*
//! the keyboard already emits, so no game logic is controller-aware: every dispatch
//! site in `main.rs` and the boards ORs a `pad.<verb>()` call in beside its
//! `is_key_*` / `touch.*` calls. A controller has far fewer buttons than the keyboard,
//! so only the essential verbs are bound; the dev-mode keys, the "banana" cheat buffer
//! and the world-seed text field stay keyboard-only (a stick can't type).
//!
//! Bindings (Xbox-style names; a stick or the d-pad both work for direction):
//!
//! | Verb | Button |
//! |---|---|
//! | steer | left stick X, or d-pad ←/→ |
//! | menu up/down/left/right | left stick, or d-pad |
//! | sail up (accelerate) / down (decelerate) | A (South) / B (East) |
//! | enter port, go ashore | X (West) |
//! | confirm / back (menus) | A (South) / B (East) |
//! | pause | Start |
//! | captain's log | Y (North) |
//! | look astern (hold) | right stick click |
//! | wares 1 / 2 / 3 | LB / RB / RT |
//! | cycle port tabs | LB / RB |
//!
//! A (South) and B (East) do double duty: they trim sail at the live helm, and act as
//! confirm / back in menus and the open log. The two never clash because sail trim is
//! suppressed while an overlay is up (the call sites gate it on `!overlay_open`), just
//! as the keyboard reserves its arrows for the book while it's open.
//!
//! `gilrs` is native-only (its wasm backend pulls in `wasm-bindgen`, which macroquad's
//! own JS glue can't satisfy), so on the web build this whole module compiles to a
//! no-op stub: [`Pad::update`] does nothing, the level snapshots stay all-default and
//! every query returns `false` / `None`. The web build keeps its on-screen touch HUD.

/// A stick pushed less than this far from centre reads as centred (rest-noise guard).
#[cfg(not(target_arch = "wasm32"))]
const DEADZONE: f32 = 0.25;

/// The digital on/off state of every bound control for one frame. Buttons are levels
/// here; the query methods below turn them into per-frame *edges* (pressed-this-frame)
/// by diffing against the previous frame, matching `is_key_pressed`. `steer` is the
/// analog helm demand in `[-1, 1]` (0 = centred).
///
/// Kept platform-independent so the query methods compile everywhere: on wasm nothing
/// ever fills one in, so it stays `default()` (all false, steer 0) and the queries read
/// as "no controller".
#[derive(Default, Clone, Copy)]
struct Levels {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    south: bool, // A: sail up (accelerate) / confirm
    east: bool,  // B: sail down (decelerate) / back
    north: bool, // Y: log
    west: bool,  // X: enter port
    start: bool, // pause
    rb: bool,    // ware 2 / tab next
    lb: bool,    // ware 1 / tab prev
    rt: bool,    // ware 3
    rthumb: bool, // right stick click: look astern
    steer: f32,
}

#[cfg(not(target_arch = "wasm32"))]
impl Levels {
    /// Fold another connected pad's state in, so *any* controller drives the game
    /// (buttons OR together; steering takes whichever stick is pushed furthest).
    fn or_in(&mut self, o: Levels) {
        self.up |= o.up;
        self.down |= o.down;
        self.left |= o.left;
        self.right |= o.right;
        self.south |= o.south;
        self.east |= o.east;
        self.north |= o.north;
        self.west |= o.west;
        self.start |= o.start;
        self.rb |= o.rb;
        self.lb |= o.lb;
        self.rt |= o.rt;
        self.rthumb |= o.rthumb;
        if o.steer.abs() > self.steer.abs() {
            self.steer = o.steer;
        }
    }
}

/// Reads one connected gamepad into a [`Levels`]. A direction is on when its d-pad
/// button is down *or* the left stick is past the deadzone (either axis form), so the
/// stick and the d-pad are interchangeable. The steer axis rescales the stick past the
/// deadzone to a clean `[-1, 1]` and falls back to the d-pad as a hard ±1.
#[cfg(not(target_arch = "wasm32"))]
fn read_levels(g: &gilrs::Gamepad) -> Levels {
    use gilrs::{Axis, Button};
    let lx = g.value(Axis::LeftStickX);
    let ly = g.value(Axis::LeftStickY); // gilrs: up is positive
    let dz = DEADZONE;
    // Some backends report the d-pad as an axis rather than discrete buttons; accept
    // both so navigation works regardless.
    let dpad_x = g.value(Axis::DPadX);
    let dpad_y = g.value(Axis::DPadY);
    let dpad_right = g.is_pressed(Button::DPadRight) || dpad_x > 0.5;
    let dpad_left = g.is_pressed(Button::DPadLeft) || dpad_x < -0.5;
    let steer = if lx.abs() > dz {
        (lx.signum() * (lx.abs() - dz) / (1.0 - dz)).clamp(-1.0, 1.0)
    } else if dpad_right {
        1.0
    } else if dpad_left {
        -1.0
    } else {
        0.0
    };
    Levels {
        up: g.is_pressed(Button::DPadUp) || dpad_y > 0.5 || ly > dz,
        down: g.is_pressed(Button::DPadDown) || dpad_y < -0.5 || ly < -dz,
        left: dpad_left || lx < -dz,
        right: dpad_right || lx > dz,
        south: g.is_pressed(Button::South),
        east: g.is_pressed(Button::East),
        north: g.is_pressed(Button::North),
        west: g.is_pressed(Button::West),
        start: g.is_pressed(Button::Start),
        rb: g.is_pressed(Button::RightTrigger),
        lb: g.is_pressed(Button::LeftTrigger),
        rt: g.is_pressed(Button::RightTrigger2),
        rthumb: g.is_pressed(Button::RightThumb),
        steer,
    }
}

/// The controller input layer: this and the previous frame's [`Levels`], diffed to
/// yield per-frame button edges. Ticked once per frame by [`Pad::update`], right
/// beside `touch.update`.
pub struct Pad {
    #[cfg(not(target_arch = "wasm32"))]
    gilrs: Option<gilrs::Gilrs>,
    now: Levels,
    prev: Levels,
}

impl Default for Pad {
    fn default() -> Self {
        Self::new()
    }
}

impl Pad {
    pub fn new() -> Pad {
        Pad {
            // A missing / unavailable gamepad subsystem just leaves this `None`, and
            // every query reads as "no controller".
            #[cfg(not(target_arch = "wasm32"))]
            gilrs: gilrs::Gilrs::new().ok(),
            now: Levels::default(),
            prev: Levels::default(),
        }
    }

    /// Pump the controller once per frame, before anything reads the verbs below.
    pub fn update(&mut self) {
        self.prev = self.now;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(gilrs) = self.gilrs.as_mut() else {
                self.now = Levels::default();
                return;
            };
            // Draining the event queue is what advances gilrs' cached button/axis state
            // that `read_levels` then reads; the events themselves we don't need.
            while gilrs.next_event().is_some() {}
            let mut lv = Levels::default();
            for (_id, g) in gilrs.gamepads() {
                lv.or_in(read_levels(&g));
            }
            self.now = lv;
        }
    }

    /// A button that went down *this* frame (edge), the gamepad twin of `is_key_pressed`.
    fn edge(&self, sel: impl Fn(&Levels) -> bool) -> bool {
        sel(&self.now) && !sel(&self.prev)
    }

    // --- menu navigation (d-pad / left stick) ---
    pub fn up(&self) -> bool {
        self.edge(|l| l.up)
    }
    pub fn down(&self) -> bool {
        self.edge(|l| l.down)
    }
    pub fn left(&self) -> bool {
        self.edge(|l| l.left)
    }
    pub fn right(&self) -> bool {
        self.edge(|l| l.right)
    }
    /// Confirm / activate (A). The same button trims sail up at the live helm
    /// ([`Pad::sail_up`]); they never both fire because sail trim is gated off while a
    /// menu / the log is open, so the name just marks intent at the menu call sites.
    pub fn confirm(&self) -> bool {
        self.edge(|l| l.south)
    }
    /// Back out / cancel (B). Doubles as furl-sail at the live helm ([`Pad::sail_down`]),
    /// disjoint for the same reason as [`Pad::confirm`].
    pub fn back(&self) -> bool {
        self.edge(|l| l.east)
    }

    // --- sailing ---
    /// The analog helm demand in `[-1, 1]`, or `None` when the stick is centred and
    /// the d-pad isn't held (so the keyboard keeps the helm).
    pub fn steer(&self) -> Option<f32> {
        (self.now.steer != 0.0).then_some(self.now.steer)
    }
    /// Deploy a sail notch, "accelerate" (A). Gated on `!overlay_open` at the call site
    /// so an open menu / log gets A back as confirm ([`Pad::confirm`]).
    pub fn sail_up(&self) -> bool {
        self.edge(|l| l.south)
    }
    /// Furl a sail notch, "decelerate" (B). Gated like [`Pad::sail_up`], freeing B for
    /// back ([`Pad::back`]) under an overlay.
    pub fn sail_down(&self) -> bool {
        self.edge(|l| l.east)
    }
    /// Enter port / go ashore (X).
    pub fn dock(&self) -> bool {
        self.edge(|l| l.west)
    }
    /// Open / close the captain's log (Y).
    pub fn log(&self) -> bool {
        self.edge(|l| l.north)
    }
    /// Raise the pause menu (Start).
    pub fn pause(&self) -> bool {
        self.edge(|l| l.start)
    }
    /// Look astern: held, not an edge (the view flips only while it's down).
    pub fn astern(&self) -> bool {
        self.now.rthumb
    }
    /// Fire an active tavern ware by slot (0..=2): LB / RB / RT.
    pub fn ware(&self, slot: usize) -> bool {
        match slot {
            0 => self.edge(|l| l.lb),
            1 => self.edge(|l| l.rb),
            2 => self.edge(|l| l.rt),
            _ => false,
        }
    }

    // --- port board tab cycling (bumpers) ---
    pub fn tab_next(&self) -> bool {
        self.edge(|l| l.rb)
    }
    pub fn tab_prev(&self) -> bool {
        self.edge(|l| l.lb)
    }
}
