//! The Town Portal Scroll at Dota 7.41f: its slot, its numbers, where it
//! carries, what breaks it, and what both sides are shown.

use bota_core::{ModifierKind, fountain_pos, hero_spawn_pos, rules};

use crate::game::{Entity, Event, UnitOrder, World, wire_id};
use crate::game_loop::visible_events;
use bota_proto::{
    AbilitySlot, DamageKind, EventKind, Fixed, ItemId, ItemSlot, Order, RejectReason, SlotId,
    StatusFlags, Target, Team, UnitKind, Vec2,
};

use super::support::*;

/// What the hero's town portal slot holds.
fn scrolls(world: &World, hero: Entity) -> Option<crate::game::ItemStack> {
    world
        .inventory
        .get(hero)
        .and_then(|bag| bag.town_portal)
        .expect("a hero has a town portal slot")
}

/// Seat 0 reads its scroll at a target, the way a player does.
fn read_at(world: &mut World, target: Target) {
    advance_validated(
        world,
        None,
        Order::Use {
            slot: ItemSlot::TOWN_PORTAL,
            target,
        },
    );
}

/// Why seat 0 may not read its scroll at a target, if it may not.
fn refusal(world: &World, target: Target) -> Result<(), RejectReason> {
    world.validate_order(
        SlotId(0),
        None,
        &Order::Use {
            slot: ItemSlot::TOWN_PORTAL,
            target,
        },
    )
}

fn pos(world: &World, hero: Entity) -> Vec2 {
    world.transform.get(hero).expect("standing").pos
}

/// The first standing building of a kind and side.
fn building(world: &World, kind: UnitKind, team: Team) -> Entity {
    world
        .entities
        .iter()
        .find(|entity| {
            world.kind.get(*entity) == Some(&kind) && world.team.get(*entity) == Some(&team)
        })
        .expect("the map has one")
}

/// Ticks a channel begun by an order takes to carry, counted from the tick
/// the order is taken on.
const CHANNEL: u32 = rules::SCROLL_CHANNEL_TICKS;

#[test]
fn every_hero_starts_with_one_free_scroll_ready_ten_seconds_past_the_horn() {
    let (mut world, hero) = match_hero();
    let scroll = scrolls(&world, hero).expect("the slot holds the starting scroll");
    assert_eq!(scroll.id, ItemId::TOWN_PORTAL_SCROLL);
    assert_eq!(scroll.charges, 1);
    assert_eq!(scroll.free_charges, 1, "and that one came for nothing");
    assert_eq!(
        rules::SCROLL_FIRST_WAIT_TICKS,
        rules::PREGAME_TICKS + 10 * rules::TICKS_PER_SECOND
    );
    run(&mut world, rules::SCROLL_FIRST_WAIT_TICKS - 1);
    let far = Vec2::from_ints(9000, 9000);
    assert_eq!(
        refusal(&world, Target::Pos(far)),
        Err(RejectReason::OnCooldown)
    );
    let shown = shown(&world, Some(Team::Radiant), hero)
        .town_portal
        .expect("the slot crosses the wire");
    assert_eq!(shown.cooldown_left, 1, "the wait shows on the scroll");
    world.step();
    assert_eq!(
        refusal(&world, Target::Pos(far)),
        Ok(()),
        "ten seconds past the horn"
    );
}

#[test]
fn a_scroll_spends_mana_and_its_wait_as_it_is_read_and_itself_on_landing() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let to = beside_own_tower(&world);
    let from = pos(&world, hero);
    let mana = world.mana.get(hero).expect("has mana").mana;
    world.set_order(
        hero,
        UnitOrder::Move {
            pos: Vec2::from_ints(8000, 12000),
        },
    );
    read_at(&mut world, Target::Pos(to));
    assert!(world.is_channelling(hero), "it stands through the channel");
    let after = world.mana.get(hero).expect("has mana").mana;
    assert!(
        mana - after >= Fixed::from_int(rules::SCROLL_MANA - 1),
        "75 mana went at once: {mana:?} then {after:?}"
    );
    assert!(
        world.owes_wait(hero, ItemId::TOWN_PORTAL_SCROLL),
        "and the wait began"
    );
    let statuses = shown(&world, Some(Team::Radiant), hero).statuses;
    assert_ne!(statuses.bits & StatusFlags::CHANNELLING, 0);
    run(&mut world, CHANNEL - 1);
    assert_eq!(
        pos(&world, hero),
        from,
        "a tick short of three seconds it is still home"
    );
    assert!(scrolls(&world, hero).is_some(), "with its scroll");
    world.step();
    assert_eq!(pos(&world, hero), to, "three seconds in it is there");
    assert!(scrolls(&world, hero).is_none(), "and the scroll is spent");
    run(&mut world, 30);
    assert_eq!(
        pos(&world, hero),
        to,
        "standing rather than walking back to what it was told"
    );
}

