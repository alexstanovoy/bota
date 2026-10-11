//! The client core, tested without a window or a socket.

use crate::slots::{Look, Press, Slot, ability_look, decide, item_look, order_for};
use crate::state::{Tap, body_in, known_in, remember, tap_again};
use bota_proto::{
    AbilityId, AbilitySlot, AbilityView, Aim, Angle, Attributes, EntityId, Fixed, HeroId, ItemId,
    ItemView, Order, PlayerView, ShopEntry, SlotId, StatusFlags, Target, Team, UnitKind, UnitView,
    Vec2, WorldView,
};

#[test]
fn the_console_turns_a_typed_line_into_a_cheat_order() {
    use bota_proto::Cheat;
    let parse = crate::console::parse_console;
    for (line, cheat) in [
        ("-gold 500", Cheat::Gold { amount: 500 }),
        ("gold", Cheat::Gold { amount: 1000 }),
        ("-lvlup 3", Cheat::Levels { count: 3 }),
        ("-refresh", Cheat::Refresh),
        ("-item Butterfly", Cheat::Item { item: ItemId(46) }),
        ("-item 42", Cheat::Item { item: ItemId(42) }),
    ] {
        assert_eq!(parse(line), Ok(Order::Cheat { cheat }), "{line:?}");
    }
    for line in ["-fly", "-gold lots", "-item wings", ""] {
        assert!(parse(line).is_err(), "{line:?} is refused");
    }
}

#[test]
fn every_catalog_entry_is_named_and_found_by_its_own_id() {
    use crate::catalog::{ABILITIES, EFFECTS, HEROES, ITEMS, ability, effect, hero, item};
    let groups: [Vec<(u16, &str, Option<&str>)>; 3] = [
        ABILITIES
            .iter()
            .map(|f| (f.id, f.name, ability(f.id).map(|x| x.name)))
            .collect(),
        ITEMS
            .iter()
            .map(|f| (f.id, f.name, item(f.id).map(|x| x.name)))
            .collect(),
        HEROES
            .iter()
            .map(|f| (f.id, f.name, hero(f.id).map(|x| x.name)))
            .collect(),
    ];
    for group in groups {
        for (index, (id, name, found)) in group.into_iter().enumerate() {
            assert_eq!(usize::from(id), index, "{name} is in place");
            assert_eq!(found, Some(name), "{name} is found by its id");
            assert!(!name.is_empty(), "entry {id} has a name");
        }
    }
    for face in EFFECTS {
        assert_eq!(effect(face.id).map(|x| x.name), Some(face.name));
        assert!(!face.name.is_empty(), "effect {} has a name", face.id);
    }
    assert!(effect(0).is_none(), "the retired effect is gone");
    assert!(ability(ABILITIES.len() as u16).is_none());
    assert!(item(u16::MAX).is_none());
    assert!(effect(u16::MAX).is_none());
    assert!(hero(HEROES.len() as u16).is_none());
}

