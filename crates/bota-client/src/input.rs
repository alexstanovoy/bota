//! Keys and mouse into orders and camera motion.

use bota_client_core::{Intent, Slot};
use macroquad::prelude::*;

use crate::camera::MAP_SIZE;
use bota_client_core::{Phase, Pin, Source};

use crate::gui::Gui;

/// Handles one frame of input.
pub fn handle(app: &mut Gui) {
    if console_controls(app) {
        return;
    }
    if is_key_pressed(KeyCode::Escape) {
        if app.attack_move_armed || app.aiming.is_some() || app.held_item.is_some() {
            app.attack_move_armed = false;
            app.aiming = None;
            app.held_item = None;
        } else if app.shop_open {
            app.shop_open = false;
        } else if app.over.is_some()
            || app.phase == Phase::Lobby
            || matches!(&app.source, Source::Replay(player)
                if app.view.is_none() || player.error().is_some() || player.finished())
        {
            app.quit = true;
        }
    }
    if app.phase == Phase::Playing && app.my_slot.is_some() && is_key_pressed(KeyCode::B) {
        app.shop_open = !app.shop_open;
    }
    if app.phase == Phase::Playing
        && app.my_slot.is_none()
        && matches!(app.source, Source::Live(_))
        && is_key_pressed(KeyCode::Tab)
    {
        next_eyes(app);
    }
    camera_controls(app);
    match app.phase {
        Phase::Lobby => lobby_controls(app),
        Phase::Playing => {
            pick_controls(app);
            replay_controls(app);
            let ui_click = ui_clicks(app);
            if !ui_click {
                // A click that confirms an armed order never doubles as a
                // selection of whatever it landed on.
                let armed = app.attack_move_armed || app.aiming.is_some();
                if app.over.is_none() {
                    order_controls(app);
                }
                if !armed {
                    selection_clicks(app);
                }
            }
        }
    }
}

/// Moves a live spectator on to the next eyes to watch through: everything,
/// then each seat in turn.
fn next_eyes(app: &mut Gui) {
    app.act(Intent::NextEyes);
}

/// Opens the console line on Enter and, while it is open, types into it.
/// Enter sends what was typed, Escape drops it. Returns whether the console
/// took the frame's keys, so typing never doubles as hotkeys.
fn console_controls(app: &mut Gui) -> bool {
    let Some(line) = app.console.as_mut() else {
        if app.phase == Phase::Playing && app.my_slot.is_some() && is_key_pressed(KeyCode::Enter) {
            app.console = Some(String::new());
            while get_char_pressed().is_some() {}
            return true;
        }
        return false;
    };
    while let Some(typed) = get_char_pressed() {
        if !typed.is_control() {
            line.push(typed);
        }
    }
    if is_key_pressed(KeyCode::Backspace) {
        line.pop();
    }
    if is_key_pressed(KeyCode::Escape) {
        app.console = None;
        return true;
    }
    if is_key_pressed(KeyCode::Enter) {
        let line = app.console.take().unwrap_or_default();
        app.act(Intent::Console(line));
    }
    true
}

/// Handles clicks landing on the HUD. Returns whether one did, so a click on
/// a panel never doubles as an order into the world behind it.
fn ui_clicks(app: &mut Gui) -> bool {
    let left = is_mouse_button_pressed(MouseButton::Left);
    let right = is_mouse_button_pressed(MouseButton::Right);
    let released = is_mouse_button_released(MouseButton::Left);
    if !left && !right && !released {
        return false;
    }
    let Some(view) = &app.view else {
        return false;
    };
    let (mx, my) = mouse_position();
    let (sw, sh) = (screen_width(), screen_height());
    // Letting go of a dragged item resolves the drag wherever it lands.
    if released {
        if app.held_item.is_some() {
            finish_item_drag(app, mx, my, sw, sh);
            return true;
        }
        return false;
    }
    let portrait = crate::hud::top_portraits(&view.players, sw)
        .into_iter()
        .find(|(_, rect)| rect.contains(mx, my));
    if let Some((slot, _)) = portrait {
        if left {
            // The body a seat last stood in, so a hero in the fog or one
            // that has fallen is still something to pick.
            app.act(Intent::SelectSeat(slot));
        }
        return true;
    }
    let minimap = crate::hud::minimap(sh);
    if minimap.contains(mx, my) {
        let wx = (mx - minimap.x) / minimap.w * MAP_SIZE;
        let wy = (1.0 - (my - minimap.y) / minimap.h) * MAP_SIZE;
        minimap_click(app, left, right, wx, wy);
        return true;
    }
    if app.my_slot.is_some() && crate::hud::shop_button(sw, sh).contains(mx, my) {
        if left {
            app.shop_open = !app.shop_open;
        }
        return true;
    }
    if app.controls_selection() && panel_clicks(app, left, right, mx, my) {
        return true;
    }
    crate::hud::bottom_panel(sw, sh).contains(mx, my)
}