#[test]
fn a_scroll_wants_its_mana() {
    let (mut world, hero) = a_hero_with_a_scroll();
    world.mana.get_mut(hero).expect("has mana").mana = Fixed::from_int(rules::SCROLL_MANA - 1);
    assert_eq!(
        refusal(&world, Target::Pos(beside_own_tower(&world))),
        Err(RejectReason::NotEnoughMana)
    );
}

#[test]
fn a_broken_channel_keeps_the_scroll_but_not_the_mana_or_the_wait() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let mana = world.mana.get(hero).expect("has mana").mana;
    let to = beside_own_tower(&world);
    read_at(&mut world, Target::Pos(to));
    stop_click(&mut world);
    assert!(
        !world.is_channelling(hero),
        "the order took the channel away"
    );
    assert!(scrolls(&world, hero).is_some(), "the scroll is still there");
    assert!(world.mana.get(hero).expect("has mana").mana < mana);
    assert!(world.owes_wait(hero, ItemId::TOWN_PORTAL_SCROLL));
    assert!(
        world.view(Team::Dire).teleports.is_empty(),
        "nobody is shown it any more"
    );
}

/// Dota's channels break on any order to the body unless the player set the
/// client's "Require Hold/Stop" option for them (see the client core); a
/// move to a spot breaks the scroll as a stop does, and one given with Shift
/// waits for it to land.
#[test]
fn a_move_breaks_the_channel_and_a_queued_one_waits_for_the_landing() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let to = beside_own_tower(&world);
    let walk = Order::Move {
        target: Target::Pos(Vec2::from_ints(5000, 5000)),
    };
    read_at(&mut world, Target::Pos(to));
    seat_order(&mut world, walk);
    assert!(!world.is_channelling(hero), "an unqueued move breaks it");
    assert!(world.owes_wait(hero, ItemId::TOWN_PORTAL_SCROLL));
    let (mut world, hero) = a_hero_with_a_scroll();
    read_at(&mut world, Target::Pos(to));
    world.advance(&[crate::game::Command {
        slot: SlotId(0),
        unit: None,
        order: walk,
        queue: true,
        seq: 0,
    }]);
    assert!(world.is_channelling(hero), "a queued move leaves it be");
    run(&mut world, rules::SCROLL_CHANNEL_TICKS + 2);
    assert!(!world.is_channelling(hero));
    assert!(
        scrolls(&world, hero).is_none(),
        "the scroll was spent: the channel landed"
    );
}

#[test]
fn a_stun_breaks_the_channel() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let from = pos(&world, hero);
    let to = beside_own_tower(&world);
    read_at(&mut world, Target::Pos(to));
    put_on(&mut world, hero, ModifierKind::Stunned, 5);
    run(&mut world, CHANNEL);
    assert_eq!(pos(&world, hero), from, "it never went");
    assert!(scrolls(&world, hero).is_some());
}

#[test]
fn damage_does_not_break_the_channel() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let to = beside_own_tower(&world);
    let beside = pos(&world, hero) + Vec2::from_ints(60, 0);
    let biter = thinking_creep_of(&mut world, Team::Dire, beside);
    let hp = world.health.get(hero).expect("alive").hp;
    read_at(&mut world, Target::Pos(to));
    run(&mut world, CHANNEL - 1);
    assert!(world.alive(biter));
    assert!(
        world.health.get(hero).expect("alive").hp < hp,
        "it was bitten"
    );
    world.step();
    assert_eq!(pos(&world, hero), to, "and went all the same");
}

