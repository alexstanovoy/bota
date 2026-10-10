//! Cheats: what each one does, and that a match without them refuses them.

use super::support::{a_spec, spec};
use crate::game::{
    Command, Entity, Event, ITEM_BUTTERFLY, ITEM_TOWN_PORTAL_SCROLL, ITEMS, ItemStack, MELEE_CREEP,
    MatchConfig, World, ability, ability_mana_cost, rules, wire_id,
};
use bota_proto::{
    AbilitySlot, Cheat, DamageKind, EntityId, EventKind, Fixed, HeroId, ItemId, MAX_MODIFIER_TICKS,
    MapId, ModifierSpec, Order, Pick, RejectReason, SlotId, Target, Team, TickMode, Vec2,
};

/// A one-seat match on the demo map, with or without cheats.
fn config(cheats: bool) -> MatchConfig {
    MatchConfig {
        match_id: 5,
        master_key: [9; 32],
        picks: vec![Pick {
            slot: SlotId(0),
            team: Team::Radiant,
            hero: HeroId(1),
        }],
        map: MapId(1),
        tick_rate: 30,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 30,
        cheats,
        spawn_modifiers: Vec::new(),
        immortal_structures: false,
        immortal_heroes: false,
    }
}

/// A world with cheats on or off, and its one hero.
fn world_with(cheats: bool) -> (World, Entity) {
    let cfg = config(cheats);
    let world = World::for_match(&cfg, cfg.rng());
    let hero = world.seats[0].unit.expect("stood up");
    (world, hero)
}

/// A world with cheats on and its one hero.
fn cheating_world() -> (World, Entity) {
    world_with(true)
}

/// Hands one cheat to the seat and runs the tick it lands in.
fn cheat(world: &mut World, cheat: Cheat) -> Vec<Event> {
    world.advance(&[Command {
        slot: SlotId(0),
        unit: None,
        order: Order::Cheat { cheat },
    }])
}

/// Cheats a modifier onto the seat's own hero.
fn cheat_modifier(world: &mut World, spec: ModifierSpec, ticks: u32) {
    let target = Target::None;
    cheat(
        world,
        Cheat::ApplyModifier {
            target,
            spec,
            ticks,
        },
    );
}

