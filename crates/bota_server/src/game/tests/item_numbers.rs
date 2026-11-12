//! The catalog at Dota 7.41f's numbers (`scripts/npc/items.txt`, client
//! 6972): what the shop charges, what an item carries, and how it builds.

use bota_core::{
    ModifierKind, agility_armor, agility_attack_speed, attribute_regen,
    intelligence_magic_resist_bp, item_def, primary_damage, rules,
};

use crate::game::{Entity, Stats, World, wire_id};
use bota_proto::{
    Attribute, Attributes, EventKind, Fixed, HeroId, ItemId, ItemSlot, Order, SlotId, Target, Team,
    Vec2,
};

use super::support::*;

/// A hero of a kind in a Map 1 match, walked out of the fountain and past
/// its lingering mending, whose share grows with the maximum.
fn hero_out_of_fountain(hero: HeroId) -> (World, Entity) {
    let mut cfg = config();
    cfg.picks[0].hero = hero;
    let mut world = World::for_match(&cfg, cfg.rng());
    let unit = world.seats[0].unit.expect("stood up");
    world.transform.get_mut(unit).expect("standing").pos = Vec2::from_ints(9000, 9216);
    run(&mut world, rules::FOUNTAIN_HEAL_LINGER_TICKS + 1);
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
    run(&mut world, 3 * rules::TICKS_PER_SECOND);
    assert!(
        !carries(&world, hero, ModifierKind::Phased),
        "for three seconds"
    );
}

#[test]
fn power_treads_build_of_any_of_three_parts_set_to_its_attribute() {
    for (part, mode) in [
        (ItemId::BELT, Attribute::Strength),
        (ItemId::ROBE, Attribute::Intelligence),
        (ItemId::BAND, Attribute::Agility),
    ] {
        let (mut world, hero) = hero_out_of_fountain(HeroId::SHADOW_FIEND);
        let bag = world.inventory.get_mut(hero).expect("a bag");
        for (at, held) in [ItemId::BOOTS, ItemId::GLOVES, part]
            .into_iter()
            .enumerate()
        {
            bag.slots[at] = Some(fresh_stack(held, 0));
        }
        world.step();
        let built = slot_of(&world, hero, 0).expect("built");
        assert_eq!((built.id, built.mode), (ItemId::POWER_TREADS, Some(mode)));
    }
    // Bought whole, the shop picks the cheapest recipe, then the one of the
    // hero's own attribute: Shadow Fiend's Band.
    let (mut world, hero) = a_hero_with_gold(10_000);
    assert!(world.buy(SlotId(0), ItemId::POWER_TREADS, &mut Vec::new()));
    world.step();
    let built = slot_of(&world, hero, 0).expect("built");
    assert_eq!(
        (built.id, built.mode),
        (ItemId::POWER_TREADS, Some(Attribute::Agility))
    );
    // A Robe in hand makes the Robe's the cheapest.
    let (mut world, hero) = a_hero_with_gold(10_000);
    world.inventory.get_mut(hero).expect("a bag").slots[0] = Some(fresh_stack(ItemId::ROBE, 0));
    assert!(world.buy(SlotId(0), ItemId::POWER_TREADS, &mut Vec::new()));
    assert_eq!(10_000 - world.seats[0].gold, 950, "boots and gloves only");
    world.step();
    let built = slot_of(&world, hero, 0).expect("built");
    assert_eq!(
        (built.id, built.mode),
        (ItemId::POWER_TREADS, Some(Attribute::Intelligence))
    );
}

/// Dota's Blink Dagger overshoots (`DOTA_ABILITY_BEHAVIOR_OVERSHOOT`): ordered past its cast
/// range it is not walked in, it lands its `blink_range_clamp` along the line at once.
#[test]
fn a_blink_ordered_past_its_range_lands_at_its_clamp_without_walking_in() {
    let (mut world, hero) = hero_out_of_fountain(HeroId::SHADOW_FIEND);
    hand_item(&mut world, hero, ItemId::BLINK_DAGGER, 0);
    world.step();
    let def = item_def(ItemId::BLINK_DAGGER).expect("sold");
    let clamp = def.value("blink_range_clamp").expect("a clamp")[0].int();
    let from = world.transform.get(hero).expect("standing").pos;
    let aim = from + Vec2::from_ints(-(def.cast_range + 300), 0);
    seat_order(
        &mut world,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::Pos(aim),
        },
    );
    run(&mut world, 3);
    let to = world.transform.get(hero).expect("standing").pos;
    let carried = (to - from).length_squared();
    let clamp_raw = i64::from(clamp) << 16;
    assert!(
        carried <= clamp_raw * clamp_raw && carried > (clamp_raw - (64 << 16)).pow(2),
        "it lands {clamp} along the line, give or take a step back off closed ground"
    );
}

#[test]
fn a_use_of_an_item_is_told_to_the_sides_that_see_it() {
    let (mut world, hero) = hero_out_of_fountain(HeroId::SHADOW_FIEND);
    hand_item(&mut world, hero, ItemId::BLINK_DAGGER, 0);
    world.step();
    let from = world.transform.get(hero).expect("standing").pos;
    let aim = from + Vec2::from_ints(-500, 0);
    let mut told = Vec::new();
    assert!(world.use_item(hero, 0, Target::Pos(aim), &mut told));
    told.retain(|event| matches!(event.kind(), EventKind::ItemUsed { .. }));
    assert_eq!(told.len(), 1, "one use");
    assert_eq!(
        told[0].kind(),
        EventKind::ItemUsed {
            caster: Some(wire_id(hero)),
            item: ItemId::BLINK_DAGGER,
            at: Some(aim),
            target: None,
        }
    );
    assert!(told[0].visible_to().tells(Team::Radiant), "its own side");
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
                per_tick: Fixed::ZERO,
                breaks: false,
            },
        )
    };
    run(&mut world, 13 * rules::TICKS_PER_SECOND - 1);
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

