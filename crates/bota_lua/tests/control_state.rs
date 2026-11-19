mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{EffectId, HeroId, SlotId, Team};
#[test]
fn control_queries_read_the_visible_rule_facts() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .unwrap();
    lua.add_script(SlotId(0),ScriptKind::Takeover,"controls",b"function Think() local bot=GetBot(); assert(not bot:IsSilenced()); assert(not bot:IsRooted()); assert(not bot:IsDisarmed()) end").unwrap();
    let turn = lua.think(&sessions[0]).unwrap();
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}
#[test]
fn projected_control_bits_equal_the_authoritative_predicates_instead_of_effect_icons() {
    use bota_core::{Control, ModifierKind};
    use bota_server::game::Modifier;
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let hero = local.hero_of(0).unwrap();
    for (control, expected) in [
        (Control::Silence, 1u16 << 13),
        (Control::Root, 1u16 << 14),
        (Control::Disarm, 1u16 << 15),
    ] {
        local.world_mut().put_modifier(
            hero,
            Modifier::new(
                ModifierKind::Control {
                    control,
                    effect: EffectId::STUNNED,
                },
                None,
                None,
                Some(3),
            ),
        );
        for _ in 0..4 {
            for (session, stream) in sessions.iter_mut().zip(local.step(&[&[], &[]]).unwrap()) {
                support::fold(session, stream);
            }
            let world = local.world();
            let sheet = sessions[0].own_hero().unwrap();
            let predicates = [
                world.silenced(hero),
                world.rooted(hero),
                world.disarmed(hero),
            ];
            for (index, on) in predicates.into_iter().enumerate() {
                assert_eq!(sheet.statuses.bits & (1 << (13 + index)) != 0, on);
            }
        }
        assert_eq!(
            sessions[0].own_hero().unwrap().statuses.bits & expected,
            0,
            "expired control is cleared"
        );
    }
}
fn step(
    local: &mut bota_server::local::LocalMatch,
    sessions: &mut [bota_client_core::ClientSession],
    commands: &[Vec<bota_proto::OutgoingOrder>; 2],
) -> Vec<Vec<bota_proto::ServerMsg>> {
    let streams = local.step(&[&commands[0], &commands[1]]).unwrap();
    for (session, stream) in sessions.iter_mut().zip(&streams) {
        support::fold(session, stream.clone());
    }
    streams
}
#[test]
fn bounded_control_cheats_project_all_three_states_and_expire_through_real_orders() {
    use bota_client_core::IssuedOrder;
    use bota_proto::{Cheat, ControlKind, Order, Target};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    for (control, bit) in [
        (ControlKind::Silence, 13),
        (ControlKind::Root, 14),
        (ControlKind::Disarm, 15),
    ] {
        let order = sessions[0]
            .issue(IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Control {
                    target: Target::None,
                    control,
                    ticks: 3,
                },
            }))
            .unwrap();
        let streams = step(&mut local, &mut sessions, &[vec![order], vec![]]);
        assert!(
            !streams[0]
                .iter()
                .any(|message| matches!(message, bota_proto::ServerMsg::OrderRejected { .. })),
            "control order accepted"
        );
        assert_ne!(
            sessions[0].own_hero().unwrap().statuses.bits & (1 << bit),
            0
        );
        for _ in 0..3 {
            step(&mut local, &mut sessions, &[vec![], vec![]]);
        }
        assert_eq!(
            sessions[0].own_hero().unwrap().statuses.bits & (1 << bit),
            0
        );
    }
}
#[test]
fn control_cheats_reject_zero_or_unbounded_ticks_and_rated_matches() {
    use bota_client_core::IssuedOrder;
    use bota_proto::{Cheat, ControlKind, Order, RejectReason, Target};
    for (enabled, ticks, reason) in [
        (true, 0, RejectReason::BadCheat),
        (
            true,
            bota_proto::MAX_MODIFIER_TICKS + 1,
            RejectReason::BadCheat,
        ),
        (false, 3, RejectReason::NoCheats),
    ] {
        let (mut local, mut sessions) = if enabled {
            support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID)
        } else {
            support::local(&[Team::Radiant, Team::Dire])
        };
        let order = sessions[0]
            .issue(IssuedOrder::hero(Order::Cheat {
                cheat: Cheat::Control {
                    target: Target::None,
                    control: ControlKind::Silence,
                    ticks,
                },
            }))
            .unwrap();
        let streams = step(&mut local, &mut sessions, &[vec![order], vec![]]);
        assert!(streams[0].iter().any(|message|matches!(message,bota_proto::ServerMsg::OrderRejected {reason:got,..} if *got==reason)),"{streams:?}");
        assert_eq!(
            sessions[0].own_hero().unwrap().statuses.bits & bota_proto::StatusFlags::SILENCED,
            0
        );
    }
}
#[test]
fn a_real_basic_dispel_clears_the_projected_silence_state() {
    use bota_client_core::IssuedOrder;
    use bota_proto::{Cheat, ControlKind, ItemSlot, Order, Target};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let item = bota_core::item_by_key("item_guardian_greaves").unwrap();
    let commands = [
        Cheat::Item { item },
        Cheat::Control {
            target: Target::None,
            control: ControlKind::Silence,
            ticks: 300,
        },
    ]
    .into_iter()
    .map(|cheat| {
        sessions[0]
            .issue(IssuedOrder::hero(Order::Cheat { cheat }))
            .unwrap()
    })
    .collect();
    step(&mut local, &mut sessions, &[commands, vec![]]);
    assert!(bota_client_core::unit_state::is_silenced(
        sessions[0].own_hero().unwrap()
    ));
    let use_item = sessions[0]
        .issue(IssuedOrder::hero(Order::Use {
            slot: ItemSlot(0),
            target: Target::None,
        }))
        .unwrap();
    let streams = step(&mut local, &mut sessions, &[vec![use_item], vec![]]);
    assert!(
        !streams[0]
            .iter()
            .any(|message| matches!(message, bota_proto::ServerMsg::OrderRejected { .. })),
        "dispel item accepted"
    );
    assert!(!bota_client_core::unit_state::is_silenced(
        sessions[0].own_hero().unwrap()
    ));
}
fn fog_reader(side: Team, slot: SlotId) -> LuaTeam {
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
    lua.add_script(slot,ScriptKind::Takeover,"fogged_controls",b"local enemy function Think() if not enemy then enemy=GetUnitList(UNIT_LIST_ENEMY_HEROES)[1]; assert(enemy) else assert(not enemy:IsSilenced()); assert(not enemy:IsRooted()); assert(not enemy:IsDisarmed()); assert(enemy:GetModifierStackCount(0)==-1); GetBot():Action_MoveToLocation(Vector(5,6,0)) end end").unwrap();
    lua
}
#[test]
fn hidden_control_changes_never_change_the_other_seats_script_results() {
    use bota_client_core::IssuedOrder;
    use bota_proto::{Cheat, ControlKind, Fixed, Order, Target, Vec2};
    for observer in 0..2 {
        let enemy = 1 - observer;
        let (mut local, mut sessions) =
            support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
        let at = bota_core::move_towards(
            sessions[enemy].map().fountains[enemy],
            Vec2::from_ints(9216, 9216),
            Fixed::from_int(2000),
        );
        let mut commands = [vec![], vec![]];
        for index in 0..2 {
            commands[index].push(
                sessions[index]
                    .issue(IssuedOrder::hero(Order::Cheat {
                        cheat: Cheat::Teleport { at },
                    }))
                    .unwrap(),
            );
        }
        step(&mut local, &mut sessions, &commands);
        let enemy_id = sessions[enemy].own_hero().unwrap().id;
        let mut a = local.clone();
        let mut b = local;
        let mut sa = sessions.clone();
        let mut sb = sessions;
        let mut la = fog_reader(sa[observer].team(), SlotId(observer as u8));
        let mut lb = fog_reader(sb[observer].team(), SlotId(observer as u8));
        assert!(la.think(&sa[observer]).unwrap().failures.is_empty());
        assert!(lb.think(&sb[observer]).unwrap().failures.is_empty());
        let mut ca = [vec![], vec![]];
        let mut cb = [vec![], vec![]];
        for (session, commands) in [
            (&mut sa[observer], &mut ca[observer]),
            (&mut sb[observer], &mut cb[observer]),
        ] {
            let at = session.map().fountains[observer];
            commands.push(
                session
                    .issue(IssuedOrder::hero(Order::Cheat {
                        cheat: Cheat::Teleport { at },
                    }))
                    .unwrap(),
            );
        }
        step(&mut a, &mut sa, &ca);
        step(&mut b, &mut sb, &cb);
        assert!(sa[observer].current().unwrap().unit(enemy_id).is_none());
        assert!(sb[observer].current().unwrap().unit(enemy_id).is_none());
        let mut hidden = [vec![], vec![]];
        for control in [ControlKind::Silence, ControlKind::Root, ControlKind::Disarm] {
            hidden[enemy].push(
                sb[enemy]
                    .issue(IssuedOrder::hero(Order::Cheat {
                        cheat: Cheat::Control {
                            target: Target::None,
                            control,
                            ticks: 300,
                        },
                    }))
                    .unwrap(),
            );
        }
        step(&mut a, &mut sa, &[vec![], vec![]]);
        step(&mut b, &mut sb, &hidden);
        assert!(bota_client_core::unit_state::is_silenced(
            sb[enemy].own_hero().unwrap()
        ));
        assert!(!bota_client_core::unit_state::is_silenced(
            sa[enemy].own_hero().unwrap()
        ));
        let aa = la.think(&sa[observer]).unwrap();
        let bb = lb.think(&sb[observer]).unwrap();
        assert!(aa.failures.is_empty(), "{:?}", aa.failures);
        assert!(bb.failures.is_empty(), "{:?}", bb.failures);
        assert_eq!(aa.bots[0].orders, bb.bots[0].orders);
    }
}

