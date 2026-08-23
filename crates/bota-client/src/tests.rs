//! Client logic that works without a window.

use bota_proto::{
    AbilityId, AbilitySlot, AbilityView, Aim, Angle, Attributes, EntityId, Fixed, HeroId, ItemId,
    ItemSlot, ItemView, Order, OrderTarget, PlayerView, ShopEntry, SlotId, StatusFlags, Team,
    UnitKind, UnitView, Vec2, WorldView,
};
use clap::Parser;

use crate::slots::{Look, Press, Slot, ability_look, decide, item_look};
use crate::state::{Tap, body_in, known_in, remember, tap_again};

use crate::Args;
use crate::camera::Camera;

#[test]
fn camera_transforms_round_trip() {
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
}

#[test]
fn the_world_y_axis_points_up_on_screen() {
    let cam = Camera {
        x: 4096.0,
        y: 4096.0,
        zoom: 0.45,
    };
    let (_, low) = cam.world_to_screen(4096.0, 0.0, 1280.0, 800.0);
    let (_, high) = cam.world_to_screen(4096.0, 8192.0, 1280.0, 800.0);
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
    use bota_proto::{HeroId, PlayerView, SlotId, Team};
    let player = |slot: u8, team: Team| PlayerView {
        slot: SlotId(slot),
        team,
        hero: HeroId(0),
        unit: None,
        level: 1,
        xp: 0,
        gold: None,
        stash: None,
        kit: None,
        kills: 0,
        deaths: 0,
        assists: 0,
        last_hits: 0,
        denies: 0,
        respawn_left: 0,
    };
    let players = [
        player(0, Team::Radiant),
        player(1, Team::Dire),
        player(2, Team::Radiant),
        player(3, Team::Dire),
    ];
    let sw = 1280.0;
    let rects = crate::hud::top_portraits(&players, sw);
    assert_eq!(rects.len(), 4);
    for (slot, rect) in &rects {
        let radiant = slot.0.is_multiple_of(2);
        if radiant {
            assert!(rect.x + rect.w < sw / 2.0, "radiant sits left of the clock");
        } else {
            assert!(rect.x > sw / 2.0, "dire sits right of the clock");
        }
        assert!(rect.contains(rect.x + 1.0, rect.y + 1.0));
        assert!(!rect.contains(rect.x - 1.0, rect.y));
    }
    // No two portraits overlap.
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
    for face in crate::catalog::ITEMS {
        let name = face.name;
        let art = face.icon.expect("every item is drawn");
        let (w, h, bytes) = crate::icons::pixels(art).expect("the drawing is readable");
        assert_eq!(w, 192, "{name} is drawn to the frame");
        assert_eq!(h, 128, "{name} is drawn to the frame");
        assert_eq!(bytes.len(), (w * h * 4) as usize);
        let painted = bytes.chunks_exact(4).filter(|px| px[3] > 0).count();
        assert!(
            painted > (w * h / 2) as usize,
            "{name} covers its frame: {painted} of {}",
            w * h
        );
    }
}

#[test]
fn every_ability_box_has_a_key_to_press() {
    let panel = crate::hud::UiRect {
        x: 0.0,
        y: 0.0,
        w: 1600.0,
        h: 200.0,
    };
    let boxes = crate::hud::ability_boxes(&panel);
    assert_eq!(
        boxes.len(),
        usize::from(crate::hud::ABILITY_BOXES),
        "the panel draws as many boxes as it says it has"
    );
    for (slot, _) in boxes {
        assert!(
            crate::render::ability_key(usize::from(slot)).is_some(),
            "slot {slot} has a key"
        );
    }
}

#[test]
fn every_catalog_entry_answers_to_its_own_id() {
    for (index, face) in crate::catalog::ABILITIES.iter().enumerate() {
        assert_eq!(
            usize::from(face.id),
            index,
            "ability {} is in place",
            face.name
        );
        assert_eq!(
            crate::catalog::ability(face.id).map(|found| found.name),
            Some(face.name)
        );
    }
    for (index, face) in crate::catalog::ITEMS.iter().enumerate() {
        assert_eq!(
            usize::from(face.id),
            index,
            "item {} is in place",
            face.name
        );
        assert_eq!(
            crate::catalog::item(face.id).map(|found| found.name),
            Some(face.name)
        );
    }
    for (index, face) in crate::catalog::EFFECTS.iter().enumerate() {
        assert_eq!(
            usize::from(face.id),
            index,
            "effect {} is in place",
            face.name
        );
        assert_eq!(
            crate::catalog::effect(face.id).map(|found| found.name),
            Some(face.name)
        );
    }
    for (index, face) in crate::catalog::HEROES.iter().enumerate() {
        assert_eq!(
            usize::from(face.id),
            index,
            "hero {} is in place",
            face.name
        );
    }
}

