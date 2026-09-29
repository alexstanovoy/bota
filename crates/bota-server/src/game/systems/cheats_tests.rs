//! Cheats: what each one does, and that a match without them refuses them.

use bota_proto::{
    AbilitySlot, Cheat, DamageKind, EventKind, Fixed, HeroId, ItemId, MapId, ModifierSpec, Order,
    Pick, RejectReason, SlotId, Target, Team, TickMode, Vec2,
};

use crate::game::{
    Command, Entity, Event, ITEM_BUTTERFLY, ItemStack, MatchConfig, World, ability,
    ability_mana_cost, rules, wire_id,
};

/// A one-seat match on the demo map, with or without cheats.
fn config(cheats: bool) -> MatchConfig {
    MatchConfig {
        match_id: 5,
        master_key: [9; 32],
        picks: vec![Pick {
            slot: SlotId(0),
            team: Team::Radiant,
            hero: HeroId(0),
        }],
        map: MapId(1),
        tick_rate: 30,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 30,
        cheats,
        spawn_modifiers: Vec::new(),
    }
}

/// A world with cheats on and its one hero.
fn cheating_world() -> (World, Entity) {
    let cfg = config(true);
    let world = World::for_match(&cfg, cfg.rng());
    let hero = world.seats[0].unit.expect("stood up");
    (world, hero)
}

/// Hands one cheat to the seat and runs the tick it lands in.
fn cheat(world: &mut World, cheat: Cheat) -> Vec<Event> {
    world.advance(&[Command {
        slot: SlotId(0),
        unit: None,
        order: Order::Cheat { cheat },
    }])
}

#[test]
fn gold_comes_out_of_nowhere_and_never_goes_below_nothing() {
    let (mut world, _hero) = cheating_world();
    let before = world.seats[0].gold;
    cheat(&mut world, Cheat::Gold { amount: 500 });
    assert_eq!(world.seats[0].gold, before + 500);
    cheat(&mut world, Cheat::Gold { amount: -10_000 });
    assert_eq!(world.seats[0].gold, 0, "taking away stops at nothing");
}

#[test]
fn levels_go_up_at_once_and_no_further_than_the_cap() {
    let (mut world, hero) = cheating_world();
    let events = cheat(&mut world, Cheat::Levels { count: 3 });
    assert_eq!(world.seats[0].level, 4);
    assert_eq!(
        world.level.get(hero).map(|level| level.0),
        Some(4),
        "the body is raised with the seat"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::LevelUp { .. }))
            .count(),
        3,
        "every level passed is told of"
    );
    cheat(&mut world, Cheat::Levels { count: 100 });
    assert_eq!(world.seats[0].level, rules::HERO_MAX_LEVEL);
}

#[test]
fn a_refresh_fills_the_pools_and_clears_every_wait() {
    let (mut world, hero) = cheating_world();
    world.health.get_mut(hero).expect("standing").hp = Fixed::from_int(1);
    world.mana.get_mut(hero).expect("has mana").mana = Fixed::ZERO;
    world.abilities.get_mut(hero).expect("has a book").slots[1].cooldown = 100;
    world.inventory.get_mut(hero).expect("has a bag").slots[0] = Some(ItemStack {
        id: ItemId(crate::game::ITEM_TOWN_PORTAL_SCROLL),
        charges: 1,
        cooldown: 50,
        mute: 20,
        mode: None,
        bought_tick: 0,
        touched: false,
        owner: SlotId(0),
        for_sale: false,
    });
    world.seats[0]
        .item_clocks
        .push((ItemId(crate::game::ITEM_TOWN_PORTAL_SCROLL), 900));
    cheat(&mut world, Cheat::Refresh);
    let stats = *world.stats.get(hero).expect("settled");
    assert_eq!(world.health.get(hero).map(|h| h.hp), Some(stats.max_hp));
    assert_eq!(world.mana.get(hero).map(|m| m.mana), Some(stats.max_mana));
    assert_eq!(
        world.abilities.get(hero).map(|book| book.slots[1].cooldown),
        Some(0)
    );
    let scroll = world.inventory.get(hero).expect("has a bag").slots[0].expect("still held");
    assert_eq!((scroll.cooldown, scroll.mute), (0, 0));
    assert!(world.seats[0].item_clocks.is_empty());
}

