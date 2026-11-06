//! Roshan's loot through real match, attack, inventory and death paths.
use super::support::*;
use crate::game::World;
use bota_proto::{DamageKind, ItemId, MapId, ObjectiveKind, ObjectiveSet};

fn match_with_roshan() -> World {
    let mut cfg = config();
    cfg.map = MapId(0);
    cfg.objectives = ObjectiveSet::of(ObjectiveKind::Roshan);
    let mut world = World::for_match(&cfg, cfg.rng());
    world.creeps_halted = true;
    world
}

#[test]
fn roshans_first_death_leaves_an_aegis_on_the_ground() {
    let mut world = match_with_roshan();
    let boss = world.lairs[0].unit.expect("boss");
    let hero = world.seats[0].unit.expect("hero");
    world.push_hit(Some(hero), boss, 100_000, DamageKind::Pure, None);
    world.step();
    let drops: Vec<_> = world
        .entities
        .iter()
        .filter_map(|e| world.loot.get(e).map(|l| l.0.id))
        .collect();
    assert_eq!(drops, vec![ItemId::AEGIS]);
}

fn kill(world: &mut World) {
    let boss = world.lairs[0].unit.expect("boss");
    world.push_hit(world.seats[0].unit, boss, 100_000, DamageKind::Pure, None);
    world.step();
}
fn revive_boss(world: &mut World) {
    let at = world.lairs[0].respawn_at;
    world.clock_shift.game = at - world.tick - 1;
    world.step();
}
fn loot(world: &World, item: ItemId) -> crate::game::Entity {
    world
        .entities
        .iter()
        .find(|entity| {
            world
                .loot
                .get(*entity)
                .is_some_and(|held| held.0.id == item)
        })
        .expect("drop")
}
fn take_aegis(world: &mut World) -> (crate::game::Entity, usize) {
    use bota_proto::{Order, Target, Vec2};
    let dropped = loot(world, ItemId::AEGIS);
    let hero = world.seats[0].unit.expect("hero");
    world.transform.get_mut(hero).expect("hero").pos =
        world.pos(dropped).expect("loot") + Vec2::from_ints(50, 0);
    world.settle();
    seat_order(
        world,
        Order::Take {
            target: Target::Unit(crate::game::wire_id(dropped)),
        },
    );
    run(world, 2);
    let at = world
        .inventory
        .get(hero)
        .expect("bag")
        .slots
        .iter()
        .position(|slot| slot.is_some_and(|held| held.id == ItemId::AEGIS))
        .expect("picked up");
    (hero, at)
}

#[test]
fn roshan_drops_match_kill_count_at_both_times_of_day_without_a_blessing() {
    let mut world = match_with_roshan();
    for count in 1..=5 {
        kill(&mut world);
        let mut drops: Vec<_> = world
            .entities
            .iter()
            .filter_map(|e| world.loot.get(e).map(|l| l.0.id))
            .collect();
        drops.sort();
        let mut expected = vec![ItemId::AEGIS];
        if count >= 2 {
            expected.push(ItemId::ROSHANS_BANNER);
        }
        if count >= 3 {
            expected.extend([ItemId::CHEESE, ItemId::REFRESHER_SHARD]);
        }
        expected.sort();
        assert_eq!(drops, expected, "kill {count}");
        if count >= 2 {
            let banner = world
                .loot
                .get(loot(&world, ItemId::ROSHANS_BANNER))
                .expect("banner")
                .0;
            assert_eq!(banner.level, (count - 1).min(3) as u8);
        }
        let ground: Vec<_> = world
            .entities
            .iter()
            .filter(|e| world.loot.get(*e).is_some())
            .collect();
        for item in ground {
            world.despawn(item);
        }
        revive_boss(&mut world);
    }
}

