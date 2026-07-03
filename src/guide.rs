//! The Guide: a parchment primer the captain can summon to learn the ropes.
//!
//! It is raised over the scene with **G** and turned with the **arrow keys** (or
//! the on-screen nav cluster on touch), exactly like the captain's log. On a
//! captain's very first voyage it opens by itself so a newcomer is never dropped
//! at the helm cold (see `save::guide_seen`); after that it is summoned on demand.
//!
//! Each page is one topic of plain prose, word-wrapped into the leaf at the
//! current [`crate::ui::scale`]. The pages, in reading order, live in [`PAGES`];
//! `main` clamps the page cursor to [`NUM_PAGES`].

use macroquad::prelude::*;

// The ink/parchment palette and the type scale are shared with the log and the
// port board; see `crate::ui`.
use crate::ui::{
    dim_ink, fs_body, fs_small, fs_title, ink, line_h, parchment, parchment_edge, px,
};

/// One page of the guide: a heading and its body paragraphs (drawn in order, with
/// a blank line between each). A paragraph beginning with the bullet glyph reads as
/// a list item.
struct GuidePage {
    title: &'static str,
    body: &'static [&'static str],
}

/// The guide's pages, in reading order. Tune the prose here; the page count and
/// the footer dots follow the array, so adding a topic needs no other change.
const PAGES: &[GuidePage] = &[
    GuidePage {
        title: "Welcome Aboard",
        body: &[
            "The horizon is yours, captain. A trim ship, a hold to fill and a \
             chain of islands waiting beyond the swell.",
            "Make your fortune by the wind: run trade contracts between ports, \
             scoop salvage from the waves, race rivals for their stake, and refit \
             your ship at the shipyard until she outruns any sea.",
            "The helm at a glance:",
            "\u{2022} Steer with A/D or the left and right arrows.",
            "\u{2022} Deploy and furl sail with W/S (or up/down).",
            "\u{2022} Press G at any time to reopen this guide for a closer read.",
        ],
    },
    GuidePage {
        title: "All the Controls",
        body: &[
            "Everything you can do from the helm:",
            "\u{2022} Steer: A / D, or the left / right arrows.",
            "\u{2022} Sail: W / S (or up / down) deploy and furl a notch at a time.",
            "\u{2022} Dock: Space, with a harbour in reach and the sail furled.",
            "\u{2022} Look astern: hold C to glance back over the stern.",
            "\u{2022} Captain's Log: L (repairs, cargo, contracts, upgrades).",
            "\u{2022} World map: M, once you've bought the chart from a tavern.",
            "\u{2022} Wares: 1 / 2 / 3 call on your active tavern wares, once a day each.",
            "\u{2022} Hide HUD: H tucks it away; press again to bring it back.",
            "\u{2022} Guide: G opens this guide. Pause: Escape.",
        ],
    },
    GuidePage {
        title: "Your First Gold",
        body: &[
            "To earn your first coin, accept a trade contract bound for the \
             nearest island you can reach. A short haul guarantees some income \
             and brings you back to port in good time.",
            "On the way you may spot flotsam drifting on the swell: boxes, barrels \
             and chests. Sail straight over them to scoop up gold. It is an easy, \
             welcome source of income, and the chests are the richest finds of all.",
        ],
    },
    GuidePage {
        title: "Cargo Hold and Sail Tolerance",
        body: &[
            "The starter ship carries a hold of 16 and a sail tolerance of 12. The \
             hold is how many units you can stow. Sail tolerance is how much weight \
             the rig can drive before your speed begins to suffer.",
            "Hint: early on, keep your cargo at 12 units or under to hold full speed.",
        ],
    },
    GuidePage {
        title: "Reading the Wind",
        body: &[
            "If the wind turns against you, don't panic. Steer to one side long \
             enough to catch it again. You won't point straight at your target, but \
             by zig-zagging (tacking) you will reach it in the end.",
            "Top speed comes when the wind is on your beam, 90 degrees off the bow. \
             Dead astern (180 degrees) you sail about a third slower. Straight \
             upwind is slowest of all, with a 35 degree dead zone either side of \
             the wind's eye where you make no way at all.",
            "As a rule, sail at a slight angle toward your target, or a wide angle \
             when beating upwind. Your sails trim themselves to the best set; you \
             need only steer the hull to a good angle. Use this when racing rivals.",
        ],
    },
    GuidePage {
        title: "Hull and Repairs",
        body: &[
            "Your hull wears 1 percent for every kilometre sailed; lying still costs \
             nothing. For every 10 percent missing she takes a stacking handicap \
             from this set: a wider dead zone, slower turning, and a lower top speed.",
            "Repair at any port for 2 gold per point mended. With planks in the hold \
             you can also caulk her at sea from the captain's log (L), though it \
             comes dearer than a proper drydock.",
        ],
    },
    GuidePage {
        title: "The Shipyard",
        body: &[
            "Every archipelago holds many islands, some with ports, but only one \
             carries a shipyard. It is marked on your chart with a blue ring and an \
             \"S\". There you can refit the ship:",
            "\u{2022} Hull: a higher top speed (and a sturdier ship).",
            "\u{2022} Sails: more cargo tolerance before you lose speed.",
            "\u{2022} Hold: more cargo capacity.",
            "Some ventures, like long-distance races, demand a minimum hull level \
             before you can take them on against the rivals.",
        ],
    },
    GuidePage {
        title: "The Tavern",
        body: &[
            "Every shipyard keeps a tavern, and each one sells a single special ware, \
             bought but once. Your home port's tavern stocks a World Map: buy it and \
             you can open a chart of every archipelago from the captain's log, or jump \
             straight to it with the M key.",
            "Other taverns sell other curios. Some are kept for good: a figurehead that \
             draws more coin from salvage, or an almanac that lays every port's prices \
             out in your log. Others are abilities you call on at the helm with the \
             number keys, each good once a day: a whistle to summon a fresh wind, a \
             draught for a burst of speed, a glass to calm a gale.",
            "Sail back any time for a ware you passed up; each tavern always keeps the \
             same stock.",
            "Press Escape to close this window and start playing.",
        ],
    },
];