#[test]
fn an_item_is_handed_out_for_nothing_and_told_of_as_bought() {
    let (mut world, hero) = cheating_world();
    let gold = world.seats[0].gold;
    let events = cheat(
        &mut world,
        Cheat::Item {
            item: ItemId(ITEM_BUTTERFLY),
        },
    );
    assert!(
        world
            .inventory
            .get(hero)
            .expect("has a bag")
            .held()
            .any(|stack| stack.id == ItemId(ITEM_BUTTERFLY)),
        "the Butterfly is in the bag"
    );
    assert_eq!(world.seats[0].gold, gold, "and nothing was paid");
    assert!(events.iter().any(|event| matches!(
        event.kind,
        EventKind::ItemBought {
            item: ItemId(ITEM_BUTTERFLY),
            ..
        }
    )));
}

#[test]
fn a_match_without_cheats_refuses_them_and_one_with_them_takes_them() {
    let order = Order::Cheat {
        cheat: Cheat::Gold { amount: 1 },
    };
    let cfg = config(false);
    let world = World::for_match(&cfg, cfg.rng());
    assert_eq!(
        world.validate_order(SlotId(0), None, &order),
        Err(RejectReason::NoCheats)
    );
    let (world, _hero) = cheating_world();
    assert_eq!(world.validate_order(SlotId(0), None, &order), Ok(()));
}

/// A spec with a non-neutral value in every field, inside the bounds.
fn a_spec() -> ModifierSpec {
    ModifierSpec {
        magic_resist: 1_000,
        status_resist: 2_000,
        physical_damage: 11_000,
        magic_damage: 12_000,
        pure_damage: 13_000,
        cooldown_rate: 9_000,
        mana_cost_rate: 8_000,
        move_speed: 8_500,
        max_hp: 11_000,
        max_mana: 12_000,
        gold_income: 7_500,
    }
}

/// A modifier order aimed at the seat's own hero.
fn modifier_order(spec: ModifierSpec, ticks: u32) -> Order {
    Order::Cheat {
        cheat: Cheat::ApplyModifier {
            target: Target::None,
            spec,
            ticks,
        },
    }
}

#[test]
fn a_modifier_cheat_is_refused_without_cheats_and_taken_with_them() {
    let order = modifier_order(a_spec(), 10);
    let cfg = config(false);
    let world = World::for_match(&cfg, cfg.rng());
    assert_eq!(
        world.validate_order(SlotId(0), None, &order),
        Err(RejectReason::NoCheats)
    );
    let (world, _hero) = cheating_world();
    assert_eq!(world.validate_order(SlotId(0), None, &order), Ok(()));
}

#[test]
fn a_modifier_cheat_lands_on_the_named_unit_alone() {
    let (mut world, hero) = cheating_world();
    let creep = world.spawn_unit(
        &crate::game::MELEE_CREEP,
        Team::Dire,
        Vec2::from_ints(7000, 7000),
    );
    world.settle();
    let order = Order::Cheat {
        cheat: Cheat::ApplyModifier {
            target: Target::Unit(wire_id(creep)),
            spec: a_spec(),
            ticks: 10,
        },
    };
    assert_eq!(world.validate_order(SlotId(0), None, &order), Ok(()));
    world.advance(&[Command {
        slot: SlotId(0),
        unit: None,
        order,
    }]);
    assert_eq!(
        world
            .applied
            .get(creep)
            .and_then(|applied| applied.iter().next())
            .map(|held| held.spec),
        Some(a_spec())
    );
    assert!(!world.applied.contains(hero), "and nowhere else");
}