/// A hero walked out of the fountain past its lingering mending, after its seat bought `items`
/// through the shop, one a tick, into its first slots.
pub(super) fn out_of_fountain_after_buying(items: &[ItemId]) -> (World, Entity) {
    let (mut world, hero) = a_hero_with_gold(30_000);
    for (slot, &item) in items.iter().enumerate() {
        let events = seat_order(&mut world, Order::Buy { item });
        assert!(
            events.iter().any(|event| matches!(
                event.kind(),
                EventKind::ItemBought { item: bought, .. } if bought == item
            )),
            "{item:?} is sold"
        );
        assert_eq!(slot_of(&world, hero, slot).map(|held| held.id), Some(item));
    }
    if items.is_empty() {
        world.step();
    }
    world.transform.get_mut(hero).expect("standing").pos = Vec2::from_ints(9000, 9216);
    run(&mut world, rules::FOUNTAIN_HEAL_LINGER_TICKS + 1);
    (world, hero)
}

/// The hero's stats once its seat bought `items` and it walked out of the fountain.
fn stats_after_buying(items: &[ItemId]) -> Stats {
    let (world, hero) = out_of_fountain_after_buying(items);
    *world.stats.get(hero).expect("derived")
}

/// The stat items of the shop whose values say something else than a bare stat: their tests
/// are their own.
const STAT_ITEMS_TESTED_APART: [ItemId; 5] = [
    ItemId::BOOTS,
    ItemId::QUARTERSTAFF,
    ItemId::TALISMAN_OF_EVASION,
    ItemId::JAVELIN,
    ItemId::WIND_LACE,
];

/// Every basic item the shop sells that does nothing but carry stats, bought through the seat:
/// the buyer gains what the item's values in the catalogue say, and nothing else. The values are
/// the install's (`tools/import_items --check` holds `data/items.rs` to it); what is checked here
/// is that each lands as the stat its Dota key names, mapped by this test apart from the rows of
/// `data/item_mechanics.rs`, with what its attributes are worth. A value of such an item this
/// test does not know fails it, so no value goes unread.
#[test]
fn every_stat_item_bought_adds_what_its_script_says_and_nothing_else() {
    let bare = stats_after_buying(&[]);
    let mut checked = 0;
    for (id, def) in bota_core::ITEMS.iter().enumerate() {
        let item = ItemId(id as u16);
        let stat_stick = !def.retired
            && !def.unbuyable
            && def.components.is_empty()
            && def.aim.is_none()
            && def.on_attack.is_none()
            && def.magic_block.is_none()
            && !def.values.is_empty();
        if !stat_stick || STAT_ITEMS_TESTED_APART.contains(&item) {
            continue;
        }
        assert_carries_its_values(item, bare);
        checked += 1;
    }
    assert_eq!(checked, 46, "the shop's stat items");
}

/// The upgrades that carry stats and nothing else, each bought whole through the seat (its
/// parts and recipe assembled into it): the buyer gains what its values say, and nothing else.
#[test]
fn every_stat_upgrade_bought_whole_adds_what_its_script_says_and_nothing_else() {
    let bare = stats_after_buying(&[]);
    for item in [
        ItemId::OBLIVION_STAFF,
        ItemId::PERSEVERANCE,
        ItemId::SOUL_BOOSTER,
        ItemId::FALCON_BLADE,
        ItemId::YASHA,
        ItemId::SANGE,
        ItemId::KAYA,
        ItemId::SANGE_AND_YASHA,
        ItemId::KAYA_AND_SANGE,
        ItemId::YASHA_AND_KAYA,
        ItemId::AETHER_LENS,
        ItemId::OCTARINE_CORE,
    ] {
        assert_carries_its_values(item, bare);
    }
}

/// `speed` hastened by `pct` percent, rounded as the pace is.
fn hastened(speed: Fixed, pct: i32) -> Fixed {
    Fixed {
        raw: (i64::from(speed.raw) * i64::from(100 + pct) / 100) as i32,
    }
}

/// Dota's Yasha-based items: "multiple sources of movement speed do not stack". Two Yashas
/// carried hasten their carrier by one Yasha's ten percent.
#[test]
fn two_yashas_carried_hasten_by_one_yashas_percent() {
    let bare = stats_after_buying(&[]).move_speed;
    let one = stats_after_buying(&[ItemId::YASHA]).move_speed;
    let two = stats_after_buying(&[ItemId::YASHA, ItemId::YASHA]).move_speed;
    assert_eq!(one, hastened(bare, 10), "one Yasha: ten percent");
    assert_eq!(two, one, "a second Yasha adds nothing");
}

/// A Sange keeps a quarter of a slow off its carrier (`slow_resistance`; Dota 7.41: "Slow
/// Resistance bonus increased from +20% to +25%"): slowed by 40%, it walks at 70% of its pace.
#[test]
fn a_sange_keeps_a_quarter_of_a_slow_off_its_carrier() {
    let pace_slowed = |items: &[ItemId]| {
        let (mut world, hero) = out_of_fountain_after_buying(items);
        let walking = world.stats.get(hero).expect("derived").move_speed;
        put_on(&mut world, hero, ModifierKind::Slowed { pct: 40 }, 100);
        world.step();
        (walking, world.stats.get(hero).expect("derived").move_speed)
    };
    let (walking, slowed) = pace_slowed(&[]);
    assert_eq!(slowed, hastened(walking, -40), "no Sange: the whole slow");
    let (walking, slowed) = pace_slowed(&[ItemId::SANGE]);
    assert_eq!(
        slowed,
        hastened(walking, -30),
        "a Sange keeps 10 of the 40 off"
    );
    let (_, twice) = pace_slowed(&[ItemId::SANGE, ItemId::SANGE]);
    assert_eq!(twice, slowed, "a second Sange keeps nothing more off");
}

