mod support;
use bota_client_core::{ClientSession, IssuedOrder};
use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{
    AbilityId, AbilitySlot, Cheat, HeroId, Order, OutgoingOrder, ServerMsg, SlotId, Target, Team,
    Vec2,
};

fn step(
    local: &mut bota_server::local::LocalMatch,
    sessions: &mut [ClientSession],
    commands: &[&[OutgoingOrder]],
) {
    for (session, stream) in sessions.iter_mut().zip(local.step(commands).expect("step")) {
        assert!(
            !stream
                .iter()
                .any(|message| matches!(message, ServerMsg::OrderRejected { .. })),
            "ordinary order accepted"
        );
        support::fold(session, stream);
    }
}
fn reader(side: Team, slot: SlotId, caster: usize) -> LuaTeam {
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
    let key = bota_core::ability_def(AbilityId::RAZE_NEAR)
        .expect("raze")
        .key;
    let source = r#"
        local phase,caster,active=0,nil,nil
        function Think()
            phase=phase+1
            local bot=GetBot()
            assert(not pcall(bot.GetCurrentActiveAbility,{}))
            if phase==1 then
                for _,u in ipairs(GetUnitList(UNIT_LIST_ALL)) do
                    if u:IsHero() and u:GetPlayerID()==__CASTER__ then caster=u end
                end
                assert(caster and caster:GetCurrentActiveAbility()==nil)
            elseif phase==2 then
                active=caster:GetCurrentActiveAbility()
                assert(active and active:GetName()=='__KEY__')
                assert(active==caster:GetAbilityByName('__KEY__'))
                assert(not pcall(bot.GetCurrentActiveAbility,active))
            elseif phase==3 then
                if caster==bot then assert(caster:GetCurrentActiveAbility()==nil)
                else assert(caster:GetCurrentActiveAbility()==nil) end
            else
                if caster==bot then assert(caster:GetCurrentActiveAbility()==nil)
                else
                    assert(not caster:IsNull(), 'public roster identity survives body-track expiry')
                    assert(caster:GetCurrentActiveAbility()==nil)
                end
            end
            if phase>=3 then
                local p=bot:GetLocation()
                bot:Action_MoveToLocation(Vector(p.x+phase,p.y,0))
            end
        end
    "#
    .replace("__CASTER__", if caster == 0 { "0" } else { "5" })
    .replace("__KEY__", key);
    lua.add_script(
        slot,
        ScriptKind::Takeover,
        "active_ability_queries",
        source.as_bytes(),
    )
    .expect("script");
    lua
}
fn think(
    readers: &mut [LuaTeam],
    sessions: &[ClientSession],
    expect_move: bool,
) -> Vec<Vec<IssuedOrder>> {
    readers
        .iter_mut()
        .zip(sessions)
        .map(|(lua, session)| {
            let turn = lua.think(session).expect("think");
            assert!(turn.failures.is_empty(), "{:?}", turn.failures);
            assert_eq!(turn.bots[0].orders.len(), usize::from(expect_move));
            turn.bots[0].orders.clone()
        })
        .collect()
}