/// How many pages the guide holds; `main` clamps the page cursor to this.
pub const NUM_PAGES: usize = PAGES.len();

/// Word-wrap `text` into lines no wider than `max_w` at font size `fs`.
fn wrap(text: &str, fs: u16, max_w: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let trial = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if !cur.is_empty() && measure_text(&trial, None, fs, 1.0).width > max_w {
            lines.push(std::mem::replace(&mut cur, word.to_string()));
        } else {
            cur = trial;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Render the open guide over the scene, dimming the world behind it first. `page`
/// is the page cursor (0..[`NUM_PAGES`]).
pub fn render(page: usize, w: f32, h: f32) {
    // Dim the world so the primer reads as the captain's focus (matching the log).
    draw_rectangle(0.0, 0.0, w, h, Color::new(0.0, 0.0, 0.0, 0.45));

    // The open leaf, centred. A touch narrower and taller than the log's spread:
    // one column of prose reads best in a portrait-ish page.
    let pw = (w * 0.88).min(px(640.0));
    let ph = (h * 0.90).min(px(540.0));
    let x0 = (w - pw) / 2.0;
    let y0 = (h - ph) / 2.0;
    draw_rectangle(x0, y0, pw, ph, parchment());
    draw_rectangle_lines(x0, y0, pw, ph, px(3.0), parchment_edge());

    let pad = px(30.0);
    let col_x = x0 + pad;
    let col_w = pw - 2.0 * pad;

    let p = &PAGES[page.min(NUM_PAGES - 1)];

    // A small overline names the primer itself, so a captain who summoned it knows
    // what they opened; the page's own topic is the heading below.
    let over = "GUIDE";
    draw_text(over, col_x, y0 + px(34.0), fs_small() as f32, dim_ink());

    // Topic heading in the serif display face, underlined to set it off.
    let title_y = y0 + px(64.0);
    crate::font::heading(|| draw_text(p.title, col_x, title_y, fs_title() as f32, ink()));
    let under = title_y + px(10.0);
    draw_line(col_x, under, col_x + col_w, under, px(1.5), dim_ink());

    // Body: each paragraph word-wrapped, with a blank line between paragraphs.
    let fs = fs_body();
    let lh = line_h(fs);
    let mut y = title_y + px(40.0);
    for para in p.body {
        for ln in wrap(para, fs, col_w) {
            draw_text(&ln, col_x, y, fs as f32, ink());
            y += lh;
        }
        y += lh * 0.5;
    }

    // --- Footer: page dots + the navigation hint --------------------------------
    let foot_y = y0 + ph - px(16.0);
    let cx = x0 + pw / 2.0;
    let gap = px(18.0);
    let dots_w = gap * (NUM_PAGES as f32 - 1.0);
    let mut dx = cx - dots_w / 2.0;
    for i in 0..NUM_PAGES {
        if i == page {
            draw_circle(dx, foot_y - px(5.0), px(4.0), ink());
        } else {
            draw_circle_lines(dx, foot_y - px(5.0), px(4.0), px(1.0), dim_ink());
        }
        dx += gap;
    }
    draw_text("\u{25C4} \u{25BA} turn the page", col_x, foot_y, fs_small() as f32, dim_ink());
    let close = "G  close";
    let cd = measure_text(close, None, fs_small(), 1.0);
    draw_text(close, x0 + pw - pad - cd.width, foot_y, fs_small() as f32, dim_ink());
}
