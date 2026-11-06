//! Item passives that lay something on whatever their carrier's attacks strike: Dota's attack
//! modifiers, every number from the install's `items.txt` through the catalogue.

use bota_core::{ModifierKind, item_def, mitigate, rules};
use bota_proto::{DamageKind, EffectId, EventKind, Fixed, ItemId, Team, Vec2};

use crate::game::{Entity, Hit, HitEffect, Laid, World, wire_id};

use super::item_numbers::{foe_hero, out_of_fountain_after_buying};
use super::support::*;

/// A world where the seat's hero bought `items` and its attack, given through the seat, has just
/// landed on what `on` put in its reach: the world, the hero, the struck and what the blow took.
fn first_landing(
    items: &[ItemId],
    on: impl FnOnce(&mut World, Vec2) -> Entity,
) -> (World, Entity, Entity, i32) {
    let (mut world, hero) = out_of_fountain_after_buying(items);
    let at = world.transform.get(hero).expect("standing").pos;
    let mark = on(&mut world, at + Vec2::from_ints(120, 0));
    world.step();
    attack_click(&mut world, mark);
    for _ in 0..300 {
        let events = world.step();
        let taken = events.iter().find_map(|event| match event.kind() {
            EventKind::Damaged {
                source: Some(source),
                target,
                amount,
                inflictor: None,
                ..
            } if source == wire_id(hero) && target == wire_id(mark) => Some(amount),
            _ => None,
        });
        if let Some(taken) = taken {
            return (world, hero, mark, taken);
        }
    }
    panic!("the attack never landed")
}

/// A swing's blow of 100 physical damage from `by` on `on`, laying `item`'s passive.
fn swing_laying(by: Entity, on: Entity, item: ItemId) -> Hit {
    let mut laid = Laid::NONE;
    laid.add(item);
    Hit {
        proc: None,
        blockable_source_team: None,
        source: Some(by),
        inflictor: None,
        target: on,
        amount: 100,
        kind: DamageKind::Physical,
        damage_amp_bp: rules::NOMINAL_BP,
        attack: true,
        spell: false,
        striker: Some(bota_core::Striker::RangedHero),
        pierces: true,
        uphill: false,
        effect: HitEffect::None,
        laid,
        removal: false,
        spares: false,
    }
}

/// What a test puts at a spot.
type Put = fn(&mut World, Vec2) -> Entity;

/// How many of `item`'s modifiers `unit` wears, and the time the one left has.
fn laid_on(world: &World, unit: Entity, item: ItemId) -> (usize, Option<u32>) {
    let held: Vec<_> = world
        .modifiers
        .get(unit)
        .into_iter()
        .flat_map(|on_it| on_it.active())
        .filter(|held| matches!(held.kind, ModifierKind::Laid { item: had, .. } if had == item))
        .collect();
    (held.len(), held.first().and_then(|held| held.ticks_left))
}

/// The armor `unit` stands at.
fn armor(world: &World, unit: Entity) -> Fixed {
    world.stats.get(unit).expect("derived").armor
}

/// The Orb of Blight's numbers, as the catalogue holds them from `item_blight_stone`.
fn corruption() -> bota_core::OnAttack {
    item_def(ItemId::BLIGHT_STONE)
        .and_then(|def| def.on_attack)
        .expect("the Orb of Blight lays its corruption")
}

