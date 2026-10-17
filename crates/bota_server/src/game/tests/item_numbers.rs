//! The catalog at Dota 7.41f's numbers (`scripts/npc/items.txt`, client
//! 6957): what the shop charges, what an item carries, and how it builds.

use bota_core::{BLINK_OVERSHOOT_RANGE, ModifierKind, rules};

use crate::game::{Entity, Stats, World};
use bota_proto::{Attributes, Fixed, HeroId, ItemId, SlotId, Target, Vec2};

use super::support::*;

/// A hero of a kind in a Map 1 match, walked out of the fountain and past
/// its lingering mending, whose share grows with the maximum.
fn hero_out_of_fountain(hero: HeroId) -> (World, Entity) {
    let mut cfg = config();
    cfg.picks[0].hero = hero;
    let mut world = World::for_match(&cfg, cfg.rng());
    let unit = world.seats[0].unit.expect("stood up");
    world.transform.get_mut(unit).expect("standing").pos = Vec2::from_ints(9000, 9216);
    for _ in 0..=rules::FOUNTAIN_HEAL_LINGER_TICKS {
        world.step();
    }
    (world, unit)
}

/// The hero's stats once it carries one item in its first slot.
fn carrying(world: &mut World, hero: Entity, item: ItemId) -> Stats {
    hand_item(world, hero, item, 0);
    world.step();
    *world.stats.get(hero).expect("derived")
}

/// Whether two fixed-point values agree to a few raw units of rounding.
fn near(a: Fixed, b: Fixed) -> bool {
    (a.raw - b.raw).abs() <= 4
}

#[test]
fn the_shop_charges_dota_741_prices() {
    for (item, price) in [
        (ItemId::IRON_BRANCH, 55),
        (ItemId::CLARITY, 60),
        (ItemId::HEALING_SALVE, 100),
        (ItemId::QUELLING_BLADE, 100),
        (ItemId::CHAINMAIL, 500),
        (ItemId::ENERGY_BOOSTER, 800),
        (ItemId::MAGIC_WAND, 460),
        (ItemId::PHASE_BOOTS, 1450),
        (ItemId::POWER_TREADS, 1400),
    ] {
        let (mut world, _hero) = a_hero_with_gold(10_000);
        assert!(world.buy(SlotId(0), item, &mut Vec::new()), "{item:?}");
        assert_eq!(10_000 - world.seats[0].gold, price, "{item:?}");
    }
}

#[test]
fn the_bands_and_the_cheap_parts_carry_their_741_bonuses() {
    let (mut world, hero) = hero_out_of_fountain(HeroId::SHADOW_FIEND);
    let bare = *world.stats.get(hero).expect("derived");
    let per_second = |hundredths: i32| Fixed::from_ratio(hundredths, 100 * 30);

    let wraith = carrying(&mut world, hero, ItemId::WRAITH_BAND);
    let band = Attributes {
        strength: Fixed::from_int(2),
        agility: Fixed::from_int(5),
        intelligence: Fixed::from_int(2),
    };
    assert_eq!(wraith.attributes, bare.attributes + band);
    assert_eq!(
        wraith.attack_speed - bare.attack_speed,
        6 + 5,
        "six attack speed besides the agility"
    );
    assert!(near(
        wraith.armor - bare.armor,
        Fixed::from_ratio(7, 4) + rules::ARMOR_PER_AGILITY * Fixed::from_int(5)
    ));

    let bracer = carrying(&mut world, hero, ItemId::BRACER);
    assert_eq!(
        bracer.attributes.strength - bare.attributes.strength,
        Fixed::from_int(5)
    );
    assert_eq!(
        bracer.attributes.agility - bare.attributes.agility,
        Fixed::from_int(2)
    );
    assert_eq!(
        bracer.max_hp - bare.max_hp,
        Fixed::from_int(50 + 5 * rules::HP_PER_STRENGTH)
    );
    assert!(near(
        bracer.hp_regen - bare.hp_regen,
        per_second(75) + rules::HP_REGEN_PER_STRENGTH * Fixed::from_int(5)
    ));

    let null = carrying(&mut world, hero, ItemId::NULL_TALISMAN);
    assert_eq!(
        null.attributes.intelligence - bare.attributes.intelligence,
        Fixed::from_int(5)
    );
    let mana = bare.max_mana + Fixed::from_int(5 * rules::MANA_PER_INTELLIGENCE);
    assert!(
        near(null.max_mana, mana + mana * Fixed::from_ratio(3, 100)),
        "three percent more mana, intelligence included"
    );
    assert!(near(
        null.mana_regen - bare.mana_regen,
        per_second(100) + rules::MANA_REGEN_PER_INTELLIGENCE * Fixed::from_int(5)
    ));

    let ring = carrying(&mut world, hero, ItemId::RING_OF_REGEN);
    assert!(near(ring.hp_regen - bare.hp_regen, per_second(125)));
    let mask = carrying(&mut world, hero, ItemId::SAGES_MASK);
    assert!(near(mask.mana_regen - bare.mana_regen, per_second(70)));
    let chainmail = carrying(&mut world, hero, ItemId::CHAINMAIL);
    assert_eq!(chainmail.armor - bare.armor, Fixed::from_int(4));
    let broadsword = carrying(&mut world, hero, ItemId::BROADSWORD);
    assert_eq!(broadsword.damage - bare.damage, 15);
    let branch = carrying(&mut world, hero, ItemId::IRON_BRANCH);
    assert_eq!(
        branch.attributes,
        bare.attributes + Attributes::all(1),
        "a branch is one of each"
    );
}

