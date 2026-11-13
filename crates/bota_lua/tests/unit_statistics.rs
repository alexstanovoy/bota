mod support;

use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{Attribute, Fixed, HeroId, Order, ServerMsg, SlotId, Target, Team, Vec2};

fn assert_script_ran(turn: &bota_lua::TeamTurn) {
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 1);
    assert_eq!(
        turn.bots[0].orders[0].order,
        Order::Move {
            target: Target::Pos(Vec2::from_ints(9339, 9672))
        }
    );
}

#[test]
fn unit_statistics_preserve_received_whole_points_fractions_and_time_units() {
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for (index, side, brads, degrees) in [
        (0, Team::Radiant, 16584, "91"),
        (1, Team::Dire, 65535, "359"),
        (0, Team::Radiant, 0, "0"),
    ] {
        let slot = SlotId(index as u8);
        let mut view = sessions[index].current().expect("view").clone();
        view.tick += 1;
        let unit = view
            .units
            .iter_mut()
            .find(|unit| unit.owner == Some(slot) && unit.hero.is_some())
            .expect("hero");
        unit.mana = 2_000_000;
        unit.max_mana = i32::MAX;
        unit.hp_regen = Fixed::from_ratio(5, 4);
        unit.mana_regen = Fixed::from_ratio(9, 4);
        unit.armor = Fixed::from_ratio(-5, 2);
        unit.attack_damage = i32::MAX;
        unit.attack_range = Fixed::from_ratio(1203, 2);
        unit.attack_time = 1250;
        unit.move_speed = Fixed::from_ratio(701, 2);
        unit.bound = Fixed::from_ratio(145, 4);
        unit.evasion = Fixed::from_ratio(1, 4);
        unit.facing = bota_proto::Angle { brads };
        unit.attributes.strength = Fixed::from_ratio(39, 2);
        unit.attributes.agility = Fixed::from_ratio(93, 4);
        unit.attributes.intelligence = Fixed::from_ratio(75, 4);
        unit.primary = Some(Attribute::Agility);
        let tick = view.tick;
        sessions[index]
            .ingest(ServerMsg::Snapshot { view }, &mut ())
            .expect("received statistics");
        sessions[index]
            .ingest(
                ServerMsg::Events {
                    tick,
                    events: Vec::new(),
                },
                &mut (),
            )
            .expect("events");
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
        lua.add_script(slot, ScriptKind::Takeover, "received_statistics", r#"
            function Think()
                local unit = GetBot()
                assert(unit:GetMana() == 2000000 and unit:GetMaxMana() == 2147483647)
                assert(unit:GetHealthRegen() == 1.25 and unit:GetManaRegen() == 2.25)
                assert(unit:GetArmor() == -2.5 and unit:GetAttackDamage() == 2147483647)
                assert(unit:GetAttackRange() == 601)
                assert(unit:GetSecondsPerAttack() == 1.25)
                assert(unit:GetCurrentMovementSpeed() == 350 and unit:GetBoundingRadius() == 36.25)
                assert(unit:GetEvasion() == 0.25 and unit:GetFacing() == __DEGREES__)
                assert(unit:GetPrimaryAttribute() == ATTRIBUTE_AGILITY)
                assert(ATTRIBUTE_STRENGTH == 0 and ATTRIBUTE_AGILITY == 1 and ATTRIBUTE_INTELLECT == 2)
                assert(unit:GetAttributeValue(ATTRIBUTE_STRENGTH) == 19)
                assert(unit:GetAttributeValue(ATTRIBUTE_AGILITY) == 23)
                assert(unit:GetAttributeValue(ATTRIBUTE_INTELLECT) == 18)
                unit:Action_MoveToLocation(Vector(123,456,0))
            end
        "#.replace("__DEGREES__", degrees).as_bytes()).expect("script");
        let turn = lua.think(&sessions[index]).expect("think");
        assert_script_ran(&turn);
    }
}

#[test]
fn unit_statistics_reject_forged_receivers_and_invalid_attribute_indices() {
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
    .expect("team");
    lua.add_script(SlotId(0), ScriptKind::Takeover, "statistic_boundaries", br#"
        function Think()
            local unit = GetBot()
            for _, name in ipairs({'GetMana','GetMaxMana','GetHealthRegen','GetManaRegen','GetArmor',
                'GetAttackDamage','GetAttackRange','GetSecondsPerAttack','GetAttackPoint','GetAttackSpeed',
                'GetCurrentMovementSpeed','GetBoundingRadius','GetEvasion','GetFacing',
                'GetPrimaryAttribute','GetAttributeValue'}) do
                local method = unit[name]
                assert(type(method) == 'function')
                assert(not pcall(method, {}, 0))
            end
            for _, value in ipairs({-1,3,0.5,1e300,-1e300,0/0,math.huge,-math.huge}) do
                assert(not pcall(unit.GetAttributeValue, unit, value))
            end
            local ok, reason = pcall(unit.GetAttributeValue, unit, -1)
            assert(not ok and string.find(tostring(reason), 'integer within bounds expected', 1, true))
            ok, reason = pcall(unit.GetAttributeValue, unit, math.huge)
            assert(not ok and string.find(tostring(reason), 'finite number expected', 1, true))
            assert(unit:GetMana() >= 0)
            unit:Action_MoveToLocation(Vector(123,456,0))
        end
    "#).expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert_script_ran(&turn);
}

#[test]
fn owned_statistics_remain_available_after_a_dead_body_track_expires() {
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let own = sessions[0].own_hero().expect("hero").id;
    let mut view = sessions[0].current().expect("view").clone();
    view.tick += 1;
    let body = view
        .units
        .iter_mut()
        .find(|unit| unit.id == own)
        .expect("hero");
    body.mana = 73;
    body.attack_speed = 250;
    body.max_mana = 1000;
    let kit = bota_proto::Kit {
        abilities: body.abilities.clone(),
        items: body.items.clone(),
        town_portal: body.town_portal,
        sellable_slots: 0,
        reincarnating: false,
    };
    let tick = view.tick;
    sessions[0]
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .expect("received live body");
    sessions[0]
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
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
    .expect("team");
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "owned_body_statistics",
        br#"
        local turns = 0
        function Think()
            turns = turns + 1
            assert(GetBot():GetMana() == 73 and GetBot():GetMaxMana() == 1000)
            assert(GetBot():GetAttackPoint() == 0.5 and GetBot():GetAttackSpeed() == 2.5)
            GetBot():Action_MoveToLocation(Vector(turns,222,0))
        end
    "#,
    )
    .expect("script");
    let turn = lua.think(&sessions[0]).expect("live frame");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(
        turn.bots[0].orders[0].order,
        Order::Move {
            target: Target::Pos(Vec2::from_ints(9217, 9438))
        }
    );
    let mut view = sessions[0].current().expect("view").clone();
    view.tick += 481;
    view.players[0].deaths = 1;
    view.players[0].respawn_left = 600;
    view.players[0].kit = Some(kit);
    view.units.retain(|unit| unit.id != own);
    view.aggro.clear();
    let tick = view.tick;
    sessions[0]
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .expect("received dead body state");
    sessions[0]
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
    assert!(sessions[0].entity(own).is_none());
    let turn = lua.think(&sessions[0]).expect("dead frame");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(
        turn.bots[0].orders[0].order,
        Order::Move {
            target: Target::Pos(Vec2::from_ints(9218, 9438))
        }
    );
}

#[test]
fn enemy_statistics_are_last_observed_only_until_the_received_track_expires() {
    use bota_proto::Cheat;
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
    let streams = local.step(&[&[], &[]]).expect("shared sight");
    for (session, stream) in sessions.iter_mut().zip(streams) {
        support::fold(session, stream);
    }
    let mut teams = Vec::new();
    let mut enemies = Vec::new();
    for (index, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let slot = SlotId(index as u8);
        enemies.push(
            sessions[index]
                .current()
                .expect("view")
                .units
                .iter()
                .find(|unit| unit.hero.is_some() && unit.team == side.enemy())
                .expect("enemy")
                .id,
        );
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
        lua.add_script(slot,ScriptKind::Takeover,"expired_enemy_statistics",br#"
            local phase, enemy, mana, ability = 0, nil, nil, nil
            function Think()
                phase = phase + 1
                if phase == 1 then
                    enemy = GetUnitList(UNIT_LIST_ENEMY_HEROES)[1]
                    assert(enemy and enemy:CanBeSeen())
                    mana = enemy:GetMana()
                    ability = enemy:GetAbilityByName('nevermore_shadowraze1')
                    assert(ability and not ability:IsUltimate() and not ability:IsTalent())
                    assert(type(ability:IsTrained()) == 'boolean')
                elseif phase == 2 then
                    assert(not enemy:CanBeSeen() and enemy:GetMana() == mana)
                    assert(not ability:IsUltimate() and not ability:IsTalent())
                else
                    for _, name in ipairs({'IsTrained','IsTalent','IsUltimate'}) do
                        local ok, reason = pcall(ability[name],ability)
                        assert(not ok and string.find(tostring(reason),'ability is no longer known',1,true))
                    end
                    for _, name in ipairs({'GetMana','GetMaxMana','GetHealthRegen','GetManaRegen','GetArmor',
                        'GetAttackDamage','GetAttackRange','GetSecondsPerAttack','GetAttackPoint','GetAttackSpeed','GetCurrentMovementSpeed',
                        'GetBoundingRadius','GetEvasion','GetFacing','GetPrimaryAttribute','GetAttributeValue'}) do
                        local ok, reason = pcall(enemy[name],enemy,0)
                        assert(not ok and string.find(tostring(reason),'unit is no longer known',1,true))
                    end
                end
                GetBot():Action_MoveToLocation(Vector(100+phase,200,0))
            end
        "#).expect("script");
        teams.push(lua);
    }
    for phase in 1..=3 {
        for (index, lua) in teams.iter_mut().enumerate() {
            if phase == 3 {
                let mut view = sessions[index].current().expect("view").clone();
                view.tick += 481;
                let tick = view.tick;
                sessions[index]
                    .ingest(ServerMsg::Snapshot { view }, &mut ())
                    .expect("expired view");
                sessions[index]
                    .ingest(
                        ServerMsg::Events {
                            tick,
                            events: Vec::new(),
                        },
                        &mut (),
                    )
                    .expect("events");
                assert!(sessions[index].entity(enemies[index]).is_none());
                assert!(
                    sessions[index].known(enemies[index]).is_some(),
                    "body memory alone must not supply enemy statistics"
                );
            }
            let turn = lua.think(&sessions[index]).expect("think");
            assert!(turn.failures.is_empty(), "{:?}", turn.failures);
            assert_eq!(turn.bots[0].orders.len(), 1);
            assert_eq!(
                turn.bots[0].orders[0].order,
                Order::Move {
                    target: Target::Pos(Vec2::from_ints(9316 + phase, 9416))
                }
            );
        }
        if phase == 1 {
            local.world_mut().cheat(
                SlotId(0),
                None,
                Cheat::AllVision { on: false },
                &mut Vec::new(),
            );
            let streams = local.step(&[&[], &[]]).expect("normal sight");
            for (session, stream) in sessions.iter_mut().zip(streams) {
                support::fold(session, stream);
            }
            for (index, session) in sessions.iter().enumerate() {
                assert!(session.entity(enemies[index]).is_some());
            }
        }
    }
}