/// A modifier order aimed at `target`.
fn modifier_order(target: Target, spec: ModifierSpec, ticks: u32) -> Order {
    Order::Cheat {
        cheat: Cheat::ApplyModifier {
            target,
            spec,
            ticks,
        },
    }
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
    let mut scroll = ItemStack::bought(ItemId(ITEM_TOWN_PORTAL_SCROLL), SlotId(0), 0)
        .expect("the scroll is sold");
    (scroll.cooldown, scroll.mute) = (50, 20);
    world.inventory.get_mut(hero).expect("has a bag").slots[0] = Some(scroll);
    world.seats[0]
        .item_clocks
        .push((ItemId(ITEM_TOWN_PORTAL_SCROLL), 900));
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
fn a_match_without_cheats_refuses_every_cheat_and_one_with_them_takes_it() {
    let (plain, _hero) = world_with(false);
    let (cheating, _hero) = cheating_world();
    for cheat in [
        Cheat::Gold { amount: 1 },
        Cheat::Levels { count: 1 },
        Cheat::Refresh,
        Cheat::Item {
            item: ItemId(ITEM_BUTTERFLY),
        },
        Cheat::ApplyModifier {
            target: Target::None,
            spec: a_spec(),
            ticks: 10,
        },
        Cheat::ClearModifiers {
            target: Target::None,
        },
    ] {
        let order = Order::Cheat { cheat };
        assert_eq!(
            plain.validate_order(SlotId(0), None, &order),
            Err(RejectReason::NoCheats),
            "{cheat:?}"
        );
        assert_eq!(
            cheating.validate_order(SlotId(0), None, &order),
            Ok(()),
            "{cheat:?}"
        );
    }
}

#[test]
fn a_modifier_cheat_lands_on_the_named_unit_alone() {
    let (mut world, hero) = cheating_world();
    let creep = world.spawn_unit(&MELEE_CREEP, Team::Dire, Vec2::from_ints(7000, 7000));
    world.settle();
    let order = modifier_order(Target::Unit(wire_id(creep)), a_spec(), 10);
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

/// One named change to a spec.
type SpecChange = (&'static str, fn(&mut ModifierSpec));

#[test]
fn an_out_of_bounds_spec_is_refused_as_a_bad_cheat() {
    let (world, _hero) = cheating_world();
    let changes: [SpecChange; 12] = [
        ("magic_resist", |s| {
            s.magic_resist = ModifierSpec::MAX_RESIST + 1
        }),
        ("status_resist low", |s| s.status_resist = -1),
        ("status_resist high", |s| {
            s.status_resist = ModifierSpec::MAX_STATUS_RESIST + 1;
        }),
        ("physical_damage", |s| {
            s.physical_damage = ModifierSpec::MIN_SCALE - 1
        }),
        ("magic_damage", |s| {
            s.magic_damage = ModifierSpec::MAX_SCALE + 1
        }),
        ("pure_damage", |s| s.pure_damage = 0),
        ("cooldown_rate", |s| {
            s.cooldown_rate = ModifierSpec::MIN_SCALE - 1
        }),
        ("mana_cost_rate", |s| {
            s.mana_cost_rate = ModifierSpec::MAX_SCALE + 1
        }),
        ("move_speed", |s| s.move_speed = ModifierSpec::MIN_SCALE - 1),
        ("max_hp", |s| s.max_hp = ModifierSpec::MAX_SCALE + 1),
        ("max_mana", |s| s.max_mana = ModifierSpec::MIN_SCALE - 1),
        ("gold_income", |s| {
            s.gold_income = ModifierSpec::MAX_SCALE + 1
        }),
    ];
    for (name, change) in changes {
        let mut spec = a_spec();
        change(&mut spec);
        assert_eq!(
            world.validate_order(SlotId(0), None, &modifier_order(Target::None, spec, 10)),
            Err(RejectReason::BadCheat),
            "{name}"
        );
    }
}

#[test]
fn a_modifier_duration_outside_its_bounds_is_refused_as_a_bad_cheat() {
    let (world, _hero) = cheating_world();
    for ticks in [0, MAX_MODIFIER_TICKS + 1] {
        assert_eq!(
            world.validate_order(
                SlotId(0),
                None,
                &modifier_order(Target::None, a_spec(), ticks)
            ),
            Err(RejectReason::BadCheat),
            "{ticks} ticks"
        );
    }
    assert_eq!(
        world.validate_order(
            SlotId(0),
            None,
            &modifier_order(Target::None, a_spec(), MAX_MODIFIER_TICKS)
        ),
        Ok(())
    );
}

#[test]
fn a_modifier_cheat_must_be_aimed_at_a_standing_unit() {
    let (mut world, _hero) = cheating_world();
    let mark = world.spawn();
    let nobody = EntityId {
        idx: 999,
        generation: 3,
    };
    for (target, refused, case) in [
        (
            Target::Pos(Vec2::from_ints(100, 100)),
            RejectReason::WrongTargetKind,
            "the ground",
        ),
        (Target::Unit(nobody), RejectReason::UnknownTarget, "nobody"),
        (
            Target::Unit(wire_id(mark)),
            RejectReason::UnknownTarget,
            "not a unit",
        ),
    ] {
        assert_eq!(
            world.validate_order(SlotId(0), None, &modifier_order(target, a_spec(), 10)),
            Err(refused),
            "{case}"
        );
    }
}

#[test]
fn a_modifier_cheat_runs_for_its_ticks_and_then_lifts() {
    let (mut world, hero) = cheating_world();
    cheat_modifier(&mut world, a_spec(), 2);
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
fn a_modifier_cheat_replaces_its_own_and_a_clear_takes_it_off_harmlessly() {
    let (mut world, hero) = cheating_world();
    cheat_modifier(&mut world, a_spec(), 100);
    let second = spec(|s| s.max_hp = 12_000);
    cheat_modifier(&mut world, second, 50);
    let applied = world.applied.get(hero).expect("on the hero");
    assert_eq!(
        applied.iter().map(|held| held.spec).collect::<Vec<_>>(),
        vec![second],
        "the second cheat replaced the first"
    );
    let clear = Cheat::ClearModifiers {
        target: Target::None,
    };
    cheat(&mut world, clear);
    assert!(!world.applied.contains(hero));
    cheat(&mut world, clear);
    assert!(!world.applied.contains(hero));
}

#[test]
fn an_unbounded_cheat_is_turned_away_even_when_it_skips_the_gate() {
    let (mut world, hero) = cheating_world();
    let unbounded = spec(|s| s.max_hp = ModifierSpec::MAX_SCALE + 1);
    for (spec, ticks, case) in [
        (unbounded, 10, "the spec was refused"),
        (a_spec(), 0, "the duration was refused"),
    ] {
        let target = Target::None;
        let apply = Cheat::ApplyModifier {
            target,
            spec,
            ticks,
        };
        world.cheat(SlotId(0), hero, apply, &mut Vec::new());
        assert!(!world.applied.contains(hero), "{case}");
    }
}

#[test]
fn a_kept_kit_is_projected_after_the_body_falls() {
    let (mut world, hero) = cheating_world();
    if let Some(book) = world.abilities.get_mut(hero) {
        book.slots[0].level = 1;
    }
    let stack = ItemStack::bought(ItemId(ITEM_BUTTERFLY), SlotId(0), world.tick)
        .expect("the Butterfly is sold");
    world.inventory.get_mut(hero).expect("has a bag").slots[0] = Some(stack);
    cheat_modifier(&mut world, spec(|s| s.mana_cost_rate = 5_000), 100);
    let live = world.view_full();
    let unit = live
        .units
        .iter()
        .find(|unit| unit.id == wire_id(hero))
        .expect("the hero is in the view");
    assert_eq!(
        unit.abilities[0].mana_cost,
        ability_mana_cost(ability::RAZE_NEAR, 1) / 2,
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
        kit.abilities[0].mana_cost,
        ability_mana_cost(ability::RAZE_NEAR, 1),
        "the modifier fell with the body, so the kept kit is nominal"
    );
    assert_eq!(kit.items[0].as_ref().map(|item| item.mana_cost), Some(0));
}

#[test]
fn a_fallen_hero_takes_its_modifier_with_it_and_respawns_clean() {
    let (mut world, hero) = cheating_world();
    cheat_modifier(&mut world, a_spec(), 100_000);
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
        hero: HeroId(1),
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
    cheat_modifier(&mut world, spec(|s| s.gold_income = 15_000), 1_000);
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
    let before = world.seats[0].gold;
    cheat_modifier(&mut world, spec(|s| s.gold_income = 20_000), 1_000);
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
    let price = ITEMS[usize::from(ITEM_BUTTERFLY)].cost;
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
        book.slots[0].level = 1;
    }
    let full = ability_mana_cost(ability::RAZE_NEAR, 1);
    cheat_modifier(&mut world, spec(|s| s.mana_cost_rate = 5_000), 100);
    let cast = Order::Cast {
        slot: AbilitySlot(0),
        target: Target::None,
    };
    world.mana.get_mut(hero).expect("has mana").mana = Fixed::from_int(full / 2);
    assert_eq!(
        world.validate_order(SlotId(0), None, &cast),
        Ok(()),
        "the scaled cost is affordable"
    );
    assert!(world.begin_ability(hero, AbilitySlot(0), Target::None));
    assert_eq!(
        world.mana.get(hero).map(|mana| mana.mana),
        Some(Fixed::ZERO),
        "and it was the scaled cost that went"
    );
    if let Some(book) = world.abilities.get_mut(hero) {
        book.slots[0].cooldown = 0;
    }
    world.mana.get_mut(hero).expect("has mana").mana = Fixed::from_int(full / 2 - 1);
    assert_eq!(
        world.validate_order(SlotId(0), None, &cast),
        Err(RejectReason::NotEnoughMana),
        "a hair short of it is not"
    );
}
