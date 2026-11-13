mod support;

use bota_client_core::{ClientSession, IssuedOrder};
use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{
    Cheat, EntityId, HeroId, ItemId, ModifierSpec, Order, SlotId, Target, Team, Vec2,
};
use bota_server::{
    game::{Entity, World},
    local::LocalMatch,
};

#[track_caller]
fn step(
    local: &mut LocalMatch,
    sessions: &mut [ClientSession],
    orders: &[&[bota_proto::OutgoingOrder]],
) {
    for (session, stream) in sessions.iter_mut().zip(local.step(orders).expect("step")) {
        assert!(
            !stream
                .iter()
                .any(|message| matches!(message, bota_proto::ServerMsg::OrderRejected { .. })),
            "ordinary orders are accepted"
        );
        support::fold(session, stream);
    }
}

fn cheat(world: &mut World, slot: SlotId, unit: Entity, cheat: Cheat) {
    world
        .check_cheat(slot, Some(unit), &cheat)
        .expect("valid fixture cheat");
    world.cheat(slot, Some(unit), cheat, &mut Vec::new());
}

fn prepare_heroes(local: &mut LocalMatch, index: usize) {
    let world = local.world_mut();
    world.cheats = true;
    let slot = SlotId(index as u8);
    let other = SlotId((1 - index) as u8);
    let own = world.seats[index].unit.expect("hero");
    let enemy = world.seats[1 - index].unit.expect("opponent");
    cheat(world, slot, own, Cheat::Levels { count: 9 });
    cheat(world, other, enemy, Cheat::Levels { count: 24 });
    cheat(
        world,
        other,
        enemy,
        Cheat::ApplyModifier {
            target: Target::None,
            spec: ModifierSpec {
                physical_damage: ModifierSpec::MAX_SCALE,
                ..ModifierSpec::NOMINAL
            },
            ticks: 4000,
        },
    );
    cheat(world, slot, own, Cheat::Gold { amount: 1000 });
    cheat(
        world,
        slot,
        own,
        Cheat::Teleport {
            at: Vec2::from_ints(9536, 9440),
        },
    );
    cheat(
        world,
        other,
        enemy,
        Cheat::Teleport {
            at: Vec2::from_ints(9936, 9440),
        },
    );
    cheat(
        world,
        slot,
        own,
        Cheat::ApplyModifier {
            target: Target::None,
            spec: ModifierSpec {
                max_hp: ModifierSpec::MIN_SCALE,
                ..ModifierSpec::NOMINAL
            },
            ticks: 4000,
        },
    );
}

fn kill_and_expire(
    local: &mut LocalMatch,
    sessions: &mut [ClientSession],
    index: usize,
    own: EntityId,
) {
    let stop = sessions[index]
        .issue(IssuedOrder::hero(Order::Move {
            target: Target::None,
        }))
        .expect("stop");
    let enemy_target = sessions[1 - index]
        .current()
        .expect("opponent view")
        .units
        .iter()
        .find(|unit| unit.owner == Some(SlotId(index as u8)) && unit.hero.is_some())
        .expect("visible target")
        .id;
    let attack = sessions[1 - index]
        .issue(IssuedOrder::hero(Order::Attack {
            target: Target::Unit(enemy_target),
        }))
        .expect("attack");
    let mut commands = [Vec::new(), Vec::new()];
    commands[index].push(stop);
    commands[1 - index].push(attack);
    step(local, sessions, &[&commands[0], &commands[1]]);
    for _ in 0..90 {
        if sessions[index].own_player().expect("player").respawn_left > 0 {
            break;
        }
        step(local, sessions, &[&[], &[]]);
    }
    assert!(
        sessions[index].own_player().expect("player").respawn_left > 481,
        "the real death leaves enough time for track expiry"
    );
    let home = bota_core::fountain_pos(sessions[1 - index].map(), sessions[1 - index].team());
    let retreat = sessions[1 - index]
        .issue(IssuedOrder::hero(Order::Move {
            target: Target::Pos(home),
        }))
        .expect("stop the attacker from acquiring a tower");
    let mut commands = [Vec::new(), Vec::new()];
    commands[1 - index].push(retreat);
    step(local, sessions, &[&commands[0], &commands[1]]);
    for _ in 0..481 {
        step(local, sessions, &[&[], &[]]);
    }
    assert!(sessions[index].own_player().expect("player").respawn_left > 0);
    assert!(sessions[index].entity(own).is_none());
}

fn script(side: Team, slot: SlotId, position: Vec2) -> LuaTeam {
    let mut lua = LuaTeam::new(
        side,
        &[BotSeat {
            slot,
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("team");
    lua.set_gc_stress(GcStress::Full);
    let source = format!(
        r#"
        function Think()
            local bot=GetBot()
            assert(not bot:IsAlive())
            local location,origin=bot:GetLocation(),bot:GetOrigin()
            assert(location.x == ({}/65536-9216) and location.y == ({}/65536-9216))
            assert(origin.x == location.x and origin.y == location.y and origin.z == location.z)
            assert(GetUnitToLocationDistance(bot,location)==0 and GetUnitToUnitDistanceSqr(bot,bot)==0)
            assert(bot:ActionImmediate_PurchaseItem('item_branches') == PURCHASE_ITEM_SUCCESS)
        end
    "#,
        position.x.raw, position.y.raw
    );
    lua.add_script(
        slot,
        ScriptKind::Takeover,
        "dead_owned_position",
        source.as_bytes(),
    )
    .expect("script");
    lua
}

#[test]
fn owned_dead_position_survives_track_expiry_and_purchase_reaches_the_server() {
    for (index, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        prepare_heroes(&mut local, index);
        step(&mut local, &mut sessions, &[&[], &[]]);
        let body = sessions[index].own_hero().expect("hero");
        let (own, position) = (body.id, body.pos);
        assert_ne!(
            position,
            Vec2::from_ints(9216, 9216),
            "the expected position is not the zero-vector fallback"
        );
        kill_and_expire(&mut local, &mut sessions, index, own);
        assert_eq!(
            sessions[index]
                .known(own)
                .expect("received body memory")
                .0
                .pos,
            position
        );
        let mut lua = script(side, SlotId(index as u8), position);
        let turn = lua.think(&sessions[index]).expect("dead think");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert_eq!(
            turn.bots[0].orders,
            vec![IssuedOrder::hero(Order::Buy {
                item: ItemId::IRON_BRANCH
            })]
        );
        let buy = sessions[index]
            .issue(turn.bots[0].orders[0])
            .expect("purchase intent");
        let mut commands = [Vec::new(), Vec::new()];
        commands[index].push(buy);
        step(&mut local, &mut sessions, &[&commands[0], &commands[1]]);
        let player = sessions[index].own_player().expect("player");
        assert!(player.respawn_left > 0);
        let kit = player.kit.as_ref().expect("owned dead kit");
        let purchased = kit
            .items
            .iter()
            .flatten()
            .chain(player.stash.as_ref().expect("stash").iter().flatten())
            .filter(|item| item.id == ItemId::IRON_BRANCH)
            .count();
        assert_eq!(
            purchased, 1,
            "the real server grants exactly one item to the dead player"
        );
    }
}