#[test]
fn current_active_ability_follows_real_casts_and_returns_nil_when_the_enemy_leaves_sight() {
    for caster in [0, 1] {
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        {
            let world = local.world_mut();
            world.cheats = true;
            world.cheat(
                SlotId(0),
                None,
                Cheat::AllVision { on: true },
                &mut Vec::new(),
            );
        }
        let learn = sessions[caster]
            .issue(IssuedOrder::hero(Order::Learn {
                slot: AbilitySlot(0),
            }))
            .expect("learn");
        let mut commands = [Vec::new(), Vec::new()];
        commands[caster].push(learn);
        step(&mut local, &mut sessions, &[&commands[0], &commands[1]]);
        assert_eq!(
            sessions[caster].own_hero().expect("hero").abilities[0].id,
            AbilityId::RAZE_NEAR
        );
        assert_eq!(
            sessions[caster].own_hero().expect("hero").abilities[0].level,
            1
        );
        let mut readers = [
            reader(Team::Radiant, SlotId(0), caster),
            reader(Team::Dire, SlotId(1), caster),
        ];
        think(&mut readers, &sessions, false);
        let cast = sessions[caster]
            .issue(IssuedOrder::hero(Order::Cast {
                slot: AbilitySlot(0),
                target: Target::None,
            }))
            .expect("cast");
        let mut commands = [Vec::new(), Vec::new()];
        commands[caster].push(cast);
        step(&mut local, &mut sessions, &[&commands[0], &commands[1]]);
        assert_eq!(
            sessions[caster]
                .own_hero()
                .expect("casting hero")
                .cast
                .expect("windup")
                .ability,
            Some(AbilityId::RAZE_NEAR)
        );
        think(&mut readers, &sessions, false);
        {
            local.world_mut().cheat(
                SlotId(0),
                None,
                Cheat::AllVision { on: false },
                &mut Vec::new(),
            );
        }
        let stop = sessions[caster]
            .issue(IssuedOrder::hero(Order::Move {
                target: Target::None,
            }))
            .expect("cancel cast");
        let mut commands = [Vec::new(), Vec::new()];
        commands[caster].push(stop);
        step(&mut local, &mut sessions, &[&commands[0], &commands[1]]);
        assert!(sessions[caster].own_hero().expect("hero").cast.is_none());
        let id = sessions[caster].own_hero().expect("hero").id;
        assert!(
            !sessions[1 - caster]
                .current()
                .expect("view")
                .units
                .iter()
                .any(|unit| unit.id == id)
        );
        assert!(
            sessions[1 - caster]
                .known(id)
                .expect("remembered cast")
                .0
                .cast
                .is_some()
        );
        let orders = think(&mut readers, &sessions, true);
        for (index, batch) in orders.iter().enumerate() {
            let p = sessions[index].own_hero().expect("hero").pos;
            assert_eq!(
                batch,
                &[IssuedOrder::hero(Order::Move {
                    target: Target::Pos(Vec2 {
                        x: p.x + bota_proto::Fixed::from_int(3),
                        y: p.y
                    })
                })]
            );
        }
        let outgoing: Vec<Vec<_>> = orders
            .iter()
            .zip(&mut sessions)
            .map(|(batch, session)| {
                batch
                    .iter()
                    .map(|&order| session.issue(order).expect("query move"))
                    .collect()
            })
            .collect();
        step(&mut local, &mut sessions, &[&outgoing[0], &outgoing[1]]);
        for _ in 0..481 {
            step(&mut local, &mut sessions, &[&[], &[]]);
        }
        assert!(
            !sessions[1 - caster]
                .entities()
                .iter()
                .any(|track| track.id == id)
        );
        let orders = think(&mut readers, &sessions, true);
        for (index, batch) in orders.iter().enumerate() {
            let p = sessions[index].own_hero().expect("hero").pos;
            assert_eq!(
                batch,
                &[IssuedOrder::hero(Order::Move {
                    target: Target::Pos(Vec2 {
                        x: p.x + bota_proto::Fixed::from_int(4),
                        y: p.y
                    })
                })]
            );
        }
    }
}

