//! The debug viewer's layout, arguments and art, tested without a window.

use bota_proto::{EntityId, HeroId, PlayerView, SlotId, Team};
use clap::Parser;

use crate::Args;
use crate::camera::Camera;

#[test]
fn the_camera_round_trips_and_shows_greater_world_y_higher() {
    let cam = Camera {
        x: 4096.0,
        y: 4096.0,
        zoom: 0.45,
    };
    let (sw, sh) = (1280.0, 800.0);
    for (wx, wy) in [
        (0.0, 0.0),
        (4096.0, 4096.0),
        (8192.0, 1.0),
        (123.5, 7000.25),
    ] {
        let (sx, sy) = cam.world_to_screen(wx, wy, sw, sh);
        let (bx, by) = cam.screen_to_world(sx, sy, sw, sh);
        assert!((bx - wx).abs() < 0.01, "{bx} vs {wx}");
        assert!((by - wy).abs() < 0.01, "{by} vs {wy}");
    }
    let (_, low) = cam.world_to_screen(4096.0, 0.0, sw, sh);
    let (_, high) = cam.world_to_screen(4096.0, 8192.0, sw, sh);
    assert!(high < low, "greater world Y must sit higher on the screen");
}

#[test]
fn zoom_stays_within_its_limits() {
    let mut cam = Camera::over(0.0, 0.0);
    for _ in 0..100 {
        cam.zoom_by(1.0);
    }
    assert!(cam.zoom <= crate::camera::ZOOM_MAX);
    for _ in 0..200 {
        cam.zoom_by(-1.0);
    }
    assert!(cam.zoom >= crate::camera::ZOOM_MIN);
}

#[test]
fn portraits_split_the_teams_around_the_clock() {
    let players = [Team::Radiant, Team::Dire, Team::Radiant, Team::Dire]
        .into_iter()
        .zip(0..)
        .map(|(team, slot)| PlayerView {
            team,
            ..a_seat(SlotId(slot), None)
        })
        .collect::<Vec<_>>();
    let sw = 1280.0;
    let rects = crate::hud::top_portraits(&players, sw);
    assert_eq!(rects.len(), 4);
    for (slot, rect) in &rects {
        if slot.0.is_multiple_of(2) {
            assert!(rect.x + rect.w < sw / 2.0, "radiant sits left of the clock");
        } else {
            assert!(rect.x > sw / 2.0, "dire sits right of the clock");
        }
        assert!(rect.contains(rect.x + 1.0, rect.y + 1.0));
        assert!(!rect.contains(rect.x - 1.0, rect.y));
    }
    for (i, (_, a)) in rects.iter().enumerate() {
        for (_, b) in &rects[i + 1..] {
            let apart = a.x + a.w <= b.x || b.x + b.w <= a.x;
            assert!(apart, "{a:?} vs {b:?}");
        }
    }
}

#[test]
fn the_bottom_panel_stays_on_screen() {
    let rect = crate::hud::bottom_panel(1280.0, 800.0);
    assert!(rect.x >= 0.0 && rect.y >= 0.0);
    assert!(rect.x + rect.w <= 1280.0);
    assert!(rect.y + rect.h <= 800.0);
    let narrow = crate::hud::bottom_panel(500.0, 400.0);
    assert!(narrow.w <= 500.0 - 40.0 + 0.5);
}

#[test]
fn arguments_parse_and_default() {
    let args = |list: &[&str]| {
        let mut whole = vec!["bota-client"];
        whole.extend_from_slice(list);
        Args::try_parse_from(whole)
    };
    let defaults = args(&[]).expect("no arguments is fine");
    assert_eq!(defaults.addr, "127.0.0.1:4455");
    assert_eq!(defaults.name, "player");
    assert!(!defaults.spectate);
    assert_eq!(defaults.replay, None);

    let full = args(&["--addr", "10.0.0.2:5000", "--name", "alice", "--spectate"])
        .expect("all flags are known");
    assert_eq!(full.addr, "10.0.0.2:5000");
    assert_eq!(full.name, "alice");
    assert!(full.spectate);

    let watching = args(&["--replay", "m.brp"]).expect("a replay is a whole way to run");
    assert_eq!(watching.replay, Some(std::path::PathBuf::from("m.brp")));

    assert!(args(&["--what"]).is_err(), "an unknown flag is refused");
    assert!(args(&["--addr"]).is_err(), "a missing value is refused");
    assert!(
        args(&["--replay", "m.brp", "--spectate"]).is_err(),
        "a file to play and a match to watch are two different runs"
    );
}