/// A Sange's Health Restoration (`hp_regen_amp`, 12% since Dota 7.41b) comes through every heal its
/// carrier takes as well as its regeneration (7.41: "Now also modifies incoming healing"); a
/// second Sange adds nothing.
#[test]
fn a_sange_amplifies_a_heal_its_carrier_takes_by_twelve_percent() {
    for items in [&[ItemId::SANGE][..], &[ItemId::SANGE, ItemId::SANGE]] {
        let (mut world, hero) = out_of_fountain_after_buying(items);
        world.health.get_mut(hero).expect("alive").hp = Fixed::from_int(100);
        let (restored, _) = world.refill_pools(hero, 200, 0);
        assert_eq!(restored, 224, "{items:?}");
    }
}

/// A Kaya amplifies its carrier's spells by 10% (`spell_amp`): a Shadowraze's magical blow on a
/// creep lands for a tenth more; a second Kaya adds nothing (Liquipedia, *Kaya*: "Multiple sources
/// of spell damage amplification do not stack").
#[test]
fn a_kaya_amplifies_a_raze_by_a_tenth_and_a_second_one_adds_nothing() {
    let creep = |world: &mut World, at| world.spawn_unit(&bota_core::MELEE_CREEP, Team::Dire, at);
    let (bare, _) = raze_of_a_buyer(&[], creep);
    let (one, _) = raze_of_a_buyer(&[ItemId::KAYA], creep);
    let (two, _) = raze_of_a_buyer(&[ItemId::KAYA, ItemId::KAYA], creep);
    assert!((one - bare * 110 / 100).abs() <= 1, "{one} of {bare}");
    assert_eq!(two, one);
}

/// A Kaya amplifies all the mana its carrier regenerates by 20% (`mana_regen_multiplier`; Dota
/// 7.41e: "decreased from 30% to 20%"), a Clarity's drink among it.
#[test]
fn a_kaya_amplifies_the_mana_a_clarity_mends_by_a_fifth() {
    let drinking = |items: &[ItemId]| {
        let (mut world, hero) = out_of_fountain_after_buying(items);
        let walking = world.stats.get(hero).expect("derived").mana_regen;
        let slot = items.len();
        world.inventory.get_mut(hero).expect("a bag").slots[slot] =
            Some(fresh_stack(ItemId::CLARITY, 1));
        assert!(world.use_item(hero, slot, Target::Unit(wire_id(hero)), &mut Vec::new()));
        world.step();
        world.stats.get(hero).expect("derived").mana_regen - walking
    };
    let (bare, kaya) = (drinking(&[]), drinking(&[ItemId::KAYA]));
    assert!(bare > Fixed::ZERO, "the Clarity mends");
    assert!(
        (kaya.raw - bare.raw * 120 / 100).abs() <= 2,
        "{kaya:?} of {bare:?}"
    );
}

/// The stats of a hero out of the fountain carrying `items` in its first slots, put there whole
/// (buying an upgrade would take a part already carried into it).
fn stats_carrying(items: &[ItemId]) -> Stats {
    let (world, hero) = carrying_world(items);
    *world.stats.get(hero).expect("derived")
}

/// A hero out of the fountain carrying `items` in its first slots, put there whole, a tick on.
fn carrying_world(items: &[ItemId]) -> (World, Entity) {
    let (mut world, hero) = out_of_fountain_after_buying(&[]);
    let bag = world.inventory.get_mut(hero).expect("a bag");
    for (at, &item) in items.iter().enumerate() {
        bag.slots[at] = Some(fresh_stack(item, 0));
    }
    world.step();
    (world, hero)
}

/// An item and what is built of it hold one bonus of a stat, the best (Liquipedia: "Multiple sources
/// of movement speed do not stack" for Yasha-based items, of slow resistance for Sange-based ones,
/// of spell amplification for Kaya-based ones): a Yasha and a Sange and Yasha hasten by the pair's
/// 12%, a Sange and a Sange and Yasha keep the pair's 30% of a slow off, and a Kaya and a Kaya and
/// Sange amplify a raze by the pair's 12%.
#[test]
fn an_item_and_what_is_built_of_it_count_once_the_better() {
    let bare = stats_after_buying(&[]).move_speed;
    let both = stats_carrying(&[ItemId::YASHA, ItemId::SANGE_AND_YASHA]);
    assert_eq!(both.move_speed, hastened(bare, 12));
    let both = stats_carrying(&[ItemId::SANGE, ItemId::SANGE_AND_YASHA]);
    assert_eq!(both.slow_resist_bp, 3_000);
    let creep = |world: &mut World, at| world.spawn_unit(&bota_core::MELEE_CREEP, Team::Dire, at);
    let (bare, _) = raze_of_a_buyer(&[], creep);
    let (world, hero) = carrying_world(&[ItemId::KAYA, ItemId::KAYA_AND_SANGE]);
    let (both, _) = raze_from(world, hero, creep);
    assert!((both - bare * 112 / 100).abs() <= 1, "{both} of {bare}");
}

/// Items of different lineages each count: a Yasha and a Kaya and Sange, neither built of the
/// other, carry the Yasha's movement speed and the pair's spell amplification side by side, and
/// Sange and Yasha's status resistance (16%) shortens a stun of 100 ticks to 84.
#[test]
fn items_of_different_lineages_each_count() {
    let stats = stats_carrying(&[ItemId::YASHA, ItemId::KAYA_AND_SANGE]);
    assert_eq!(
        stats.move_speed,
        hastened(stats_after_buying(&[]).move_speed, 10)
    );
    assert_eq!(stats.spell_amp_pct, 12);
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::SANGE_AND_YASHA]);
    put_on(&mut world, hero, ModifierKind::Stunned, 100);
    let left = world
        .modifiers
        .get(hero)
        .and_then(|on_it| {
            on_it
                .active()
                .find(|held| held.kind == ModifierKind::Stunned)
        })
        .and_then(|held| held.ticks_left);
    assert_eq!(left, Some(84));
}