/// An Orb of Blight bought through the shop: its carrier's attack leaves the struck wearing
/// Corruption (`modifier_blight_stone_buff`), 2 armor less for 3 seconds (`corruption_armor`,
/// `corruption_duration`), told to the struck's side; then it is gone and the armor back.
#[test]
fn an_orb_of_blight_takes_two_armor_off_what_its_carrier_strikes_for_three_seconds() {
    let laid = corruption();
    assert_eq!(laid.armor, Fixed::from_int(2));
    assert_eq!(laid.ticks, 3 * rules::TICKS_PER_SECOND);
    assert_eq!(laid.effect, EffectId::CORRUPTION);
    let (mut world, hero, mark, _) = first_landing(&[ItemId::BLIGHT_STONE], foe_hero);
    world.set_order(hero, crate::game::UnitOrder::Stand);
    let (count, left) = laid_on(&world, mark, ItemId::BLIGHT_STONE);
    assert_eq!(count, 1, "the struck wears it");
    let bare = {
        let (world, _, mark, _) = first_landing(&[], foe_hero);
        armor(&world, mark)
    };
    world.step();
    assert_eq!(armor(&world, mark), bare - laid.armor);
    let shown = shown(&world, Some(Team::Dire), mark);
    assert!(
        shown
            .effects
            .iter()
            .any(|effect| effect.id == EffectId::CORRUPTION),
        "the struck's side is told of it"
    );
    run(&mut world, left.expect("timed"));
    assert_eq!(
        laid_on(&world, mark, ItemId::BLIGHT_STONE).0,
        0,
        "and it runs out"
    );
    assert_eq!(armor(&world, mark), bare);
}

/// The corruption is on before the blow that lays it lands (Dota: "The attacks first apply the
/// debuff, then their own attack damage"): the first blow already meets the lowered armor.
#[test]
fn the_first_blow_of_an_orb_of_blight_lands_on_the_corrupted_armor() {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::BLIGHT_STONE]);
    let at = world.transform.get(hero).expect("standing").pos;
    let mark = foe_hero(&mut world, at + Vec2::from_ints(300, 0));
    world.step();
    let before = armor(&world, mark);
    world
        .hits
        .push_back(swing_laying(hero, mark, ItemId::BLIGHT_STONE));
    let events = world.step();
    let taken = events
        .iter()
        .find_map(|event| match event.kind() {
            EventKind::Damaged { target, amount, .. } if target == wire_id(mark) => Some(amount),
            _ => None,
        })
        .expect("the blow lands");
    let corrupted = before - corruption().armor;
    assert_eq!(taken, mitigate(100, DamageKind::Physical, corrupted, 0));
    assert_ne!(taken, mitigate(100, DamageKind::Physical, before, 0));
}

/// Two Orbs of Blight carried corrupt as one: one modifier, two armor.
#[test]
fn two_orbs_of_blight_corrupt_as_one() {
    let (mut world, _, mark, _) =
        first_landing(&[ItemId::BLIGHT_STONE, ItemId::BLIGHT_STONE], foe_hero);
    assert_eq!(laid_on(&world, mark, ItemId::BLIGHT_STONE).0, 1);
    let bare = {
        let (world, _, mark, _) = first_landing(&[], foe_hero);
        armor(&world, mark)
    };
    world.step();
    assert_eq!(armor(&world, mark), bare - corruption().armor);
}

/// Corruption goes through spell immunity (`SpellImmunityType` `SPELL_IMMUNITY_ENEMIES_YES`) and
/// lands on whatever an attack strikes (Liquipedia, *Orb of Blight*: it affects "Allied units",
/// "Buildings", "Ward-type units", and excludes nothing, where Orb of Corrosion's page names
/// "Couriers" among what it does not affect): a courier, immune to spells, a tower, a ward and an
/// allied creep struck by a carrier of an Orb of Blight wear it.
#[test]
fn an_orb_of_blight_corrupts_through_spell_immunity_and_on_buildings_wards_and_allies() {
    let marks: [(&str, Put); 4] = [
        ("a courier", |world, at| {
            world.spawn_unit(&bota_core::COURIER, Team::Dire, at)
        }),
        ("a tower", |world, at| {
            world.spawn_unit(bota_core::tower_def(1), Team::Dire, at)
        }),
        ("a ward", |world, at| {
            world.spawn_unit(&bota_core::OBSERVER_WARD, Team::Dire, at)
        }),
        ("an allied creep", |world, at| {
            world.spawn_unit(&bota_core::MELEE_CREEP, Team::Radiant, at)
        }),
    ];
    for (case, mark) in marks {
        let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::BLIGHT_STONE]);
        let at = world.transform.get(hero).expect("standing").pos;
        let mark = mark(&mut world, at + Vec2::from_ints(300, 0));
        world.step();
        world
            .hits
            .push_back(swing_laying(hero, mark, ItemId::BLIGHT_STONE));
        world.step();
        assert_eq!(laid_on(&world, mark, ItemId::BLIGHT_STONE).0, 1, "{case}");
    }
}

