use super::*;
use bota_proto::{
    AbilityId, AbilitySlot, EffectId, Fixed, ItemId, ModifierSpec, StatusFlags, UnitKind,
};

fn cheats(local: &mut LocalMatch, sessions: &mut [ClientSession], seat: usize, values: &[Cheat]) {
    let sent: Vec<_> = values
        .iter()
        .map(|&cheat| IssuedOrder::hero(Order::Cheat { cheat }))
        .collect();
    advance(local, sessions, seat, &sent);
}

fn equip(
    local: &mut LocalMatch,
    sessions: &mut [ClientSession],
    seat: usize,
    items: &[ItemId],
    spec: ModifierSpec,
) {
    let mut setup = vec![
        Cheat::AllVision { on: true },
        Cheat::Levels { count: 29 },
        Cheat::Invulnerable { on: seat == 1 },
    ];
    setup.extend(items.iter().map(|&item| Cheat::Item { item }));
    setup.push(Cheat::ApplyModifier {
        target: Target::None,
        spec,
        ticks: 3000,
    });
    cheats(local, sessions, seat, &setup);
    cheats(local, sessions, seat, &[Cheat::Refresh]);
}

fn attack(local: &mut LocalMatch, sessions: &mut [ClientSession], target: bota_proto::EntityId) {
    advance(
        local,
        sessions,
        1,
        &[IssuedOrder::hero(Order::Attack {
            target: Target::Unit(target),
        })],
    );
}

#[test]
fn a_destroyed_mid_tower_leaves_the_pregame_slice() {
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    equip(
        &mut local,
        &mut sessions,
        1,
        &[ItemId::DAEDALUS; 6],
        ModifierSpec {
            physical_damage: ModifierSpec::MAX_SCALE,
            ..ModifierSpec::NOMINAL
        },
    );
    let front = sessions[0]
        .map()
        .radiant_towers
        .iter()
        .find(|&&(lane, tier, _)| lane == bota_core::rules::LANE_MID && tier == 1)
        .expect("front")
        .2;
    teleport(
        &mut local,
        &mut sessions,
        1,
        front + Vec2::from_ints(400, 0),
    );
    let tower = sessions[1]
        .current()
        .expect("view")
        .units
        .iter()
        .find(|unit| {
            unit.kind == UnitKind::Tower && unit.team == Team::Radiant && unit.pos == front
        })
        .expect("visible front")
        .id;
    attack(&mut local, &mut sessions, tower);
    for _ in 0..180 {
        if !sessions[0]
            .current()
            .expect("view")
            .units
            .iter()
            .any(|unit| {
                unit.kind == UnitKind::Tower
                    && unit.team == Team::Radiant
                    && unit.pos == front
                    && unit.hp > 0
            })
        {
            break;
        }
        advance(&mut local, &mut sessions, 1, &[]);
    }
    let view = sessions[0].current().expect("final view");
    assert!(view.game_tick() < sessions[0].map().pregame_ticks);
    assert!(
        !view.units.iter().any(|unit| unit.kind == UnitKind::Tower
            && unit.team == Team::Radiant
            && unit.pos == front
            && unit.hp > 0),
        "the attack destroys the real front"
    );
    let sent = orders(&mut laning(Team::Radiant, SlotId(0), ""), &sessions[0]);
    assert!(
        matches!(
            sent.as_slice(),
            [IssuedOrder {
                order: Order::Attack {
                    target: Target::Pos(_)
                },
                ..
            }]
        ),
        "no recovered move without its live tower: {sent:?}"
    );
}

#[test]
fn a_dead_respawning_hero_receives_no_laning_body_order() {
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    equip(
        &mut local,
        &mut sessions,
        1,
        &[ItemId::DAEDALUS; 6],
        ModifierSpec {
            physical_damage: ModifierSpec::MAX_SCALE,
            ..ModifierSpec::NOMINAL
        },
    );
    teleport(&mut local, &mut sessions, 0, Vec2::from_ints(9000, 9000));
    teleport(&mut local, &mut sessions, 1, Vec2::from_ints(9200, 9000));
    let victim = sessions[1]
        .current()
        .expect("view")
        .units
        .iter()
        .find(|unit| unit.kind == UnitKind::Hero && unit.team == Team::Radiant)
        .expect("victim")
        .id;
    attack(&mut local, &mut sessions, victim);
    for _ in 0..90 {
        if sessions[0].current().expect("view").players[0].respawn_left > 0 {
            break;
        }
        advance(&mut local, &mut sessions, 1, &[]);
    }
    assert!(sessions[0].current().expect("view").players[0].respawn_left > 0);
    let mut team = laning(Team::Radiant, SlotId(0), "");
    assert!(orders(&mut team, &sessions[0]).is_empty());
    cheats(&mut local, &mut sessions, 0, &[Cheat::Respawn]);
    assert_eq!(
        sessions[0].current().expect("view").players[0].respawn_left,
        0
    );
    assert!(matches!(
        orders(&mut team, &sessions[0]).as_slice(),
        [IssuedOrder {
            order: Order::Move {
                target: Target::Pos(_)
            },
            ..
        }]
    ));
}