#[test]
fn the_channel_breaks_when_its_building_falls() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let to = beside_own_tower(&world);
    let from = pos(&world, hero);
    read_at(&mut world, Target::Pos(to));
    let tower = building(&world, UnitKind::Tower, Team::Radiant);
    let standing = pos(&world, tower);
    fell_at(&mut world, standing);
    run(&mut world, CHANNEL);
    assert!(!world.is_channelling(hero));
    assert_eq!(pos(&world, hero), from, "there was nowhere left to go");
    assert!(scrolls(&world, hero).is_some());
}

#[test]
fn a_spot_carries_to_the_nearest_building_and_lands_within_its_band() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let tower = building(&world, UnitKind::Tower, Team::Radiant);
    let centre = pos(&world, tower);
    // Far beyond the tower, straight out from the fountain's side of it.
    let aim = centre + Vec2::from_ints(3000, 0);
    let nearest = world
        .entities
        .iter()
        .filter(|entity| {
            world.team.get(*entity) == Some(&Team::Radiant)
                && matches!(
                    world.kind.get(*entity),
                    Some(
                        UnitKind::Tower
                            | UnitKind::Barracks
                            | UnitKind::Ancient
                            | UnitKind::Fountain
                    )
                )
        })
        .min_by_key(|entity| pos(&world, *entity).distance_squared(aim))
        .expect("buildings stand");
    let (went_to, landing) = world
        .scroll_destination(hero, Target::Pos(aim))
        .expect("a spot always has a nearest building");
    assert_eq!(went_to, nearest);
    let from_it = pos(&world, nearest);
    assert!(landing.within(from_it, rules::units(rules::SCROLL_MAX_DISTANCE + 1)));
    assert!(!landing.within(from_it, rules::units(rules::SCROLL_MIN_DISTANCE - 1)));
    read_at(&mut world, Target::Pos(aim));
    run(&mut world, CHANNEL);
    assert_eq!(pos(&world, hero), landing);
}

#[test]
fn a_building_named_is_landed_beside_and_an_enemy_one_is_refused() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let ours = building(&world, UnitKind::Tower, Team::Radiant);
    let theirs = building(&world, UnitKind::Tower, Team::Dire);
    assert_eq!(
        refusal(&world, Target::Unit(wire_id(theirs))),
        Err(RejectReason::WrongTargetKind)
    );
    read_at(&mut world, Target::Unit(wire_id(ours)));
    run(&mut world, CHANNEL);
    let at = pos(&world, hero);
    let centre = pos(&world, ours);
    assert!(at.within(centre, rules::units(rules::SCROLL_MAX_DISTANCE)));
    assert!(!at.within(centre, rules::units(rules::SCROLL_MIN_DISTANCE - 1)));
}

#[test]
fn nothing_aimed_carries_to_the_fountain_but_not_from_beside_it() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let home = fountain_pos(world.map, Team::Radiant);
    let spawn = hero_spawn_pos(world.map, Team::Radiant);
    world.transform.get_mut(hero).expect("standing").pos = spawn;
    assert_eq!(
        refusal(&world, Target::None),
        Err(RejectReason::WrongTargetKind)
    );
    let away = an_empty_spot(&world);
    assert!(!away.within(home, rules::units(rules::SCROLL_FOUNTAIN_MIN_DISTANCE)));
    world.transform.get_mut(hero).expect("standing").pos = away;
    read_at(&mut world, Target::None);
    run(&mut world, CHANNEL);
    assert_eq!(pos(&world, hero), spawn, "it lands where heroes come in");
}

#[test]
fn both_sides_are_shown_where_a_scroll_lands_and_how_long_it_has() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let to = beside_own_tower(&world);
    read_at(&mut world, Target::Pos(to));
    let seen = world.view(Team::Dire).teleports;
    assert_eq!(seen.len(), 1, "the enemy is shown it through the fog");
    let shown = seen[0];
    assert_eq!(shown.slot, SlotId(0));
    assert_eq!(shown.team, Team::Radiant);
    assert_eq!(shown.destination, to);
    assert_eq!(shown.ticks_total, CHANNEL);
    assert_eq!(shown.ticks_left, CHANNEL, "it lands that many steps on");
    assert_eq!(
        shown.building,
        wire_id(building(&world, UnitKind::Tower, Team::Radiant))
    );
    run(&mut world, CHANNEL - 1);
    assert_eq!(world.view(Team::Dire).teleports[0].ticks_left, 1);
    world.step();
    assert!(world.view(Team::Dire).teleports.is_empty(), "landed");
    assert_eq!(pos(&world, hero), to);
}