/// A basic dispel takes Corruption off (`SpellDispellableType` `SPELL_DISPELLABLE_YES`).
#[test]
fn a_basic_dispel_takes_corruption_off() {
    let (mut world, _, mark, _) = first_landing(&[ItemId::BLIGHT_STONE], foe_hero);
    world.dispel(
        mark,
        crate::game::Dispel::Debuffs,
        crate::game::DispelPower::Basic,
    );
    assert_eq!(laid_on(&world, mark, ItemId::BLIGHT_STONE).0, 0);
}

/// An illusion of a hero carrying an Orb of Blight lays nothing with its attacks (Liquipedia,
/// *Orb of Blight*: "This ability is not bestowed by Illusions").
#[test]
fn an_illusion_lays_no_corruption() {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::BLIGHT_STONE]);
    world.make_rune_illusions(hero);
    let illusion = world
        .entities
        .iter()
        .find(|&unit| world.illusion.get(unit).is_some_and(|made| made.of == hero))
        .expect("an illusion");
    let at = world.transform.get(illusion).expect("standing").pos;
    let mark = foe_hero(&mut world, at + Vec2::from_ints(120, 0));
    world.step();
    world.advance_trusted(&[crate::game::Command {
        slot: bota_proto::SlotId(0),
        unit: Some(wire_id(illusion)),
        order: bota_proto::Order::Attack {
            target: bota_proto::Target::Unit(wire_id(mark)),
        },
        queue: false,
        seq: 0,
    }]);
    for _ in 0..300 {
        let events = world.step();
        let struck = events.iter().any(|event| {
            matches!(event.kind(), EventKind::Damaged { source: Some(source), target, .. }
                if source == wire_id(illusion) && target == wire_id(mark))
        });
        if struck {
            assert_eq!(laid_on(&world, mark, ItemId::BLIGHT_STONE).0, 0);
            return;
        }
    }
    panic!("the illusion never struck")
}

/// The Orb of Venom's numbers, as the catalogue holds them from `item_orb_of_venom`.
fn poison() -> bota_core::OnAttack {
    item_def(ItemId::ORB_OF_VENOM)
        .and_then(|def| def.on_attack)
        .expect("the Orb of Venom lays its poison")
}

/// The ticks from now on which `mark` loses health to the Orb of Venom, with what each took, over
/// `ticks` ticks.
fn poison_beats(world: &mut World, mark: Entity, ticks: u32) -> Vec<(u32, i32)> {
    let mut beats = Vec::new();
    for at in 1..=ticks {
        for event in world.step() {
            if let EventKind::Damaged {
                target,
                amount,
                kind: DamageKind::Magical,
                inflictor: Some(bota_proto::Inflictor::Item(ItemId::ORB_OF_VENOM)),
                ..
            } = event.kind()
                && target == wire_id(mark)
            {
                beats.push((at, amount));
            }
        }
    }
    beats
}

/// An Orb of Venom bought through the shop poisons what its carrier's attack strikes: three
/// beats of 12 magical damage a second apart, the first a second after the blow (`damage`,
/// `tick_interval`, `poison_duration`; Liquipedia: "starting 1 second after the debuff is applied,
/// resulting in 3 instances"), each cut by the struck's magic resistance.
#[test]
fn an_orb_of_venom_poisons_for_three_beats_of_twelve() {
    let laid = poison();
    assert_eq!(laid.dps, 12);
    assert_eq!(laid.effect, EffectId::POISON_ATTACK);
    let (mut world, hero, mark, _) = first_landing(&[ItemId::ORB_OF_VENOM], foe_hero);
    world.set_order(hero, crate::game::UnitOrder::Stand);
    assert_eq!(laid_on(&world, mark, ItemId::ORB_OF_VENOM).0, 1);
    let resist = world.stats.get(mark).expect("derived").magic_resist_bp;
    let each = mitigate(12, DamageKind::Magical, Fixed::ZERO, resist);
    let second = rules::TICKS_PER_SECOND;
    assert_eq!(
        poison_beats(&mut world, mark, 4 * second),
        vec![(second, each), (2 * second, each), (3 * second, each)]
    );
    assert_eq!(
        laid_on(&world, mark, ItemId::ORB_OF_VENOM).0,
        0,
        "and it is gone"
    );
}

