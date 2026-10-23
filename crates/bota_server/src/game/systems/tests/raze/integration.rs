use bota_core::{ModifierKind, item_def, rules, tower_def};

use crate::game::{ItemStack, Modifier, Modifiers, World, wire_id};
use bota_proto::{
    AbilityId, EffectId, EffectView, Fixed, HeroId, Inflictor, ItemId, LEN_PREFIX, SlotId, Team,
    WorldView, decode_payload, encode_frame_to_vec,
};

use super::fixtures::*;

/// What these records name as the raze that put them on.
const RAZE: Option<Inflictor> = Some(Inflictor::Ability(AbilityId::RAZE_NEAR));

#[test]
fn auras_and_shadowraze_project_distinct_ids_and_round_trip_together() {
    let (mut world, caster, target) = fixture();
    world.modifiers.insert(target, Modifiers::default());
    let modifiers = world.modifiers.get_mut(target).unwrap();
    modifiers.put(Modifier {
        kind: ModifierKind::Guarded {
            armor: 3,
            hp_per_second: 100,
        },
        source: None,
        inflictor: None,
        ticks_left: Some(15),
        ticks_total: Some(15),
    });
    modifiers.put(Modifier {
        kind: ModifierKind::Inspired {
            hp_per_second: 300,
            magic_resist_pct: 0,
        },
        source: None,
        inflictor: None,
        ticks_left: Some(14),
        ticks_total: Some(14),
    });
    modifiers.stack_raze(caster, RAZE);
    modifiers.stack_raze(caster, RAZE);
    let expected = vec![
        EffectView {
            id: EffectId::GUARDED,
            ticks_left: Some(15),
            stacks: None,
            caster: None,
            inflictor: None,
            ticks_total: Some(15),
        },
        EffectView {
            id: EffectId::INSPIRED,
            ticks_left: Some(14),
            stacks: None,
            caster: None,
            inflictor: None,
            ticks_total: Some(14),
        },
        EffectView {
            caster: Some(wire_id(caster)),
            inflictor: RAZE,
            ticks_total: Some(rules::RAZE_DEBUFF_TICKS),
            ..effect(2, 180)
        },
    ];

    for view in [
        world.view_full(),
        world.view(Team::Dire),
        world.view(Team::Radiant),
    ] {
        let encoded = encode_frame_to_vec(&view).unwrap();
        let decoded: WorldView = decode_payload(&encoded[LEN_PREFIX..]).unwrap();
        assert_eq!(decoded, view);
        let unit = decoded
            .units
            .iter()
            .find(|unit| unit.id == wire_id(target))
            .unwrap();
        assert_eq!(unit.effects, expected);
    }
}

#[test]
fn expired_auras_are_not_projected_or_counted_as_shadowraze() {
    let (mut world, caster, target) = fixture();
    world.modifiers.insert(
        target,
        Modifiers(vec![
            Modifier {
                kind: ModifierKind::Guarded {
                    armor: 3,
                    hp_per_second: 100,
                },
                source: None,
                inflictor: None,
                ticks_left: Some(1),
                ticks_total: Some(1),
            },
            Modifier {
                kind: ModifierKind::Inspired {
                    hp_per_second: 300,
                    magic_resist_pct: 0,
                },
                source: None,
                inflictor: None,
                ticks_left: Some(1),
                ticks_total: Some(1),
            },
        ]),
    );
    world
        .modifiers
        .get_mut(target)
        .unwrap()
        .stack_raze(caster, RAZE);

    world.tick_gear();

    assert_eq!(
        shown(&world, None, target).effects,
        vec![EffectView {
            caster: Some(wire_id(caster)),
            inflictor: RAZE,
            ticks_total: Some(rules::RAZE_DEBUFF_TICKS),
            ..effect(1, 179)
        }]
    );
    assert_eq!(world.modifiers.get(target).unwrap().raze_stacks(caster), 1);
}

#[test]
fn tower_and_mango_bonuses_survive_shadowraze_refresh() {
    let mut world = World::new();
    let target = world.spawn_hero(Team::Dire, TARGET, SlotId(1), HeroId::SHADOW_FIEND);
    let caster = world.spawn_hero(Team::Radiant, ORIGIN, SlotId(0), HeroId::SHADOW_FIEND);
    world.settle();
    let plain = *world.stats.get(target).unwrap();
    world.spawn_unit(tower_def(1), Team::Dire, TARGET);
    world.inventory.get_mut(target).unwrap().slots[0] =
        ItemStack::bought(ItemId::MANGO, SlotId(1), 0);
    world.step();
    world
        .modifiers
        .get_mut(target)
        .unwrap()
        .stack_raze(caster, RAZE);
    world
        .modifiers
        .get_mut(target)
        .unwrap()
        .stack_raze(caster, RAZE);

    world.settle();

    let stats = world.stats.get(target).unwrap();
    assert_eq!(
        stats.armor,
        plain.armor + Fixed::from_int(rules::TOWER_AURA_ARMOR[0])
    );
    assert_eq!(
        stats.hp_regen,
        plain.hp_regen
            + Fixed::from_ratio(
                rules::TOWER_AURA_REGEN[0],
                100 * rules::TICKS_PER_SECOND as i32
            )
            + item_def(ItemId::MANGO).unwrap().carried.hp_regen
    );
    let unit = shown(&world, None, target);
    for id in [13, 15] {
        assert_eq!(
            unit.effects
                .iter()
                .filter(|effect| effect.id == EffectId(id))
                .count(),
            1
        );
    }
    assert_eq!(effects(&world, target), vec![effect(2, 180)]);
}