#[test]
fn a_scroll_shows_its_side_the_ground_it_lands_on_a_moment_past_the_channel() {
    let (mut world, _) = a_hero_with_a_scroll();
    // A spot in reach of a building of its own that its side does not see:
    // behind trees or up a cliff from everything it has, and out of the
    // fountain's reach, which shoots whatever its side comes to see.
    let tower = building(&world, UnitKind::Tower, Team::Radiant);
    let centre = pos(&world, tower);
    let fountain = world.map.fountains[0];
    let out_of_fountain_reach = rules::FOUNTAIN_ATTACK_RANGE
        + rules::FOUNTAIN_BOUND
        + rules::MELEE_CREEP_BOUND
        + rules::ATTACK_RANGE_LEEWAY;
    let hidden = (-7..=7)
        .flat_map(|dx: i32| (-7..=7).map(move |dy: i32| (dx, dy)))
        .map(|(dx, dy)| centre + Vec2::from_ints(dx * 100, dy * 100))
        .find(|spot| {
            world.clearance.stands_clear(*spot)
                && !spot.within(centre, rules::units(rules::SCROLL_MIN_DISTANCE))
                && !spot.within(fountain, rules::units(out_of_fountain_reach))
                && !world.can_see_point(Team::Radiant, *spot)
        });
    let Some(hidden) = hidden else {
        panic!("a tower has some ground it does not see within 700 of it");
    };
    let lurker = a_creep_at(&mut world, Team::Dire, hidden);
    world.step();
    assert!(!world.can_see(Team::Radiant, lurker), "hidden before");
    read_at(&mut world, Target::Pos(hidden));
    world.step();
    assert!(
        world.can_see(Team::Radiant, lurker),
        "shown while the scroll is read"
    );
    stop_click(&mut world);
    for _ in 0..rules::SCROLL_SIGHT_LINGER_TICKS - 1 {
        world.step();
        assert!(world.can_see(Team::Radiant, lurker), "and a moment past it");
    }
    run(&mut world, 2);
    assert!(!world.can_see(Team::Radiant, lurker), "then no more");
}

#[test]
fn a_death_hands_out_a_scroll_unless_neutrals_dealt_it() {
    for (killer_team, handed) in [(Team::Dire, 1), (Team::Neutral, 0)] {
        let (mut world, hero) = a_hero_with_a_scroll();
        let beside = pos(&world, hero) + Vec2::from_ints(80, 0);
        let killer = a_creep_at(&mut world, killer_team, beside);
        world.bury(vec![(hero, Some(killer))], &mut Vec::new());
        let kept = world.seats[0]
            .kept
            .as_ref()
            .and_then(|kept| kept.bag.town_portal)
            .flatten()
            .expect("the starting scroll is kept");
        assert_eq!(kept.charges, 1 + handed, "a death to {killer_team:?}");
        assert_eq!(kept.free_charges, 1 + handed);
        let hero = back_on_its_feet(&mut world);
        assert_eq!(scrolls(&world, hero).map(|s| s.charges), Some(1 + handed));
    }
}

#[test]
fn a_scroll_bought_at_the_shop_joins_the_slot_and_one_bought_away_waits_in_the_stash() {
    let (mut world, hero) = a_hero_with_gold(1000);
    advance_validated(
        &mut world,
        None,
        Order::Buy {
            item: ItemId::TOWN_PORTAL_SCROLL,
        },
    );
    let held = scrolls(&world, hero).expect("held");
    assert_eq!((held.charges, held.free_charges), (2, 1));
    assert_eq!(world.seats[0].gold, 900, "a scroll is 100 gold");
    world.transform.get_mut(hero).expect("standing").pos = an_empty_spot(&world);
    advance_validated(
        &mut world,
        None,
        Order::Buy {
            item: ItemId::TOWN_PORTAL_SCROLL,
        },
    );
    assert_eq!(scrolls(&world, hero).map(|s| s.charges), Some(2));
    assert!(
        world.seats[0].stash.slots[0].is_some_and(|stack| stack.is_scroll()),
        "the one bought away waits in the stash"
    );
}

