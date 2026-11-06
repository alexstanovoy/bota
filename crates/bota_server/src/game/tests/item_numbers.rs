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
fn out_of_fountain_after_buying(items: &[ItemId]) -> (World, Entity) {
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
    let per_tick = |per_second: bota_core::Dec| {
        Fixed::from_ratio(per_second.0, 1000 * rules::TICKS_PER_SECOND as i32)
    };
    let mut checked = 0;
    for (id, def) in bota_core::ITEMS.iter().enumerate() {
        let item = ItemId(id as u16);
        let stat_stick = !def.retired
            && !def.unbuyable
            && def.components.is_empty()
            && def.aim.is_none()
            && !def.values.is_empty();
        if !stat_stick || STAT_ITEMS_TESTED_APART.contains(&item) {
            continue;
        }
        let got = stats_after_buying(&[item]);
        let mut expected = bare;
        let mut attributes = Attributes::ZERO;
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
                "lifesteal_percent" => expected.creep_cut_lifesteal_pct += value.int(),
                "spell_lifesteal" => expected.spell_lifesteal_pct += value.int(),
                "bonus_magical_armor" => {
                    // Of what the hero's own resistance lets through, the item keeps out its
                    // percent.
                    let through = (10_000 - expected.magic_resist_bp) * (100 - value.int()) / 100;
                    expected.magic_resist_bp = 10_000 - through;
                }
                other => panic!("{}: a value this test does not know: {other}", def.key),
            }
        }
        checked += 1;
        add_worth_of_attributes(&mut expected, attributes);
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
    assert_eq!(checked, 45, "the shop's stat items");
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
fn within_a_hundredth(a: Fixed, b: Fixed) -> bool {
    (a - b).raw.abs() <= Fixed::from_ratio(1, 100).raw
}

/// What one attack of a hero that bought a Morbid Mask deals `on`, and what it heals back of it,
/// regeneration aside.
fn morbid_mask_attack(on: impl FnOnce(&mut World, Vec2) -> Entity) -> (i32, Fixed) {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::MORBID_MASK]);
    let at = world.transform.get(hero).expect("standing").pos;
    let mark = on(&mut world, at + Vec2::from_ints(120, 0));
    world.health.get_mut(hero).expect("alive").hp = Fixed::from_int(200);
    attack_click(&mut world, mark);
    for _ in 0..120 {
        let before = world.health.get(hero).expect("alive").hp;
        let regen = world.stats.get(hero).expect("derived").hp_regen;
        let events = world.step();
        let dealt = events.iter().find_map(|event| match event.kind() {
            EventKind::Damaged {
                source: Some(source),
                target,
                amount,
                inflictor: None,
                ..
            } if source == wire_id(hero) && target == wire_id(mark) => Some(amount),
            _ => None,
        });
        if let Some(dealt) = dealt {
            let healed = world.health.get(hero).expect("alive").hp - before - regen;
            return (dealt, healed);
        }
    }
    panic!("the attack never landed")
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
    let (mut world, hero) = out_of_fountain_after_buying(items);
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