#[test]
fn channel_identity_unobserved_items_and_owned_death_have_distinct_results() {
    for (index, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let heroes = if index == 0 {
            [
                (Team::Radiant, HeroId::PUDGE),
                (Team::Dire, HeroId::SHADOW_FIEND),
            ]
        } else {
            [
                (Team::Radiant, HeroId::SHADOW_FIEND),
                (Team::Dire, HeroId::PUDGE),
            ]
        };
        let (_, mut sessions) = support::local_with_modifiers(&heroes, Vec::new());
        let own = sessions[index].own_hero().expect("hero").id;
        let mut view = sessions[index].current().expect("view").clone();
        let slot = SlotId(index as u8);
        let mut lua = LuaTeam::new(
            side,
            &[BotSeat {
                slot,
                hero: HeroId::PUDGE,
            }],
            Limits::DEFAULT,
            Keys {
                hash: [17, 19],
                random: 23,
            },
        )
        .expect("team");
        lua.set_gc_stress(GcStress::Full);
        let key = bota_core::ability_def(AbilityId::DISMEMBER)
            .expect("dismember")
            .key;
        let source=r#"
            local phase=0
            function Think()
                phase=phase+1
                local bot=GetBot()
                if phase==1 then
                    local active=bot:GetCurrentActiveAbility()
                    assert(active and active:GetName()=='__KEY__')
                    assert(active==bot:GetAbilityByName('__KEY__'))
                elseif phase==2 then
                    local ok,reason=pcall(bot.GetCurrentActiveAbility,bot)
                    assert(not ok and string.find(tostring(reason),'active ability is not observed',1,true))
                else assert(bot:GetCurrentActiveAbility()==nil) end
                bot:Action_MoveToLocation(Vector(phase,200,0))
            end
        "#.replace("__KEY__",key);
        lua.add_script(
            slot,
            ScriptKind::Takeover,
            "active_ability_observation",
            source.as_bytes(),
        )
        .expect("script");
        for phase in 1..=3 {
            view.tick += 1;
            if phase < 3 {
                let unit = view
                    .units
                    .iter_mut()
                    .find(|unit| unit.id == own)
                    .expect("hero");
                unit.abilities
                    .iter_mut()
                    .find(|entry| entry.id == AbilityId::DISMEMBER)
                    .expect("dismember slot")
                    .level = 1;
                unit.statuses.bits &=
                    !(bota_proto::StatusFlags::CHANNELLING | bota_proto::StatusFlags::CASTING);
                unit.statuses.bits |= if phase == 1 {
                    bota_proto::StatusFlags::CHANNELLING
                } else {
                    bota_proto::StatusFlags::CASTING
                };
                unit.channel = if phase == 1 {
                    Some(bota_proto::ChannelView {
                        ability: AbilityId::DISMEMBER,
                        elapsed: 1,
                    })
                } else {
                    None
                };
                unit.cast = if phase == 2 {
                    Some(bota_proto::CastView {
                        ability: None,
                        goes_off_in: 2,
                    })
                } else {
                    None
                };
            } else {
                let unit = view.units.iter().find(|unit| unit.id == own).expect("hero");
                let kit = bota_proto::Kit {
                    abilities: unit.abilities.clone(),
                    items: unit.items.clone(),
                    town_portal: unit.town_portal,
                    sellable_slots: 0,
                    reincarnating: false,
                };
                view.units.retain(|unit| unit.id != own);
                view.aggro.clear();
                let player = view
                    .players
                    .iter_mut()
                    .find(|player| player.slot == slot)
                    .expect("player");
                player.respawn_left = 100;
                player.deaths = 1;
                player.kit = Some(kit);
            }
            sessions[index]
                .ingest(ServerMsg::Snapshot { view: view.clone() }, &mut ())
                .expect("received observation");
            sessions[index]
                .ingest(
                    ServerMsg::Events {
                        tick: view.tick,
                        events: Vec::new(),
                    },
                    &mut (),
                )
                .expect("complete observation");
            if phase == 3 {
                assert!(
                    sessions[index]
                        .known(own)
                        .expect("remembered body")
                        .0
                        .cast
                        .is_some()
                );
            }
            let turn = lua.think(&sessions[index]).expect("think");
            assert!(turn.failures.is_empty(), "{:?}", turn.failures);
            assert_eq!(
                turn.bots[0].orders,
                vec![IssuedOrder::hero(Order::Move {
                    target: Target::Pos(Vec2::from_ints(9216 + phase, 9416))
                })]
            );
        }
    }
}