#[test]
fn a_courier_hands_scrolls_into_the_town_portal_slot_whatever_the_inventory_holds() {
    let (mut world, hero, courier) = courier_on_delivery();
    if let Some(bag) = world.inventory.get_mut(hero) {
        for slot in bag.slots.iter_mut() {
            *slot = Some(fresh_stack(ItemId::BOOTS, 0));
        }
    }
    hand_item(&mut world, courier, ItemId::TOWN_PORTAL_SCROLL, 1);
    for _ in 0..30 * rules::TICKS_PER_SECOND {
        world.step();
        if scrolls(&world, hero).is_some_and(|stack| stack.charges == 2) {
            return;
        }
    }
    panic!("the scroll never reached the town portal slot");
}

#[test]
fn the_town_portal_slot_takes_scrolls_and_gives_nothing_up() {
    let (mut world, hero) = a_hero_with_gold(0);
    let swap = |from: u8, to: u8| Order::Swap {
        from: ItemSlot(from),
        to: ItemSlot(to),
    };
    let out = swap(ItemSlot::TOWN_PORTAL.0, 0);
    assert_eq!(
        world.validate_order(SlotId(0), None, &out),
        Err(RejectReason::WrongTargetKind)
    );
    world.seats[0].stash.slots[0] = Some(fresh_stack(ItemId::BOOTS, 0));
    let boots_in = swap(9, ItemSlot::TOWN_PORTAL.0);
    assert_eq!(
        world.validate_order(SlotId(0), None, &boots_in),
        Err(RejectReason::WrongTargetKind)
    );
    world.seats[0].stash.slots[1] = Some(fresh_stack(ItemId::TOWN_PORTAL_SCROLL, 2));
    advance_validated(&mut world, None, swap(10, ItemSlot::TOWN_PORTAL.0));
    assert_eq!(scrolls(&world, hero).map(|s| s.charges), Some(3));
    assert!(world.seats[0].stash.slots[1].is_none());
}

#[test]
fn a_free_scroll_is_never_bought_back_and_is_spent_first() {
    let (mut world, hero) = a_hero_with_a_scroll();
    world.seats[0].gold = 100;
    world.transform.get_mut(hero).expect("standing").pos = hero_spawn_pos(world.map, Team::Radiant);
    let sell = Order::Sell {
        slot: ItemSlot::TOWN_PORTAL,
    };
    assert_eq!(
        world.validate_order(SlotId(0), None, &sell),
        Err(RejectReason::Unsellable)
    );
    advance_validated(
        &mut world,
        None,
        Order::Buy {
            item: ItemId::TOWN_PORTAL_SCROLL,
        },
    );
    world.transform.get_mut(hero).expect("standing").pos = an_empty_spot(&world);
    let to = beside_own_tower(&world);
    read_at(&mut world, Target::Pos(to));
    run(&mut world, CHANNEL);
    let left = scrolls(&world, hero).expect("one left");
    assert_eq!(
        (left.charges, left.free_charges),
        (1, 0),
        "the free one went first"
    );
    world.transform.get_mut(hero).expect("standing").pos = hero_spawn_pos(world.map, Team::Radiant);
    let gold = world.seats[0].gold;
    advance_validated(&mut world, None, sell);
    assert_eq!(
        world.seats[0].gold - gold,
        50,
        "a used scroll sells for half"
    );
    assert!(scrolls(&world, hero).is_none());
}

#[test]
fn a_cast_breaks_the_channel_and_goes_off() {
    let (mut world, hero) = a_hero_with_a_scroll();
    set_level(&mut world, hero, 0, 1);
    let to = beside_own_tower(&world);
    read_at(&mut world, Target::Pos(to));
    advance_validated(
        &mut world,
        None,
        Order::Cast {
            slot: AbilitySlot(0),
            target: Target::None,
        },
    );
    assert!(
        !world.is_channelling(hero),
        "the raze took the channel's place"
    );
    run(&mut world, 30);
    let raze = world.abilities.get(hero).expect("has a book").slots[0];
    assert!(raze.cooldown > 0, "and went off");
}

#[test]
fn phase_boots_leave_the_channel_be_and_a_feast_breaks_it() {
    let (mut world, hero) = a_hero_with_a_scroll();
    hand_item(&mut world, hero, ItemId::PHASE_BOOTS, 0);
    let to = beside_own_tower(&world);
    read_at(&mut world, Target::Pos(to));
    advance_validated(
        &mut world,
        None,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::None,
        },
    );
    assert!(
        world.is_channelling(hero),
        "phasing is no order to the channel"
    );
    assert!(carries(&world, hero, ModifierKind::Phased), "and it phased");
    set_level(&mut world, hero, 3, 1);
    cast(&mut world, 3, Target::None);
    assert!(!world.is_channelling(hero), "a feast is");
}