/// Kaya and Sange cuts the mana its carrier pays by a quarter (`manacost_reduction`): a
/// Shadowraze's cost comes to three quarters.
#[test]
fn kaya_and_sange_cuts_a_razes_mana_cost_by_a_quarter() {
    let cost = |items: &[ItemId]| {
        let (world, hero) = out_of_fountain_after_buying(items);
        world.ability_mana_cost(hero, bota_proto::AbilityId::RAZE_NEAR, 1)
    };
    let bare = cost(&[]);
    assert!(bare > 0);
    assert_eq!(cost(&[ItemId::KAYA_AND_SANGE]), bare * 75 / 100);
}

/// Yasha and Kaya's cast speed (`cast_speed_pct` 25) runs a cast point at 125% of its pace: a
/// Shadowraze's 0.55 seconds go off in 0.44.
#[test]
fn yasha_and_kaya_casts_a_raze_at_five_fourths_of_its_pace() {
    let ticks_to_land = |items: &[ItemId]| {
        let (mut world, hero) = out_of_fountain_after_buying(items);
        set_level(&mut world, hero, 0, 1);
        world.transform.get_mut(hero).expect("standing").facing = bota_proto::Angle { brads: 0 };
        let at = world.transform.get(hero).expect("standing").pos
            + Vec2::from_ints(rules::RAZE_DISTANCE[0], 0);
        let mark = world.spawn_unit(&bota_core::MELEE_CREEP, Team::Dire, at);
        cast(&mut world, 0, Target::None);
        (1..60)
            .find(|_| !burns_on(&world.step(), mark).is_empty())
            .expect("the raze lands")
    };
    let ms = |ticks: u32| ticks * 1000 / rules::TICKS_PER_SECOND;
    let (bare, quick) = (ticks_to_land(&[]), ticks_to_land(&[ItemId::YASHA_AND_KAYA]));
    assert!(
        ms(bare).abs_diff(550) <= 1000 / rules::TICKS_PER_SECOND,
        "{bare} ticks"
    );
    assert!(
        ms(quick).abs_diff(440) <= 1000 / rules::TICKS_PER_SECOND,
        "{quick} ticks"
    );
}

/// An Octarine Core cuts every cooldown its carrier starts by a quarter (`bonus_cooldown`): a
/// Shadowraze's and a Phase Boots' alike; a second one adds nothing.
#[test]
fn an_octarine_core_cuts_a_razes_and_an_items_cooldown_by_a_quarter() {
    let cooldowns = |items: &[ItemId]| {
        let (world, hero) = carrying_world(items);
        (
            world.ability_cooldown(hero, bota_proto::AbilityId::RAZE_NEAR, 1),
            world.item_cooldown(hero, ItemId::PHASE_BOOTS),
        )
    };
    let (raze, phase) = cooldowns(&[]);
    assert!(raze > 0 && phase > 0);
    let cut = |ticks: u32| bota_core::cooldown_after(ticks, 7_500);
    assert_eq!(cooldowns(&[ItemId::OCTARINE_CORE]), (cut(raze), cut(phase)));
    assert_eq!(
        cooldowns(&[ItemId::OCTARINE_CORE, ItemId::OCTARINE_CORE]),
        (cut(raze), cut(phase))
    );
}

/// A Yasha's percent is summed with every other percent change of the pace and taken once, as
/// Dota sums movement speed bonuses: with Phase Boots' phase of a ranged hero (ten percent) the
/// pace is twenty percent above the walk, not 1.1 times 1.1.
#[test]
fn a_yashas_percent_adds_to_a_phase_rather_than_multiplying() {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::PHASE_BOOTS, ItemId::YASHA]);
    let boots_only = {
        let (world, hero) = out_of_fountain_after_buying(&[ItemId::PHASE_BOOTS]);
        world.stats.get(hero).expect("derived").move_speed
    };
    assert!(world.use_item(hero, 0, Target::None, &mut Vec::new()));
    world.step();
    let phased = world.stats.get(hero).expect("derived").move_speed;
    assert_eq!(phased, hastened(boots_only, 20));
}