#[test]
fn a_part_in_hand_comes_off_what_the_shop_asks() {
    let entry = |id: u16, cost: i32, parts: &[u16]| ShopEntry {
        id: ItemId(id),
        cost,
        components: parts.iter().copied().map(ItemId).collect(),
    };
    let shop = vec![
        entry(0, 500, &[]),
        entry(19, 450, &[]),
        entry(13, 450, &[]),
        entry(29, 1400, &[0, 19, 13]),
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

/// A hero body with every field named.
fn a_unit() -> UnitView {
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
        attack_damage_min: 55,
        attack_damage_max: 55,
        attack_range: Fixed::from_int(600),
        attack_time: 1700,
        attack_point: 500,
        attack_speed: 100,
        armor: Fixed::from_int(3),
        magic_resist: Fixed::from_ratio(25, 100),
        collision: Fixed::from_int(27),
        bound: Fixed::from_int(24),
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

#[test]
fn a_press_means_what_its_slot_and_its_state_settle() {
    // Nothing here looks at cooldown, mana or points: the server is what
    // turns a press down, and it has to be reached to do it.
    let me = EntityId {
        idx: 3,
        generation: 1,
    };
    let own = Some(Some(Aim::Own));
    let point = Some(Some(Aim::Point));
    let unit = Some(Some(Aim::Unit));
    let (q, e, salve) = (Slot::Ability(0), Slot::Ability(2), Slot::Item(0));
    let cast = Press::Send(order_for(q, Target::None));
    let drink = |target| Press::Send(order_for(salve, target));
    let learn = Press::Send(Order::Learn {
        slot: AbilitySlot(2),
    });
    let cases = [
        // What needs no aim goes at once, and so does a passive.
        (q, false, own, None, cast.clone()),
        (q, false, Some(None), None, cast),
        // What is aimed waits for a click.
        (q, false, point, None, Press::Aim(q)),
        // Control levels an ability, and is nothing to an item.
        (e, true, point, None, learn),
        (salve, true, own, None, drink(Target::None)),
        // An empty slot, the backpack and the stash answer nothing.
        (q, false, None, None, Press::Nothing),
        (Slot::Item(7), false, own, None, Press::Nothing),
        (Slot::Item(11), false, own, None, Press::Nothing),
        // A salve is taken up, and a second press drinks it; what is aimed
        // at the ground has no such shortcut.
        (salve, false, unit, None, Press::Aim(salve)),
        (salve, false, unit, Some(salve), drink(Target::Unit(me))),
        (salve, false, point, Some(salve), Press::Aim(salve)),
    ];
    for (case, (slot, ctrl, held, aiming, expected)) in cases.into_iter().enumerate() {
        let pressed = decide(slot, ctrl, held, aiming, Some(me));
        assert_eq!(pressed, expected, "case {case}");
    }
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
        mute_left: 0,
        mode: None,
        mana_cost,
        range: 0,
        aim,
        for_sale: false,
        owner: SlotId(0),
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

    let unit = |idx| Tap::Unit(EntityId { idx, generation: 1 });
    let (_, kept) = tap_again(None, unit(4));
    let (again, _) = tap_again(kept, unit(5));
    assert!(!again, "two units clicked in turn are two picks");
}

/// A seat with every field named.
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

#[test]
fn a_click_takes_the_ground_item_nearest_the_cursor_and_only_within_reach() {
    let handle = |idx: u32| EntityId { idx, generation: 1 };
    let lying_at = |idx: u32, x: i32, y: i32| bota_proto::LootView {
        id: handle(idx),
        pos: Vec2::from_ints(x, y),
        item: ItemId(0),
        charges: None,
    };
    let mut view = a_tick(0, Vec::new(), Vec::new());
    view.loot = vec![lying_at(21, 1000, 1000), lying_at(22, 1040, 1000)];
    assert_eq!(
        crate::intent::loot_under_cursor(&view, (1010.0, 1000.0)),
        Some(handle(21)),
        "the nearer of the two"
    );
    assert_eq!(
        crate::intent::loot_under_cursor(&view, (1044.0, 1000.0)),
        Some(handle(22)),
        "and the nearer from the other side"
    );
    assert_eq!(
        crate::intent::loot_under_cursor(&view, (5000.0, 5000.0)),
        None,
        "far from both, the click is for the ground"
    );
}

#[test]
fn an_attack_click_sticks_only_to_an_enemy_and_only_near() {
    let handle = |idx: u32| EntityId { idx, generation: 1 };
    let mut enemy = a_unit();
    enemy.id = handle(8);
    enemy.team = Team::Dire;
    enemy.pos = Vec2::from_ints(7000, 7000);
    let mut own = a_unit();
    own.id = handle(9);
    own.pos = Vec2::from_ints(7400, 7000);
    let view = a_tick(100, vec![enemy, own], Vec::new());
    let mine = Some(Team::Radiant);
    assert_eq!(
        crate::intent::enemy_near_cursor(&view, (7040.0, 7000.0), mine),
        Some(handle(8)),
        "a click just off the enemy's edge sticks to it"
    );
    assert_eq!(
        crate::intent::enemy_near_cursor(&view, (7060.0, 7000.0), mine),
        None,
        "one further off is for the ground"
    );
    assert_eq!(
        crate::intent::enemy_near_cursor(&view, (7390.0, 7010.0), mine),
        None,
        "one of this side's own never sticks"
    );
    assert_eq!(
        crate::intent::enemy_near_cursor(&view, (9000.0, 9000.0), mine),
        None,
        "far from everybody the click is for the ground"
    );
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
        loot: Vec::new(),
        aggro: Vec::new(),
        order_delays: Vec::new(),
        skipped_ticks: Vec::new(),
        stock: Vec::new(),
        talents: Vec::new(),
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
fn a_unit_is_known_live_first_and_otherwise_as_it_was_and_how_long_ago() {
    let mut seen = Vec::new();
    let hero = owned(1, UnitKind::Hero, Some(SlotId(0)));
    let id = hero.id;
    remember(&mut seen, &a_tick(10, vec![hero], Vec::new()));

    let now = a_tick(10, Vec::new(), Vec::new());
    let (held, stale) = known_in(&seen, &now, id).expect("it is remembered");
    assert_eq!(held.id, id, "the body it was is still there to look at");
    assert_eq!(stale, 0, "and this tick it is as fresh as the memory");

    let later = a_tick(70, Vec::new(), Vec::new());
    let (_, stale) = known_in(&seen, &later, id).expect("still remembered");
    assert_eq!(stale, 60, "sixty ticks later it is sixty ticks old");

    let mut now = owned(1, UnitKind::Hero, Some(SlotId(0)));
    now.hp = 42;
    let back = a_tick(90, vec![now], Vec::new());
    let (held, stale) = known_in(&seen, &back, id).expect("it stands");
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

/// An app that has heard nothing, over an empty recording.
fn a_silent_app() -> crate::state::App {
    let player = crate::replay_play::ReplayPlayer::from_reader(std::io::Cursor::new(Vec::new()));
    crate::state::App::new(crate::state::Source::Replay(Box::new(player)))
}

#[test]
fn the_home_shop_is_where_the_match_put_the_fountain() {
    let mut app = a_silent_app();
    app.my_slot = Some(SlotId(0));
    app.fountains = [Vec2::from_ints(3000, 3000), Vec2::from_ints(9000, 9000)];
    app.shop_range = 1000;
    let mut body = a_unit();
    body.id = EntityId {
        idx: 1,
        generation: 1,
    };
    body.pos = Vec2::from_ints(3500, 3000);
    let mut seat = a_player();
    seat.unit = Some(body.id);
    app.view = Some(a_tick(1, vec![body.clone()], vec![seat]));
    assert!(
        app.at_home_shop(),
        "inside the range of this map's fountain"
    );

    body.pos = Vec2::from_ints(1760, 2278);
    let mut seat = a_player();
    seat.unit = Some(body.id);
    app.view = Some(a_tick(2, vec![body], vec![seat]));
    assert!(
        !app.at_home_shop(),
        "the main map's fountain means nothing here"
    );
}

#[test]
fn only_what_the_seat_bought_and_keeps_counts_toward_the_price() {
    let mut body = a_unit();
    body.id = EntityId {
        idx: 1,
        generation: 1,
    };
    let kept = ItemView {
        id: ItemId(0),
        ..an_item(None, 0, 0, None)
    };
    let sold = ItemView {
        id: ItemId(19),
        for_sale: true,
        ..kept
    };
    let found = ItemView {
        id: ItemId(13),
        owner: SlotId(1),
        ..kept
    };
    body.items = vec![Some(kept), Some(sold), Some(found)];
    let mut seat = a_player();
    seat.unit = Some(body.id);
    seat.stash = Some(vec![Some(ItemView {
        id: ItemId(20),
        ..kept
    })]);
    let view = a_tick(1, vec![body], vec![seat.clone()]);
    assert_eq!(
        crate::shop::held_items(&view, &seat),
        vec![ItemId(0), ItemId(20)],
        "a part marked for sale or bought by another seat is not spent on a build"
    );
}

#[test]
fn the_next_hit_is_cut_by_armor_as_the_server_cuts_it() {
    use crate::lasthit::after_armor;
    // 50 * 100 / (100 + 6 * 2) = 44.6, rounded down.
    assert_eq!(after_armor(50, Fixed::from_int(2)), 44);
    assert_eq!(after_armor(50, Fixed::ZERO), 50);
    // Below zero each point lets more through: 50 * (112 + 12) / 112.
    assert_eq!(after_armor(50, Fixed::from_int(-2)), 55);
    // Half points count: 61 * 100 / (100 + 6 * 1.5) = 55.96.
    assert_eq!(after_armor(61, Fixed::from_ratio(3, 2)), 55);
}

#[test]
fn a_unit_is_drawn_between_two_ticks_and_jumps_are_not_drawn_out() {
    let mut walker = a_unit();
    walker.id = EntityId {
        idx: 1,
        generation: 1,
    };
    let mut blinker = walker.clone();
    blinker.id = EntityId {
        idx: 2,
        generation: 1,
    };
    walker.pos = Vec2::from_ints(1000, 1000);
    blinker.pos = Vec2::from_ints(1000, 1000);
    let before = a_tick(1, vec![walker.clone(), blinker.clone()], vec![]);
    walker.pos = Vec2::from_ints(1030, 1000);
    blinker.pos = Vec2::from_ints(2000, 1000);
    let after = a_tick(2, vec![walker, blinker], vec![]);
    let bodies = crate::smooth::bodies(&after, Some(&before), 0.5);
    assert!((bodies[0].x - 1015.0).abs() < 0.01, "halfway along a step");
    assert!(
        (bodies[1].x - 2000.0).abs() < 0.01,
        "a blink is shown at once"
    );
}

#[test]
fn the_damage_range_is_the_wires_with_the_bonus_split_off_a_hero() {
    use crate::catalog::damage_range;
    // Shadow Fiend: 16 base and 25 agility make 41; 12 of items on top.
    let mut fiend = a_unit();
    fiend.hero = Some(HeroId(1));
    fiend.primary = Some(bota_proto::Attribute::Agility);
    fiend.attributes.agility = Fixed::from_int(25);
    fiend.attack_damage_min = 41 + 12;
    fiend.attack_damage_max = 47 + 12;
    let range = damage_range(&fiend);
    assert_eq!((range.min, range.max, range.bonus), (41, 47, 12));
    // A creep has no base to split: its whole range is the range.
    let mut creep = a_unit();
    creep.hero = None;
    creep.attack_damage_min = 19;
    creep.attack_damage_max = 23;
    let range = damage_range(&creep);
    assert_eq!((range.min, range.max, range.bonus), (19, 23, 0));
}

/// The one talent tree is Shadow Fiend's, whatever id the hero list gives him.
#[test]
fn the_talent_tree_is_shadow_fiends() {
    let fiend = crate::HEROES
        .iter()
        .find(|h| h.name == "Shadow Fiend")
        .expect("Shadow Fiend is a hero");
    assert!(crate::talents(fiend.id).is_some());
    for other in crate::HEROES.iter().filter(|h| h.id != fiend.id) {
        assert!(
            crate::talents(other.id).is_none(),
            "{} has no tree",
            other.name
        );
    }
}

/// The catalog of `space` holds, as `held` and through `found`, exactly the
/// ids the registry has live.
fn holds_the_live_ids(
    space: bota_proto::IdSpace,
    held: impl Iterator<Item = u16>,
    found: impl Fn(u16) -> bool,
) {
    let mut live: Vec<u16> = bota_proto::registered(space)
        .filter(|entry| !entry.retired)
        .map(|entry| entry.id)
        .collect();
    live.sort_unstable();
    let mut held: Vec<u16> = held.collect();
    held.sort_unstable();
    assert_eq!(held, live, "the catalog's {space:?} ids are the live ones");
    for entry in bota_proto::registered(space) {
        assert_eq!(found(entry.id), !entry.retired, "{space:?} {}", entry.id);
    }
}

#[test]
fn the_catalog_holds_every_live_id_and_no_other() {
    use crate::catalog::{ABILITIES, EFFECTS, HEROES, ITEMS, ability, effect, hero, item};
    use bota_proto::IdSpace;
    holds_the_live_ids(IdSpace::Hero, HEROES.iter().map(|face| face.id), |id| {
        hero(id).is_some_and(|face| face.id == id)
    });
    holds_the_live_ids(
        IdSpace::Ability,
        ABILITIES.iter().map(|face| face.id),
        |id| ability(id).is_some_and(|face| face.id == id),
    );
    holds_the_live_ids(IdSpace::Item, ITEMS.iter().map(|face| face.id), |id| {
        item(id).is_some_and(|face| face.id == id)
    });
    holds_the_live_ids(IdSpace::Effect, EFFECTS.iter().map(|face| face.id), |id| {
        effect(id).is_some_and(|face| face.id == id)
    });
}
