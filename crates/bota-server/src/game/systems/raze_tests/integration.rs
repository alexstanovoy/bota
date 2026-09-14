use bota_proto::{EffectId, EffectView, Fixed, HeroId, ItemId, SlotId, Team, WorldView};

use crate::game::{ItemStack, Modifier, ModifierKind, Modifiers, World, rules, wire_id};

use super::fixtures::*;

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
        ticks_left: Some(15),
    });
    modifiers.put(Modifier {
        kind: ModifierKind::Inspired { hp_per_second: 300 },
        source: None,
        ticks_left: Some(14),
    });
    modifiers.stack_raze(caster);
    modifiers.stack_raze(caster);
    let expected = vec![
        EffectView {
            id: EffectId(13),
            ticks_left: Some(15),
            stacks: None,
        },
        EffectView {
            id: EffectId(14),
            ticks_left: Some(14),
            stacks: None,
        },
        EffectView {
            id: EffectId(15),
            ticks_left: Some(240),
            stacks: Some(2),
        },
    ];

    for view in [
        world.view_full(),
        world.view(Team::Dire),
        world.view(Team::Radiant),
    ] {
        let encoded = bota_proto::encode_frame_to_vec(&view).unwrap();
        let decoded: WorldView =
            bota_proto::decode_payload(&encoded[bota_proto::LEN_PREFIX..]).unwrap();
        assert_eq!(decoded, view);
        let unit = decoded
            .units
            .iter()
            .find(|unit| unit.id == wire_id(target))
            .unwrap();
        assert_eq!(unit.effects, expected);
    }
    assert_eq!(crate::game::EFFECT_SHADOWRAZE, 15);
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
                ticks_left: Some(1),
            },
            Modifier {
                kind: ModifierKind::Inspired { hp_per_second: 300 },
                source: None,
                ticks_left: Some(1),
            },
        ]),
    );
    world.modifiers.get_mut(target).unwrap().stack_raze(caster);

    world.tick_gear();

    let view = world.view_full();
    let unit = view
        .units
        .iter()
        .find(|unit| unit.id == wire_id(target))
        .unwrap();
    assert_eq!(unit.effects, vec![effect(1, 239)]);
    assert_eq!(world.modifiers.get(target).unwrap().raze_stacks(caster), 1);
}

#[test]
fn tower_flagbearer_and_mango_bonuses_survive_shadowraze_refresh() {
    let mut world = World::new();
    let target = world.spawn_hero(Team::Dire, TARGET, SlotId(1), HeroId(2));
    let caster = world.spawn_hero(Team::Radiant, ORIGIN, SlotId(0), HeroId(2));
    world.settle();
    let plain = *world.stats.get(target).unwrap();
    world.spawn_unit(crate::game::tower_def(1), Team::Dire, TARGET);
    world.spawn_unit(&crate::game::FLAGBEARER_CREEP, Team::Dire, TARGET);
    world.inventory.get_mut(target).unwrap().slots[0] = ItemStack::bought(ItemId(42), SlotId(1), 0);
    world.step();
    world.modifiers.get_mut(target).unwrap().stack_raze(caster);
    world.modifiers.get_mut(target).unwrap().stack_raze(caster);

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
            + Fixed::from_ratio(
                rules::FLAGBEARER_AURA_REGEN,
                100 * rules::TICKS_PER_SECOND as i32
            )
            + crate::game::item_def(ItemId(42)).unwrap().carried.hp_regen
    );
    let view = world.view_full();
    let unit = view
        .units
        .iter()
        .find(|unit| unit.id == wire_id(target))
        .unwrap();
    for id in [13, 14, 15] {
        assert_eq!(
            unit.effects
                .iter()
                .filter(|effect| effect.id == EffectId(id))
                .count(),
            1
        );
    }
    assert_eq!(effects(&world, target), vec![effect(2, 240)]);
}