/// The poison is laid at most once every 9 seconds (`AbilityCooldown`; Dota 7.38: "Poison Attack
/// now has a 9s cooldown"), its wait started by the swing that lays it and shown on the item.
#[test]
fn an_orb_of_venom_poisons_once_each_cooldown() {
    let (mut world, hero, mark, _) = first_landing(&[ItemId::ORB_OF_VENOM], foe_hero);
    let wait = slot_of(&world, hero, 0).expect("carried").cooldown;
    assert!(
        wait > 9 * rules::TICKS_PER_SECOND - 30 && wait <= 9 * rules::TICKS_PER_SECOND,
        "the wait runs: {wait}"
    );
    let laid_at = |world: &World| {
        world.modifiers.get(mark).and_then(|on_it| {
            on_it.active().find_map(|held| match held.kind {
                ModifierKind::Laid { item, since } if item == ItemId::ORB_OF_VENOM => Some(since),
                _ => None,
            })
        })
    };
    let first = laid_at(&world).expect("poisoned");
    // The hero swings on; within the wait nothing is laid again.
    run(&mut world, 4 * rules::TICKS_PER_SECOND);
    assert_eq!(
        laid_at(&world),
        None,
        "the poison ran out and was not laid again"
    );
    run(&mut world, wait);
    let again = laid_at(&world).expect("poisoned again once the wait is over");
    assert!(again >= first + 9 * rules::TICKS_PER_SECOND);
}

/// The poison lands on enemy units alone (Liquipedia, Orb of Venom: not "Buildings", "Ward-type
/// units", "Ally units"; Dota 2012-06-11: "Fixed Poison Attack working on denies", 2011-07-01:
/// "Fixed ability affecting buildings"), and spell immunity keeps it off (no `SpellImmunityType`;
/// 7.20: "no longer pierces spell immunity"): a tower, a ward, an allied creep and a courier
/// struck wear none.
#[test]
fn an_orb_of_venom_poisons_enemy_units_alone() {
    let marks: [(&str, Put); 4] = [
        ("a tower", |world, at| {
            world.spawn_unit(bota_core::tower_def(1), Team::Dire, at)
        }),
        ("a ward", |world, at| {
            world.spawn_unit(&bota_core::OBSERVER_WARD, Team::Dire, at)
        }),
        ("an allied creep", |world, at| {
            world.spawn_unit(&bota_core::MELEE_CREEP, Team::Radiant, at)
        }),
        ("a courier", |world, at| {
            world.spawn_unit(&bota_core::COURIER, Team::Dire, at)
        }),
    ];
    for (case, mark) in marks {
        let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::ORB_OF_VENOM]);
        let at = world.transform.get(hero).expect("standing").pos;
        let mark = mark(&mut world, at + Vec2::from_ints(300, 0));
        world.step();
        world
            .hits
            .push_back(swing_laying(hero, mark, ItemId::ORB_OF_VENOM));
        world.step();
        assert_eq!(laid_on(&world, mark, ItemId::ORB_OF_VENOM).0, 0, "{case}");
    }
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::ORB_OF_VENOM]);
    let at = world.transform.get(hero).expect("standing").pos;
    let creep = world.spawn_unit(
        &bota_core::MELEE_CREEP,
        Team::Dire,
        at + Vec2::from_ints(300, 0),
    );
    world.step();
    world
        .hits
        .push_back(swing_laying(hero, creep, ItemId::ORB_OF_VENOM));
    world.step();
    assert_eq!(
        laid_on(&world, creep, ItemId::ORB_OF_VENOM).0,
        1,
        "an enemy creep"
    );
}

