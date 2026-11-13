#[path = "laning/conditions.rs"]
mod conditions;
mod support;

use bota_client_core::{ClientSession, IssuedOrder};
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ModuleBank};
use bota_proto::{Cheat, HeroId, Order, ServerMsg, SlotId, Target, Team, Vec2};
use bota_server::local::LocalMatch;

fn laning(side: Team, slot: SlotId, think: &str) -> LuaTeam {
    let mut modules = ModuleBank::new(Limits::DEFAULT);
    let source = format!("function GetDesire() return 1 end {think}");
    modules
        .insert("mode_laning_generic", source.as_bytes())
        .expect("mode");
    modules.insert("ability_item_usage_generic", b"function AbilityUsageThink() end function ItemUsageThink() end function CourierUsageThink() end function AbilityLevelUpThink() end").expect("usage");
    modules
        .insert("item_purchase_generic", b"function ItemPurchaseThink() end")
        .expect("purchase");
    let mut team = LuaTeam::new(
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
    team.set_modules(modules).expect("modules");
    team.load_pack().expect("pack");
    team
}

fn advance(
    local: &mut LocalMatch,
    sessions: &mut [ClientSession],
    seat: usize,
    orders: &[IssuedOrder],
) {
    let sent: Vec<_> = orders
        .iter()
        .map(|&order| sessions[seat].issue(order).expect("issue"))
        .collect();
    let mut batches: Vec<&[_]> = vec![&[]; sessions.len()];
    batches[seat] = &sent;
    for (session, stream) in sessions
        .iter_mut()
        .zip(local.step(&batches).expect("server step"))
    {
        assert!(
            !stream
                .iter()
                .any(|message| matches!(message, ServerMsg::OrderRejected { .. })),
            "fixture and bot orders must be accepted: {stream:?}"
        );
        support::fold(session, stream);
    }
}

fn target(session: &ClientSession, side: Team) -> Vec2 {
    support::pregame_laning_goal(session.map(), side)
}

fn teleport(local: &mut LocalMatch, sessions: &mut [ClientSession], seat: usize, at: Vec2) {
    advance(
        local,
        sessions,
        seat,
        &[
            IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Teleport { at },
            }),
            IssuedOrder::hero(Order::Attack {
                target: Target::None,
            }),
        ],
    );
    assert_eq!(
        sessions[seat].own_hero().expect("hero").pos,
        at,
        "fixture reaches its exact public position"
    );
}

fn orders(team: &mut LuaTeam, session: &ClientSession) -> Vec<IssuedOrder> {
    let turn = team.think(session).expect("turn");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    turn.bots.into_iter().next().expect("seat").orders
}

#[test]
fn pregame_laning_moves_without_attacking_for_either_side() {
    for (seat, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let (mut local, mut sessions) =
            support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
        advance(
            &mut local,
            &mut sessions,
            seat,
            &[IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Teleport {
                    at: Vec2::from_ints(8000, 8000),
                },
            })],
        );
        assert!(
            sessions[seat].current().expect("view").game_tick()
                < sessions[seat].map().pregame_ticks
        );
        let sent = orders(&mut laning(side, SlotId(seat as u8), ""), &sessions[seat]);
        assert_eq!(
            sent,
            vec![IssuedOrder::hero(Order::Move {
                target: Target::Pos(target(&sessions[seat], side))
            })]
        );
        advance(&mut local, &mut sessions, seat, &sent);
    }
}

#[test]
fn the_native_controller_retries_a_dropped_move_and_clears_its_own_action_at_arrival() {
    for (seat, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let (mut local, mut sessions) =
            support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
        teleport(&mut local, &mut sessions, seat, Vec2::from_ints(8000, 8000));
        let goal = target(&sessions[seat], side);
        let mut team = laning(side, SlotId(seat as u8), "");
        let moving = vec![IssuedOrder::hero(Order::Move {
            target: Target::Pos(goal),
        })];
        assert_eq!(orders(&mut team, &sessions[seat]), moving);
        for _ in 1..bota_core::rules::TICKS_PER_SECOND {
            advance(&mut local, &mut sessions, seat, &[]);
            assert!(
                orders(&mut team, &sessions[seat]).is_empty(),
                "the pending move retains its lease"
            );
        }
        advance(&mut local, &mut sessions, seat, &[]);
        assert_eq!(orders(&mut team, &sessions[seat]), moving);
        teleport(&mut local, &mut sessions, seat, goal);
        let stopped = vec![IssuedOrder::hero(Order::Move {
            target: Target::None,
        })];
        assert_eq!(orders(&mut team, &sessions[seat]), stopped);
        advance(&mut local, &mut sessions, seat, &stopped);
        assert!(
            orders(&mut team, &sessions[seat]).is_empty(),
            "the cleared controller does not keep interrupting idle attacks"
        );
    }
}

