//! Screen layout of the HUD panels.
//!
//! Pure geometry, no drawing: the renderer draws these rectangles and the
//! input layer hit-tests them, so the two always agree and the math is
//! testable without a window.

use bota_proto::{PlayerView, SlotId, Team};

/// A screen-space rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiRect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl UiRect {
    /// Whether a point is inside.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}

/// Width of one top-bar portrait.
pub const PORTRAIT_W: f32 = 52.0;
/// Height of one top-bar portrait.
pub const PORTRAIT_H: f32 = 56.0;
/// Gap between portraits.
pub const PORTRAIT_GAP: f32 = 8.0;
/// Half-width kept free around the screen center for the clock.
pub const CLOCK_GAP: f32 = 56.0;
/// Top margin of the portrait row.
pub const PORTRAIT_TOP: f32 = 8.0;

/// The portrait of every seat: Radiant grows leftwards from the clock, Dire
/// rightwards, both in slot order.
pub fn top_portraits(players: &[PlayerView], screen_w: f32) -> Vec<(SlotId, UiRect)> {
    let center = screen_w / 2.0;
    let mut out = Vec::new();
    let radiant: Vec<&PlayerView> = players.iter().filter(|p| p.team == Team::Radiant).collect();
    let dire: Vec<&PlayerView> = players.iter().filter(|p| p.team == Team::Dire).collect();
    for (i, p) in radiant.iter().enumerate() {
        let from_clock = (radiant.len() - i) as f32;
        let x = center - CLOCK_GAP - from_clock * (PORTRAIT_W + PORTRAIT_GAP);
        out.push((
            p.slot,
            UiRect {
                x,
                y: PORTRAIT_TOP,
                w: PORTRAIT_W,
                h: PORTRAIT_H,
            },
        ));
    }
    for (i, p) in dire.iter().enumerate() {
        let x = center + CLOCK_GAP + PORTRAIT_GAP + i as f32 * (PORTRAIT_W + PORTRAIT_GAP);
        out.push((
            p.slot,
            UiRect {
                x,
                y: PORTRAIT_TOP,
                w: PORTRAIT_W,
                h: PORTRAIT_H,
            },
        ));
    }
    out
}

/// The bottom panel showing the selected hero.
pub fn bottom_panel(screen_w: f32, screen_h: f32) -> UiRect {
    let w = 840.0_f32.min(screen_w - 40.0);
    UiRect {
        x: (screen_w - w) / 2.0,
        y: screen_h - 152.0,
        w,
        h: 120.0,
    }
}

/// The ability boxes of the bottom panel, four to a row.
///
/// Four across is what fits before the item boxes begin; a courier carries
/// more than four, and the rest wrap onto a second row rather than running
/// under the items.
/// How many ability boxes the panel has room for. A hero carries four; a
/// courier carries more.
pub const ABILITY_BOXES: u8 = 6;

pub fn ability_boxes(panel: &UiRect) -> Vec<(u8, UiRect)> {
    (0..ABILITY_BOXES)
        .map(|i| {
            let (col, row) = (f32::from(i % 4), f32::from(i / 4));
            (
                i,
                UiRect {
                    x: panel.x + 435.0 + col * 44.0,
                    y: panel.y + 14.0 + row * 44.0,
                    w: 40.0,
                    h: 40.0,
                },
            )
        })
        .collect()
}

/// The nine carried item boxes of the bottom panel: two rows of three for
/// the inventory, one dimmer row below for the backpack.
pub fn item_boxes(panel: &UiRect) -> Vec<(u8, UiRect)> {
    let ix = panel.x + 623.0;
    let mut out = Vec::new();
    for slot in 0..9u8 {
        let (col, row) = (f32::from(slot % 3), f32::from(slot / 3));
        out.push((
            slot,
            UiRect {
                x: ix + col * 36.0,
                y: panel.y + 10.0 + row * 30.0,
                w: 32.0,
                h: 26.0,
            },
        ));
    }
    out
}

/// The six stash boxes, a strip sitting on top of the bottom panel. Shown
/// only within the home shop area.
pub fn stash_boxes(panel: &UiRect) -> Vec<(u8, UiRect)> {
    (0..6u8)
        .map(|i| {
            (
                9 + i,
                UiRect {
                    x: panel.x + 623.0 + f32::from(i % 3) * 36.0,
                    y: panel.y - 74.0 + f32::from(i / 3) * 30.0,
                    w: 32.0,
                    h: 26.0,
                },
            )
        })
        .collect()
}

/// The shop panel on the right edge. Shown only within the home shop area.
pub fn shop_panel(screen_w: f32, screen_h: f32) -> UiRect {
    let h = 620.0_f32.min(screen_h - 180.0).max(160.0);
    UiRect {
        x: screen_w - 250.0,
        y: (screen_h - h) / 2.0,
        w: 240.0,
        h,
    }
}

/// Height of one shop row.
const SHOP_ROW: f32 = 26.0;

/// How many rows the shop panel shows at once.
pub fn shop_row_count(panel: &UiRect) -> usize {
    (((panel.h - 30.0 - 34.0) / SHOP_ROW).floor().max(1.0)) as usize
}

/// How far down the catalog the shop may be scrolled and still fill.
pub fn shop_scroll_end(panel: &UiRect, items: usize) -> usize {
    items.saturating_sub(shop_row_count(panel))
}

/// One clickable row per catalog item the shop shows, starting at `from`.
pub fn shop_rows(panel: &UiRect, items: usize, from: usize) -> Vec<(u16, UiRect)> {
    let from = from.min(shop_scroll_end(panel, items));
    (from..items.min(from + shop_row_count(panel)))
        .enumerate()
        .map(|(row, item)| {
            (
                item as u16,
                UiRect {
                    x: panel.x + 6.0,
                    y: panel.y + 30.0 + row as f32 * SHOP_ROW,
                    w: panel.w - 12.0,
                    h: 24.0,
                },
            )
        })
        .collect()
}

/// The sell strip at the bottom of the shop panel: a held item dropped here
/// is sold.
pub fn sell_strip(panel: &UiRect) -> UiRect {
    UiRect {
        x: panel.x + 6.0,
        y: panel.y + panel.h - 34.0,
        w: panel.w - 12.0,
        h: 28.0,
    }
}

/// The minimap square in the bottom-left corner.
pub fn minimap(screen_h: f32) -> UiRect {
    UiRect {
        x: 10.0,
        y: screen_h - 210.0,
        w: 200.0,
        h: 200.0,
    }
}

/// The button toggling the shop panel, in the bottom-right corner.
pub fn shop_button(screen_w: f32, screen_h: f32) -> UiRect {
    UiRect {
        x: screen_w - 104.0,
        y: screen_h - 40.0,
        w: 96.0,
        h: 28.0,
    }
}

/// The chips of the timed effects on the panel's hero, sitting on top of the
/// bottom panel.
pub fn effect_boxes(panel: &UiRect, count: usize) -> Vec<UiRect> {
    (0..count)
        .map(|i| UiRect {
            x: panel.x + 175.0 + i as f32 * 72.0,
            y: panel.y - 26.0,
            w: 68.0,
            h: 22.0,
        })
        .collect()
}