/// Buys `item` whole through the seat and checks that the buyer gains what its values in the
/// catalogue say, each as the stat its Dota key names (mapped here, apart from the rows of
/// `data/item_mechanics.rs`), with what its attributes are worth, and nothing else. A value this
/// does not know fails it, so no value goes unread.
fn assert_carries_its_values(item: ItemId, bare: Stats) {
    let def = bota_core::item_def(item).expect("an item");
    let per_tick = |per_second: bota_core::Dec| {
        Fixed::from_ratio(per_second.0, 1000 * rules::TICKS_PER_SECOND as i32)
    };
    let got = stats_after_buying(&[item]);
    let mut expected = bare;
    let mut attributes = Attributes::ZERO;
    let mut restoration_amp_bp = 0;
    let mut mana_regen_amp_pct = 0;
    for (key, levels) in def.values {
        if key.starts_with("tooltip_") {
            // A number only the tooltip shows: another of the item's, as Dota words it.
            continue;
        }
        let [value] = levels else {
            panic!("{}: {key} has levels", def.key);
        };
        let points = |value: bota_core::Dec| Fixed::from_ratio(value.0, 1000);
        match *key {
            "bonus_all_stats" => attributes += Attributes::all(value.int()),
            "bonus_strength" => attributes.strength += points(*value),
            "bonus_agility" => attributes.agility += points(*value),
            "bonus_intellect" => attributes.intelligence += points(*value),
            "bonus_damage" => expected.damage += value.int(),
            "bonus_attack_speed" => expected.attack_speed += value.int(),
            "bonus_armor" => expected.armor += points(*value),
            "bonus_health" => expected.max_hp += points(*value),
            "bonus_mana" => expected.max_mana += points(*value),
            "bonus_health_regen" | "bonus_regen" => expected.hp_regen += per_tick(*value),
            "bonus_mana_regen" => expected.mana_regen += per_tick(*value),
            "lifesteal_percent" => expected.lifesteal_pct += value.int(),
            "spell_lifesteal" => expected.spell_lifesteal_pct += value.int(),
            "aoe_bonus" => expected.aoe_bonus += value.int(),
            "slow_resistance" => {
                // A resistance: of what the slows let through, the item keeps off its percent.
                let through = (10_000 - expected.slow_resist_bp) * (100 - value.int()) / 100;
                expected.slow_resist_bp = 10_000 - through;
            }
            "hp_regen_amp" => restoration_amp_bp += value.int() * 100,
            "spell_amp" => expected.spell_amp_pct += value.int(),
            "cast_speed_pct" => expected.cast_speed_pct += value.int(),
            "cast_range_bonus" => expected.cast_range_bonus += value.int(),
            "bonus_cooldown" => {
                expected.cooldown_rate_bp = expected.cooldown_rate_bp * (100 - value.int()) / 100;
            }
            "status_resistance" => {
                let through = (10_000 - expected.status_resist_bp) * (100 - value.int()) / 100;
                expected.status_resist_bp = 10_000 - through;
            }
            "manacost_reduction" => {
                expected.mana_cost_rate_bp = expected.mana_cost_rate_bp * (100 - value.int()) / 100;
            }
            // The buyer is a ranged hero: it reads `movement_speed_percent_bonus`.
            "movement_speed_percent_bonus_melee" => {}
            "mana_regen_multiplier" => mana_regen_amp_pct += value.int(),
            "movement_speed_percent_bonus" => {
                // A percent of the pace, on a hero nothing else hastes or slows.
                expected.move_speed = hastened(bare.move_speed, value.int());
            }
            "bonus_magical_armor" => {
                // Of what the hero's own resistance lets through, the item keeps out its
                // percent.
                let through = (10_000 - expected.magic_resist_bp) * (100 - value.int()) / 100;
                expected.magic_resist_bp = 10_000 - through;
            }
            other => panic!("{}: a value this test does not know: {other}", def.key),
        }
    }
    add_worth_of_attributes(&mut expected, attributes);
    // Health Restoration takes its share of all the health regenerated.
    expected.restoration_bp += restoration_amp_bp;
    expected.hp_regen = expected.hp_regen * Fixed::from_ratio(expected.restoration_bp, 10_000);
    // A mana regeneration amplification takes its share of all the mana regenerated.
    expected.mana_regen = expected.mana_regen * Fixed::from_ratio(100 + mana_regen_amp_pct, 100);
    assert!(
        near(got.hp_regen, expected.hp_regen),
        "{}: health regeneration",
        def.key
    );
    assert!(
        near(got.mana_regen, expected.mana_regen),
        "{}: mana regeneration",
        def.key
    );
    expected.hp_regen = got.hp_regen;
    expected.mana_regen = got.mana_regen;
    assert_eq!(got, expected, "{}", def.key);
}

/// `stats` with `more` attributes and what they are worth: health, mana and their regeneration,
/// armor and attack speed, primary damage and magic resistance. The worth itself is
/// `every_attribute_pays_for_what_it_is_worth`'s.
fn add_worth_of_attributes(stats: &mut Stats, more: Attributes) {
    let (was, now) = (stats.attributes, stats.attributes + more);
    stats.attributes = now;
    stats.max_hp += Fixed::from_int(rules::HP_PER_STRENGTH) * more.strength;
    stats.max_mana += Fixed::from_int(rules::MANA_PER_INTELLIGENCE) * more.intelligence;
    let (regen_was, regen_now) = (attribute_regen(was), attribute_regen(now));
    stats.hp_regen += regen_now.hp - regen_was.hp;
    stats.mana_regen += regen_now.mana - regen_was.mana;
    stats.armor += agility_armor(now) - agility_armor(was);
    stats.attack_speed += agility_attack_speed(now) - agility_attack_speed(was);
    stats.damage += primary_damage(now, stats.primary) - primary_damage(was, stats.primary);
    stats.magic_resist_bp += intelligence_magic_resist_bp(now) - intelligence_magic_resist_bp(was);
}

/// Shawl and Cloak, bought through the shop, each keep out their share of the magic that the
/// hero's own resistance and the other let through (Dota: "Stacks multiplicatively with other
/// sources of magic resistance"): a magical blow lands for that much less, and a Cloak lifts a
/// base 25% to 38.5% (Liquipedia, *Cloak*).
#[test]
fn shawl_and_cloak_keep_out_their_share_of_the_magic_the_rest_let_through() {
    let magic_taken = |items: &[ItemId]| {
        let (mut world, hero) = out_of_fountain_after_buying(items);
        world.push_hit(None, hero, 400, bota_proto::DamageKind::Magical, None);
        let events = world.step();
        events
            .iter()
            .find_map(|event| match event.kind() {
                EventKind::Damaged { amount, .. } => Some(amount),
                _ => None,
            })
            .expect("the blow lands")
    };
    let bare = magic_taken(&[]);
    let cloak = magic_taken(&[ItemId::CLOAK]);
    let both = magic_taken(&[ItemId::SHAWL, ItemId::CLOAK]);
    assert!((cloak - bare * 82 / 100).abs() <= 1, "{bare} then {cloak}");
    assert!(
        (both - bare * 82 * 90 / 10_000).abs() <= 1,
        "{bare} then {both}"
    );
    assert_eq!(
        bota_core::resist_both(2500, 1800),
        3850,
        "a Cloak on a base 25%"
    );
}

/// Whether two amounts of health agree to a hundredth of a point: a share of a blow taken as a
/// fixed-point product.
pub(super) fn within_a_hundredth(a: Fixed, b: Fixed) -> bool {
    (a - b).raw.abs() <= Fixed::from_ratio(1, 100).raw
}

/// The first landing of an attack by a hero that bought `items` on what `on` puts at a spot in
/// its reach: what the swing dealt, the magic that rode with it, and what the hero healed in that
/// tick, regeneration aside. With `rider`, the first landing that magic rode with.
struct Landing {
    physical: i32,
    magical: i32,
    healed: Fixed,
}