/// A click on the minimap at a world point: right click orders a move there,
/// A-click an attack-move, a plain left click sends the camera.
fn minimap_click(app: &mut Gui, left: bool, right: bool, wx: f32, wy: f32) {
    let commands = app.controls_selection() && app.my_hero().is_some();
    if right && commands {
        app.act(Intent::MoveTo((wx, wy)));
    } else if left && app.attack_move_armed && commands {
        app.attack_move_armed = false;
        app.act(Intent::AttackMoveTo((wx, wy)));
    } else if left {
        // Sending the camera somewhere lets go of what carried it, or it
        // would be pulled straight back.
        app.pinned = None;
        app.camera.x = wx;
        app.camera.y = wy;
    }
}

/// Handles a click on the slot boxes or the open shop of a selection this
/// seat drives. Returns whether one landed there.
fn panel_clicks(app: &mut Gui, left: bool, right: bool, mx: f32, my: f32) -> bool {
    let (sw, sh) = (screen_width(), screen_height());
    let panel = crate::hud::bottom_panel(sw, sh);
    // Only the boxes this unit actually carries: the rest are not drawn,
    // and a box that is not drawn must not swallow a click either.
    let carried = app.slot_unit().map_or(0, |unit| unit.abilities.len());
    for (slot, rect) in crate::hud::ability_boxes(&panel) {
        if usize::from(slot) < carried && rect.contains(mx, my) {
            if left {
                let ctrl = is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl);
                do_press(app, Slot::Ability(slot), ctrl);
            } else if right {
                app.aiming = None;
            }
            return true;
        }
    }
    if let Some(slot) = item_box_under(mx, my, sw, sh) {
        if left && app.item_in(slot).is_some() {
            app.held_item = Some(slot);
        } else if right {
            app.held_item = None;
            app.aiming = None;
        }
        return true;
    }
    let shop = crate::hud::shop_panel(sw, sh);
    if !app.shop_open || !shop.contains(mx, my) {
        return false;
    }
    if left {
        for (id, rect) in
            crate::hud::shop_rows(&shop, bota_client_core::ITEMS.len(), app.shop_scroll)
        {
            if rect.contains(mx, my) {
                app.act(Intent::Buy(id));
            }
        }
    }
    true
}

/// The carried or stash item box under the cursor.
fn item_box_under(mx: f32, my: f32, sw: f32, sh: f32) -> Option<u8> {
    let panel = crate::hud::bottom_panel(sw, sh);
    for (slot, rect) in crate::hud::item_boxes(&panel) {
        if rect.contains(mx, my) {
            return Some(slot);
        }
    }
    for (slot, rect) in crate::hud::stash_boxes(&panel) {
        if rect.contains(mx, my) {
            return Some(slot);
        }
    }
    None
}

/// Ends an item drag: dropped on itself a consumable is used, on another box
/// it moves, on the sell strip it is sold or marked for sale, on an ally's
/// portrait it is handed over, and over the open world it is put out of the
/// bag: into an allied bag under the cursor, or onto the ground. The rest of
/// the HUD swallows the drop.
fn finish_item_drag(app: &mut Gui, mx: f32, my: f32, sw: f32, sh: f32) {
    let Some(from) = app.held_item.take() else {
        return;
    };
    if let Some(to) = item_box_under(mx, my, sw, sh) {
        if to == from {
            // Taken up and set down where it came from, a click is a press of
            // that slot and nothing more.
            do_press(app, Slot::Item(from), false);
        } else {
            app.act(Intent::Swap { from, to });
        }
        return;
    }
    if app.shop_open {
        let shop = crate::hud::shop_panel(sw, sh);
        if crate::hud::sell_strip(&shop).contains(mx, my) {
            app.act(Intent::Sell(from));
            return;
        }
        if shop.contains(mx, my) {
            return;
        }
    }
    if drop_on_portrait(app, from, mx, my, sw) {
        return;
    }
    if crate::hud::bottom_panel(sw, sh).contains(mx, my)
        || crate::hud::minimap(sh).contains(mx, my)
        || crate::hud::shop_button(sw, sh).contains(mx, my)
    {
        return;
    }
    // Over the open world.
    let (wx, wy) = app.camera.screen_to_world(mx, my, sw, sh);
    app.act(Intent::PutAt {
        slot: from,
        at: (wx, wy),
    });
}