#[test]
fn ethereal_item_use_sets_the_cannot_attack_fact_for_every_client() {
    use bota_client_core::IssuedOrder;
    use bota_proto::{Cheat, ItemId, ItemSlot, Order, StatusFlags, Target};
    let (mut local, mut sessions) =
        support::local_with_cheats(&[Team::Radiant, Team::Dire], bota_core::MAP2_ID);
    let give = sessions[0]
        .issue(IssuedOrder::hero(Order::Cheat {
            cheat: Cheat::Item {
                item: ItemId::GHOST_SCEPTER,
            },
        }))
        .unwrap();
    step(&mut local, &mut sessions, &[vec![give], vec![]]);
    let cast = sessions[0]
        .issue(IssuedOrder::hero(Order::Use {
            slot: ItemSlot(0),
            target: Target::None,
        }))
        .unwrap();
    step(&mut local, &mut sessions, &[vec![cast], vec![]]);
    let hero = local.hero_of(0).unwrap();
    assert!(local.world().attack_immune(hero));
    assert!(local.world().disarmed(hero));
    let received = sessions[0].own_hero().unwrap();
    assert_ne!(received.statuses.bits & StatusFlags::DISARMED, 0);
    assert!(bota_client_core::unit_state::is_disarmed(received));
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .unwrap();
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "ethereal_state",
        b"function Think() assert(GetBot():IsDisarmed()) end",
    )
    .unwrap();
    assert!(lua.think(&sessions[0]).unwrap().failures.is_empty());
}