#[test]
fn boots_and_the_blade_carry_less_to_a_ranged_hero() {
    // Phase Boots: +50 speed and +4 armor to both; +18 damage melee, +12
    // ranged. Treads: +55 speed melee, +45 ranged. Quelling Blade: +8 melee,
    // +4 ranged against creeps.
    for (hero, damage, treads_speed, quell) in [
        (HeroId::PUDGE, 18, 55, 8),
        (HeroId::SHADOW_FIEND, 12, 45, 4),
    ] {
        let (mut world, unit) = hero_out_of_fountain(hero);
        let bare = *world.stats.get(unit).expect("derived");
        let phase = carrying(&mut world, unit, ItemId::PHASE_BOOTS);
        assert_eq!(phase.damage - bare.damage, damage, "{hero:?}");
        assert_eq!(phase.armor - bare.armor, Fixed::from_int(4));
        assert_eq!(phase.move_speed - bare.move_speed, Fixed::from_int(50));
        let treads = carrying(&mut world, unit, ItemId::POWER_TREADS);
        assert_eq!(
            treads.move_speed - bare.move_speed,
            Fixed::from_int(treads_speed),
            "{hero:?}"
        );
        let blade = carrying(&mut world, unit, ItemId::QUELLING_BLADE);
        assert_eq!(blade.damage_to_creeps, quell, "{hero:?}");
    }
}

#[test]
fn phase_boots_build_of_boots_chainmail_and_blades_and_phase_a_ranged_hero_by_ten_percent() {
    let (mut world, hero) = hero_out_of_fountain(HeroId::SHADOW_FIEND);
    let bag = world.inventory.get_mut(hero).expect("a bag");
    for (at, part) in [ItemId::BOOTS, ItemId::CHAINMAIL, ItemId::BLADES_OF_ATTACK]
        .into_iter()
        .enumerate()
    {
        bag.slots[at] = Some(fresh_stack(part, 0));
    }
    world.step();
    assert_eq!(
        slot_of(&world, hero, 0).map(|stack| stack.id),
        Some(ItemId::PHASE_BOOTS)
    );
    let walking = world.stats.get(hero).expect("derived").move_speed;
    assert!(world.use_item(hero, 0, Target::None, &mut Vec::new()));
    world.step();
    let phased = world.stats.get(hero).expect("derived").move_speed;
    assert_eq!(
        phased.to_int(),
        walking.to_int() * 110 / 100,
        "ten percent for a ranged hero"
    );
    for _ in 0..3 * rules::TICKS_PER_SECOND {
        world.step();
    }
    assert!(
        !carries(&world, hero, ModifierKind::Phased),
        "for three seconds"
    );
}

#[test]
fn a_blink_aimed_past_its_range_lands_at_nine_hundred_sixty() {
    let (mut world, hero) = hero_out_of_fountain(HeroId::SHADOW_FIEND);
    hand_item(&mut world, hero, ItemId::BLINK_DAGGER, 0);
    world.step();
    let from = world.transform.get(hero).expect("standing").pos;
    let aim = from + Vec2::from_ints(-1500, 0);
    assert!(world.use_item(hero, 0, Target::Pos(aim), &mut Vec::new()));
    let to = world.transform.get(hero).expect("standing").pos;
    let carried = (to - from).length_squared();
    let overshoot = i64::from(BLINK_OVERSHOOT_RANGE) << 16;
    assert!(
        carried <= overshoot * overshoot && carried > (overshoot - (64 << 16)).pow(2),
        "it lands {BLINK_OVERSHOOT_RANGE} along the line, give or take a step back off closed ground"
    );
}

#[test]
fn a_healing_salve_mends_thirty_a_second_for_thirteen_seconds() {
    let (mut world, hero) = hero_out_of_fountain(HeroId::SHADOW_FIEND);
    world.health.insert(hero, health(100));
    let base = world.stats.get(hero).expect("derived").hp_regen;
    hand_item(&mut world, hero, ItemId::HEALING_SALVE, 1);
    assert!(world.use_item(hero, 0, Target::None, &mut Vec::new()));
    let mending = |world: &World| {
        carries(
            world,
            hero,
            ModifierKind::Mending {
                per_tick: 0,
                breaks: false,
            },
        )
    };
    for _ in 0..13 * rules::TICKS_PER_SECOND - 1 {
        world.step();
    }
    assert!(
        mending(&world),
        "still mending just short of thirteen seconds"
    );
    world.step();
    assert!(!mending(&world), "and done at thirteen");
    let gained = world.health.get(hero).expect("alive").hp - Fixed::from_int(100);
    let own = base * Fixed::from_int(13 * 30);
    assert!(
        (gained - own).to_int().abs_diff(390) <= 2,
        "390 mended: {gained} with {own} of its own"
    );
}