fn attack_landing(
    items: &[ItemId],
    on: impl FnOnce(&mut World, Vec2) -> Entity,
    rider: bool,
) -> Landing {
    let (mut world, hero) = out_of_fountain_after_buying(items);
    let at = world.transform.get(hero).expect("standing").pos;
    let mark = on(&mut world, at + Vec2::from_ints(120, 0));
    // A tick for the side to see what was put there, then the attack order.
    world.step();
    attack_click(&mut world, mark);
    for _ in 0..600 {
        world.health.get_mut(hero).expect("alive").hp = Fixed::from_int(200);
        let regen = world.stats.get(hero).expect("derived").hp_regen;
        let events = world.step();
        let mut landing = Landing {
            physical: 0,
            magical: 0,
            healed: world.health.get(hero).expect("alive").hp - Fixed::from_int(200) - regen,
        };
        for event in &events {
            if let EventKind::Damaged {
                source: Some(source),
                target,
                amount,
                kind,
                ..
            } = event.kind()
                && source == wire_id(hero)
                && target == wire_id(mark)
            {
                match kind {
                    bota_proto::DamageKind::Physical => landing.physical += amount,
                    _ => landing.magical += amount,
                }
            }
        }
        if landing.physical > 0 && (!rider || landing.magical > 0) {
            return landing;
        }
    }
    panic!("no such attack landed")
}

/// What one attack of a hero that bought a Morbid Mask deals `on`, and what it heals back of it,
/// regeneration aside.
fn morbid_mask_attack(on: impl FnOnce(&mut World, Vec2) -> Entity) -> (i32, Fixed) {
    let landing = attack_landing(&[ItemId::MORBID_MASK], on, false);
    (landing.physical, landing.healed)
}

/// A Morbid Mask bought through the shop heals its carrier for 18% of the physical damage its
/// attack deals a hero, and 60% of that off a creep (Dota's creep factor since 7.33c: Liquipedia,
/// *Morbid Mask*, "+18% Lifesteal (Hero) +10.8% Lifesteal (Creep)").
#[test]
fn a_morbid_mask_heals_eighteen_percent_of_a_blow_on_a_hero_and_less_on_a_creep() {
    let (dealt, healed) = morbid_mask_attack(|world, at| {
        world.spawn_hero(Team::Dire, at, SlotId(1), HeroId::SHADOW_FIEND)
    });
    assert!(
        within_a_hundredth(healed, Fixed::from_ratio(dealt * 18, 100)),
        "{healed} of {dealt}"
    );
    let (dealt, healed) =
        morbid_mask_attack(|world, at| world.spawn_unit(&bota_core::MELEE_CREEP, Team::Dire, at));
    assert!(
        within_a_hundredth(healed, Fixed::from_ratio(dealt * 108, 1000)),
        "{healed} of {dealt}"
    );
}

/// What the near Shadowraze of a hero that bought `items` deals `on`, standing where it lands,
/// and what the hero heals back in that tick, regeneration aside.
fn raze_of_a_buyer(items: &[ItemId], on: impl FnOnce(&mut World, Vec2) -> Entity) -> (i32, Fixed) {
    let (world, hero) = out_of_fountain_after_buying(items);
    raze_from(world, hero, on)
}

/// What the first raze of `hero` deals to the one `on` stands in its way, and what it healed.
fn raze_from(
    mut world: World,
    hero: Entity,
    on: impl FnOnce(&mut World, Vec2) -> Entity,
) -> (i32, Fixed) {
    set_level(&mut world, hero, 0, 1);
    let transform = world.transform.get_mut(hero).expect("standing");
    transform.facing = bota_proto::Angle { brads: 0 };
    let at = transform.pos + Vec2::from_ints(rules::RAZE_DISTANCE[0], 0);
    let mark = on(&mut world, at);
    world.health.get_mut(hero).expect("alive").hp = Fixed::from_int(200);
    cast(&mut world, 0, Target::None);
    for _ in 0..60 {
        let before = world.health.get(hero).expect("alive").hp;
        let regen = world.stats.get(hero).expect("derived").hp_regen;
        let events = world.step();
        if let Some(&dealt) = burns_on(&events, mark).first() {
            let healed = world.health.get(hero).expect("alive").hp - before - regen;
            return (dealt, healed);
        }
    }
    panic!("the raze never landed")
}

/// A Voodoo Mask bought through the shop heals its carrier for 15% of what a Shadowraze takes off
/// an enemy hero, a fifth of that off a creep (Liquipedia, *Voodoo Mask*: 15%, 3% against creeps),
/// nothing off an illusion; a second Voodoo Mask adds nothing (one source), and without one a
/// raze heals nothing.
#[test]
fn a_voodoo_mask_heals_fifteen_percent_of_a_raze_on_a_hero_and_a_fifth_of_that_on_a_creep() {
    let foe =
        |world: &mut World, at| world.spawn_hero(Team::Dire, at, SlotId(1), HeroId::SHADOW_FIEND);
    let creep = |world: &mut World, at| world.spawn_unit(&bota_core::MELEE_CREEP, Team::Dire, at);
    let (dealt, healed) = raze_of_a_buyer(&[ItemId::VOODOO_MASK], foe);
    let share = |pct: i32| Fixed::from_int(dealt) * Fixed::from_ratio(pct, 10_000);
    assert!(
        within_a_hundredth(healed, share(1500)),
        "{healed} of {dealt}"
    );
    let (twice, _) = raze_of_a_buyer(&[ItemId::VOODOO_MASK, ItemId::VOODOO_MASK], foe);
    assert_eq!(twice, dealt);
    let (_, healed_twice) = raze_of_a_buyer(&[ItemId::VOODOO_MASK, ItemId::VOODOO_MASK], foe);
    assert!(
        within_a_hundredth(healed_twice, share(1500)),
        "two masks: {healed_twice}"
    );
    let (dealt, healed) = raze_of_a_buyer(&[ItemId::VOODOO_MASK], creep);
    let share = Fixed::from_int(dealt) * Fixed::from_ratio(300, 10_000);
    assert!(
        within_a_hundredth(healed, share),
        "{healed} of {dealt} off a creep"
    );
    let (_, healed) = raze_of_a_buyer(&[ItemId::VOODOO_MASK], |world, at| {
        let illusion = foe(world, at);
        put_on(world, illusion, bota_core::ModifierKind::Illusion, 300);
        illusion
    });
    assert!(
        within_a_hundredth(healed, Fixed::ZERO),
        "{healed} off an illusion"
    );
    let (_, healed) = raze_of_a_buyer(&[], foe);
    assert!(
        within_a_hundredth(healed, Fixed::ZERO),
        "{healed} without a mask"
    );
}

