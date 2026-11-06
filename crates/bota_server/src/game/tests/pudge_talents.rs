//! Pudge's talents at Dota 7.41f: learned through an order, and each worth what its bonus in
//! his script says (`npc_dota_hero_pudge.txt`, the dota2.com hero data's talent list).

use bota_core::{ModifierKind, rules};

use crate::game::{Entity, Level, World, wire_id};
use bota_proto::{
    AbilityId, EventKind, Fixed, HeroId, Inflictor, Order, SlotId, TalentSlot, Target, Vec2,
};

use super::pudge::{dismember, heap, pudge_and_a_hero_mark, until_the_hook_flies};
use super::support::*;

/// Talent slots of Pudge's tree, as `TalentSlot` numbers them.
const ROT_SLOW: u8 = 0;
const ARMOR: u8 = 1;
const HOOK_DAMAGE: u8 = 2;
const SPELL_LIFESTEAL: u8 = 3;
const HOOK_COOLDOWN: u8 = 4;
const DISMEMBER_DURATION: u8 = 5;
const FLESH_HEAP: u8 = 6;
const DISMEMBER_DAMAGE: u8 = 7;

/// Pudge at level 25 and an enemy Shadow Fiend `apart` east, every slot at its first level,
/// with or without one talent, standing.
fn talented(apart: i32, talent: Option<u8>) -> (World, Entity, Entity) {
    let (mut world, pudge, mark) = pudge_and_a_hero_mark(apart);
    world.level.insert(pudge, Level(rules::HERO_MAX_LEVEL));
    for slot in 0..4 {
        set_level(&mut world, pudge, slot, 1);
    }
    if let Some(slot) = talent {
        assert!(
            world.learn_talent(pudge, TalentSlot(slot)),
            "Pudge learns talent {slot}"
        );
    }
    world.settle();
    world.fill_pools(pudge);
    world.fill_pools(mark);
    stop_click(&mut world);
    (world, pudge, mark)
}

/// A match Pudge plays at level ten learns a talent of his tree through the order a player
/// gives.
#[test]
fn pudge_learns_a_talent_of_his_tree_through_an_order_at_level_ten() {
    let mut cfg = config();
    cfg.picks[0].hero = HeroId::PUDGE;
    let mut world = World::for_match(&cfg, cfg.rng());
    world.grant_xp(0, rules::XP_THRESHOLDS[9], &mut Vec::new());
    world.step();
    let order = Order::LearnTalent {
        talent: TalentSlot(ARMOR),
    };
    assert_eq!(world.validate_order(SlotId(0), None, &order), Ok(()));
    seat_order(&mut world, order);
    let hero = world.seats[0].unit.expect("stood up");
    assert!(
        world
            .abilities
            .get(hero)
            .is_some_and(|book| book.talents.has(TalentSlot(ARMOR))),
        "and holds it"
    );
}

/// `special_bonus_armor_5`: five armor more.
#[test]
fn the_armor_talent_adds_five_armor() {
    let armor = |talent| {
        let (world, pudge, _) = talented(2000, talent);
        world.stats.get(pudge).expect("settled").armor
    };
    assert_eq!(armor(Some(ARMOR)) - armor(None), Fixed::from_int(5));
}

/// `special_bonus_unique_pudge_4`: Rot slows ten percent more, 24% at its first level.
#[test]
fn the_rot_talent_slows_ten_percent_more() {
    let slow = |talent| {
        let (mut world, pudge, mark) = talented(150, talent);
        cast(&mut world, 1, Target::None);
        run(&mut world, 3);
        world.modifiers.get(mark).and_then(|on_it| {
            on_it.active().find_map(|held| match held.kind {
                ModifierKind::Rotting { slow_pct, .. } if held.source == Some(pudge) => {
                    Some(slow_pct)
                }
                _ => None,
            })
        })
    };
    assert_eq!(slow(None), Some(14), "14% at the first level");
    assert_eq!(slow(Some(ROT_SLOW)), Some(24), "and 24% with the talent");
}

/// What a hook's impact deals its mark.
fn hook_impact(world: &mut World, mark: Entity) -> Option<i32> {
    let home = world.pos(mark).expect("standing");
    cast(world, 0, Target::Pos(home));
    until_the_hook_flies(world);
    for _ in 0..60 {
        for event in world.step() {
            if let EventKind::Damaged {
                target,
                amount,
                inflictor: Some(Inflictor::Ability(AbilityId::MEAT_HOOK)),
                ..
            } = event.kind()
                && target == wire_id(mark)
            {
                return Some(amount);
            }
        }
    }
    None
}

/// `special_bonus_unique_pudge_7`: the hook deals 150 more, 300 pure at its first level.
#[test]
fn the_hook_damage_talent_adds_one_hundred_fifty() {
    let (mut world, _, mark) = talented(600, Some(HOOK_DAMAGE));
    assert_eq!(hook_impact(&mut world, mark), Some(300));
}