/// A drop on a portrait: on an ally's, a bag item is handed to that seat's
/// body wherever it stands. Returns whether the drop landed on a portrait at
/// all.
fn drop_on_portrait(app: &mut Gui, from: u8, mx: f32, my: f32, sw: f32) -> bool {
    let Some(view) = &app.view else {
        return false;
    };
    let Some((seat, _)) = crate::hud::top_portraits(&view.players, sw)
        .into_iter()
        .find(|(_, rect)| rect.contains(mx, my))
    else {
        return false;
    };
    app.act(Intent::HandTo { slot: from, seat });
    true
}

/// A free left click picks what it lands on, whatever that is; the ground
/// picks nothing. A second click on the same one pins the camera to it.
fn selection_clicks(app: &mut Gui) {
    if !is_mouse_button_pressed(MouseButton::Left) {
        return;
    }
    let (sx, sy) = mouse_position();
    let at = app
        .camera
        .screen_to_world(sx, sy, screen_width(), screen_height());
    app.act(Intent::SelectAt(at));
}

/// How near an edge the cursor drives the camera that way, in pixels.
const EDGE_MARGIN: f32 = 6.0;

/// F1 picks one's own hero and F2 one's own courier; pressed twice, either
/// one pins the camera to what it picked. F3 sends the courier for the stash
/// without looking away from the fight, and picks nothing.
fn pick_controls(app: &mut Gui) {
    if is_key_pressed(KeyCode::F1) {
        app.act(Intent::SelectHero);
    }
    if is_key_pressed(KeyCode::F2) {
        app.act(Intent::SelectCourier);
    }
    if is_key_pressed(KeyCode::F3) {
        app.act(Intent::CourierDeliver);
    }
}

fn lobby_controls(app: &mut Gui) {
    let picks = [
        KeyCode::Key1,
        KeyCode::Key2,
        KeyCode::Key3,
        KeyCode::Key4,
        KeyCode::Key5,
    ];
    if app.my_slot.is_some() {
        for (id, key) in picks
            .into_iter()
            .enumerate()
            .take(bota_client_core::HEROES.len())
        {
            if is_key_pressed(key) {
                app.act(Intent::PickHero(id as u16));
            }
        }
    }
    if app.my_slot.is_some() && is_key_pressed(KeyCode::R) {
        app.act(Intent::ToggleReady);
    }
}

fn camera_controls(app: &mut Gui) {
    let (_, wheel) = mouse_wheel();
    if wheel != 0.0 {
        // Over the shop the wheel runs the catalog; anywhere else it zooms.
        let (sw, sh) = (screen_width(), screen_height());
        let (mx, my) = mouse_position();
        let shop = crate::hud::shop_panel(sw, sh);
        if app.shop_open && shop.contains(mx, my) {
            let end = crate::hud::shop_scroll_end(&shop, bota_client_core::ITEMS.len());
            app.shop_scroll = if wheel > 0.0 {
                app.shop_scroll.saturating_sub(1)
            } else {
                (app.shop_scroll + 1).min(end)
            };
        } else {
            app.camera.zoom_by(wheel.signum());
        }
    }
    let dt = get_frame_time();
    let pan = 900.0 * dt;
    // The arrows always drive the camera. W, A, S and D are orders once there
    // is a hero to give them to, and only a seat with none may pan by them.
    let letters = app.my_hero().is_none();
    let mut steer = (0.0, 0.0);
    if is_key_down(KeyCode::Up) || (letters && is_key_down(KeyCode::W)) {
        steer.1 -= pan;
    }
    if is_key_down(KeyCode::Down) || (letters && is_key_down(KeyCode::S)) {
        steer.1 += pan;
    }
    if is_key_down(KeyCode::Left) || (letters && is_key_down(KeyCode::A)) {
        steer.0 -= pan;
    }
    if is_key_down(KeyCode::Right) || (letters && is_key_down(KeyCode::D)) {
        steer.0 += pan;
    }
    let (mx, my) = mouse_position();
    let (edge_x, edge_y) = edge_of(mx, my, screen_width(), screen_height(), pan);
    steer = (steer.0 + edge_x, steer.1 + edge_y);
    if steer != (0.0, 0.0) {
        // Driving the camera by hand lets go of whatever it was pinned to.
        app.pinned = None;
        app.camera.pan(steer.0, steer.1);
        return;
    }
    // The camera is pinned to one's own hero the first time it stands.
    if !app.found_my_hero
        && app.my_hero().is_some()
        && let Some(slot) = app.my_slot
    {
        app.found_my_hero = true;
        app.pinned = Some(Pin::Hero(slot));
    }
    if let Some(pinned) = app.pinned_unit()
        && let Some(view) = app.view.as_ref()
        && let Some(unit) = view.units.iter().find(|u| u.id == pinned)
    {
        let (x, y) = (unit.pos.x.to_f32(), unit.pos.y.to_f32());
        app.camera.follow(x, y, dt);
    }
}

