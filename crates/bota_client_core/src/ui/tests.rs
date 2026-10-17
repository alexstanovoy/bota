//! The client core, tested without a window or a socket.

use crate::ui::app::{App, Tap, tap_again};
use crate::ui::slots::{Look, Press, Slot, ability_look, decide, item_look, order_for};
use bota_proto::{
    AbilityId, AbilitySlot, AbilityView, Aim, Angle, Attributes, EntityId, Fixed, HeroId, ItemId,
    ItemView, Order, PlayerView, SlotId, StatusFlags, Target, Team, UnitKind, UnitView, Vec2,
    WorldView,
};

#[test]
fn the_console_turns_a_typed_line_into_a_cheat_order() {
    use bota_proto::Cheat;
    let parse = crate::ui::console::parse_console;
    for (line, cheat) in [
        ("-gold 500", Cheat::Gold { amount: 500 }),
        ("gold", Cheat::Gold { amount: 1000 }),
        ("-lvlup 3", Cheat::Levels { count: 3 }),
        ("-refresh", Cheat::Refresh),
        (
            "-item Butterfly",
            Cheat::Item {
                item: ItemId::BUTTERFLY,
            },
        ),
        (
            "-item 42",
            Cheat::Item {
                item: ItemId::MANGO,
            },
        ),
    ] {
        assert_eq!(parse(line), Ok(Order::Cheat { cheat }), "{line:?}");
    }
    for line in ["-fly", "-gold lots", "-item wings", ""] {
        assert!(parse(line).is_err(), "{line:?} is refused");
    }
}

#[test]
fn every_catalog_entry_is_named_and_found_by_its_own_id() {
    use crate::catalog::{ABILITIES, EFFECTS, ITEMS, ability, effect, item};
    let groups: [Vec<(u16, &str, Option<&str>)>; 2] = [
        ABILITIES
            .iter()
            .map(|f| (f.id, f.name, ability(f.id).map(|x| x.name)))
            .collect(),
        ITEMS
            .iter()
            .map(|f| (f.id, f.name, item(f.id).map(|x| x.name)))
            .collect(),
    ];
    for group in groups {
        for (index, (id, name, found)) in group.into_iter().enumerate() {
            assert_eq!(usize::from(id), index, "{name} is in place");
            assert_eq!(found, Some(name), "{name} is found by its id");
            assert!(!name.is_empty(), "entry {id} has a name");
        }
    }
    for (index, face) in EFFECTS.iter().enumerate().skip(1) {
        assert_eq!(usize::from(face.id), index, "{} is in place", face.name());
        assert_eq!(
            effect(face.id),
            Some(face),
            "{} is found by its id",
            face.name()
        );
        assert!(!face.name().is_empty(), "effect {index} has a name");
        let rules = bota_core::effect_def(bota_proto::EffectId(face.id)).map(|def| def.name);
        assert_ne!(
            face.label,
            rules,
            "{} keeps a label only where it differs",
            face.name()
        );
    }
    assert!(effect(0).is_none(), "the retired effect is gone");
    assert!(ability(ABILITIES.len() as u16).is_none());
    assert!(item(u16::MAX).is_none());
    assert!(effect(u16::MAX).is_none());
}