/// `special_bonus_unique_pudge_5`: the hook waits four seconds less, 14 at its first level.
#[test]
fn the_hook_cooldown_talent_takes_four_seconds_off() {
    let (mut world, pudge, _) = talented(4000, Some(HOOK_COOLDOWN));
    let home = world.pos(pudge).expect("standing");
    cast(&mut world, 0, Target::Pos(home + Vec2::from_ints(600, 0)));
    until_the_hook_flies(&mut world);
    let waiting = world.abilities.get(pudge).expect("a kit").slots[0].cooldown;
    assert!(
        (14 * rules::TICKS_PER_SECOND - 2..=14 * rules::TICKS_PER_SECOND).contains(&waiting),
        "14 seconds: {waiting} ticks"
    );
}

/// The ticks after the dismember takes hold at which its mark loses health, over `ticks`.
fn bites_of(world: &mut World, mark: Entity, ticks: u32) -> Vec<u32> {
    let mut left = world.health.get(mark).expect("standing").hp;
    let mut bites = Vec::new();
    for tick in 0..ticks {
        let now = world.health.get(mark).expect("standing").hp;
        // Its own regeneration gives back a little between two readings.
        if left - now > Fixed::from_int(5) {
            bites.push(tick);
        }
        left = now;
        world.step();
    }
    bites
}

/// `special_bonus_unique_pudge_6`: a dismember holds 0.75 s longer and bites eight times.
#[test]
fn the_dismember_duration_talent_holds_longer_and_bites_eight_times() {
    let (mut world, pudge, mark) = talented(150, Some(DISMEMBER_DURATION));
    dismember(&mut world, mark);
    let total = world.modifiers.get(mark).and_then(|on_it| {
        on_it.active().find_map(|held| match held.kind {
            ModifierKind::Dismembered { .. } => held.ticks_total,
            _ => None,
        })
    });
    assert_eq!(total, Some(105), "3.5 seconds of hold");
    // The take-hold bite landed with the cast; seven more follow, the last at tick 92.
    let bites = bites_of(&mut world, mark, 95);
    assert_eq!(bites.len(), 7, "eight bites in all: {bites:?}");
    assert!(
        world.is_channelling(pudge),
        "and the channel runs on past where 2.75 seconds end"
    );
}

/// `special_bonus_unique_pudge_1`: a flesh heap stack is worth three strength, and the meat
/// shield blocks twelve at its first level.
#[test]
fn the_flesh_heap_talent_makes_a_stack_three_strength_and_the_block_twelve() {
    let (mut world, pudge, _) = talented(2000, Some(FLESH_HEAP));
    let bare = world.stats.get(pudge).expect("settled").attributes.strength;
    world
        .modifiers
        .get_mut(pudge)
        .expect("a hero wears modifiers")
        .set_count(ModifierKind::flesh_heap, 2, (pudge, AbilityId::FLESH_HEAP));
    world.step();
    assert_eq!(heap(&world, pudge), 2);
    assert_eq!(
        world.stats.get(pudge).map(|s| s.attributes.strength),
        Some(bare + Fixed::from_int(6)),
        "two stacks are worth three strength each"
    );
    cast(&mut world, 2, Target::None);
    let block = world.modifiers.get(pudge).and_then(|on_it| {
        on_it.active().find_map(|held| match held.kind {
            ModifierKind::MeatShield { amount } => Some(amount),
            _ => None,
        })
    });
    assert_eq!(block, Some(12), "eight blocked times 1.5");
}

/// `special_bonus_unique_pudge_3`: a dismember bite is the second's damage times 1.5 and
/// strength's share times 1.8 (`x1.5` and `x1.8` in the script).
#[test]
fn the_dismember_damage_talent_bites_harder() {
    let (mut world, pudge, mark) = talented(150, Some(DISMEMBER_DAMAGE));
    let strength = world.stats.get(pudge).expect("settled").attributes.strength;
    let full = world.health.get(mark).expect("standing").hp;
    dismember(&mut world, mark);
    let taken = full - world.health.get(mark).expect("standing").hp;
    let per_second = Fixed::from_int(120) + strength * Fixed::from_ratio(54, 100);
    let bite = (per_second * Fixed::from_ratio(2750, 6000)).to_int();
    let resist = world.stats.get(mark).expect("settled").magic_resist_bp;
    let after = bite * (rules::NOMINAL_BP - resist) / rules::NOMINAL_BP;
    assert!(
        (after - 1..=after + 1).contains(&taken.to_int()),
        "the take-hold bite takes {after} after resistance: took {taken}"
    );
}

/// What the take-hold bite of a dismember on the mark heals Pudge, by talent.
fn take_hold_heal(talent: Option<u8>) -> i32 {
    let (mut world, pudge, mark) = talented(150, talent);
    world.health.insert(
        pudge,
        crate::game::Health {
            hp: Fixed::from_int(300),
        },
    );
    world.step();
    let start = world.health.get(pudge).expect("standing").hp;
    dismember(&mut world, mark);
    (world.health.get(pudge).expect("standing").hp - start).to_int()
}

/// `special_bonus_spell_lifesteal_8`: his spells heal him by 8% of what they take of a hero,
/// on top of what a dismember heals by itself.
#[test]
fn the_spell_lifesteal_talent_heals_eight_percent_of_a_spells_damage() {
    let (without, with) = (take_hold_heal(None), take_hold_heal(Some(SPELL_LIFESTEAL)));
    assert!(
        (2..=4).contains(&(with - without)),
        "a bite of some forty after resistance heals three more: {without} without, {with} with"
    );
}