#[test]
fn pregame_laning_arrival_is_inclusive_at_one_hundred_units() {
    for (seat, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        for distance in [99, 100, 101] {
            let (mut local, mut sessions) =
                support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
            teleport(&mut local, &mut sessions, seat, Vec2::from_ints(8000, 8000));
            let goal = target(&sessions[seat], side);
            let mut team = laning(side, SlotId(seat as u8), "");
            let _ = orders(&mut team, &sessions[seat]);
            teleport(
                &mut local,
                &mut sessions,
                seat,
                goal + Vec2::from_ints(distance, 0),
            );
            let sent = orders(&mut team, &sessions[seat]);
            if distance <= 100 {
                assert_eq!(
                    sent,
                    vec![IssuedOrder::hero(Order::Move {
                        target: Target::None
                    })],
                    "side={side:?} distance={distance}"
                );
            } else {
                assert!(
                    sent.is_empty(),
                    "the out-of-arrival move keeps its unexpired lease"
                );
                for _ in 0..30 {
                    advance(&mut local, &mut sessions, seat, &[]);
                }
                assert_eq!(
                    orders(&mut team, &sessions[seat]),
                    vec![IssuedOrder::hero(Order::Move {
                        target: Target::Pos(goal)
                    })]
                );
            }
        }
    }
}

#[test]
fn an_idle_controller_at_the_lane_goal_does_not_issue_a_stop() {
    for (seat, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let (mut local, mut sessions) =
            support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
        let goal = target(&sessions[seat], side);
        teleport(&mut local, &mut sessions, seat, goal);
        assert!(orders(&mut laning(side, SlotId(seat as u8), ""), &sessions[seat]).is_empty());
    }
}

#[test]
fn startgame_leaves_the_pregame_slice_using_the_public_game_clock() {
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let mut team = laning(Team::Radiant, SlotId(0), "");
    assert!(matches!(
        orders(&mut team, &sessions[0])[0].order,
        Order::Move { .. }
    ));
    advance(
        &mut local,
        &mut sessions,
        0,
        &[IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::StartGame,
        })],
    );
    advance(
        &mut local,
        &mut sessions,
        0,
        &[IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::KillCreeps {
                which: bota_proto::CreepSide::All,
            },
        })],
    );
    assert!(
        !sessions[0]
            .current()
            .expect("view")
            .units
            .iter()
            .any(|unit| { unit.team == Team::Radiant && bota_core::kind_def(unit.kind).creep }),
        "the first wave must not mask a clock regression with the dynamic-front scope guard"
    );
    let view = sessions[0].current().expect("view");
    assert!(
        view.tick < sessions[0].map().pregame_ticks,
        "wall tick is still pregame-sized"
    );
    assert!(
        view.game_tick() >= sessions[0].map().pregame_ticks,
        "cheat advances the game clock"
    );
    let sent = orders(&mut team, &sessions[0]);
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
        "existing post-horn fallback: {sent:?}"
    );
    advance(&mut local, &mut sessions, 0, &sent);
}

#[test]
fn pregame_creeps_keep_the_unported_dynamic_front_path_on_the_existing_fallback() {
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    advance(
        &mut local,
        &mut sessions,
        0,
        &[IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::CreateCreeps {
                enemy: false,
                at: Vec2::from_ints(7000, 7000),
            },
        })],
    );
    assert!(
        sessions[0]
            .current()
            .expect("view")
            .units
            .iter()
            .any(|unit| { unit.team == Team::Radiant && bota_core::kind_def(unit.kind).creep }),
        "the server spawned the scope-boundary condition"
    );
    let sent = orders(&mut laning(Team::Radiant, SlotId(0), ""), &sessions[0]);
    let middle = bota_core::rules::MAP_SIZE / 2;
    assert_eq!(
        sent,
        vec![IssuedOrder::hero(Order::Attack {
            target: Target::Pos(Vec2::from_ints(middle, middle)),
        })],
        "the existing approximation is not relabelled as the recovered no-creep slice"
    );
    advance(&mut local, &mut sessions, 0, &sent);
}