/// A Faerie Fire bought through the shop adds its 2 damage while carried, and used, mends 85
/// health at once and is gone (`item_faerie_fire`: `bonus_damage` 2, `hp_restore` 85).
#[test]
fn a_faerie_fire_adds_two_damage_and_eaten_mends_eighty_five_at_once() {
    let bare = stats_after_buying(&[]);
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::FAERIE_FIRE]);
    assert_eq!(
        world.stats.get(hero).expect("derived").damage,
        bare.damage + 2
    );
    world.health.get_mut(hero).expect("alive").hp = Fixed::from_int(200);
    let regen = world.stats.get(hero).expect("derived").hp_regen;
    let events = seat_order(
        &mut world,
        Order::Use {
            slot: ItemSlot(0),
            target: Target::None,
        },
    );
    assert!(
        events.iter().any(|event| matches!(
            event.kind(),
            EventKind::ItemUsed {
                item: ItemId::FAERIE_FIRE,
                ..
            }
        )),
        "told as used"
    );
    assert_eq!(
        world.health.get(hero).expect("alive").hp,
        Fixed::from_int(285) + regen
    );
    assert_eq!(slot_of(&world, hero, 0), None, "and gone");
    world.step();
    assert_eq!(
        world.stats.get(hero).expect("derived").damage,
        bare.damage,
        "with its damage"
    );
}

/// An enemy hero at a spot.
pub(super) fn foe_hero(world: &mut World, at: Vec2) -> Entity {
    world.spawn_hero(Team::Dire, at, SlotId(1), HeroId::SHADOW_FIEND)
}

/// An illusion of an enemy hero at a spot: its hero stands far off.
fn foe_illusion(world: &mut World, at: Vec2) -> Entity {
    let hero = foe_hero(world, at + Vec2::from_ints(3000, 0));
    world.settle();
    world.make_rune_illusions(hero);
    let illusion = world
        .entities
        .iter()
        .find(|&unit| world.illusion.get(unit).is_some_and(|made| made.of == hero))
        .expect("an illusion");
    world.transform.get_mut(illusion).expect("standing").pos = at;
    world.set_order(illusion, crate::game::UnitOrder::Stand);
    illusion
}

/// What a hero that bought a Morbid Mask heals, regeneration aside, when a swing of `amount`
/// physical damage it threw lands on what `on` puts at a spot, and what the blow took.
fn morbid_swing_heal(amount: i32, on: impl FnOnce(&mut World, Vec2) -> Entity) -> (i32, Fixed) {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::MORBID_MASK]);
    let at = world.transform.get(hero).expect("standing").pos;
    let mark = on(&mut world, at + Vec2::from_ints(300, 0));
    world.step();
    world.health.get_mut(hero).expect("alive").hp = Fixed::from_int(200);
    let regen = world.stats.get(hero).expect("derived").hp_regen;
    world.hits.push_back(crate::game::Hit {
        proc: None,
        blockable_source_team: None,
        source: Some(hero),
        inflictor: None,
        target: mark,
        amount,
        kind: bota_proto::DamageKind::Physical,
        damage_amp_bp: rules::NOMINAL_BP,
        attack: true,
        spell: false,
        striker: Some(bota_core::Striker::RangedHero),
        pierces: true,
        uphill: false,
        effect: crate::game::HitEffect::None,
        laid: crate::game::Laid::NONE,
        removal: false,
        spares: false,
    });
    let events = world.step();
    let taken = events
        .iter()
        .find_map(|event| match event.kind() {
            EventKind::Damaged { target, amount, .. } if target == wire_id(mark) => Some(amount),
            _ => None,
        })
        .expect("the swing lands");
    (
        taken,
        world.health.get(hero).expect("alive").hp - Fixed::from_int(200) - regen,
    )
}

/// Lifesteal heals off what the attack dealt, not off the more an illusion takes (Dota 7.41:
/// lifesteal counts damage amplification but increased damage against illusions): as much off an
/// illusion as off its hero, which it counts as.
#[test]
fn a_morbid_mask_heals_as_much_off_an_illusion_as_off_a_hero() {
    let (taken, healed) = morbid_swing_heal(100, foe_hero);
    let (taken_by_illusion, healed_off_illusion) = morbid_swing_heal(100, foe_illusion);
    assert!(taken_by_illusion > taken, "the illusion takes more");
    assert!(
        within_a_hundredth(healed_off_illusion, healed),
        "{healed_off_illusion} off the illusion, {healed} off a hero"
    );
}

/// A killing blow heals off all the damage the attack dealt, not only the health that was left
/// (Liquipedia, *Lifesteal*: the heal is "based on the attack damage value the dying target
/// would have received").
#[test]
fn a_morbid_mask_heals_a_killing_blow_whole() {
    let creep = |world: &mut World, at| world.spawn_unit(&bota_core::MELEE_CREEP, Team::Dire, at);
    let (dealt, healed) = morbid_mask_attack(creep);
    let (dealt_killing, healed_killing) = morbid_mask_attack(|world, at| {
        let mark = creep(world, at);
        world.settle();
        world.health.get_mut(mark).expect("alive").hp = Fixed::from_int(3);
        mark
    });
    assert!(
        dealt_killing < dealt,
        "the killing blow takes what was left"
    );
    assert!(
        within_a_hundredth(healed_killing, healed),
        "{healed_killing} off the killing blow, {healed} off a whole one"
    );
}