#[test]
fn an_out_of_bounds_spec_is_refused_as_a_bad_cheat() {
    let (world, _hero) = cheating_world();
    let mut specs = Vec::new();
    let mut magic = a_spec();
    magic.magic_resist = ModifierSpec::MAX_RESIST + 1;
    specs.push(magic);
    let mut status_low = a_spec();
    status_low.status_resist = -1;
    specs.push(status_low);
    let mut status_high = a_spec();
    status_high.status_resist = ModifierSpec::MAX_STATUS_RESIST + 1;
    specs.push(status_high);
    let mut physical = a_spec();
    physical.physical_damage = ModifierSpec::MIN_SCALE - 1;
    specs.push(physical);
    let mut magical = a_spec();
    magical.magic_damage = ModifierSpec::MAX_SCALE + 1;
    specs.push(magical);
    let mut pure = a_spec();
    pure.pure_damage = 0;
    specs.push(pure);
    let mut cooldown = a_spec();
    cooldown.cooldown_rate = ModifierSpec::MIN_SCALE - 1;
    specs.push(cooldown);
    let mut mana = a_spec();
    mana.mana_cost_rate = ModifierSpec::MAX_SCALE + 1;
    specs.push(mana);
    let mut speed = a_spec();
    speed.move_speed = ModifierSpec::MIN_SCALE - 1;
    specs.push(speed);
    let mut health = a_spec();
    health.max_hp = ModifierSpec::MAX_SCALE + 1;
    specs.push(health);
    let mut mana_pool = a_spec();
    mana_pool.max_mana = ModifierSpec::MIN_SCALE - 1;
    specs.push(mana_pool);
    let mut gold = a_spec();
    gold.gold_income = ModifierSpec::MAX_SCALE + 1;
    specs.push(gold);
    for spec in specs {
        assert_eq!(
            world.validate_order(SlotId(0), None, &modifier_order(spec, 10)),
            Err(RejectReason::BadCheat),
            "{spec:?}"
        );
    }
}

#[test]
fn a_modifier_duration_outside_its_bounds_is_refused_as_a_bad_cheat() {
    let (world, _hero) = cheating_world();
    for ticks in [0, bota_proto::MAX_MODIFIER_TICKS + 1] {
        assert_eq!(
            world.validate_order(SlotId(0), None, &modifier_order(a_spec(), ticks)),
            Err(RejectReason::BadCheat),
            "{ticks} ticks"
        );
    }
    assert_eq!(
        world.validate_order(
            SlotId(0),
            None,
            &modifier_order(a_spec(), bota_proto::MAX_MODIFIER_TICKS)
        ),
        Ok(())
    );
}

#[test]
fn a_modifier_cheat_aimed_at_the_ground_is_the_wrong_target_kind() {
    let (world, _hero) = cheating_world();
    let order = Order::Cheat {
        cheat: Cheat::ApplyModifier {
            target: Target::Pos(Vec2::from_ints(100, 100)),
            spec: a_spec(),
            ticks: 10,
        },
    };
    assert_eq!(
        world.validate_order(SlotId(0), None, &order),
        Err(RejectReason::WrongTargetKind)
    );
}

#[test]
fn a_modifier_cheat_aimed_at_nobody_is_an_unknown_target() {
    let (world, _hero) = cheating_world();
    let order = Order::Cheat {
        cheat: Cheat::ApplyModifier {
            target: Target::Unit(bota_proto::EntityId {
                idx: 999,
                generation: 3,
            }),
            spec: a_spec(),
            ticks: 10,
        },
    };
    assert_eq!(
        world.validate_order(SlotId(0), None, &order),
        Err(RejectReason::UnknownTarget)
    );
}

#[test]
fn a_modifier_cheat_runs_for_its_ticks_and_then_lifts() {
    let (mut world, hero) = cheating_world();
    cheat(
        &mut world,
        Cheat::ApplyModifier {
            target: Target::None,
            spec: a_spec(),
            ticks: 2,
        },
    );
    assert_eq!(
        world
            .applied
            .get(hero)
            .and_then(|applied| applied.iter().next())
            .map(|held| held.ticks_left),
        Some(Some(1)),
        "the tick that applied it is one of its own"
    );
    world.step();
    assert!(!world.applied.contains(hero), "two ticks only");
}

#[test]
fn a_clear_cheat_takes_the_modifier_off_and_is_harmless_without_one() {
    let (mut world, hero) = cheating_world();
    cheat(
        &mut world,
        Cheat::ApplyModifier {
            target: Target::None,
            spec: a_spec(),
            ticks: 100,
        },
    );
    assert!(world.applied.contains(hero));
    cheat(
        &mut world,
        Cheat::ClearModifiers {
            target: Target::None,
        },
    );
    assert!(!world.applied.contains(hero));
    cheat(
        &mut world,
        Cheat::ClearModifiers {
            target: Target::None,
        },
    );
    assert!(!world.applied.contains(hero));
}