#[test]
fn aegis_reincarnates_at_the_death_position_without_rewards_death_counts_or_cooldown_reset() {
    use bota_core::rules;
    use bota_proto::{EventKind, Fixed, Vec2};
    let mut world = match_with_roshan();
    kill(&mut world);
    let (hero, slot) = take_aegis(&mut world);
    let at = Vec2::from_ints(9000, 9000);
    world.transform.get_mut(hero).expect("hero").pos = at;
    world.abilities.get_mut(hero).expect("book").slots[0].cooldown = 600;
    let deaths = world.seats[0].deaths;
    world.push_hit(None, hero, 100_000, DamageKind::Pure, None);
    let events = world.step();
    assert!(
        !events.iter().any(
            |e| matches!(e.kind(),EventKind::Died{unit,..} if unit==crate::game::wire_id(hero))
        )
    );
    assert_eq!(world.seats[0].deaths, deaths);
    assert!(world.pos(hero).is_none());
    assert!(world.inventory.get(hero).expect("bag").slots[slot].is_none());
    assert_eq!(world.seats[0].respawn_left, 5 * rules::TICKS_PER_SECOND);
    run(&mut world, 5 * rules::TICKS_PER_SECOND - 1);
    assert!(world.pos(hero).is_none());
    world.step();
    assert_eq!(world.pos(hero), Some(at));
    assert_eq!(
        world.health.get(hero).expect("health").hp,
        world.stats.get(hero).expect("stats").max_hp
    );
    assert_eq!(
        world.mana.get(hero).expect("mana").mana,
        world.stats.get(hero).expect("stats").max_mana
    );
    assert!(world.abilities.get(hero).expect("book").slots[0].cooldown > 300);
    assert!(
        !world
            .modifiers
            .get(hero)
            .expect("effects")
            .active()
            .any(|held| held.kind == bota_core::ModifierKind::FountainInvulnerable)
    );
    assert!(world.health.get(hero).expect("health").hp > Fixed::ZERO);
}

#[test]
fn aegis_lifetime_begins_on_pickup_and_expiry_restores_the_current_pools() {
    use bota_core::rules;
    use bota_proto::Fixed;
    let mut world = match_with_roshan();
    kill(&mut world);
    run(&mut world, 60 * rules::TICKS_PER_SECOND);
    assert_eq!(
        world
            .loot
            .get(loot(&world, ItemId::AEGIS))
            .expect("ground")
            .0
            .life,
        0
    );
    let (hero, slot) = take_aegis(&mut world);
    let life = world.inventory.get(hero).expect("bag").slots[slot]
        .expect("aegis")
        .life;
    assert!((300 * rules::TICKS_PER_SECOND - 5..=300 * rules::TICKS_PER_SECOND).contains(&life));
    world.inventory.get_mut(hero).expect("bag").slots[slot]
        .as_mut()
        .expect("aegis")
        .life = 1;
    world.health.get_mut(hero).expect("health").hp = Fixed::ONE;
    world.mana.get_mut(hero).expect("mana").mana = Fixed::ONE;
    world.step();
    assert!(world.inventory.get(hero).expect("bag").slots[slot].is_none());
    run(&mut world, 5 * rules::TICKS_PER_SECOND);
    assert_eq!(
        world.health.get(hero).expect("health").hp,
        world.stats.get(hero).expect("stats").max_hp
    );
    assert_eq!(
        world.mana.get(hero).expect("mana").mana,
        world.stats.get(hero).expect("stats").max_mana
    );
}

#[test]
fn an_aegis_cannot_be_dropped_sold_or_put_in_a_backpack() {
    use bota_proto::{ItemSlot, Order, RejectReason, Target};
    let mut world = match_with_roshan();
    kill(&mut world);
    let (_, slot) = take_aegis(&mut world);
    for (order, refused) in [
        (
            Order::Put {
                slot: ItemSlot(slot as u8),
                target: Target::None,
            },
            RejectReason::ItemRestricted,
        ),
        (
            Order::Swap {
                from: ItemSlot(slot as u8),
                to: ItemSlot(6),
            },
            RejectReason::ItemRestricted,
        ),
        (
            Order::Sell {
                slot: ItemSlot(slot as u8),
            },
            RejectReason::Unsellable,
        ),
    ] {
        assert_eq!(
            world.validate_order(world.seats[0].slot, None, &order),
            Err(refused)
        );
    }
}

#[test]
fn an_attack_can_deny_a_ground_aegis_without_a_creep_bounty_or_last_hit() {
    use bota_core::rules;
    use bota_proto::{Order, Target, Vec2};
    let mut world = match_with_roshan();
    kill(&mut world);
    let dropped = loot(&world, ItemId::AEGIS);
    let hero = world.seats[0].unit.expect("hero");
    world.transform.get_mut(hero).expect("hero").pos =
        world.pos(dropped).expect("drop") + Vec2::from_ints(50, 0);
    world.settle();
    let last_hits = world.seats[0].last_hits;
    seat_order(
        &mut world,
        Order::Attack {
            target: Target::Unit(crate::game::wire_id(dropped)),
        },
    );
    run(&mut world, 2 * rules::TICKS_PER_SECOND);
    assert!(!world.entities.contains(dropped));
    assert_eq!(world.seats[0].last_hits, last_hits);
}