#[test]
fn a_part_in_hand_comes_off_what_the_shop_asks() {
    use crate::shop::{price_for, whole_price};
    let parts = [ItemId::BOOTS, ItemId::GLOVES, ItemId::BELT];
    let built: i32 = parts.iter().map(|part| whole_price(*part)).sum();
    assert_eq!(
        price_for(ItemId::POWER_TREADS, &[], None),
        built,
        "with nothing in hand the parts are asked for"
    );
    assert_eq!(
        price_for(ItemId::POWER_TREADS, &[ItemId::BOOTS], None),
        built - whole_price(ItemId::BOOTS),
        "the boots already worn are not asked for twice"
    );
    assert_eq!(
        price_for(ItemId::BOOTS, &[ItemId::BOOTS], None),
        whole_price(ItemId::BOOTS),
        "but a second of what is bought whole costs the whole of it"
    );
    assert_eq!(
        price_for(ItemId::POWER_TREADS, &[ItemId::POWER_TREADS], None),
        built,
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
        hp_regen: Fixed::ZERO,
        mana_regen: Fixed::ZERO,
        town_portal: None,
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
        hero: Some(HeroId::PUDGE),
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
        id: AbilityId::MEAT_HOOK,
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
        id: ItemId::BOOTS,
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
        hero: HeroId::PUDGE,
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
        item: ItemId::BOOTS,
        charges: None,
    };
    let mut view = a_tick(0, Vec::new(), Vec::new());
    view.loot = vec![lying_at(21, 1000, 1000), lying_at(22, 1040, 1000)];
    assert_eq!(
        crate::ui::intent::loot_under_cursor(&view, (1010.0, 1000.0)),
        Some(handle(21)),
        "the nearer of the two"
    );
    assert_eq!(
        crate::ui::intent::loot_under_cursor(&view, (1044.0, 1000.0)),
        Some(handle(22)),
        "and the nearer from the other side"
    );
    assert_eq!(
        crate::ui::intent::loot_under_cursor(&view, (5000.0, 5000.0)),
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
        crate::ui::intent::enemy_near_cursor(&view, (7040.0, 7000.0), mine),
        Some(handle(8)),
        "a click just off the enemy's edge sticks to it"
    );
    assert_eq!(
        crate::ui::intent::enemy_near_cursor(&view, (7060.0, 7000.0), mine),
        None,
        "one further off is for the ground"
    );
    assert_eq!(
        crate::ui::intent::enemy_near_cursor(&view, (7390.0, 7010.0), mine),
        None,
        "one of this side's own never sticks"
    );
    assert_eq!(
        crate::ui::intent::enemy_near_cursor(&view, (9000.0, 9000.0), mine),
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
        teleports: Vec::new(),
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
fn a_seat_is_picked_by_the_body_it_last_stood_in() {
    let mut app = a_started_app();
    let first = owned(1, UnitKind::Hero, Some(SlotId(0)));
    let id = first.id;
    app.handle(snapshot_of(
        10,
        vec![first],
        vec![a_seat(SlotId(0), Some(id))],
    ));
    app.handle(snapshot_of(60, Vec::new(), vec![a_seat(SlotId(0), None)]));
    assert_eq!(
        app.body_of(SlotId(0)),
        Some(id),
        "the fallen body is the handle"
    );
    assert_eq!(
        app.body_of(SlotId(1)),
        None,
        "nothing is known of another seat"
    );
    let (held, stale) = app.known(id).expect("it is remembered");
    assert_eq!((held.id, stale), (id, 50), "as it was, fifty ticks ago");

    let again = owned(2, UnitKind::Hero, Some(SlotId(0)));
    let fresh = again.id;
    app.handle(snapshot_of(
        600,
        vec![again],
        vec![a_seat(SlotId(0), Some(fresh))],
    ));
    app.handle(snapshot_of(601, Vec::new(), vec![a_seat(SlotId(0), None)]));
    assert_eq!(
        app.body_of(SlotId(0)),
        Some(fresh),
        "the seat answers with the body it stood in last"
    );
}

fn snapshot_of(tick: u32, units: Vec<UnitView>, players: Vec<PlayerView>) -> bota_proto::ServerMsg {
    bota_proto::ServerMsg::Snapshot {
        view: a_tick(tick, units, players),
    }
}

/// An app watching a replay of a match on an open 128-cell map with these
/// fountains, a seat for one.
fn a_started_app() -> App {
    let mut app = a_silent_app();
    app.my_slot = Some(SlotId(0));
    app.handle(bota_proto::ServerMsg::MatchStart {
        info: bota_proto::MatchInfo {
            match_id: 1,
            map: bota_proto::MapId(0),
            tick_rate: 30,
            pregame_ticks: 0,
            trees: Vec::new(),
            terrain_cells: 128,
            terrain_rle: vec![(16384, 0x80)],
            opaque_cells: Vec::new(),
            mode: bota_proto::TickMode::Lockstep,
            picks: vec![bota_proto::Pick {
                slot: SlotId(0),
                team: Team::Radiant,
                hero: HeroId::PUDGE,
            }],
            shop: Vec::new(),
            fountains: [Vec2::ZERO; 2],
            shop_range: 0,
        },
    });
    app
}

#[test]
fn a_cast_point_shortens_while_a_feast_runs_on_a_hero_with_the_talent() {
    let mut app = a_started_app();
    let mut fiend = owned(1, UnitKind::Hero, Some(SlotId(0)));
    fiend.hero = Some(HeroId::SHADOW_FIEND);
    let id = fiend.id;
    let seat = vec![a_seat(SlotId(0), Some(id))];
    let told = bota_proto::TalentView {
        slot: SlotId(0),
        learned: bota_proto::Talents::default().with(bota_proto::TalentSlot(7)),
        learnable: bota_proto::Talents::default(),
        attribute_bonus: 0,
        attribute_bonus_learnable: false,
    };
    let mut view = a_tick(10, vec![fiend.clone()], seat);
    view.talents = vec![told];
    app.handle(bota_proto::ServerMsg::Snapshot { view });
    let raze = AbilityId::RAZE_NEAR;
    assert_eq!(app.cast_point_secs(&fiend, raze), Some(0.55));
    fiend.effects.push(bota_proto::EffectView {
        id: bota_proto::EffectId::FEASTING,
        ticks_left: None,
        stacks: None,
    });
    assert_eq!(
        app.cast_point_secs(&fiend, raze),
        Some(0.385),
        "the level-25 talent cuts the cast point to 70% while a feast runs"
    );
    assert_eq!(
        app.cast_point_secs(&fiend, AbilityId::MEAT_HOOK),
        Some(0.0),
        "the hook has no cast point of its own"
    );
}

/// An app that has heard nothing, over an empty recording.
fn a_silent_app() -> crate::ui::app::App {
    let player = crate::ui::replay::ReplayPlayer::from_reader(std::io::Cursor::new(Vec::new()));
    crate::ui::app::App::new(crate::ui::app::Source::Replay(Box::new(player)))
}

#[test]
fn only_what_the_seat_bought_and_keeps_counts_toward_the_price() {
    let mut body = a_unit();
    body.id = EntityId {
        idx: 1,
        generation: 1,
    };
    let kept = ItemView {
        id: ItemId::BOOTS,
        ..an_item(None, 0, 0, None)
    };
    let sold = ItemView {
        id: ItemId::GLOVES,
        for_sale: true,
        ..kept
    };
    let found = ItemView {
        id: ItemId::BELT,
        owner: SlotId(1),
        ..kept
    };
    body.items = vec![Some(kept), Some(sold), Some(found)];
    let mut seat = a_player();
    seat.unit = Some(body.id);
    seat.stash = Some(vec![Some(ItemView {
        id: ItemId::BLADES_OF_ATTACK,
        ..kept
    })]);
    let view = a_tick(1, vec![body], vec![seat.clone()]);
    assert_eq!(
        crate::shop::held_items(&view, &seat),
        vec![ItemId::BOOTS, ItemId::BLADES_OF_ATTACK],
        "a part marked for sale or bought by another seat is not spent on a build"
    );
}

#[test]
fn the_next_hit_is_what_the_server_would_land() {
    use crate::{HitRange, next_hit_raised};
    let next_hit =
        |attacker: &UnitView, target: &UnitView| next_hit_raised(attacker, target, target.armor, 0);
    let blade = |mute_left| ItemView {
        id: ItemId::QUELLING_BLADE,
        mute_left,
        ..an_item(None, 0, 0, None)
    };
    let mut hero = a_unit();
    hero.attack_damage = 45;
    hero.attack_damage_min = 40;
    hero.attack_damage_max = 50;
    // Two working blades (each adds against creeps), one muted, one in the backpack.
    hero.items = vec![
        Some(blade(0)),
        Some(blade(30)),
        Some(blade(0)),
        None,
        None,
        None,
        Some(blade(0)),
    ];
    let mut creep = a_unit();
    creep.kind = UnitKind::CreepMelee;
    creep.team = Team::Dire;
    creep.armor = Fixed::from_int(2);
    // (40 + 16), (45 + 16) and (50 + 16), each times 100 / 112, rounded down.
    assert_eq!(
        next_hit(&hero, &creep),
        HitRange {
            least: 50,
            mid: 54,
            most: 58
        }
    );
    // A ranged hero's blade adds 4, not 8: (40 + 8), (45 + 8), (50 + 8).
    let mut fiend = hero.clone();
    fiend.hero = Some(bota_proto::HeroId::SHADOW_FIEND);
    assert_eq!(
        next_hit(&fiend, &creep),
        HitRange {
            least: 42,
            mid: 47,
            most: 51
        }
    );

    // The creep bonus is worth nothing against a tower, which also takes half.
    let mut tower = creep.clone();
    tower.kind = UnitKind::Tower;
    tower.armor = Fixed::ZERO;
    assert_eq!(
        next_hit(&hero, &tower),
        HitRange {
            least: 20,
            mid: 22,
            most: 25
        }
    );
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
    let bodies = crate::ui::smooth::bodies(&after, Some(&before), 0.5);
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
    fiend.hero = Some(HeroId::SHADOW_FIEND);
    fiend.level = 1;
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
    for (id, hero) in bota_core::HEROES.iter().enumerate() {
        assert_eq!(
            crate::catalog::talents(id as u16).is_some(),
            hero.name == "Shadow Fiend",
            "{}",
            hero.name
        );
    }
}

/// The catalog of `space` holds, as `held` and through `found`, exactly the
/// ids the registry has live.
/// The damage panel splits Dota's way: a level-1 Shadow Fiend's base range
/// with his starting agility white, the rest green.
#[test]
fn damage_splits_into_base_and_bonus() {
    let mut fiend = a_unit();
    fiend.hero = Some(HeroId::SHADOW_FIEND);
    fiend.level = 1;
    fiend.primary = Some(bota_proto::Attribute::Agility);
    // 16-22 base + 25 agility, and 9 from items and souls on top.
    fiend.attack_damage_min = 50;
    fiend.attack_damage_max = 56;
    let d = crate::catalog::damage_range(&fiend);
    assert_eq!((d.min, d.max, d.bonus), (41, 47, 9));
    fiend.level = 11;
    // Growth is Fixed arithmetic, as the server's own: 19 + 2.7 * 10 stands a
    // hair under 46, and the server floors it too.
    assert_eq!(crate::catalog::base_attributes(&fiend), Some([45, 60, 37]));
}

#[test]
fn every_blurb_takes_its_numbers_from_the_rules() {
    use crate::catalog::{ABILITIES, EFFECTS, ITEMS, talents};
    for template in ABILITIES
        .iter()
        .map(|f| f.template)
        .chain(EFFECTS.iter().map(|f| f.template))
        .chain(ITEMS.iter().flat_map(|f| [f.template, f.stats_template]))
    {
        assert!(
            !template.chars().any(|c| c.is_ascii_digit()),
            "{template:?} types a number the rules own"
        );
    }
    // Filling panics on a key nothing answers to, so one pass proves them all.
    for face in ABILITIES {
        assert!(!face.blurb().contains('{'), "{}", face.name);
    }
    for face in ITEMS {
        assert!(
            !face.blurb().is_empty() && !face.stats().contains('{'),
            "{}",
            face.name
        );
    }
    for face in EFFECTS {
        assert!(!face.blurb().contains('{'), "{}", face.name());
    }
    let tree = talents(bota_proto::HeroId::SHADOW_FIEND.0).expect("Shadow Fiend has a tree");
    assert!(
        tree.iter()
            .all(|t| !t.name.contains('{') && !t.blurb.contains('{'))
    );
    // The numbers are the rules': Quelling's bonus and Hook's reach.
    let quelling = ITEMS[usize::from(ItemId::QUELLING_BLADE.0)];
    assert_eq!(quelling.stats(), "+8/4 creep", "melee, then ranged");
    assert!(
        ABILITIES[usize::from(AbilityId::MEAT_HOOK.0)]
            .blurb()
            .contains("range 1300")
    );
}