/// Lifesteal heals off an attack's physical blow alone: the magic riding with it heals nothing
/// (Liquipedia, *Morbid Mask*: "Only works with physical attack damage").
#[test]
fn a_morbid_mask_heals_nothing_off_the_magic_riding_with_an_attack() {
    let landing = attack_landing(
        &[ItemId::MORBID_MASK, ItemId::MONKEY_KING_BAR],
        |world, at| world.spawn_unit(&bota_core::MELEE_CREEP, Team::Dire, at),
        true,
    );
    let share = Fixed::from_int(landing.physical) * Fixed::from_ratio(1080, 10_000);
    assert!(
        within_a_hundredth(landing.healed, share),
        "{} healed off {} physical and {} magical",
        landing.healed,
        landing.physical,
        landing.magical
    );
}

/// Lifesteal heals nothing off a courier (Dota 7.38: "Attack damage against wards, buildings, or
/// couriers" gives none).
#[test]
fn a_morbid_mask_heals_nothing_off_a_courier() {
    let (taken, healed) = morbid_swing_heal(100, |world, at| {
        world.spawn_unit(&bota_core::COURIER, Team::Dire, at)
    });
    assert!(taken > 0, "the courier is struck");
    assert!(within_a_hundredth(healed, Fixed::ZERO), "{healed}");
}

/// What a hero that bought a Voodoo Mask heals when a magical blow of 200 it deals lands on what
/// `on` puts at a spot, regeneration aside.
fn voodoo_heal_off(
    kind: bota_proto::DamageKind,
    on: impl FnOnce(&mut World, Vec2) -> Entity,
) -> Fixed {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::VOODOO_MASK]);
    let at = world.transform.get(hero).expect("standing").pos;
    let mark = on(&mut world, at + Vec2::from_ints(300, 0));
    world.settle();
    world.health.get_mut(hero).expect("alive").hp = Fixed::from_int(200);
    let regen = world.stats.get(hero).expect("derived").hp_regen;
    world.push_hit(
        Some(hero),
        mark,
        200,
        kind,
        Some(bota_proto::Inflictor::Ability(
            bota_proto::AbilityId::RAZE_NEAR,
        )),
    );
    world.step();
    world.health.get(hero).expect("alive").hp - Fixed::from_int(200) - regen
}

/// Spell lifesteal heals off a spell's magic on an enemy hero, and nothing off pure damage, an
/// ally, a building, a ward or a courier (Liquipedia, *Voodoo Mask*).
#[test]
fn a_voodoo_mask_heals_nothing_off_pure_damage_allies_buildings_wards_or_couriers() {
    use bota_proto::DamageKind::{Magical, Pure};
    assert!(
        voodoo_heal_off(Magical, foe_hero) > Fixed::ZERO,
        "a spell's magic heals"
    );
    let none: [(&str, Fixed); 5] = [
        ("pure", voodoo_heal_off(Pure, foe_hero)),
        (
            "an ally",
            voodoo_heal_off(Magical, |world, at| {
                world.spawn_hero(Team::Radiant, at, SlotId(1), HeroId::SHADOW_FIEND)
            }),
        ),
        (
            "a tower",
            voodoo_heal_off(Magical, |world, at| {
                world.spawn_unit(bota_core::tower_def(1), Team::Dire, at)
            }),
        ),
        (
            "a ward",
            voodoo_heal_off(Magical, |world, at| {
                world.spawn_unit(&bota_core::OBSERVER_WARD, Team::Dire, at)
            }),
        ),
        (
            "a courier",
            voodoo_heal_off(Magical, |world, at| {
                world.spawn_unit(&bota_core::COURIER, Team::Dire, at)
            }),
        ),
    ];
    for (case, healed) in none {
        assert!(within_a_hundredth(healed, Fixed::ZERO), "{case}: {healed}");
    }
}

/// Spell lifesteal heals off the magic riding with an attack (Dota 7.38: "magical damage from
/// spells or attacks"), and nothing off the attack's own physical blow.
#[test]
fn a_voodoo_mask_heals_off_the_magic_riding_with_an_attack_alone() {
    let landing = attack_landing(
        &[ItemId::VOODOO_MASK, ItemId::MONKEY_KING_BAR],
        |world, at| world.spawn_unit(&bota_core::MELEE_CREEP, Team::Dire, at),
        true,
    );
    let share = Fixed::from_int(landing.magical) * Fixed::from_ratio(300, 10_000);
    assert!(
        within_a_hundredth(landing.healed, share),
        "{} healed off {} physical and {} magical",
        landing.healed,
        landing.physical,
        landing.magical
    );
}

/// What splashes off an attack is no spell: spell lifesteal heals nothing off it (Dota 7.38:
/// spell lifesteal takes physical damage from spells only).
#[test]
fn a_voodoo_mask_heals_nothing_off_what_splashes_off_an_attack() {
    let landing = attack_landing(
        &[ItemId::VOODOO_MASK],
        |world, at| {
            let mark = world.spawn_unit(&bota_core::MELEE_CREEP, Team::Dire, at);
            world.spawn_unit(
                &bota_core::MELEE_CREEP,
                Team::Dire,
                at + Vec2::from_ints(60, 60),
            );
            let hero = world.seats[0].unit.expect("seated");
            put_on(world, hero, ModifierKind::FountainPassive, 3000);
            mark
        },
        false,
    );
    assert!(
        within_a_hundredth(landing.healed, Fixed::ZERO),
        "{} healed",
        landing.healed
    );
}

/// An illusion takes no magic resistance from what it carries (Liquipedia, *Illusions*): a
/// Cloak keeps out nothing for one.
#[test]
fn a_cloak_keeps_out_nothing_for_an_illusion() {
    let resist = |items: &[ItemId]| {
        let (mut world, hero) = out_of_fountain_after_buying(items);
        put_on(&mut world, hero, ModifierKind::Illusion, 300);
        world.step();
        world.stats.get(hero).expect("derived").magic_resist_bp
    };
    assert_eq!(resist(&[ItemId::CLOAK]), resist(&[]));
}