#[test]
fn every_item_drawing_rasterises_to_something_visible() {
    for face in bota_client_core::ITEMS {
        let name = face.name;
        let art = crate::icons::item_art(face.id).expect("every item has a drawing");
        let (w, h, bytes) = crate::icons::pixels(art).expect("the drawing is readable");
        assert_eq!(w, 192, "{name} is drawn to the frame");
        assert_eq!(h, 128, "{name} is drawn to the frame");
        assert_eq!(bytes.len(), (w * h * 4) as usize);
        let painted = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[3] > 0)
            .count();
        assert!(
            painted > (w * h / 2) as usize,
            "{name} covers its frame: {painted} of {}",
            w * h
        );
    }
}

#[test]
fn every_ability_box_has_a_key_to_press() {
    let panel = crate::hud::bottom_panel(1280.0, 800.0);
    let boxes = crate::hud::ability_boxes(&panel);
    assert_eq!(boxes.len(), usize::from(crate::hud::ABILITY_BOXES));
    for (slot, _) in boxes {
        assert!(
            crate::render::ability_key(usize::from(slot)).is_some(),
            "slot {slot} has a key"
        );
    }
}

#[test]
fn no_two_boxes_of_the_panel_sit_on_one_another() {
    let panel = crate::hud::bottom_panel(1280.0, 800.0);
    let mut boxes: Vec<(String, crate::hud::UiRect)> = Vec::new();
    for (slot, r) in crate::hud::ability_boxes(&panel) {
        boxes.push((format!("ability {slot}"), r));
    }
    for (slot, r) in crate::hud::item_boxes(&panel) {
        boxes.push((format!("item {slot}"), r));
    }
    for (slot, r) in crate::hud::stash_boxes(&panel) {
        boxes.push((format!("stash {slot}"), r));
    }
    for (one, a) in &boxes {
        for (other, b) in &boxes {
            if one == other {
                continue;
            }
            let apart =
                a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
            assert!(
                apart,
                "{one} and {other} overlap: a click would land on both"
            );
        }
    }
    // The panel's own boxes stay inside it; the stash strip sits above it.
    for (which, r) in boxes.iter().filter(|(name, _)| !name.starts_with("stash")) {
        assert!(
            r.x >= panel.x
                && r.y >= panel.y
                && r.x + r.w <= panel.x + panel.w
                && r.y + r.h <= panel.y + panel.h,
            "{which} hangs off the panel"
        );
    }
}

#[test]
fn the_cursor_pushes_the_camera_only_at_the_very_edge() {
    let (sw, sh, pan) = (1280.0, 800.0, 9.0);
    for (at, push, why) in [
        ((640.0, 400.0), (0.0, 0.0), "the middle pushes nothing"),
        ((1.0, 400.0), (-pan, 0.0), "the left edge pushes left"),
        ((sw - 1.0, 400.0), (pan, 0.0), "the right edge right"),
        ((640.0, 1.0), (0.0, -pan), "the top edge up"),
        ((1.0, sh - 1.0), (-pan, pan), "a corner both ways at once"),
        ((-4.0, 400.0), (0.0, 0.0), "one out of the window nothing"),
        ((640.0, sh + 40.0), (0.0, 0.0), "nor one below it"),
    ] {
        let pushed = crate::input::edge_of(at.0, at.1, sw, sh, pan);
        assert_eq!(pushed, push, "{why}");
    }
}

/// A seat with the body it stands in, or none while it is down.
fn a_seat(slot: SlotId, unit: Option<EntityId>) -> PlayerView {
    let mut seat = a_player();
    seat.slot = slot;
    seat.unit = unit;
    seat
}

fn a_player() -> PlayerView {
    PlayerView {
        slot: SlotId(0),
        team: Team::Radiant,
        hero: HeroId(0),
        unit: None,
        level: 1,
        xp: 0,
        gold: Some(600),
        stash: Some(vec![None; 6]),
        kit: None,
        kills: 0,
        deaths: 0,
        assists: 0,
        last_hits: 0,
        denies: 0,
        respawn_left: 0,
    }
}
