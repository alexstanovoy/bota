//! Keys and mouse into orders and camera motion.

use bota_proto::{
    AbilitySlot, Aim, ClientMsg, EntityId, ItemId, ItemSlot, Order, Target, Team, UnitKind, Vec2,
    WorldView,
};
use macroquad::prelude::*;

use crate::slots::{Press, Slot, order_for, press};
use crate::state::{App, Phase, Source, Tap};

/// Handles one frame of input.
pub fn handle(app: &mut App) {
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
    // A live spectator borrows eyes with Tab: everything, then each seat in
    // turn.
    if app.phase == Phase::Playing
        && app.my_slot.is_none()
        && matches!(app.source, Source::Live(_))
        && is_key_pressed(KeyCode::Tab)
    {
        let seats: Vec<bota_proto::SlotId> = app
            .view
            .as_ref()
            .map(|view| view.players.iter().map(|player| player.slot).collect())
            .unwrap_or_default();
        app.eyes = match app.eyes {
            None => seats.first().copied(),
            Some(current) => seats
                .iter()
                .position(|slot| *slot == current)
                .and_then(|at| seats.get(at + 1))
                .copied(),
        };
        let seat = app.eyes;
        app.source.send(&ClientMsg::ViewAs { seat });
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

/// Handles clicks landing on the HUD. Returns whether one did, so a click on
/// a panel never doubles as an order into the world behind it.
fn ui_clicks(app: &mut App) -> bool {
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
    for (slot, rect) in crate::hud::top_portraits(&view.players, sw) {
        if rect.contains(mx, my) {
            if left {
                // The body a seat last stood in, so a hero in the fog or one
                // that has fallen is still something to pick.
                let body = app.body_of(slot);
                let again = app.tapped_twice(Tap::Seat(slot));
                app.choose(body, again);
            }
            return true;
        }
    }
    // The minimap: right click orders a move there, A-click an attack-move,
    // a plain left click sends the camera.
    let minimap = crate::hud::minimap(sh);
    if minimap.contains(mx, my) {
        let wx = (mx - minimap.x) / minimap.w * 18432.0;
        let wy = (1.0 - (my - minimap.y) / minimap.h) * 18432.0;
        let ground = world_vec(wx, wy);
        let commands = app.controls_selection() && app.my_hero().is_some();
        if right && commands {
            app.send_order(Order::Move {
                target: Target::Pos(ground),
            });
        } else if left && app.attack_move_armed && commands {
            app.attack_move_armed = false;
            app.send_order(Order::Attack {
                target: Target::Pos(ground),
            });
        } else if left {
            // Sending the camera somewhere lets go of what carried it, or it
            // would be pulled straight back.
            app.pinned = None;
            app.camera.x = wx;
            app.camera.y = wy;
        }
        return true;
    }
    if app.my_slot.is_some() && crate::hud::shop_button(sw, sh).contains(mx, my) {
        if left {
            app.shop_open = !app.shop_open;
        }
        return true;
    }
    let panel = crate::hud::bottom_panel(sw, sh);
    if app.controls_selection() {
        // Only the boxes this unit actually carries: the rest are not drawn,
        // and a box that is not drawn must not swallow a click either.
        let carried = app.slot_unit().map_or(0, |unit| unit.abilities.len());
        for (slot, rect) in crate::hud::ability_boxes(&panel) {
            if usize::from(slot) < carried && rect.contains(mx, my) {
                if left {
                    let ctrl =
                        is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl);
                    do_press(app, Slot::Ability(slot), ctrl);
                } else if right {
                    app.aiming = None;
                }
                return true;
            }
        }
        if let Some(slot) = item_box_under(mx, my, sw, sh) {
            if left && app.item_at(slot) {
                app.held_item = Some(slot);
            } else if right {
                app.held_item = None;
                app.aiming = None;
            }
            return true;
        }
        if app.shop_open {
            let shop = crate::hud::shop_panel(sw, sh);
            if shop.contains(mx, my) {
                if left {
                    for (id, rect) in
                        crate::hud::shop_rows(&shop, crate::catalog::ITEMS.len(), app.shop_scroll)
                    {
                        if rect.contains(mx, my) {
                            app.send_order(Order::Buy { item: ItemId(id) });
                        }
                    }
                }
                return true;
            }
        }
    }
    panel.contains(mx, my)
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
/// bag — into an allied bag under the cursor, or onto the ground. The rest
/// of the HUD swallows the drop.
fn finish_item_drag(app: &mut App, mx: f32, my: f32, sw: f32, sh: f32) {
    let Some(from) = app.held_item.take() else {
        return;
    };
    if let Some(to) = item_box_under(mx, my, sw, sh) {
        if to == from {
            // Taken up and set down where it came from, a click is a press of
            // that slot and nothing more.
            do_press(app, Slot::Item(from), false);
        } else {
            app.send_order(Order::Swap {
                from: ItemSlot(from),
                to: ItemSlot(to),
            });
        }
        return;
    }
    if app.shop_open {
        let shop = crate::hud::shop_panel(sw, sh);
        if crate::hud::sell_strip(&shop).contains(mx, my) {
            app.send_order(Order::Sell {
                slot: ItemSlot(from),
            });
            return;
        }
        if shop.contains(mx, my) {
            return;
        }
    }
    // A drop on an ally's portrait hands the item to that seat's body,
    // wherever it stands: the walk is the server's business.
    let portraits = app
        .view
        .as_ref()
        .map(|view| crate::hud::top_portraits(&view.players, sw))
        .unwrap_or_default();
    for (seat, rect) in portraits {
        if !rect.contains(mx, my) {
            continue;
        }
        let ally = app
            .view
            .as_ref()
            .and_then(|view| view.players.iter().find(|p| p.slot == seat))
            .is_some_and(|p| Some(p.team) == app.my_team());
        if from < crate::slots::BAG_SLOTS
            && ally
            && let Some(body) = app.body_of(seat)
        {
            app.send_order(Order::Put {
                slot: ItemSlot(from),
                target: Target::Unit(body),
            });
        }
        return;
    }
    if crate::hud::bottom_panel(sw, sh).contains(mx, my)
        || crate::hud::minimap(sh).contains(mx, my)
        || crate::hud::shop_button(sw, sh).contains(mx, my)
    {
        return;
    }
    // Over the open world. What the stash holds stays put: it is a shelf at
    // the shop, not a pair of hands.
    if from >= crate::slots::BAG_SLOTS {
        return;
    }
    let (wx, wy) = app.camera.screen_to_world(mx, my, sw, sh);
    // Whoever the panel is about is the one that puts: the courier lays its
    // own load out the same way the hero does. Its own body under the cursor
    // reads as the ground at its feet.
    let carrier = app.commanded();
    let target = app
        .view
        .as_ref()
        .and_then(|view| unit_under_cursor(view, wx, wy, carrier, true))
        .filter(|id| takes_a_handover(app, *id))
        .map(Target::Unit)
        .unwrap_or(Target::Pos(world_vec(wx, wy)));
    app.send_order(Order::Put {
        slot: ItemSlot(from),
        target,
    });
}

/// Whether a unit is one an item can be handed to: an allied hero or courier.
fn takes_a_handover(app: &App, id: EntityId) -> bool {
    let Some(view) = &app.view else {
        return false;
    };
    view.units.iter().find(|u| u.id == id).is_some_and(|u| {
        Some(u.team) == app.my_team() && matches!(u.kind, UnitKind::Hero | UnitKind::Courier)
    })
}

/// A free left click picks what it lands on, whatever that is; the ground
/// picks nothing. A second click on the same one pins the camera to it.
fn selection_clicks(app: &mut App) {
    if !is_mouse_button_pressed(MouseButton::Left) {
        return;
    }
    let (sx, sy) = mouse_position();
    let (wx, wy) = app
        .camera
        .screen_to_world(sx, sy, screen_width(), screen_height());
    let Some(view) = &app.view else {
        return;
    };
    // One rule for everything that stands: a hero, a courier, a creep, a
    // building, one's own side or the other. A click picks it; a second click
    // on the same one pins the camera to it.
    let hit = unit_under_cursor(view, wx, wy, None, false);
    let again = hit.is_some_and(|id| app.tapped_twice(Tap::Unit(id)));
    app.choose(hit, again);
}

/// How near an edge the cursor drives the camera that way, in pixels.
const EDGE_MARGIN: f32 = 6.0;

/// Which ability of a courier fetches the stash.
const TAKE_STASH: u16 = 10;

/// F1 picks one's own hero and F2 one's own courier; pressed twice, either
/// one pins the camera to what it picked. F3 sends the courier for the stash
/// without looking away from the fight, and picks nothing.
fn pick_controls(app: &mut App) {
    if is_key_pressed(KeyCode::F1) {
        let hero = app.my_hero();
        let again = app.tapped_twice(Tap::Hero);
        app.choose(hero, again);
    }
    if is_key_pressed(KeyCode::F2)
        && let Some(courier) = app.my_courier()
    {
        let again = app.tapped_twice(Tap::Courier);
        app.choose(Some(courier), again);
    }
    if is_key_pressed(KeyCode::F3)
        && let Some(courier) = app.my_courier()
        && let Some(slot) = app.courier_slot(TAKE_STASH)
    {
        app.send_order_to(
            Some(courier),
            Order::Cast {
                slot: AbilitySlot(slot),
                target: Target::None,
            },
        );
    }
}

fn lobby_controls(app: &mut App) {
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
            .take(crate::catalog::HEROES.len())
        {
            if is_key_pressed(key) {
                app.source.send(&ClientMsg::PickHero {
                    hero: bota_proto::HeroId(id as u16),
                });
            }
        }
    }
    if app.my_slot.is_some() && is_key_pressed(KeyCode::R) {
        app.ready = !app.ready;
        let ready = app.ready;
        app.source.send(&ClientMsg::SetReady(ready));
    }
}

fn camera_controls(app: &mut App) {
    let (_, wheel) = mouse_wheel();
    if wheel != 0.0 {
        // Over the shop the wheel runs the catalog; anywhere else it zooms.
        let (sw, sh) = (screen_width(), screen_height());
        let (mx, my) = mouse_position();
        let shop = crate::hud::shop_panel(sw, sh);
        if app.shop_open && shop.contains(mx, my) {
            let end = crate::hud::shop_scroll_end(&shop, crate::catalog::ITEMS.len());
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
    let (edge_x, edge_y) = edge_push(pan);
    steer = (steer.0 + edge_x, steer.1 + edge_y);
    if steer != (0.0, 0.0) {
        // Driving the camera by hand lets go of whatever it was pinned to:
        // asking to look elsewhere is asking to stop being carried.
        app.pinned = None;
        app.camera.pan(steer.0, steer.1);
        return;
    }
    // The first hero to stand is looked at once, so a match does not open on
    // an empty middle of the map.
    if !app.found_my_hero
        && app.my_hero().is_some()
        && let Some(slot) = app.my_slot
    {
        app.found_my_hero = true;
        app.pinned = Some(crate::state::Pin::Hero(slot));
    }
    if let Some(pinned) = app.pinned_unit()
        && let Some(view) = app.view.as_ref()
        && let Some(unit) = view.units.iter().find(|u| u.id == pinned)
    {
        let (x, y) = (unit.pos.x.to_f32(), unit.pos.y.to_f32());
        app.camera.follow(x, y, dt);
    }
}

/// How hard the cursor at the edge of the screen pushes the camera.
///
/// Nothing at all while the cursor is outside the window, so a click on
/// another window does not drag the map with it.
fn edge_push(pan: f32) -> (f32, f32) {
    let (mx, my) = mouse_position();
    edge_of(mx, my, screen_width(), screen_height(), pan)
}

/// How hard a cursor at one spot on a screen of that size pushes the camera.
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

fn replay_controls(app: &mut App) {
    let Source::Replay(player) = &mut app.source else {
        return;
    };
    if is_key_pressed(KeyCode::Space) {
        player.paused = !player.paused;
    }
    if is_key_pressed(KeyCode::Equal) || is_key_pressed(KeyCode::KpAdd) {
        player.speed = (player.speed * 2.0).min(16.0);
    }
    if is_key_pressed(KeyCode::Minus) || is_key_pressed(KeyCode::KpSubtract) {
        player.speed = (player.speed / 2.0).max(0.25);
    }
    if player.paused && is_key_pressed(KeyCode::Period) {
        player.advance_ticks(1.0);
    }
}

fn order_controls(app: &mut App) {
    if app.my_hero().is_none() || !app.controls_selection() {
        return;
    }
    // A player's hero cannot pan by keys, so the letters are free for orders.
    if is_key_pressed(KeyCode::A) {
        app.attack_move_armed = true;
    }
    if is_key_pressed(KeyCode::S) {
        app.attack_move_armed = false;
        app.aiming = None;
        app.send_order(Order::Move {
            target: Target::None,
        });
    }
    if is_key_pressed(KeyCode::H) {
        app.attack_move_armed = false;
        app.send_order(Order::Attack {
            target: Target::None,
        });
    }
    ability_keys(app);
    item_keys(app);
    let (sx, sy) = mouse_position();
    let (wx, wy) = app
        .camera
        .screen_to_world(sx, sy, screen_width(), screen_height());
    let ground = world_vec(wx, wy);
    // A slot taken up spends the next left click, on the ground or on a unit.
    if let Some(slot) = app.aiming
        && is_mouse_button_pressed(MouseButton::Left)
    {
        app.aiming = None;
        let target = match app.aim_in(slot) {
            // A tree and a landing spot are both named by the ground under
            // the cursor; which one was meant is the server's to settle.
            Some(Aim::Point | Aim::Tree | Aim::Building) => Some(Target::Pos(ground)),
            // One's own hero is a target like any other here: a salve is
            // drunk by clicking the one drinking it.
            Some(Aim::Unit) => app
                .view
                .as_ref()
                .and_then(|view| unit_under_cursor(view, wx, wy, None, true))
                .map(Target::Unit),
            Some(Aim::Own) | None => Some(Target::None),
        };
        if let Some(target) = target {
            app.aimed_from = Some((app.seq + 1, slot));
            app.send_order(order_for(slot, target));
        }
        return;
    }
    if app.attack_move_armed && is_mouse_button_pressed(MouseButton::Left) {
        app.attack_move_armed = false;
        // A unit under the cursor takes the attack order itself: on an ally
        // this is the aggro-drop click. Near an enemy the order sticks to it.
        let me = app.my_hero();
        let target = app.view.as_ref().and_then(|view| {
            unit_under_cursor(view, wx, wy, me, true)
                .or_else(|| enemy_near_cursor(view, wx, wy, app.my_team()))
        });
        match target {
            Some(target) => app.send_order(Order::Attack {
                target: Target::Unit(target),
            }),
            None => app.send_order(Order::Attack {
                target: Target::Pos(ground),
            }),
        }
        return;
    }
    if is_mouse_button_pressed(MouseButton::Right) {
        app.attack_move_armed = false;
        app.aiming = None;
        let me = app.my_hero();
        let target = app
            .view
            .as_ref()
            .and_then(|view| unit_under_cursor(view, wx, wy, me, true));
        // A right click reads the ground the way Dota does: an enemy under
        // the cursor is attacked, one of this side's own is followed, an
        // item lying there is picked up, open ground is walked to. Denying
        // is the attack click's business.
        let lying = match target {
            Some(_) => None,
            None => app
                .view
                .as_ref()
                .and_then(|view| loot_under_cursor(view, wx, wy)),
        };
        let own = target.is_some_and(|id| {
            app.view
                .as_ref()
                .and_then(|view| view.units.iter().find(|unit| unit.id == id))
                .is_some_and(|unit| Some(unit.team) == app.my_team())
        });
        match (target, lying) {
            (Some(target), _) if own => app.send_order(Order::Move {
                target: Target::Unit(target),
            }),
            (Some(target), _) => app.send_order(Order::Attack {
                target: Target::Unit(target),
            }),
            (None, Some(item)) => app.send_order(Order::Take {
                target: Target::Unit(item),
            }),
            // Near an enemy the click still attacks it; only open ground is
            // walked to.
            (None, None) => {
                let near = app
                    .view
                    .as_ref()
                    .and_then(|view| enemy_near_cursor(view, wx, wy, app.my_team()));
                match near {
                    Some(mark) => app.send_order(Order::Attack {
                        target: Target::Unit(mark),
                    }),
                    None => app.send_order(Order::Move {
                        target: Target::Pos(ground),
                    }),
                }
            }
        }
    }
}

/// The ground item nearest the cursor, if the cursor is close enough.
pub fn loot_under_cursor(view: &WorldView, wx: f32, wy: f32) -> Option<EntityId> {
    let mut best: Option<(f32, EntityId)> = None;
    for lying in &view.loot {
        let dx = lying.pos.x.to_f32() - wx;
        let dy = lying.pos.y.to_f32() - wy;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist <= 35.0 && best.is_none_or(|(b, _)| dist < b) {
            best = Some((dist, lying.id));
        }
    }
    best.map(|(_, id)| id)
}

/// The number keys use the six inventory slots.
fn item_keys(app: &mut App) {
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

/// Answers a press of one slot, however it was pressed.
///
/// The one door a key and a click both go through, so neither can grow a rule
/// the other does not have. What a press means is [`press`]; nothing is asked
/// here about whether the order would be carried out.
pub fn do_press(app: &mut App, slot: Slot, ctrl: bool) {
    match press(app, slot, ctrl) {
        Press::Send(order) => {
            app.aiming = None;
            app.aimed_from = Some((app.seq + 1, slot));
            app.send_order(order);
        }
        Press::Aim(slot) => {
            app.attack_move_armed = false;
            app.aiming = Some(slot);
        }
        Press::Nothing => {}
    }
}

/// Q, W, E, R, T and G cast the slots of whatever is selected; with Control
/// held they spend a skill point instead. One that is aimed arms and waits
/// for a click. Nothing is sent for a unit this seat does not drive.
fn ability_keys(app: &mut App) {
    let keys = [
        (KeyCode::Q, 0u8),
        (KeyCode::W, 1),
        (KeyCode::E, 2),
        (KeyCode::R, 3),
        (KeyCode::T, 4),
        (KeyCode::G, 5),
    ];
    // What is selected may be looked at whatever it is; the keys answer only
    // for what this seat drives.
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

/// The clickable unit nearest the cursor, if the cursor is close enough.
///
/// `exclude` names a unit that never matches — one's own hero when aiming an
/// order, so a click on it is a move like any other. `for_orders` also skips
/// what cannot be a target at all; selection wants those too.
pub fn unit_under_cursor(
    view: &WorldView,
    wx: f32,
    wy: f32,
    exclude: Option<EntityId>,
    for_orders: bool,
) -> Option<EntityId> {
    let mut best: Option<(f32, EntityId)> = None;
    for u in &view.units {
        if Some(u.id) == exclude || (for_orders && u.kind == UnitKind::Fountain) {
            continue;
        }
        let dx = u.pos.x.to_f32() - wx;
        let dy = u.pos.y.to_f32() - wy;
        let dist = (dx * dx + dy * dy).sqrt();
        let slack = u.radius.to_f32().max(20.0) + 15.0;
        if dist <= slack && best.is_none_or(|(b, _)| dist < b) {
            best = Some((dist, u.id));
        }
    }
    best.map(|(_, id)| id)
}

/// How far past a body's edge an attack click still sticks to it, in world
/// units.
const ATTACK_SNAP: f32 = 25.0;

/// The enemy nearest the cursor within the attack snap, if one is that near.
///
/// Only enemies stick. An order at one of this side's own — a follow, an
/// aggro drop or a deny — is asked for by the click that lands on the body
/// itself.
pub fn enemy_near_cursor(
    view: &WorldView,
    wx: f32,
    wy: f32,
    mine: Option<Team>,
) -> Option<EntityId> {
    let mut best: Option<(f32, EntityId)> = None;
    for u in &view.units {
        if u.kind == UnitKind::Fountain || Some(u.team) == mine {
            continue;
        }
        let dx = u.pos.x.to_f32() - wx;
        let dy = u.pos.y.to_f32() - wy;
        let dist = (dx * dx + dy * dy).sqrt();
        let reach = u.radius.to_f32() + ATTACK_SNAP;
        if dist <= reach && best.is_none_or(|(b, _)| dist < b) {
            best = Some((dist, u.id));
        }
    }
    best.map(|(_, id)| id)
}

/// A world position as the wire carries it.
pub fn world_vec(x: f32, y: f32) -> Vec2 {
    Vec2 {
        x: bota_proto::Fixed {
            raw: (x * 65536.0) as i32,
        },
        y: bota_proto::Fixed {
            raw: (y * 65536.0) as i32,
        },
    }
}