/// The poison removes health, so nothing that answers to damage answers to it (Dota 7.07: "flagged
/// as HP Removal"; Liquipedia, Orb of Venom: it "does not trigger any on-damage effects"): the
/// struck's Healing Salve keeps mending, and it never takes the last point.
#[test]
fn the_poison_removes_health_and_never_the_last_point() {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::ORB_OF_VENOM]);
    let at = world.transform.get(hero).expect("standing").pos;
    // Out of reach of the hero's own attacks, which would put the salve out.
    let mark = foe_hero(&mut world, at + Vec2::from_ints(1500, 0));
    world.step();
    world
        .hits
        .push_back(swing_laying(hero, mark, ItemId::ORB_OF_VENOM));
    world.step();
    hand_item(&mut world, mark, ItemId::HEALING_SALVE, 1);
    assert!(world.use_item(mark, 0, bota_proto::Target::None, &mut Vec::new()));
    let beats = poison_beats(&mut world, mark, 2 * rules::TICKS_PER_SECOND);
    assert_eq!(beats.len(), 2, "beats land");
    assert!(
        carries(
            &world,
            mark,
            ModifierKind::Mending {
                per_tick: Fixed::ZERO,
                breaks: false,
            }
        ),
        "the salve keeps mending"
    );
    assert_eq!(
        laid_on(&world, mark, ItemId::ORB_OF_VENOM).0,
        1,
        "still poisoned"
    );
    world.health.get_mut(mark).expect("alive").hp = Fixed::from_int(5);
    run(&mut world, 2 * rules::TICKS_PER_SECOND);
    assert!(world.alive(mark), "never the last point");
}

/// The health a carrier of an Orb of Venom and a Voodoo Mask has after two beats of the poison on
/// an enemy hero out of its reach, or after the same time beside an enemy it never poisoned.
fn carrier_health_after_two_beats(poisoned: bool) -> Fixed {
    let (mut world, hero) =
        out_of_fountain_after_buying(&[ItemId::ORB_OF_VENOM, ItemId::VOODOO_MASK]);
    let at = world.transform.get(hero).expect("standing").pos;
    let mark = foe_hero(&mut world, at + Vec2::from_ints(1500, 0));
    world.step();
    world.health.get_mut(hero).expect("alive").hp = Fixed::from_int(200);
    if poisoned {
        world
            .hits
            .push_back(swing_laying(hero, mark, ItemId::ORB_OF_VENOM));
    }
    world.step();
    let beats = poison_beats(&mut world, mark, 2 * rules::TICKS_PER_SECOND);
    assert_eq!(beats.len(), if poisoned { 2 } else { 0 });
    world.health.get(hero).expect("alive").hp
}

/// The carrier's Voodoo Mask heals nothing off the poison (Dota 7.38: no spell lifesteal off
/// "Spells that remove Health instead of dealing direct damage"; Liquipedia, Damage Types: "All
/// damage with HP Removal flag will not generate spell lifesteal"): it ends up with exactly the
/// health it has after the same time poisoning nothing.
#[test]
fn the_poison_heals_its_carrier_no_spell_lifesteal() {
    assert_eq!(
        carrier_health_after_two_beats(true),
        carrier_health_after_two_beats(false)
    );
}

/// However much a beat grows on its way, it never takes the last point (Dota 7.07: Poison Attack
/// "is no longer lethal"): a beat of 12 from a carrier under an Amplify Damage Rune's spell
/// amplification comes to 13, and an enemy creep with 13 health left keeps one of it.
#[test]
fn an_amplified_beat_of_poison_never_takes_the_last_point() {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::ORB_OF_VENOM]);
    put_on(
        &mut world,
        hero,
        ModifierKind::RuneAmplifyDamage,
        60 * rules::TICKS_PER_SECOND,
    );
    let at = world.transform.get(hero).expect("standing").pos;
    let creep = world.spawn_unit(
        &bota_core::MELEE_CREEP,
        Team::Dire,
        at + Vec2::from_ints(1500, 0),
    );
    world.set_order(creep, crate::game::UnitOrder::Stand);
    world.step();
    world
        .hits
        .push_back(swing_laying(hero, creep, ItemId::ORB_OF_VENOM));
    world.step();
    let resist = world.stats.get(creep).expect("derived").magic_resist_bp;
    let amp = world
        .stats
        .get(hero)
        .expect("derived")
        .damage_amp_bp(DamageKind::Magical);
    let beat = mitigate(
        bota_core::amplify(poison().dps, amp),
        DamageKind::Magical,
        Fixed::ZERO,
        resist,
    );
    assert_eq!(
        beat, 13,
        "the amplified beat outgrows what is left above the last point"
    );
    world.health.get_mut(creep).expect("alive").hp = Fixed::from_int(13);
    let beats = poison_beats(&mut world, creep, rules::TICKS_PER_SECOND);
    assert_eq!(beats, vec![(rules::TICKS_PER_SECOND, 12)]);
    assert!(world.alive(creep), "never the last point");
}