#[test]
fn a_refresher_shard_resets_abilities_but_preserves_item_cooldowns_and_spends_mana() {
    use crate::game::ItemStack;
    use bota_proto::{Fixed, Target};
    let mut world = match_with_roshan();
    let hero = world.seats[0].unit.expect("hero");
    world.inventory.get_mut(hero).expect("bag").slots[0] = Some(ItemStack::granted(
        ItemId::REFRESHER_SHARD,
        world.seats[0].slot,
        world.tick,
        1,
    ));
    world.inventory.get_mut(hero).expect("bag").slots[1] =
        ItemStack::bought(ItemId::BLINK_DAGGER, world.seats[0].slot, 0);
    world.inventory.get_mut(hero).expect("bag").slots[1]
        .as_mut()
        .expect("blink")
        .cooldown = 100;
    world.abilities.get_mut(hero).expect("book").slots[0].cooldown = 600;
    world.mana.get_mut(hero).expect("mana").mana = Fixed::from_int(250);
    assert!(world.begin_item(hero, 0, Target::None));
    assert_eq!(
        world.abilities.get(hero).expect("book").slots[0].cooldown,
        0
    );
    assert_eq!(
        world.inventory.get(hero).expect("bag").slots[1]
            .expect("blink")
            .cooldown,
        100
    );
    assert_eq!(
        world.mana.get(hero).expect("mana").mana,
        Fixed::from_int(50)
    );
    assert!(world.inventory.get(hero).expect("bag").slots[0].is_none());
}

#[test]
fn roshan_reclaims_a_late_picked_aegis_and_an_unclaimed_ground_aegis() {
    use bota_core::rules;
    let mut world = match_with_roshan();
    kill(&mut world);
    let ground = loot(&world, ItemId::AEGIS);
    revive_boss(&mut world);
    assert!(!world.entities.contains(ground));
    kill(&mut world);
    let return_at = world.lairs[0].respawn_at;
    world.clock_shift.game = return_at - 10 * rules::TICKS_PER_SECOND - world.tick;
    world.step();
    let (hero, slot) = take_aegis(&mut world);
    revive_boss(&mut world);
    assert!(world.inventory.get(hero).expect("bag").slots[slot].is_none());
    assert!(
        world
            .modifiers
            .get(hero)
            .expect("effects")
            .active()
            .any(|held| matches!(held.kind, bota_core::ModifierKind::Recovery { .. }))
    );
    assert!(
        world.entities.iter().any(|e| world
            .loot
            .get(e)
            .is_some_and(|loot| loot.0.id == ItemId::ROSHANS_BANNER)),
        "other drops stay"
    );
}

#[test]
fn expiry_on_the_lethal_tick_precedes_reincarnation_and_counts_an_ordinary_death() {
    use bota_proto::EventKind;
    let mut world = match_with_roshan();
    kill(&mut world);
    let (hero, slot) = take_aegis(&mut world);
    world.inventory.get_mut(hero).expect("bag").slots[slot]
        .as_mut()
        .expect("aegis")
        .life = 1;
    world.push_hit(None, hero, 100_000, DamageKind::Pure, None);
    let events = world.step();
    assert_eq!(world.seats[0].deaths, 1);
    assert!(world.seats[0].reincarnating_at.is_none());
    assert!(
        events.iter().any(
            |e| matches!(e.kind(),EventKind::Died {unit,..} if unit==crate::game::wire_id(hero))
        )
    );
}

#[test]
fn expiry_recovery_is_broken_by_an_allied_players_damage_but_not_self_damage() {
    use bota_proto::{HeroId, SlotId, Team, Vec2};
    let mut world = match_with_roshan();
    kill(&mut world);
    let (hero, slot) = take_aegis(&mut world);
    world.inventory.get_mut(hero).expect("bag").slots[slot]
        .as_mut()
        .expect("aegis")
        .life = 1;
    world.step();
    let recovering = |world: &World| {
        world
            .modifiers
            .get(hero)
            .expect("effects")
            .active()
            .any(|held| matches!(held.kind, bota_core::ModifierKind::Recovery { .. }))
    };
    assert!(recovering(&world));
    world.push_hit(Some(hero), hero, 1, DamageKind::Pure, None);
    world.step();
    assert!(recovering(&world));
    let ally = world.spawn_hero(
        Team::Radiant,
        world.pos(hero).expect("hero") + Vec2::from_ints(200, 0),
        SlotId(1),
        HeroId::PUDGE,
    );
    world.settle();
    world.push_hit(Some(ally), hero, 1, DamageKind::Pure, None);
    world.step();
    assert!(!recovering(&world));
}