#[test]
fn every_catalog_entry_has_something_to_say() {
    for face in crate::catalog::ABILITIES {
        assert!(
            !face.name.is_empty() && !face.blurb.is_empty(),
            "{}",
            face.id
        );
    }
    for face in crate::catalog::ITEMS {
        assert!(
            !face.name.is_empty() && !face.blurb.is_empty() && !face.stats.is_empty(),
            "{}",
            face.id
        );
    }
    for face in crate::catalog::EFFECTS {
        assert!(
            !face.name.is_empty() && !face.blurb.is_empty(),
            "{}",
            face.id
        );
    }
    for face in crate::catalog::HEROES {
        assert!(!face.name.is_empty(), "{}", face.id);
    }
}

#[test]
fn a_part_in_hand_comes_off_what_the_shop_asks() {
    let shop = vec![
        ShopEntry {
            id: ItemId(0),
            cost: 500,
            components: Vec::new(),
        },
        ShopEntry {
            id: ItemId(19),
            cost: 450,
            components: Vec::new(),
        },
        ShopEntry {
            id: ItemId(13),
            cost: 450,
            components: Vec::new(),
        },
        ShopEntry {
            id: ItemId(29),
            cost: 1400,
            components: vec![ItemId(0), ItemId(19), ItemId(13)],
        },
    ];
    assert_eq!(
        crate::catalog::price_for(&shop, ItemId(29), &[]),
        1400,
        "with nothing in hand the whole is asked for"
    );
    assert_eq!(
        crate::catalog::price_for(&shop, ItemId(29), &[ItemId(0)]),
        900,
        "the boots already worn are not asked for twice"
    );
    assert_eq!(
        crate::catalog::price_for(&shop, ItemId(0), &[ItemId(0)]),
        500,
        "but a second of what is bought whole costs the whole of it"
    );
    assert_eq!(
        crate::catalog::price_for(&shop, ItemId(29), &[ItemId(29)]),
        1400,
        "and a second built one is built out of fresh parts"
    );
}

/// A hero body with every field named, so a new one in `UnitView` has to be
/// thought about here before these tests compile again.
pub fn a_unit() -> UnitView {
    UnitView {
        id: EntityId {
            idx: 3,
            generation: 1,
        },
        kind: UnitKind::Hero,
        team: Team::Radiant,
        pos: Vec2 {
            x: Fixed::from_int(5000),
            y: Fixed::from_int(5000),
        },
        facing: Angle::default(),
        hp: 600,
        max_hp: 620,
        mana: 300,
        max_mana: 300,
        move_speed: Fixed::from_int(300),
        attack_damage: 55,
        attack_range: Fixed::from_int(600),
        attack_interval: 51,
        attack_speed: 100,
        armor: Fixed::from_int(3),
        magic_resist: Fixed::from_ratio(25, 100),
        radius: Fixed::from_int(24),
        vision_radius: Fixed::from_int(1800),
        true_sight_radius: Fixed::ZERO,
        statuses: StatusFlags::default(),
        attributes: Attributes::ZERO,
        primary: None,
        hero: Some(HeroId(0)),
        owner: Some(SlotId(0)),
        level: 4,
        abilities: Vec::new(),
        items: vec![None; 9],
        effects: Vec::new(),
    }
}

/// A slot holding something aimed the given way.
fn holding(aim: Option<Aim>) -> Option<Option<Aim>> {
    Some(aim)
}

/// The one this seat commands, for the self-target case.
fn me() -> Option<EntityId> {
    Some(EntityId {
        idx: 3,
        generation: 1,
    })
}

#[test]
fn a_press_is_sent_whatever_state_the_slot_is_in() {
    // Nothing here looks at cooldown, mana or points: the server is what
    // turns a press down, and it has to be reached to do it.
    let slot = Slot::Ability(0);
    assert_eq!(
        decide(slot, false, holding(Some(Aim::Own)), None, me()),
        Press::Send(Order::CastAbility {
            slot: AbilitySlot(0),
            target: OrderTarget::None,
        }),
        "one that needs no aiming goes at once"
    );
    assert_eq!(
        decide(slot, false, holding(None), None, me()),
        Press::Send(Order::CastAbility {
            slot: AbilitySlot(0),
            target: OrderTarget::None,
        }),
        "and so does a passive, which the server answers for"
    );
    assert_eq!(
        decide(slot, false, holding(Some(Aim::Point)), None, me()),
        Press::Aim(slot),
        "one that is aimed waits for the click"
    );
}