#[test]
fn an_unbounded_cheat_is_turned_away_even_when_it_skips_the_gate() {
    let (mut world, hero) = cheating_world();
    let mut unbounded = a_spec();
    unbounded.max_hp = ModifierSpec::MAX_SCALE + 1;
    world.cheat(
        SlotId(0),
        hero,
        Cheat::ApplyModifier {
            target: Target::None,
            spec: unbounded,
            ticks: 10,
        },
        &mut Vec::new(),
    );
    assert!(!world.applied.contains(hero), "the spec was refused");
    world.cheat(
        SlotId(0),
        hero,
        Cheat::ApplyModifier {
            target: Target::None,
            spec: a_spec(),
            ticks: 0,
        },
        &mut Vec::new(),
    );
    assert!(!world.applied.contains(hero), "the duration was refused");
}

#[test]
fn a_modifier_cheat_aimed_at_something_that_is_not_a_unit_is_an_unknown_target() {
    let (mut world, _hero) = cheating_world();
    let mark = world.spawn();
    let order = Order::Cheat {
        cheat: Cheat::ApplyModifier {
            target: Target::Unit(wire_id(mark)),
            spec: a_spec(),
            ticks: 10,
        },
    };
    assert_eq!(
        world.validate_order(SlotId(0), None, &order),
        Err(RejectReason::UnknownTarget)
    );
}

#[test]
fn a_kept_kit_is_projected_after_the_body_falls() {
    let (mut world, hero) = cheating_world();
    if let Some(book) = world.abilities.get_mut(hero) {
        book.slots[1].level = 1;
    }
    let stack = ItemStack::bought(ItemId(ITEM_BUTTERFLY), SlotId(0), world.tick)
        .expect("the Butterfly is sold");
    world.inventory.get_mut(hero).expect("has a bag").slots[0] = Some(stack);
    let mut spec = ModifierSpec::NOMINAL;
    spec.mana_cost_rate = 5_000;
    cheat(
        &mut world,
        Cheat::ApplyModifier {
            target: Target::None,
            spec,
            ticks: 100,
        },
    );
    let live = world.view_full();
    let unit = live
        .units
        .iter()
        .find(|unit| unit.id == wire_id(hero))
        .expect("the hero is in the view");
    assert_eq!(
        unit.abilities[1].mana_cost,
        ability_mana_cost(ability::FRENZY, 1) / 2,
        "the living view carries the rate"
    );
    assert_eq!(
        unit.items[0].as_ref().map(|item| item.mana_cost),
        Some(0),
        "and the item's own cost"
    );
    world.push_hit(None, hero, 1_000_000, DamageKind::Pure);
    world.step();
    let fallen = world.view_full();
    let seat = fallen
        .players
        .iter()
        .find(|player| player.slot == SlotId(0))
        .expect("the seat is in the view");
    let kit = seat.kit.as_ref().expect("the kit outlives the body");
    assert_eq!(
        kit.abilities[1].mana_cost,
        ability_mana_cost(ability::FRENZY, 1),
        "the modifier fell with the body, so the kept kit is nominal"
    );
    assert_eq!(kit.items[0].as_ref().map(|item| item.mana_cost), Some(0));
}

#[test]
fn a_fallen_hero_takes_its_modifier_with_it_and_respawns_clean() {
    let (mut world, hero) = cheating_world();
    cheat(
        &mut world,
        Cheat::ApplyModifier {
            target: Target::None,
            spec: a_spec(),
            ticks: 100_000,
        },
    );
    assert!(world.applied.contains(hero));
    world.push_hit(None, hero, 100_000, DamageKind::Pure);
    world.step();
    assert!(!world.alive(hero), "the blow was fatal");
    assert!(!world.applied.contains(hero), "the body took it with it");
    let wait = world.seats[0].respawn_left;
    assert!(wait > 0);
    for _ in 0..=wait {
        world.step();
    }
    let reborn = world.seats[0].unit.expect("the hero comes back");
    assert!(!world.applied.contains(reborn), "and starts clean");
}

/// A two-seat match, both seats holding cheats, so a hero can fall.
fn two_seat_config() -> MatchConfig {
    let mut cfg = config(true);
    cfg.picks.push(Pick {
        slot: SlotId(1),
        team: Team::Dire,
        hero: HeroId(0),
    });
    cfg
}