/// The poison is told as magical damage that removes health (Liquipedia, Orb of Venom: "Spell /
/// Magical / HP Removal"), so a client can tell it from damage dealt; the attack's own blow is not.
#[test]
fn the_poison_is_told_as_magical_health_removal() {
    let (mut world, hero, mark, _) = first_landing(&[ItemId::ORB_OF_VENOM], foe_hero);
    world.set_order(hero, crate::game::UnitOrder::Stand);
    let mut told = Vec::new();
    for _ in 0..2 * rules::TICKS_PER_SECOND {
        for event in world.step() {
            if let EventKind::Damaged {
                target,
                kind,
                inflictor,
                removal,
                ..
            } = event.kind()
                && target == wire_id(mark)
            {
                told.push((kind, inflictor, removal));
            }
        }
    }
    let poison = Some(bota_proto::Inflictor::Item(ItemId::ORB_OF_VENOM));
    assert_eq!(
        told,
        vec![(DamageKind::Magical, poison, true); 2],
        "two beats, each told as a removal"
    );
}

/// An attack modifier rides on the attack and is lost with it (Liquipedia, Attack Modifier:
/// "attack modifiers are not applied if the attack is missed"), while the wait is decided at the
/// attack point (Liquipedia, Orb of Venom: "The effects are calculated upon reaching the attack
/// point, not upon landing an attack"; Dota 7.39c: "Poison Attack no longer has True Strike on the
/// attack"): a poisoning swing that the struck evades lays nothing and its 9 seconds still run.
#[test]
fn an_evaded_poison_attack_lays_nothing_and_still_starts_the_wait() {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::ORB_OF_VENOM]);
    let at = world.transform.get(hero).expect("standing").pos;
    let mark = foe_hero(&mut world, at + Vec2::from_ints(120, 0));
    // Six Butterflies: an evasion of 1 - 0.65^6, so that a poisoning swing is soon evaded. Their
    // damage would bring the carrier down: no hero falls.
    world.immortal_heroes = true;
    if let Some(bag) = world.inventory.get_mut(mark) {
        for slot in bag.slots.iter_mut().take(rules::INVENTORY_SLOTS) {
            *slot = Some(fresh_stack(ItemId::BUTTERFLY, 0));
        }
    }
    world.step();
    attack_click(&mut world, mark);
    let wait = |world: &World| slot_of(world, hero, 0).expect("carried").cooldown;
    let mut poisoning = false;
    for _ in 0..60 * rules::TICKS_PER_SECOND {
        let before = wait(&world);
        let events = world.step();
        poisoning |= before == 0 && wait(&world) > 0;
        let (evaded, struck) = events
            .iter()
            .fold((false, false), |(evaded, struck), event| {
                match event.kind() {
                    EventKind::Missed {
                        source: Some(source),
                        target,
                    } if source == wire_id(hero) && target == wire_id(mark) => (true, struck),
                    EventKind::Damaged {
                        source: Some(source),
                        target,
                        inflictor: None,
                        ..
                    } if source == wire_id(hero) && target == wire_id(mark) => (evaded, true),
                    _ => (evaded, struck),
                }
            });
        if poisoning && evaded {
            assert_eq!(
                laid_on(&world, mark, ItemId::ORB_OF_VENOM).0,
                0,
                "nothing laid"
            );
            assert!(wait(&world) > 0, "the wait runs");
            return;
        }
        if struck {
            poisoning = false;
        }
    }
    panic!("no poisoning swing was evaded");
}