#[test]
fn swapping_aegis_into_an_in_flight_item_errand_cannot_drop_or_share_it() {
    use crate::game::{ItemStack, wire_id};
    use bota_core::rules;
    use bota_proto::{ItemSlot, Order, Target, Vec2};
    for hand_over in [false, true] {
        let mut world = match_with_roshan();
        kill(&mut world);
        let (hero, aegis_slot) = take_aegis(&mut world);
        let at = Vec2::from_ints(8000, 9600);
        world.transform.get_mut(hero).expect("hero").pos = at;
        let courier = world.seats[0].courier.expect("courier");
        world.transform.get_mut(courier).expect("courier").pos = at + Vec2::from_ints(800, 0);
        world.settle();
        let other_slot = if aegis_slot == 0 { 1 } else { 0 };
        world.inventory.get_mut(hero).expect("bag").slots[other_slot] =
            ItemStack::bought(ItemId::BOOTS, world.seats[0].slot, 0);
        let to = if hand_over {
            Target::Unit(wire_id(courier))
        } else {
            Target::Pos(at + Vec2::from_ints(800, 0))
        };
        seat_order(
            &mut world,
            Order::Put {
                slot: ItemSlot(other_slot as u8),
                target: to,
            },
        );
        assert!(world.handling.get(hero).is_some());
        seat_order(
            &mut world,
            Order::Swap {
                from: ItemSlot(aegis_slot as u8),
                to: ItemSlot(other_slot as u8),
            },
        );
        run(&mut world, 10 * rules::TICKS_PER_SECOND);
        assert!(world.handling.get(hero).is_none());
        assert!(
            world
                .inventory
                .get(hero)
                .expect("bag")
                .held()
                .any(|held| held.id == ItemId::AEGIS),
            "Aegis must remain with its carrier, hand_over={hand_over}"
        );
    }
}

#[test]
fn an_illusion_copy_of_aegis_does_not_grant_expiry_recovery() {
    let mut world = match_with_roshan();
    kill(&mut world);
    let (hero, slot) = take_aegis(&mut world);
    world.inventory.get_mut(hero).expect("bag").slots[slot]
        .as_mut()
        .expect("aegis")
        .life = 2;
    world.make_rune_illusions(hero);
    let copies: Vec<_> = world
        .entities
        .iter()
        .filter(|entity| world.illusion.get(*entity).is_some())
        .collect();
    assert!(!copies.is_empty());
    run(&mut world, 3);
    for copy in copies {
        assert!(
            !world
                .modifiers
                .get(copy)
                .expect("effects")
                .active()
                .any(|held| matches!(held.kind, bota_core::ModifierKind::Recovery { .. }))
        );
    }
}

#[test]
fn refresher_copies_share_the_use_cooldown_and_pause_it_in_the_backpack() {
    use crate::game::ItemStack;
    use bota_core::rules;
    use bota_proto::{Fixed, Target};
    let mut world = match_with_roshan();
    let hero = world.seats[0].unit.expect("hero");
    for slot in 0..2 {
        world.inventory.get_mut(hero).expect("bag").slots[slot] = Some(ItemStack::granted(
            ItemId::REFRESHER_SHARD,
            world.seats[0].slot,
            world.tick,
            1,
        ));
    }
    world.mana.get_mut(hero).expect("mana").mana = Fixed::from_int(250);
    assert!(world.begin_item(hero, 0, Target::None));
    let cooldown = 180 * rules::TICKS_PER_SECOND;
    assert_eq!(
        world.inventory.get(hero).expect("bag").slots[1]
            .expect("other shard")
            .cooldown,
        cooldown
    );
    world.inventory.get_mut(hero).expect("bag").swap_slots(1, 6);
    run(&mut world, 10);
    assert_eq!(
        world.inventory.get(hero).expect("bag").slots[6]
            .expect("backpack shard")
            .cooldown,
        cooldown
    );
    world.inventory.get_mut(hero).expect("bag").swap_slots(6, 1);
    world.step();
    assert_eq!(
        world.inventory.get(hero).expect("bag").slots[1]
            .expect("inventory shard")
            .cooldown,
        cooldown - 1
    );
}