#[test]
fn gold_income_scales_the_bounty_a_kill_pays() {
    let cfg = two_seat_config();
    let mut world = World::for_match(&cfg, cfg.rng());
    let killer = world.seats[0].unit.expect("the killer stands");
    let victim = world.seats[1].unit.expect("the victim stands");
    world.seats[1].streak = 3;
    let composed = World::hero_bounty(3);
    let mut spec = ModifierSpec::NOMINAL;
    spec.gold_income = 15_000;
    cheat(
        &mut world,
        Cheat::ApplyModifier {
            target: Target::None,
            spec,
            ticks: 1_000,
        },
    );
    let before = world.seats[0].gold;
    let net_worth = world.seats[0].net_worth;
    world.push_hit(Some(killer), victim, 100_000, DamageKind::Pure);
    let events = world.step();
    let expected = composed * 15_000 / 10_000;
    assert_eq!(
        world.seats[0].gold,
        before + expected,
        "the composed bounty of {composed} is scaled once"
    );
    assert_eq!(world.seats[0].net_worth, net_worth + expected);
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, EventKind::Died { gold, .. } if gold == expected)),
        "the death tells of what was paid"
    );
}

#[test]
fn gold_income_leaves_starting_passive_and_refunded_gold_alone() {
    let (mut world, hero) = cheating_world();
    let mut spec = ModifierSpec::NOMINAL;
    spec.gold_income = 20_000;
    let before = world.seats[0].gold;
    cheat(
        &mut world,
        Cheat::ApplyModifier {
            target: Target::None,
            spec,
            ticks: 1_000,
        },
    );
    assert_eq!(world.seats[0].gold, before, "no gold arrives with it");
    world.tick = rules::PREGAME_TICKS + rules::PASSIVE_GOLD_PERIOD_TICKS;
    world.passive_gold();
    assert_eq!(
        world.seats[0].gold,
        before + 1,
        "passive income is one a period either way"
    );
    let mut stack = ItemStack::bought(ItemId(ITEM_BUTTERFLY), SlotId(0), world.tick)
        .expect("the Butterfly is sold");
    stack.touched = true;
    world.inventory.get_mut(hero).expect("has a bag").slots[0] = Some(stack);
    let price = crate::game::ITEMS[usize::from(ITEM_BUTTERFLY)].cost;
    let gold = world.seats[0].gold;
    assert!(world.sell_item(SlotId(0), hero, 0), "it sells at the shop");
    assert_eq!(
        world.seats[0].gold,
        gold + price * rules::SELL_PCT / 100,
        "a refund is the same either way"
    );
}

#[test]
fn a_scaled_mana_cost_is_what_the_cast_checks_and_spends() {
    let (mut world, hero) = cheating_world();
    if let Some(book) = world.abilities.get_mut(hero) {
        book.slots[1].level = 1;
    }
    let full = ability_mana_cost(ability::FRENZY, 1);
    let mut spec = ModifierSpec::NOMINAL;
    spec.mana_cost_rate = 5_000;
    cheat(
        &mut world,
        Cheat::ApplyModifier {
            target: Target::None,
            spec,
            ticks: 100,
        },
    );
    let cast = Order::Cast {
        slot: AbilitySlot(1),
        target: Target::None,
    };
    world.mana.get_mut(hero).expect("has mana").mana = Fixed::from_int(full / 2);
    assert_eq!(
        world.validate_order(SlotId(0), None, &cast),
        Ok(()),
        "the scaled cost is affordable"
    );
    assert!(world.begin_ability(hero, AbilitySlot(1), Target::None));
    assert_eq!(
        world.mana.get(hero).map(|mana| mana.mana),
        Some(Fixed::ZERO),
        "and it was the scaled cost that went"
    );
    if let Some(book) = world.abilities.get_mut(hero) {
        book.slots[1].cooldown = 0;
    }
    world.mana.get_mut(hero).expect("has mana").mana = Fixed::from_int(full / 2 - 1);
    assert_eq!(
        world.validate_order(SlotId(0), None, &cast),
        Err(RejectReason::NotEnoughMana),
        "a hair short of it is not"
    );
}