#[test]
fn a_stunned_hero_receives_no_laning_body_order() {
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    equip(
        &mut local,
        &mut sessions,
        0,
        &[ItemId::HEART; 6],
        ModifierSpec {
            max_hp: ModifierSpec::MAX_SCALE,
            physical_damage: ModifierSpec::MIN_SCALE,
            ..ModifierSpec::NOMINAL
        },
    );
    equip(
        &mut local,
        &mut sessions,
        1,
        &[
            ItemId::SKULL_BASHER,
            ItemId::BUTTERFLY,
            ItemId::BUTTERFLY,
            ItemId::BUTTERFLY,
            ItemId::BUTTERFLY,
            ItemId::BUTTERFLY,
        ],
        ModifierSpec {
            physical_damage: ModifierSpec::MIN_SCALE,
            ..ModifierSpec::NOMINAL
        },
    );
    teleport(&mut local, &mut sessions, 0, Vec2::from_ints(9000, 9000));
    teleport(&mut local, &mut sessions, 1, Vec2::from_ints(9200, 9000));
    let victim = sessions[1]
        .current()
        .expect("view")
        .units
        .iter()
        .find(|unit| unit.kind == UnitKind::Hero && unit.team == Team::Radiant)
        .expect("victim")
        .id;
    attack(&mut local, &mut sessions, victim);
    for _ in 0..800 {
        if sessions[0].own_hero().expect("living victim").statuses.bits & StatusFlags::STUNNED != 0
        {
            break;
        }
        advance(&mut local, &mut sessions, 1, &[]);
    }
    let own = sessions[0].own_hero().expect("living victim");
    assert_ne!(own.statuses.bits & StatusFlags::STUNNED, 0);
    assert_eq!(
        own.statuses.bits & (StatusFlags::FEARED | StatusFlags::CASTING | StatusFlags::CHANNELLING),
        0,
        "isolate stun from other dispatcher guards"
    );
    assert!(sessions[0].current().expect("view").game_tick() < sessions[0].map().pregame_ticks);
    assert!(orders(&mut laning(Team::Radiant, SlotId(0), ""), &sessions[0]).is_empty());
}

#[test]
fn a_feared_hero_receives_no_laning_body_order() {
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    equip(
        &mut local,
        &mut sessions,
        0,
        &[ItemId::HEART; 6],
        ModifierSpec {
            max_hp: ModifierSpec::MAX_SCALE,
            ..ModifierSpec::NOMINAL
        },
    );
    equip(
        &mut local,
        &mut sessions,
        1,
        &[],
        ModifierSpec {
            magic_damage: ModifierSpec::MAX_SCALE,
            ..ModifierSpec::NOMINAL
        },
    );
    cheats(&mut local, &mut sessions, 1, &[Cheat::Wtf { on: true }]);
    let slot = |id| {
        AbilitySlot(
            sessions[1]
                .own_hero()
                .expect("caster")
                .abilities
                .iter()
                .position(|ability| ability.id == id)
                .expect("ability") as u8,
        )
    };
    let raze = slot(AbilityId::RAZE_NEAR);
    let requiem = slot(AbilityId::REQUIEM);
    let learn: Vec<_> = [raze, raze, raze, raze, requiem, requiem, requiem]
        .into_iter()
        .map(|slot| IssuedOrder::hero(Order::Learn { slot }))
        .collect();
    advance(&mut local, &mut sessions, 1, &learn);
    teleport(&mut local, &mut sessions, 1, Vec2::from_ints(9000, 9000));
    let caster = sessions[1].own_hero().expect("caster");
    let spot = bota_core::point_ahead(caster.pos, caster.facing, Fixed::from_int(200));
    cheats(
        &mut local,
        &mut sessions,
        1,
        &[Cheat::CreateCreeps {
            enemy: true,
            at: spot,
        }],
    );
    advance(
        &mut local,
        &mut sessions,
        1,
        &[IssuedOrder::hero(Order::Cast {
            slot: raze,
            target: Target::None,
        })],
    );
    for _ in 0..60 {
        if sessions[1]
            .own_hero()
            .expect("caster")
            .effects
            .iter()
            .any(|effect| {
                effect.id == EffectId::SOULS && effect.stacks.is_some_and(|count| count > 0)
            })
        {
            break;
        }
        advance(&mut local, &mut sessions, 1, &[]);
    }
    assert!(
        sessions[1]
            .own_hero()
            .expect("caster")
            .effects
            .iter()
            .any(|effect| effect.id == EffectId::SOULS
                && effect.stacks.is_some_and(|count| count > 0)),
        "the real raze supplies souls"
    );
    cheats(
        &mut local,
        &mut sessions,
        1,
        &[Cheat::KillCreeps {
            which: bota_proto::CreepSide::All,
        }],
    );
    teleport(&mut local, &mut sessions, 0, Vec2::from_ints(9100, 9000));
    advance(
        &mut local,
        &mut sessions,
        1,
        &[IssuedOrder::hero(Order::Cast {
            slot: requiem,
            target: Target::None,
        })],
    );
    for _ in 0..90 {
        if sessions[0].own_hero().expect("living victim").statuses.bits & StatusFlags::FEARED != 0 {
            break;
        }
        advance(&mut local, &mut sessions, 1, &[]);
    }
    let own = sessions[0].own_hero().expect("living victim");
    assert_ne!(own.statuses.bits & StatusFlags::FEARED, 0);
    assert_eq!(
        own.statuses.bits
            & (StatusFlags::STUNNED | StatusFlags::CASTING | StatusFlags::CHANNELLING),
        0,
        "isolate fear from other dispatcher guards"
    );
    assert!(orders(&mut laning(Team::Radiant, SlotId(0), ""), &sessions[0]).is_empty());
}