#[test]
fn control_spends_a_point_instead_of_casting() {
    assert_eq!(
        decide(
            Slot::Ability(2),
            true,
            holding(Some(Aim::Point)),
            None,
            me()
        ),
        Press::Send(Order::LevelUpAbility {
            slot: AbilitySlot(2),
        }),
        "held down, control levels whatever the slot is"
    );
    assert_eq!(
        decide(Slot::Item(1), true, holding(Some(Aim::Own)), None, me()),
        Press::Send(Order::UseItem {
            slot: ItemSlot(1),
            target: OrderTarget::None,
        }),
        "an item has no points to spend, so control is nothing to it"
    );
}

#[test]
fn an_empty_slot_and_a_pocket_answer_to_nothing() {
    assert_eq!(
        decide(Slot::Ability(0), false, None, None, me()),
        Press::Nothing,
        "nothing in the slot, nothing to press"
    );
    assert_eq!(
        decide(Slot::Item(7), false, holding(Some(Aim::Own)), None, me()),
        Press::Nothing,
        "the backpack is reached by dragging, not by pressing"
    );
    assert_eq!(
        decide(Slot::Item(11), false, holding(Some(Aim::Own)), None, me()),
        Press::Nothing,
        "and so is the stash"
    );
}

#[test]
fn reaching_twice_for_what_is_aimed_at_a_unit_aims_it_at_oneself() {
    let slot = Slot::Item(0);
    assert_eq!(
        decide(slot, false, holding(Some(Aim::Unit)), None, me()),
        Press::Aim(slot),
        "the first press takes it up"
    );
    assert_eq!(
        decide(slot, false, holding(Some(Aim::Unit)), Some(slot), me()),
        Press::Send(Order::UseItem {
            slot: ItemSlot(0),
            target: OrderTarget::Unit {
                target: me().expect("a commander"),
            },
        }),
        "the second drinks it"
    );
    assert_eq!(
        decide(slot, false, holding(Some(Aim::Point)), Some(slot), me()),
        Press::Aim(slot),
        "one aimed at the ground has no such shortcut"
    );
}

/// An ability slot with the numbers given.
fn an_ability(level: u8, cooldown: u32, mana_cost: i32, passive: bool, on: bool) -> AbilityView {
    AbilityView {
        id: AbilityId(0),
        level,
        max_level: 4,
        cooldown_left: cooldown,
        mana_cost,
        range: 600,
        aim: Aim::Unit,
        passive,
        on,
        can_level: false,
    }
}

#[test]
fn what_keeps_an_ability_from_working_is_told_apart_from_what_it_is() {
    let ready = ability_look(Some(&an_ability(1, 0, 50, false, false)), 100);
    assert!(ready.filled && ready.learned && ready.usable, "ready");
    assert!(!ready.passive && !ready.toggled);

    let unlearned = ability_look(Some(&an_ability(0, 0, 50, false, false)), 100);
    assert!(unlearned.filled, "it is still in the slot");
    assert!(!unlearned.learned, "with no points in it");
    assert!(!unlearned.usable, "which is also why it cannot be used");

    let waiting = ability_look(Some(&an_ability(1, 45, 50, false, false)), 100);
    assert!(waiting.learned, "learned all the same");
    assert!(!waiting.usable);
    assert_eq!(waiting.cooldown_left, 45, "and the wait is worth showing");

    let poor = ability_look(Some(&an_ability(1, 0, 50, false, false)), 20);
    assert!(poor.learned && !poor.usable, "mana keeps it from working");
    assert_eq!(poor.cooldown_left, 0, "with nothing on the clock");

    let idle = ability_look(Some(&an_ability(1, 0, 0, true, false)), 100);
    assert!(idle.passive && !idle.usable, "a passive is never used");

    let running = ability_look(Some(&an_ability(1, 0, 0, false, true)), 100);
    assert!(running.toggled, "a toggle that is on says so");

    assert_eq!(
        ability_look(None, 100),
        Look::default(),
        "a slot this one does not carry is empty"
    );
}