#[test]
fn public_foreign_death_overrides_a_cast_even_when_a_body_row_remains_visible() {
    use bota_proto::ModifierSpec;
    for caster in [0, 1] {
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        {
            let world = local.world_mut();
            world.cheats = true;
            world
                .check_cheat(SlotId(0), None, &Cheat::StartGame)
                .expect("start cheat");
            world.cheat(SlotId(0), None, Cheat::StartGame, &mut Vec::new());
            world.cheat(
                SlotId(0),
                None,
                Cheat::AllVision { on: true },
                &mut Vec::new(),
            );
            for (index, at) in [
                (caster, Vec2::from_ints(9000, 9000)),
                (1 - caster, Vec2::from_ints(9100, 9000)),
            ] {
                let slot = SlotId(index as u8);
                let unit = world.seats[index].unit.expect("hero");
                let scale = if index == caster {
                    ModifierSpec {
                        max_hp: ModifierSpec::MIN_SCALE,
                        ..ModifierSpec::NOMINAL
                    }
                } else {
                    ModifierSpec {
                        physical_damage: ModifierSpec::MAX_SCALE,
                        ..ModifierSpec::NOMINAL
                    }
                };
                for cheat in [
                    Cheat::Levels {
                        count: if index == caster { 5 } else { 24 },
                    },
                    Cheat::Teleport { at },
                    Cheat::ApplyModifier {
                        target: Target::None,
                        spec: scale,
                        ticks: 4000,
                    },
                ] {
                    world
                        .check_cheat(slot, Some(unit), &cheat)
                        .expect("valid setup cheat");
                    world.cheat(slot, Some(unit), cheat, &mut Vec::new());
                }
            }
        }
        step(&mut local, &mut sessions, &[&[], &[]]);
        let stops: Vec<_> = sessions
            .iter_mut()
            .map(|session| {
                session
                    .issue(IssuedOrder::hero(Order::Move {
                        target: Target::None,
                    }))
                    .expect("stand still")
            })
            .collect();
        step(&mut local, &mut sessions, &[&[stops[0]], &[stops[1]]]);
        // Start the fight outside fountain protection; there is no aura-clearing cheat.
        for _ in 0..120 {
            if sessions.iter().all(|session| {
                session.own_hero().expect("hero").statuses.bits
                    & bota_proto::StatusFlags::INVULNERABLE
                    == 0
            }) {
                break;
            }
            step(&mut local, &mut sessions, &[&[], &[]]);
        }
        assert!(
            sessions
                .iter()
                .all(|session| session.own_hero().expect("hero").statuses.bits
                    & bota_proto::StatusFlags::INVULNERABLE
                    == 0)
        );
        let victim = sessions[caster].own_hero().expect("caster").id;
        let ultimate = bota_core::hero_def(HeroId::SHADOW_FIEND)
            .expect("hero")
            .abilities
            .last()
            .copied()
            .expect("ultimate slot");
        let ability_slot = AbilitySlot(5);
        assert_eq!(
            sessions[caster].own_hero().expect("caster").abilities[5].id,
            ultimate
        );
        let learn = sessions[caster]
            .issue(IssuedOrder::hero(Order::Learn { slot: ability_slot }))
            .expect("learn");
        let mut commands = [Vec::new(), Vec::new()];
        commands[caster].push(learn);
        step(&mut local, &mut sessions, &[&commands[0], &commands[1]]);
        let observer = 1 - caster;
        let before_hp = sessions[caster].own_hero().expect("caster").hp;
        let attack = sessions[observer]
            .issue(IssuedOrder::hero(Order::Attack {
                target: Target::Unit(victim),
            }))
            .expect("attack");
        let mut commands = [Vec::new(), Vec::new()];
        commands[observer].push(attack);
        step(&mut local, &mut sessions, &[&commands[0], &commands[1]]);
        for _ in 0..90 {
            if sessions[caster]
                .own_hero()
                .expect("survives first attack")
                .hp
                < before_hp
            {
                break;
            }
            step(&mut local, &mut sessions, &[&[], &[]]);
        }
        assert!(sessions[caster].own_hero().expect("wounded caster").hp < before_hp);
        let cast = sessions[caster]
            .issue(IssuedOrder::hero(Order::Cast {
                slot: ability_slot,
                target: Target::None,
            }))
            .expect("cast");
        let mut commands = [Vec::new(), Vec::new()];
        commands[caster].push(cast);
        step(&mut local, &mut sessions, &[&commands[0], &commands[1]]);
        assert_eq!(
            sessions[caster]
                .own_hero()
                .expect("caster")
                .cast
                .expect("cast underway")
                .ability,
            Some(ultimate)
        );
        let observer = 1 - caster;
        let side = if observer == 0 {
            Team::Radiant
        } else {
            Team::Dire
        };
        let slot = SlotId(observer as u8);
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
        let source=r#"
            local phase,enemy=0,nil
            function Think()
                phase=phase+1
                if phase==1 then
                    enemy=GetUnitList(UNIT_LIST_ENEMY_HEROES)[1]
                    assert(enemy and enemy:GetCurrentActiveAbility():GetName()=='__KEY__')
                else
                    assert(enemy:GetCurrentActiveAbility()==nil,'public death must override a retained cast')
                    GetBot():Action_MoveToLocation(Vector(phase,100,0))
                end
            end
        "#.replace("__KEY__",bota_core::ability_def(ultimate).expect("ability").key);
        lua.add_script(
            slot,
            ScriptKind::Takeover,
            "foreign_death_during_cast",
            source.as_bytes(),
        )
        .expect("script");
        let turn = lua.think(&sessions[observer]).expect("observed cast");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        for _ in 0..90 {
            if sessions[caster].own_player().expect("player").respawn_left > 0 {
                break;
            }
            assert!(
                sessions[caster]
                    .own_hero()
                    .expect("alive caster")
                    .cast
                    .is_some(),
                "death must occur before the cast finishes: tick={} hp={} attacker={:?} aggro={:?}",
                sessions[caster].current().expect("view").tick,
                sessions[caster].own_hero().expect("caster").hp,
                sessions[observer].own_hero().expect("attacker").statuses,
                sessions[observer].current().expect("view").aggro
            );
            step(&mut local, &mut sessions, &[&[], &[]]);
        }
        assert!(
            sessions[caster]
                .own_player()
                .expect("dead player")
                .respawn_left
                > 0
        );
        let remembered = sessions[observer]
            .known(victim)
            .expect("last visible body")
            .0
            .clone();
        assert_eq!(
            remembered.cast.expect("retained interrupted cast").ability,
            Some(ultimate)
        );
        assert!(
            sessions[observer]
                .current()
                .expect("view")
                .players
                .iter()
                .any(|player| player.slot == SlotId(caster as u8) && player.respawn_left > 0)
        );
        let turn = lua.think(&sessions[observer]).expect("public death");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert_eq!(
            turn.bots[0].orders,
            vec![IssuedOrder::hero(Order::Move {
                target: Target::Pos(Vec2::from_ints(9218, 9316))
            })]
        );
        let movement = sessions[observer]
            .issue(turn.bots[0].orders[0])
            .expect("query move");
        let mut commands = [Vec::new(), Vec::new()];
        commands[observer].push(movement);
        step(&mut local, &mut sessions, &[&commands[0], &commands[1]]);
        // Public death wins even if an accepted snapshot also carries a stale body row.
        let mut view = sessions[observer].current().expect("view").clone();
        view.tick += 1;
        view.units.retain(|unit| unit.id != victim);
        view.units.push(remembered);
        view.units.sort_by_key(|unit| unit.id);
        let tick = view.tick;
        sessions[observer]
            .ingest(ServerMsg::Snapshot { view }, &mut ())
            .expect("accepted stale body with public death");
        sessions[observer]
            .ingest(
                ServerMsg::Events {
                    tick,
                    events: Vec::new(),
                },
                &mut (),
            )
            .expect("complete frame");
        let turn = lua
            .think(&sessions[observer])
            .expect("public death with visible row");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
        assert_eq!(
            turn.bots[0].orders,
            vec![IssuedOrder::hero(Order::Move {
                target: Target::Pos(Vec2::from_ints(9219, 9316))
            })]
        );
    }
}