/// The teleports each side was told ended in a tick's events, Radiant's
/// first: seat and whether the hero landed.
fn ends_told(events: &[Event]) -> [Vec<(SlotId, bool)>; 2] {
    [Team::Radiant, Team::Dire].map(|side| {
        visible_events(events, Some(side))
            .into_iter()
            .filter_map(|kind| match kind {
                EventKind::Teleported { slot, landed } => Some((slot, landed)),
                _ => None,
            })
            .collect()
    })
}

/// The ways a channel breaks before it lands.
#[derive(Clone, Copy, Debug)]
enum Break {
    /// The hero is ordered to stop.
    Order,
    /// The hero is stunned.
    Stun,
    /// The building it was aimed at falls.
    Building,
    /// The hero is killed.
    Death,
}

/// Breaks seat 0's channel one way, and answers the events of the tick it
/// breaks on.
fn break_channel(world: &mut World, hero: Entity, how: Break) -> Vec<Event> {
    match how {
        Break::Order => seat_order(
            world,
            Order::Move {
                target: Target::None,
            },
        ),
        Break::Stun => {
            put_on(world, hero, ModifierKind::Stunned, 5);
            world.advance(&[])
        }
        Break::Building => {
            let tower = building(world, UnitKind::Tower, Team::Radiant);
            let standing = structure_at(world, pos(world, tower));
            world.bury(vec![(standing, None)], &mut Vec::new());
            world.advance(&[])
        }
        Break::Death => {
            world.push_hit(None, hero, 100_000, DamageKind::Pure, None);
            world.advance(&[])
        }
    }
}

#[test]
fn a_channel_that_runs_out_is_told_to_both_sides_as_landed_on_the_tick_it_lands() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let to = beside_own_tower(&world);
    read_at(&mut world, Target::Pos(to));
    for _ in 1..CHANNEL {
        assert_eq!(
            ends_told(&world.advance(&[])),
            [vec![], vec![]],
            "still under way"
        );
    }
    let events = world.advance(&[]);
    assert_eq!(
        ends_told(&events),
        [vec![(SlotId(0), true)], vec![(SlotId(0), true)]]
    );
    assert!(
        world.view(Team::Dire).teleports.is_empty(),
        "the views drop it on the same tick"
    );
    assert_eq!(pos(&world, hero), to);
}

#[test]
fn a_channel_broken_off_is_told_to_both_sides_as_not_landed_once_on_the_tick_it_breaks() {
    for how in [Break::Order, Break::Stun, Break::Building, Break::Death] {
        let (mut world, hero) = a_hero_with_a_scroll();
        let to = beside_own_tower(&world);
        read_at(&mut world, Target::Pos(to));
        run(&mut world, 5);
        let events = break_channel(&mut world, hero, how);
        assert_eq!(
            ends_told(&events),
            [vec![(SlotId(0), false)], vec![(SlotId(0), false)]],
            "{how:?}"
        );
        assert!(world.view(Team::Dire).teleports.is_empty(), "{how:?}");
        for _ in 0..CHANNEL {
            assert_eq!(
                ends_told(&world.advance(&[])),
                [vec![], vec![]],
                "{how:?}: not told again"
            );
        }
    }
}

#[test]
fn a_channel_with_no_open_ground_to_land_on_is_told_as_not_landed_and_keeps_its_scroll() {
    let (mut world, hero) = a_hero_with_a_scroll();
    let from = pos(&world, hero);
    let to = beside_own_tower(&world);
    read_at(&mut world, Target::Pos(to));
    run(&mut world, 5);
    stand_a_wall(&mut world, to, rules::units(1000));
    for _ in 0..CHANNEL - 6 {
        assert_eq!(ends_told(&world.advance(&[])), [vec![], vec![]]);
    }
    let events = world.advance(&[]);
    assert_eq!(
        ends_told(&events),
        [vec![(SlotId(0), false)], vec![(SlotId(0), false)]]
    );
    assert_eq!(pos(&world, hero), from, "the hero never went");
    assert!(scrolls(&world, hero).is_some(), "and kept its scroll");
}