#[test]
fn a_banner_preserves_its_level_when_picked_up_dropped_and_taken_again() {
    use bota_proto::{Order, Target};
    let (mut world, hero) = super::support::match_hero();
    let at = super::support::an_empty_spot(&world);
    world.transform.get_mut(hero).expect("hero").pos = at;
    let mut stack = crate::game::ItemStack::dropped(ItemId::ROSHANS_BANNER).expect("drop");
    stack.level = 3;
    let dropped = world.lay_loot(stack, at);
    world.settle();
    seat_order(
        &mut world,
        Order::Take {
            target: Target::Unit(crate::game::wire_id(dropped)),
        },
    );
    run(&mut world, 2);
    let slot = world
        .inventory
        .get(hero)
        .expect("bag")
        .slots
        .iter()
        .position(|s| s.is_some_and(|s| s.id == ItemId::ROSHANS_BANNER))
        .expect("picked up");
    assert_eq!(
        world.inventory.get(hero).expect("bag").slots[slot]
            .expect("held")
            .level,
        3
    );
    let view = world.view(bota_proto::Team::Radiant);
    assert_eq!(
        view.units
            .iter()
            .find(|u| u.id == crate::game::wire_id(hero))
            .expect("hero")
            .items[slot]
            .expect("item")
            .level,
        3
    );
    seat_order(
        &mut world,
        Order::Put {
            slot: bota_proto::ItemSlot(slot as u8),
            target: Target::Pos(at),
        },
    );
    run(&mut world, 2);
    let dropped = loot(&world, ItemId::ROSHANS_BANNER);
    assert_eq!(world.loot.get(dropped).expect("drop").0.level, 3);
    seat_order(
        &mut world,
        Order::Take {
            target: Target::Unit(crate::game::wire_id(dropped)),
        },
    );
    run(&mut world, 2);
    assert_eq!(
        world.inventory.get(hero).expect("bag").slots[slot]
            .expect("taken again")
            .level,
        3
    );
}

#[test]
fn shopping_during_aegis_reincarnation_keeps_the_purchase_in_stash_after_return() {
    use bota_core::rules;
    use bota_proto::{Order, Vec2};
    let mut world = match_with_roshan();
    kill(&mut world);
    let (hero, _) = take_aegis(&mut world);
    let at = Vec2::from_ints(9000, 9000);
    world.transform.get_mut(hero).expect("hero").pos = at;
    world.seats[0].gold = 5000;
    world.settle();
    world.push_hit(None, hero, 100000, DamageKind::Pure, None);
    world.step();
    assert!(world.seats[0].reincarnating_at.is_some());
    let before = world.seats[0].gold;
    seat_order(
        &mut world,
        Order::Buy {
            item: ItemId::TANGO,
        },
    );
    assert!(world.seats[0].gold < before, "the purchase succeeded");
    assert!(
        world.seats[0]
            .stash
            .slots
            .iter()
            .flatten()
            .any(|s| s.id == ItemId::TANGO)
    );
    run(&mut world, 5 * rules::TICKS_PER_SECOND + 2);
    assert!(world.alive(hero));
    assert_eq!(world.pos(hero), Some(at));
    assert!(
        world.seats[0]
            .stash
            .slots
            .iter()
            .flatten()
            .any(|s| s.id == ItemId::TANGO)
    );
    assert!(
        !world
            .inventory
            .get(hero)
            .expect("bag")
            .held()
            .any(|s| s.id == ItemId::TANGO)
    );
}

#[test]
fn cheese_from_two_separate_ground_pickups_combines_into_one_charged_stack() {
    use bota_proto::{Order, Target, Vec2};
    let (mut world, hero) = match_hero();
    let at = world.pos(hero).expect("hero");
    for _ in 0..2 {
        let dropped = world.lay_loot(
            crate::game::ItemStack::dropped(ItemId::CHEESE).expect("Cheese"),
            at + Vec2::from_ints(40, 0),
        );
        world.settle();
        seat_order(
            &mut world,
            Order::Take {
                target: Target::Unit(crate::game::wire_id(dropped)),
            },
        );
        run(&mut world, 3);
        assert!(!world.entities.contains(dropped));
    }
    let stacks: Vec<_> = world
        .inventory
        .get(hero)
        .expect("bag")
        .held()
        .filter(|s| s.id == ItemId::CHEESE)
        .collect();
    assert_eq!(stacks.len(), 1);
    assert_eq!(stacks[0].charges, 2);
}

#[test]
fn killing_roshan_does_not_capture_any_watchers_after_734_removed_that_reward() {
    use bota_proto::UnitKind;
    let mut cfg = config();
    cfg.map = MapId(0);
    cfg.objectives = ObjectiveSet::of(ObjectiveKind::Roshan).with(ObjectiveKind::Watchers);
    let mut world = World::for_match(&cfg, cfg.rng());
    world.creeps_halted = true;
    let owners = |world: &World| {
        world
            .entities
            .iter()
            .filter(|u| world.kind.get(*u) == Some(&UnitKind::Watcher))
            .map(|u| (u, world.team.get(u).copied()))
            .collect::<Vec<_>>()
    };
    let before = owners(&world);
    assert!(!before.is_empty());
    kill(&mut world);
    assert_eq!(owners(&world), before);
}