/// The Orb of Frost's numbers, as the catalogue holds them from `item_orb_of_frost`.
fn frost() -> bota_core::OnAttack {
    item_def(ItemId::ORB_OF_FROST)
        .and_then(|def| def.on_attack)
        .expect("the Orb of Frost lays its frost")
}

/// What `mark`'s stats come to with and without the Frost an Orb of Frost's attack laid on it.
fn frosted(on: Put) -> (crate::game::Stats, crate::game::Stats) {
    let (world, _, mark, _) = first_landing(&[ItemId::ORB_OF_FROST], on);
    assert_eq!(laid_on(&world, mark, ItemId::ORB_OF_FROST).0, 1, "frosted");
    let mut world = world;
    world.step();
    let with = *world.stats.get(mark).expect("derived");
    let (mut world, _, mark, _) = first_landing(&[], on);
    world.step();
    (with, *world.stats.get(mark).expect("derived"))
}

/// An Orb of Frost bought through the shop slows what its carrier's attack strikes, 13% a ranged
/// target and 6% a melee one (`slow_ranged`, `slow_melee`; Dota 7.38b: "Melee Movement Slow
/// increased from 5% to 6%"), for 3 seconds (`duration`).
#[test]
fn an_orb_of_frost_slows_a_ranged_target_thirteen_percent_and_a_melee_one_six() {
    let laid = frost();
    assert_eq!((laid.ranged_slow_pct, laid.slow_pct), (13, 6));
    assert_eq!(laid.ticks, 3 * rules::TICKS_PER_SECOND);
    let (with, bare) = frosted(foe_hero);
    let close = |a: Fixed, b: Fixed| (a - b).raw.abs() <= Fixed::from_ratio(1, 100).raw;
    assert!(
        close(
            with.move_speed,
            bare.move_speed * Fixed::from_ratio(87, 100)
        ),
        "{:?} of {:?}",
        with.move_speed,
        bare.move_speed
    );
    let (with, bare) =
        frosted(|world, at| world.spawn_unit(&bota_core::MELEE_CREEP, Team::Dire, at));
    assert!(
        close(
            with.move_speed,
            bare.move_speed * Fixed::from_ratio(94, 100)
        ),
        "{:?} of {:?}",
        with.move_speed,
        bare.move_speed
    );
}

/// Frost cuts the struck's Health Restoration by 15% (`heal_reduction`; Dota 7.41e: "increased
/// from 13% to 15%"): what it regenerates and every heal it takes (7.41: "Now also modifies
/// incoming healing").
#[test]
fn an_orb_of_frost_cuts_health_restoration_fifteen_percent() {
    assert_eq!(frost().restoration_cut_pct, 15);
    let (with, bare) = frosted(foe_hero);
    assert_eq!(with.restoration_bp, 8_500);
    assert!(
        (with.hp_regen.raw - bare.hp_regen.raw * 85 / 100).abs() <= 1,
        "{:?} of {:?}",
        with.hp_regen,
        bare.hp_regen
    );
    let (mut world, hero, mark, _) = first_landing(&[ItemId::ORB_OF_FROST], foe_hero);
    world.set_order(hero, crate::game::UnitOrder::Stand);
    world.step();
    world.health.get_mut(mark).expect("alive").hp = Fixed::from_int(100);
    let regen = world.stats.get(mark).expect("derived").hp_regen;
    let (restored, _) = world.refill_pools(mark, 200, 0);
    assert_eq!(restored, 170, "a heal of 200 comes through as 170");
    world.step();
    assert!(world.health.get(mark).expect("alive").hp >= Fixed::from_int(270) + regen);
}

