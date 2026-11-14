mod support;
use bota_client_core::{ClientSession, IssuedOrder};
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{Cheat, HeroId, Order, SlotId, Target, Team, UnitKind, Vec2};
use bota_server::{game::Tree, local::LocalMatch};
fn step(
    world: &mut LocalMatch,
    sessions: &mut [ClientSession],
    commands: &[Vec<bota_proto::OutgoingOrder>],
) {
    for (session, stream) in sessions
        .iter_mut()
        .zip(world.step(&[&commands[0], &commands[1]]).unwrap())
    {
        assert!(
            !stream
                .iter()
                .any(|m| matches!(m, bota_proto::ServerMsg::OrderRejected { .. })),
            "scenario order accepted"
        );
        support::fold(session, stream);
    }
}
fn reader(side: Team, slot: SlotId) -> LuaTeam {
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
    .unwrap();
    lua.add_script(slot,ScriptKind::Takeover,"observations",br#"
 local enemy=nil
 function Think()
  local bot=GetBot();local front=GetLaneFrontAmount(GetOpposingTeam(),LANE_MID,true)
  if enemy==nil then
   enemy=GetUnitList(UNIT_LIST_ENEMY_HEROES)[1];assert(enemy)
   assert(type(enemy:GetNearbyTrees(256))=='table');assert(front>0 and front<1)
  else
   assert(enemy:GetNearbyTrees(256)==nil);assert(front==1)
  end
  assert(enemy:NumQueuedActions()==-1)
  assert(GetNumCouriers()==1 and GetCourier(1)==nil)
  assert(GetCourier(0):GetTeam()==GetTeam())
  local trees=bot:GetNearbyTrees(256)
  bot:Action_MoveToLocation(Vector(100+#trees+GetNumCouriers(),100+front+enemy:NumQueuedActions(),0))
 end"#).unwrap();
    lua
}
#[test]
fn hidden_trees_creeps_courier_orders_and_foreign_queues_cannot_change_observations() {
    for observer in 0..2 {
        let enemy = 1 - observer;
        let (mut base, mut sessions) =
            support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
        let map = sessions[observer].map();
        let towers = if enemy == 0 {
            map.radiant_towers
        } else {
            map.dire_towers
        };
        let spot = towers
            .iter()
            .find(|&&(lane, tier, _)| lane == bota_core::rules::LANE_MID && tier == 3)
            .unwrap()
            .2;
        let mut commands = [Vec::new(), Vec::new()];
        for index in 0..2 {
            commands[index].push(
                sessions[index]
                    .issue(IssuedOrder::hero(Order::Cheat {
                        cheat: Cheat::Teleport { at: spot },
                    }))
                    .unwrap(),
            );
        }
        commands[enemy].push(
            sessions[enemy]
                .issue(IssuedOrder::hero(Order::Cheat {
                    cheat: Cheat::CreateCreeps {
                        enemy: false,
                        at: spot,
                    },
                }))
                .unwrap(),
        );
        step(&mut base, &mut sessions, &commands);
        let mut a = base.clone();
        let mut b = base;
        let mut sa = sessions.clone();
        let mut sb = sessions;
        let side = sa[observer].team();
        let mut la = reader(side, SlotId(observer as u8));
        let mut lb = reader(side, SlotId(observer as u8));
        for (lua, session) in [(&mut la, &sa[observer]), (&mut lb, &sb[observer])] {
            let turn = lua.think(session).unwrap();
            assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        }
        let mut ca = [Vec::new(), Vec::new()];
        let mut cb = [Vec::new(), Vec::new()];
        for (session, commands) in [
            (&mut sa[observer], &mut ca[observer]),
            (&mut sb[observer], &mut cb[observer]),
        ] {
            commands.push(
                session
                    .issue(IssuedOrder::hero(Order::Cheat {
                        cheat: Cheat::Teleport {
                            at: map.fountains[observer],
                        },
                    }))
                    .unwrap(),
            );
        }
        let gloves = bota_core::item_by_key("item_gloves").unwrap();
        cb[enemy].push(
            sb[enemy]
                .issue(IssuedOrder::hero(Order::Cheat {
                    cheat: Cheat::Item { item: gloves },
                }))
                .unwrap(),
        );
        let destination = map.fountains[enemy];
        cb[enemy].push(
            sb[enemy]
                .issue(IssuedOrder::hero(Order::Move {
                    target: Target::Pos(destination),
                }))
                .unwrap(),
        );
        let mut queued = IssuedOrder::hero(Order::Move {
            target: Target::Pos(destination + Vec2::from_ints(300, 0)),
        });
        queued.queue = true;
        cb[enemy].push(sb[enemy].issue(queued).unwrap());
        let courier = sb[enemy].own_courier().unwrap();
        let courier_id = courier.id;
        let courier_at = courier.pos;
        cb[enemy].push(
            sb[enemy]
                .issue(IssuedOrder::courier(
                    courier_id,
                    Order::Move {
                        target: Target::Pos(courier_at + Vec2::from_ints(512, 0)),
                    },
                ))
                .unwrap(),
        );
        step(&mut a, &mut sa, &ca);
        step(&mut b, &mut sb, &cb);
        assert_ne!(
            sa[enemy].own_hero().unwrap().attack_speed,
            sb[enemy].own_hero().unwrap().attack_speed
        );
        assert_ne!(
            sa[enemy].own_hero().unwrap().queue.len(),
            sb[enemy].own_hero().unwrap().queue.len()
        );
        let tree = sa[observer]
            .statics()
            .trees()
            .iter()
            .enumerate()
            .filter(|(index, at)| {
                a.world().trees.rooted_stands(*index) && !sa[observer].observes_locally(**at)
            })
            .min_by_key(|(_, at)| at.distance_squared(spot))
            .unwrap()
            .0;
        let by = b.hero_of(enemy).unwrap();
        b.world_mut().fell_tree(Tree::Rooted(tree), by);
        assert_ne!(
            a.world().trees.rooted_stands(tree),
            b.world().trees.rooted_stands(tree)
        );
        step(&mut a, &mut sa, &[Vec::new(), Vec::new()]);
        step(&mut b, &mut sb, &[Vec::new(), Vec::new()]);
        assert_ne!(
            sa[enemy].own_courier().unwrap().pos,
            sb[enemy].own_courier().unwrap().pos
        );
        assert_eq!(
            sa[observer].current().unwrap().felled_trees,
            sb[observer].current().unwrap().felled_trees
        );
        for session in [&sa[observer], &sb[observer]] {
            assert!(
                !session
                    .current()
                    .unwrap()
                    .units
                    .iter()
                    .any(|u| u.team == side.enemy()
                        && (u.kind == UnitKind::Hero || bota_core::kind_def(u.kind).lane_creep))
            );
            let entity = session
                .current()
                .unwrap()
                .players
                .iter()
                .find(|p| p.team == side.enemy())
                .unwrap()
                .unit
                .unwrap();
            assert!(
                session.entity(entity).is_some(),
                "the fogged body is remembered, not absent"
            );
        }
        let aa = la.think(&sa[observer]).unwrap();
        let bb = lb.think(&sb[observer]).unwrap();
        assert!(aa.failures.is_empty(), "{:?}", aa.failures);
        assert!(bb.failures.is_empty(), "{:?}", bb.failures);
        assert_eq!(aa.bots[0].orders, bb.bots[0].orders);
    }
}