#[test]
fn removed_creeps_in_client_history_do_not_disable_the_no_creep_slice() {
    for (seat, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let (mut local, mut sessions) =
            support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
        advance(
            &mut local,
            &mut sessions,
            seat,
            &[IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::CreateCreeps {
                    enemy: false,
                    at: Vec2::from_ints(7000, 7000),
                },
            })],
        );
        advance(
            &mut local,
            &mut sessions,
            seat,
            &[IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::KillCreeps {
                    which: bota_proto::CreepSide::All,
                },
            })],
        );
        assert!(
            !sessions[seat]
                .current()
                .expect("view")
                .units
                .iter()
                .any(|unit| { unit.team == side && bota_core::kind_def(unit.kind).creep }),
            "the current server view has no creeps"
        );
        assert!(
            sessions[seat].entities().iter().any(|track| {
                track.unit.team == side
                    && track.unit.hp > 0
                    && bota_core::kind_def(track.unit.kind).creep
            }),
            "the fixture retains the last observed creep in client history"
        );
        let sent = orders(&mut laning(side, SlotId(seat as u8), ""), &sessions[seat]);
        assert_eq!(
            sent,
            vec![IssuedOrder::hero(Order::Move {
                target: Target::Pos(target(&sessions[seat], side)),
            })]
        );
        advance(&mut local, &mut sessions, seat, &sent);
    }
}

#[test]
fn an_invoked_laning_think_consumes_native_actions_even_when_returning_false() {
    for think in ["function Think() end", "function Think() return false end"] {
        let (mut local, mut sessions) =
            support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
        advance(
            &mut local,
            &mut sessions,
            0,
            &[IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Teleport {
                    at: Vec2::from_ints(8000, 8000),
                },
            })],
        );
        assert_eq!(
            orders(&mut laning(Team::Radiant, SlotId(0), ""), &sessions[0]),
            vec![IssuedOrder::hero(Order::Move {
                target: Target::Pos(target(&sessions[0], Team::Radiant)),
            })],
            "the identical missing-hook case must reach the new movement slice"
        );
        let turn = laning(Team::Radiant, SlotId(0), think)
            .think(&sessions[0])
            .expect("override");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert!(
            turn.bots[0].orders.is_empty(),
            "existing Think consumes the native pass"
        );
    }
}

#[test]
fn laning_move_orders_are_lease_bounded_over_six_hundred_ticks() {
    for (seat, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let (mut local, mut sessions) =
            support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
        teleport(&mut local, &mut sessions, seat, Vec2::from_ints(8000, 8000));
        let mut team = laning(side, SlotId(seat as u8), "");
        let goal = target(&sessions[seat], side);
        let mut moves = Vec::new();
        let mut stops = 0;
        for frame in 0..600 {
            let sent = orders(&mut team, &sessions[seat]);
            for command in &sent {
                match command.order {
                    Order::Move {
                        target: Target::Pos(at),
                    } => {
                        assert_eq!(at, goal);
                        moves.push(frame);
                    }
                    Order::Move {
                        target: Target::None,
                    } => stops += 1,
                    other => panic!("unexpected laning command: {other:?}"),
                }
            }
            advance(&mut local, &mut sessions, seat, &sent);
        }
        assert_eq!(moves.first(), Some(&0));
        assert!(
            moves.len() <= 20,
            "{side:?}: {} movement orders",
            moves.len()
        );
        assert!(
            moves.windows(2).all(|ticks| ticks[1] - ticks[0] >= 30),
            "{moves:?}"
        );
        assert_eq!(stops, 1, "{side:?} arrives and stops once");
        eprintln!(
            "laning_cadence side={side:?} ticks=600 moves={} stops={stops}",
            moves.len()
        );
    }
}

#[test]
fn a_hero_already_past_the_goal_moves_back_to_it() {
    for (seat, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let (mut local, mut sessions) =
            support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
        let goal = target(&sessions[seat], side);
        let towers = if side == Team::Radiant {
            sessions[seat].map().radiant_towers
        } else {
            sessions[seat].map().dire_towers
        };
        let front = towers
            .iter()
            .find(|&&(lane, tier, _)| lane == bota_core::rules::LANE_MID && tier == 1)
            .expect("front")
            .2;
        let ahead = bota_core::point_along(goal, front, bota_proto::Fixed::from_int(110));
        teleport(&mut local, &mut sessions, seat, ahead);
        let sent = orders(&mut laning(side, SlotId(seat as u8), ""), &sessions[seat]);
        assert_eq!(
            sent,
            vec![IssuedOrder::hero(Order::Move {
                target: Target::Pos(goal)
            })]
        );
        advance(&mut local, &mut sessions, seat, &sent);
    }
}