/// How hard a cursor at one spot on a screen of that size pushes the camera.
///
/// Nothing at all while the cursor is outside the window, so a click on
/// another window does not drag the map with it.
pub fn edge_of(mx: f32, my: f32, sw: f32, sh: f32, pan: f32) -> (f32, f32) {
    if mx < 0.0 || my < 0.0 || mx > sw || my > sh {
        return (0.0, 0.0);
    }
    let mut push = (0.0, 0.0);
    if mx <= EDGE_MARGIN {
        push.0 = -pan;
    } else if mx >= sw - EDGE_MARGIN {
        push.0 = pan;
    }
    if my <= EDGE_MARGIN {
        push.1 = -pan;
    } else if my >= sh - EDGE_MARGIN {
        push.1 = pan;
    }
    push
}

fn replay_controls(app: &mut Gui) {
    if !matches!(app.source, Source::Replay(_)) {
        return;
    }
    if is_key_pressed(KeyCode::Space) {
        app.act(Intent::TogglePause);
    }
    if is_key_pressed(KeyCode::Equal) || is_key_pressed(KeyCode::KpAdd) {
        app.act(Intent::Faster);
    }
    if is_key_pressed(KeyCode::Minus) || is_key_pressed(KeyCode::KpSubtract) {
        app.act(Intent::Slower);
    }
    if is_key_pressed(KeyCode::Period) {
        app.act(Intent::Step);
    }
}

fn order_controls(app: &mut Gui) {
    if app.my_hero().is_none() || !app.controls_selection() {
        return;
    }
    if is_key_pressed(KeyCode::A) {
        app.attack_move_armed = true;
    }
    if is_key_pressed(KeyCode::S) {
        app.attack_move_armed = false;
        app.aiming = None;
        app.act(Intent::Stop);
    }
    if is_key_pressed(KeyCode::H) {
        app.attack_move_armed = false;
        app.act(Intent::Hold);
    }
    ability_keys(app);
    item_keys(app);
    let (sx, sy) = mouse_position();
    let (wx, wy) = app
        .camera
        .screen_to_world(sx, sy, screen_width(), screen_height());
    if is_mouse_button_pressed(MouseButton::Left) {
        // A slot taken up spends the next left click, on the ground or on a
        // unit.
        if app.aiming.is_some() {
            app.act(Intent::Aimed((wx, wy)));
            return;
        }
        if app.attack_move_armed {
            app.attack_move_armed = false;
            app.act(Intent::AttackAt((wx, wy)));
            return;
        }
    }
    if is_mouse_button_pressed(MouseButton::Right) {
        app.attack_move_armed = false;
        app.aiming = None;
        app.act(Intent::Smart((wx, wy)));
    }
}

/// The number keys use the six inventory slots.
fn item_keys(app: &mut Gui) {
    let keys = [
        KeyCode::Key1,
        KeyCode::Key2,
        KeyCode::Key3,
        KeyCode::Key4,
        KeyCode::Key5,
        KeyCode::Key6,
    ];
    for (i, key) in keys.into_iter().enumerate() {
        if is_key_pressed(key) {
            do_press(app, Slot::Item(i as u8), false);
        }
    }
}

/// Answers a press of one slot, by key or by click: an aimed slot is taken
/// up and disarms a pending attack-move.
pub fn do_press(app: &mut Gui, slot: Slot, ctrl: bool) {
    app.act(Intent::Press { slot, ctrl });
    if app.aiming.is_some() {
        app.attack_move_armed = false;
    }
}

/// Q, W, E, R, T and G cast the slots of whatever is selected; with Control
/// held they spend a skill point instead. One that is aimed arms and waits
/// for a click. Nothing is sent for a unit this seat does not drive.
fn ability_keys(app: &mut Gui) {
    let keys = [
        (KeyCode::Q, 0u8),
        (KeyCode::W, 1),
        (KeyCode::E, 2),
        (KeyCode::R, 3),
        (KeyCode::T, 4),
        (KeyCode::G, 5),
    ];
    if !app.commanded().is_some_and(|unit| app.drives(unit)) {
        return;
    }
    let ctrl = is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl);
    for (key, slot) in keys {
        if is_key_pressed(key) {
            do_press(app, Slot::Ability(slot), ctrl);
        }
    }
}