/// An item view with the numbers given.
fn an_item(charges: Option<u8>, cooldown: u32, mana_cost: i32, aim: Option<Aim>) -> ItemView {
    ItemView {
        id: ItemId(0),
        charges,
        cooldown_left: cooldown,
        mode: None,
        mana_cost,
        range: 0,
        aim,
    }
}

#[test]
fn an_item_with_no_charges_at_all_is_not_an_empty_one() {
    let boots = an_item(None, 0, 0, None);
    let worn = item_look(Some(&boots), 0, 0);
    assert!(worn.filled && worn.passive, "boots are worn, not used");
    assert!(!worn.usable, "and pressing them does nothing");

    let stick = an_item(Some(0), 0, 0, Some(Aim::Own));
    assert!(
        !item_look(Some(&stick), 0, 0).usable,
        "an empty stack cannot be spent"
    );
    let charged = an_item(Some(3), 0, 0, Some(Aim::Own));
    assert!(
        item_look(Some(&charged), 0, 0).usable,
        "one with charges can"
    );
    assert!(
        !item_look(Some(&charged), 7, 0).usable,
        "the same stack in the backpack cannot"
    );

    let dear = an_item(None, 0, 75, Some(Aim::Own));
    assert!(!item_look(Some(&dear), 0, 50).usable, "mana is short");
    assert!(item_look(Some(&dear), 0, 75).usable, "and now it is not");

    let waiting = an_item(None, 30, 0, Some(Aim::Own));
    let look = item_look(Some(&waiting), 0, 0);
    assert!(
        !look.usable && look.cooldown_left == 30,
        "still on the clock"
    );

    assert_eq!(item_look(None, 0, 0), Look::default(), "an empty slot");
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
    // The panel's own boxes stay inside it. The stash strip is drawn above
    // the panel on purpose and is left out of this.
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
fn a_second_reach_for_the_same_thing_makes_a_pair() {
    let hero = Tap::Hero;
    let (again, kept) = tap_again(None, hero);
    assert!(!again, "the first press is only a press");
    assert!(kept.is_some(), "and it is remembered for a moment");

    let (again, kept) = tap_again(kept, hero);
    assert!(again, "the second makes the pair");
    assert!(
        kept.is_none(),
        "which is spent: a third press starts over rather than pairing again"
    );

    let (again, kept) = tap_again(kept, hero);
    assert!(!again, "so the third is a single press");
    let (again, _) = tap_again(kept, Tap::Courier);
    assert!(!again, "and reaching for something else never pairs");
}

#[test]
fn a_pair_of_reaches_for_two_different_things_is_no_pair() {
    let one = Tap::Unit(EntityId {
        idx: 4,
        generation: 1,
    });
    let other = Tap::Unit(EntityId {
        idx: 5,
        generation: 1,
    });
    let (_, kept) = tap_again(None, one);
    let (again, _) = tap_again(kept, other);
    assert!(!again, "two units clicked in turn are two picks");
}

#[test]
fn the_cursor_pushes_the_camera_only_at_the_very_edge() {
    let (sw, sh, pan) = (1280.0, 800.0, 9.0);
    assert_eq!(
        crate::input::edge_of(640.0, 400.0, sw, sh, pan),
        (0.0, 0.0),
        "the middle of the screen pushes nothing"
    );
    assert_eq!(
        crate::input::edge_of(1.0, 400.0, sw, sh, pan),
        (-pan, 0.0),
        "the left edge pushes left"
    );
    assert_eq!(
        crate::input::edge_of(sw - 1.0, 400.0, sw, sh, pan),
        (pan, 0.0),
        "and the right edge right"
    );
    assert_eq!(
        crate::input::edge_of(640.0, 1.0, sw, sh, pan),
        (0.0, -pan),
        "the top edge pushes up"
    );
    assert_eq!(
        crate::input::edge_of(1.0, sh - 1.0, sw, sh, pan),
        (-pan, pan),
        "a corner pushes both ways at once"
    );
    assert_eq!(
        crate::input::edge_of(-4.0, 400.0, sw, sh, pan),
        (0.0, 0.0),
        "a cursor gone out of the window pushes nothing"
    );
    assert_eq!(
        crate::input::edge_of(640.0, sh + 40.0, sw, sh, pan),
        (0.0, 0.0),
        "and neither does one below it"
    );
}

/// A seat with every field named, so a new one in `PlayerView` has to be
/// thought about here before these tests compile again.
pub fn a_player() -> PlayerView {
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

/// A tick holding these units and these seats.
fn a_tick(tick: u32, units: Vec<UnitView>, players: Vec<PlayerView>) -> WorldView {
    WorldView {
        tick,
        viewer: None,
        units,
        projectiles: Vec::new(),
        players,
        felled_trees: Vec::new(),
        planted_trees: Vec::new(),
    }
}

/// A seat with the body it stands in, or none while it is down.
fn a_seat(slot: SlotId, unit: Option<EntityId>) -> PlayerView {
    let mut seat = a_player();
    seat.slot = slot;
    seat.unit = unit;
    seat
}

/// A unit of one kind belonging to one seat.
fn owned(idx: u32, kind: UnitKind, slot: Option<SlotId>) -> UnitView {
    let mut body = a_unit();
    body.id = EntityId { idx, generation: 1 };
    body.kind = kind;
    body.owner = slot;
    body
}

#[test]
fn only_what_a_seat_owns_is_worth_remembering() {
    let mut seen = Vec::new();
    let hero = owned(1, UnitKind::Hero, Some(SlotId(0)));
    let creep = owned(2, UnitKind::CreepMelee, None);
    remember(&mut seen, &a_tick(10, vec![hero, creep], Vec::new()));
    assert_eq!(seen.len(), 1, "the creep is not kept, the hero is");
    assert_eq!(seen[0].1.kind, UnitKind::Hero);
}

#[test]
fn a_unit_out_of_sight_is_known_as_it_was_and_how_long_ago() {
    let mut seen = Vec::new();
    let hero = owned(1, UnitKind::Hero, Some(SlotId(0)));
    let id = hero.id;
    let seat = a_seat(SlotId(0), Some(id));
    remember(&mut seen, &a_tick(10, vec![hero], vec![seat.clone()]));

    let live = a_tick(10, Vec::new(), vec![seat.clone()]);
    let (held, stale) = known_in(&seen, &live, id).expect("it is remembered");
    assert_eq!(held.id, id, "the body it was is still there to look at");
    assert_eq!(stale, 0, "and this tick it is as fresh as the memory");

    let later = a_tick(70, Vec::new(), vec![seat]);
    let (_, stale) = known_in(&seen, &later, id).expect("still remembered");
    assert_eq!(stale, 60, "sixty ticks later it is sixty ticks old");
}

#[test]
fn what_stands_now_is_known_before_what_was_remembered() {
    let mut seen = Vec::new();
    let was = owned(1, UnitKind::Hero, Some(SlotId(0)));
    let id = was.id;
    remember(&mut seen, &a_tick(10, vec![was], Vec::new()));
    let mut now = owned(1, UnitKind::Hero, Some(SlotId(0)));
    now.hp = 42;
    let view = a_tick(90, vec![now], Vec::new());
    let (held, stale) = known_in(&seen, &view, id).expect("it stands");
    assert_eq!(held.hp, 42, "the live body wins over the kept one");
    assert_eq!(stale, 0, "and nothing about it is old");
}

#[test]
fn a_seat_with_its_hero_down_is_still_picked_by_the_body_it_left() {
    let mut seen = Vec::new();
    let hero = owned(1, UnitKind::Hero, Some(SlotId(0)));
    let id = hero.id;
    remember(
        &mut seen,
        &a_tick(10, vec![hero], vec![a_seat(SlotId(0), Some(id))]),
    );
    let dead = a_tick(60, Vec::new(), vec![a_seat(SlotId(0), None)]);
    assert_eq!(
        body_in(&seen, &dead, SlotId(0)),
        Some(id),
        "the fallen body is the handle the seat is picked by"
    );
    assert_eq!(
        body_in(&seen, &dead, SlotId(1)),
        None,
        "a seat nothing is known of is picked by nothing"
    );
}

#[test]
fn a_hero_that_comes_back_leaves_only_one_body_behind() {
    let mut seen = Vec::new();
    let first = owned(1, UnitKind::Hero, Some(SlotId(0)));
    remember(&mut seen, &a_tick(10, vec![first], Vec::new()));
    let again = owned(2, UnitKind::Hero, Some(SlotId(0)));
    let fresh = again.id;
    remember(&mut seen, &a_tick(600, vec![again], Vec::new()));
    assert_eq!(seen.len(), 1, "the body it left is dropped for the new one");
    assert_eq!(seen[0].1.id, fresh);
    let view = a_tick(600, Vec::new(), vec![a_seat(SlotId(0), None)]);
    assert_eq!(
        body_in(&seen, &view, SlotId(0)),
        Some(fresh),
        "and the seat answers with the one it stood in last"
    );
}