/// Frost lands on enemy units alone (Liquipedia, Orb of Frost: not "Buildings", "Ward-type units",
/// "Couriers"; Dota 7.41e: "Frost no longer applies when attacking allies"), and spell immunity
/// keeps it off (no `SpellImmunityType`).
#[test]
fn an_orb_of_frost_lands_on_enemy_units_alone() {
    let marks: [(&str, Put); 4] = [
        ("a tower", |world, at| {
            world.spawn_unit(bota_core::tower_def(1), Team::Dire, at)
        }),
        ("a ward", |world, at| {
            world.spawn_unit(&bota_core::OBSERVER_WARD, Team::Dire, at)
        }),
        ("an allied creep", |world, at| {
            world.spawn_unit(&bota_core::MELEE_CREEP, Team::Radiant, at)
        }),
        ("a courier", |world, at| {
            world.spawn_unit(&bota_core::COURIER, Team::Dire, at)
        }),
    ];
    for (case, mark) in marks {
        let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::ORB_OF_FROST]);
        let at = world.transform.get(hero).expect("standing").pos;
        let mark = mark(&mut world, at + Vec2::from_ints(300, 0));
        world.step();
        world
            .hits
            .push_back(swing_laying(hero, mark, ItemId::ORB_OF_FROST));
        world.step();
        assert_eq!(laid_on(&world, mark, ItemId::ORB_OF_FROST).0, 0, "{case}");
    }
}

/// A basic dispel takes Frost off (Liquipedia, Orb of Frost: "Dispellable by any Dispel
/// sources"), and the struck walks and mends at its own pace again.
#[test]
fn a_basic_dispel_takes_frost_off() {
    let (mut world, hero, mark, _) = first_landing(&[ItemId::ORB_OF_FROST], foe_hero);
    world.set_order(hero, crate::game::UnitOrder::Stand);
    world.dispel(
        mark,
        crate::game::Dispel::Debuffs,
        crate::game::DispelPower::Basic,
    );
    world.step();
    assert_eq!(laid_on(&world, mark, ItemId::ORB_OF_FROST).0, 0);
    assert_eq!(
        world.stats.get(mark).expect("derived").restoration_bp,
        rules::NOMINAL_BP
    );
}

/// Frost from two carriers is one Frost (Liquipedia, Orb of Frost: "Reapplying the debuff refreshes
/// the duration"; Dota 7.39: the slow and Health Restoration cut of Orb of Frost-based items "no
/// longer stack, the highest values take priority"), while a slow of another kind adds to it, as
/// Dota adds the percent changes of a pace: a ranged hero struck by two carriers walks 13% slower,
/// and 13% and 20% slower under a slow of 20% besides.
#[test]
fn frost_from_two_carriers_slows_once_and_adds_to_another_slow() {
    let (mut world, hero) = out_of_fountain_after_buying(&[ItemId::ORB_OF_FROST]);
    let at = world.transform.get(hero).expect("standing").pos;
    let other = world.spawn_hero(
        Team::Radiant,
        at + Vec2::from_ints(0, 200),
        bota_proto::SlotId(2),
        bota_proto::HeroId::PUDGE,
    );
    let mark = foe_hero(&mut world, at + Vec2::from_ints(1500, 0));
    world.set_order(mark, crate::game::UnitOrder::Stand);
    world.step();
    let bare = world.stats.get(mark).expect("derived").move_speed;
    for by in [hero, other] {
        world
            .hits
            .push_back(swing_laying(by, mark, ItemId::ORB_OF_FROST));
    }
    world.step();
    world.step();
    assert_eq!(
        laid_on(&world, mark, ItemId::ORB_OF_FROST).0,
        1,
        "one Frost"
    );
    let close = |a: Fixed, b: Fixed| (a - b).raw.abs() <= Fixed::from_ratio(1, 100).raw;
    let frosted = world.stats.get(mark).expect("derived").move_speed;
    assert!(
        close(frosted, bare * Fixed::from_ratio(87, 100)),
        "{frosted:?} of {bare:?}"
    );
    put_on(
        &mut world,
        mark,
        ModifierKind::Slowed { pct: 20 },
        rules::TICKS_PER_SECOND,
    );
    world.step();
    let both = world.stats.get(mark).expect("derived").move_speed;
    assert!(
        close(both, bare * Fixed::from_ratio(67, 100)),
        "{both:?} of {bare:?}"
    );
}
