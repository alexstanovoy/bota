mod support;

use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{EntityId, HeroId, Order, SlotId, Target, Team, UnitView, Vec2};

fn assert_move(turn: &bota_lua::TeamTurn, x: i32, y: i32) {
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(
        turn.bots[0].orders.len(),
        1,
        "the script reached its final action"
    );
    assert_eq!(
        turn.bots[0].orders[0].order,
        Order::Move {
            target: Target::Pos(Vec2::from_ints(x, y))
        }
    );
}

fn enemy_observation(body: &UnitView, id: EntityId, position: Vec2) -> UnitView {
    let mut shown = body.clone();
    let previous = shown.id;
    shown.id = id;
    shown.pos = position;
    for ability in &mut shown.abilities {
        ability.learnable = None;
    }
    for item in shown
        .items
        .iter_mut()
        .flatten()
        .chain(shown.town_portal.iter_mut())
    {
        item.selling = None;
        item.locked = None;
        item.disassembly = None;
        item.rune = None;
    }
    shown.effects.retain(|effect| {
        bota_core::effect_def(effect.id).is_some_and(|definition| !definition.own_side_only)
    });
    for effect in &mut shown.effects {
        effect.caster = effect
            .caster
            .filter(|caster| *caster == previous)
            .map(|_| id);
    }
    shown.queue.clear();
    shown
}

fn team(team: Team, slot: SlotId) -> LuaTeam {
    LuaTeam::new(
        team,
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
    .expect("team")
}

#[test]
fn player_queries_translate_both_sides_and_preserve_hidden_locations() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for stress in [GcStress::Full, GcStress::Incremental] {
        for (index, side, player, opponent) in [(0, Team::Radiant, 0, 5), (1, Team::Dire, 5, 0)] {
            let slot = SlotId(index as u8);
            let mut lua = team(side, slot);
            lua.set_gc_stress(stress);
            let source = r#"
            function Think()
                local bot = GetBot()
                assert(bot:GetPlayerID() == __PLAYER__)
                local radiant, dire = GetTeamPlayers(TEAM_RADIANT), GetTeamPlayers(TEAM_DIRE)
                assert(#radiant == 1 and radiant[1] == 0)
                assert(#dire == 1 and dire[1] == 5)
                assert(GetTeamForPlayer(0) == TEAM_RADIANT)
                assert(GetTeamForPlayer(5) == TEAM_DIRE)
                assert(GetSelectedHeroName(5) == 'npc_dota_hero_nevermore')
                assert(GetOpposingTeam() == __OPPOSING_TEAM__)
                assert(IsPlayerBot(__PLAYER__))
                assert(not IsPlayerBot(__OPPONENT__))
                assert(GetHeroLevel(__PLAYER__) == 1)
                assert(GetHeroLevel(__OPPONENT__) == 1)
                assert(GetHeroDeaths(__OPPONENT__) == 0)
                assert(IsHeroAlive(__OPPONENT__))
                local known = GetHeroLastSeenInfo(__PLAYER__)
                assert(#known == 1 and known[1].location == bot:GetLocation())
                assert(known[1].time_since_seen == 0)
                assert(#GetHeroLastSeenInfo(__OPPONENT__) == 0)
                bot:Action_MoveToLocation(Vector(123,456,0))
            end
        "#
            .replace("__PLAYER__", &player.to_string())
            .replace("__OPPONENT__", &opponent.to_string())
            .replace(
                "__OPPOSING_TEAM__",
                if side == Team::Radiant {
                    "TEAM_DIRE"
                } else {
                    "TEAM_RADIANT"
                },
            );
            lua.add_script(
                slot,
                ScriptKind::Takeover,
                "player_queries",
                source.as_bytes(),
            )
            .expect("script");
            let turn = lua.think(&sessions[index]).expect("think");
            assert_move(&turn, 9339, 9672);
        }
    }
}

#[test]
fn player_queries_reject_malformed_ids_and_forged_receivers() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(Team::Radiant, SlotId(0));
    lua.add_script(SlotId(0), ScriptKind::Takeover, "player_boundaries", br#"
        function Think()
            local bot = GetBot()
            assert(not pcall(bot.GetPlayerID, {}))
            assert(not pcall(bot.GetPlayerID, setmetatable({}, {__index=bot})))
            for _, value in ipairs({-1, 10, 0.5, 1e300, -1e300, 0/0, math.huge, -math.huge}) do
                for _, name in ipairs({'GetTeamForPlayer', 'IsPlayerBot', 'GetHeroLevel', 'GetHeroDeaths', 'IsHeroAlive', 'GetHeroLastSeenInfo'}) do
                    local query = _G[name]
                    assert(type(query) == 'function')
                    assert(not pcall(query, value))
                end
            end
            for _, value in ipairs({-1, 0, 1, 4, 1e300, 0/0, math.huge}) do
                assert(not pcall(GetTeamPlayers, value))
            end
            for _, name in ipairs({'GetTeamForPlayer','GetHeroLevel','GetHeroDeaths','IsHeroAlive',
                                  'IsPlayerBot','GetHeroLastSeenInfo','GetSelectedHeroName'}) do
                local ok, reason = pcall(_G[name], -1)
                assert(not ok and string.find(tostring(reason), 'integer within bounds expected', 1, true))
            end
            local ok, reason = pcall(GetTeamPlayers, 2.5)
            assert(not ok and string.find(tostring(reason), 'integer within bounds expected', 1, true))
            for _, id in ipairs({1,6}) do
                for _, name in ipairs({'GetTeamForPlayer','GetHeroLevel','GetHeroDeaths',
                                      'IsHeroAlive','IsPlayerBot','GetHeroLastSeenInfo'}) do
                    local ok, reason = pcall(_G[name], id)
                    assert(not ok and string.find(tostring(reason), 'player is not in the received roster', 1, true))
                end
                assert(GetSelectedHeroName(id) == '')
            end
            assert(bot:GetPlayerID() == 0 and IsPlayerBot(0))
            bot:Action_MoveToLocation(Vector(123,456,0))
        end
    "#).expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert_move(&turn, 9339, 9672);
}

#[test]
fn host_bot_declarations_are_bounded_and_distinct_and_frozen_before_execution() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(Team::Radiant, SlotId(0));
    let all = std::array::from_fn::<_, 10, _>(|index| SlotId(index as u8));
    lua.set_bot_declarations(&all)
        .expect("all ten distinct slots, including nine");
    assert_eq!(
        lua.set_bot_declarations(&[SlotId(10)])
            .expect_err("invalid slot")
            .to_string(),
        "declared bot slot is outside 0..10"
    );
    assert_eq!(
        lua.set_bot_declarations(&[SlotId(1), SlotId(1)])
            .expect_err("duplicate")
            .to_string(),
        "declared bot slots must be distinct"
    );
    assert_eq!(
        lua.set_bot_declarations(&[SlotId(0); 11])
            .expect_err("too many")
            .to_string(),
        "at most ten bot declarations are permitted"
    );
    lua.set_bot_declarations(&[SlotId(1)])
        .expect("foreign wire slot");
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "bot_declarations",
        br#"
        function Think() assert(IsPlayerBot(0) and IsPlayerBot(5)) GetBot():Action_MoveToLocation(Vector(123,456,0)) end
    "#,
    )
    .expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert_move(&turn, 9339, 9672);
    assert_eq!(
        lua.set_bot_declarations(&[])
            .expect_err("already started")
            .to_string(),
        "bot declarations must be set before execution"
    );
}

#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
#[test]
fn native_host_bot_declarations_reach_scripts_and_invalid_slots_are_rejected() {
    use bota_lua::{
        ModuleBank,
        native::{HeroSelection, NativeConfig, NativeError, NativeTeam},
    };
    for declared in [false, true] {
        let mut config = NativeConfig::new(
            Team::Radiant,
            vec![BotSeat {
                slot: SlotId(0),
                hero: HeroId::SHADOW_FIEND,
            }],
            Keys {
                hash: [17, 19],
                random: 23,
            },
        );
        config.hero_selection = HeroSelection::Fixed;
        config.think_timeout = std::time::Duration::from_secs(10);
        config.declared_bot_slots = vec![SlotId(10)];
        assert!(
            matches!(NativeTeam::new(config.clone(), ModuleBank::new(Limits::DEFAULT)),
            Err(NativeError::Setup(message)) if message == "declared bot slot is outside 0..10")
        );
        config.declared_bot_slots = if declared {
            vec![SlotId(1)]
        } else {
            Vec::new()
        };
        let mut modules = ModuleBank::new(Limits::DEFAULT);
        let source = if declared {
            b"function Think() assert(IsPlayerBot(0) and IsPlayerBot(5)) GetBot():Action_MoveToLocation(Vector(123,456,0)) end".as_slice()
        } else {
            b"function Think() assert(IsPlayerBot(0) and not IsPlayerBot(5)) GetBot():Action_MoveToLocation(Vector(123,456,0)) end".as_slice()
        };
        modules.insert("bot_nevermore", source).expect("script");
        let mut worker = NativeTeam::new(config, modules).expect("worker");
        let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let turn = worker.think(&sessions[0]).expect("think");
        assert_move(&turn, 9339, 9672);
    }
}

#[test]
fn last_seen_info_uses_received_body_memory_after_entity_tracking_expires() {
    use bota_proto::{ServerMsg, Vec2};
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let enemy = sessions[0].current().expect("view").players[1]
        .unit
        .expect("enemy identity");
    let shown = enemy_observation(
        sessions[1].own_hero().expect("enemy body"),
        enemy,
        Vec2::from_ints(9316, 9416),
    );
    let mut view = sessions[0].current().expect("view").clone();
    view.tick += 1;
    view.players[1].seen_at = Some(view.tick);
    view.units.push(shown);
    view.units.sort_unstable_by_key(|unit| unit.id);
    let tick = view.tick;
    sessions[0]
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .expect("visible observation");
    sessions[0]
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
    let mut lua = team(Team::Radiant, SlotId(0));
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "remembered_location",
        br#"
        local turns = 0
        function Think()
            turns = turns + 1
            local seen = GetHeroLastSeenInfo(5)
            assert(#seen == 1 and seen[1].location == Vector(100,200,0))
            if turns == 1 then assert(seen[1].time_since_seen == 0)
            else assert(math.abs(seen[1].time_since_seen - 481/30) < 0.000001) end
            GetBot():Action_MoveToLocation(Vector(turns,222,0))
        end
    "#,
    )
    .expect("script");
    let turn = lua.think(&sessions[0]).expect("visible frame");
    assert_move(&turn, 9217, 9438);
    let mut view = sessions[0].current().expect("view").clone();
    view.tick += 481;
    view.units.retain(|unit| unit.id != enemy);
    let tick = view.tick;
    sessions[0]
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .expect("later hidden frame");
    sessions[0]
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
    assert!(
        sessions[0].entity(enemy).is_none(),
        "the short-lived entity track expired"
    );
    let turn = lua.think(&sessions[0]).expect("remembered frame");
    assert_move(&turn, 9218, 9438);
}

#[test]
fn player_queries_keep_roster_ranks_with_interleaved_wire_seats_and_distinct_values() {
    use bota_proto::ServerMsg;
    let heroes = std::array::from_fn::<_, 10, _>(|index| {
        if index % 2 == 0 {
            (Team::Radiant, HeroId::SHADOW_FIEND)
        } else {
            (Team::Dire, HeroId::PUDGE)
        }
    });
    let (_, mut sessions) = support::local_with_modifiers(&heroes, Vec::new());
    let mut view = sessions[9].current().expect("view").clone();
    view.tick += 1;
    for player in &mut view.players {
        player.level = player.slot.0 + 2;
        player.deaths = u16::from(player.slot.0) * 3;
    }
    for unit in &mut view.units {
        if unit.hero.is_some()
            && let Some(owner) = unit.owner
        {
            unit.level = owner.0 + 2;
        }
    }
    let deceased = view.players[1].unit.expect("a hero");
    view.players[1].respawn_left = 30;
    view.units.retain(|unit| unit.id != deceased);
    view.aggro.retain(|entry| entry.unit != deceased);
    let tick = view.tick;
    sessions[9]
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .expect("received roster");
    sessions[9]
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
    let mut lua = LuaTeam::new(
        Team::Dire,
        &[BotSeat {
            slot: SlotId(9),
            hero: HeroId::PUDGE,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("team");
    lua.set_bot_declarations(&[SlotId(1)])
        .expect("declared foreign bot");
    lua.add_script(
        SlotId(9),
        ScriptKind::Takeover,
        "roster_ranks",
        br#"
        function Think()
            local radiant, dire = GetTeamPlayers(TEAM_RADIANT), GetTeamPlayers(TEAM_DIRE)
            assert(#radiant == 5 and #dire == 5)
            for i=1,5 do assert(radiant[i] == i-1 and dire[i] == i+4) end
            assert(GetBot():GetPlayerID() == 9)
            assert(GetTeamForPlayer(4) == TEAM_RADIANT and GetTeamForPlayer(9) == TEAM_DIRE)
            assert(GetSelectedHeroName(4) == 'npc_dota_hero_nevermore')
            assert(GetSelectedHeroName(9) == 'npc_dota_hero_pudge')
            assert(GetHeroLevel(0) == 2 and GetHeroLevel(4) == 10)
            assert(GetHeroLevel(5) == 3 and GetHeroLevel(9) == 11)
            assert(GetHeroDeaths(4) == 24 and GetHeroDeaths(9) == 27)
            assert(IsHeroAlive(0) and not IsHeroAlive(5))
            assert(IsPlayerBot(9) and IsPlayerBot(5) and not IsPlayerBot(6))
            GetBot():Action_MoveToLocation(Vector(123,456,0))
        end
    "#,
    )
    .expect("script");
    assert_move(&lua.think(&sessions[9]).expect("think"), 9339, 9672);
}

#[test]
fn hero_name_lookup_does_not_guess_ranks_before_a_roster_is_received() {
    let mut lua = team(Team::Radiant, SlotId(1));
    lua.add_script(
        SlotId(1),
        ScriptKind::HeroSelection,
        "selection_roster",
        br#"
        function Think()
            assert(GetSelectedHeroName(0) == '')
            SelectHero(1, 'npc_dota_hero_pudge')
            error('selection rank checked')
        end
    "#,
    )
    .expect("script");
    let selected = lua.select_heroes(0).expect("selection");
    assert_eq!(selected.failures.len(), 1);
    assert!(
        selected.failures[0]
            .error
            .to_string()
            .contains("selection rank checked")
    );
    assert_eq!(
        selected.seats[0].hero,
        HeroId::SHADOW_FIEND,
        "an unproven engine rank cannot select a wire seat"
    );
    assert_eq!(
        lua.set_bot_declarations(&[])
            .expect_err("selection already started")
            .to_string(),
        "bot declarations must be set before execution"
    );
}

#[test]
fn selection_queries_and_picks_share_proven_engine_ids_on_both_sides() {
    for (side, base) in [(Team::Radiant, 0), (Team::Dire, 5)] {
        let mut lua = LuaTeam::new(
            side,
            &[
                BotSeat {
                    slot: SlotId(0),
                    hero: HeroId::SHADOW_FIEND,
                },
                BotSeat {
                    slot: SlotId(1),
                    hero: HeroId::SHADOW_FIEND,
                },
            ],
            Limits::DEFAULT,
            Keys {
                hash: [17, 19],
                random: 23,
            },
        )
        .expect("team");
        let source = r#"
            function Think()
                assert(GetSelectedHeroName(__FIRST__) == 'npc_dota_hero_nevermore')
                assert(GetSelectedHeroName(__SECOND__) == 'npc_dota_hero_nevermore')
                SelectHero(__SECOND__, 'npc_dota_hero_pudge')
                assert(GetSelectedHeroName(__SECOND__) == 'npc_dota_hero_pudge')
            end
        "#
        .replace("__FIRST__", &base.to_string())
        .replace("__SECOND__", &(base + 1).to_string());
        lua.add_script(
            SlotId(0),
            ScriptKind::HeroSelection,
            "selection_ids",
            source.as_bytes(),
        )
        .expect("script");
        let selected = lua.select_heroes(0).expect("selection");
        assert!(selected.failures.is_empty(), "{:?}", selected.failures);
        assert_eq!(selected.seats[1].hero, HeroId::PUDGE);
    }
}

#[test]
fn last_seen_info_reports_received_candidates_without_identifying_a_hidden_real_hero() {
    use bota_proto::ServerMsg;
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut view = sessions[0].current().expect("view").clone();
    view.tick += 1;
    view.players[1].unit = None;
    view.players[1].seen_at = Some(view.tick);
    let next = view
        .units
        .iter()
        .map(|unit| unit.id.idx)
        .max()
        .expect("units")
        + 1;
    let body = sessions[1].own_hero().expect("hero");
    for (offset, position) in [
        (0, Vec2::from_ints(9316, 9416)),
        (1, Vec2::from_ints(9516, 9616)),
    ] {
        view.units.push(enemy_observation(
            body,
            EntityId {
                idx: next + offset,
                generation: 0,
            },
            position,
        ));
    }
    view.units.sort_unstable_by_key(|unit| unit.id);
    let tick = view.tick;
    sessions[0]
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .expect("ambiguous visible bodies");
    sessions[0]
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
    let mut lua = team(Team::Radiant, SlotId(0));
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "last_seen_candidates",
        br#"
        function Think()
            local seen, first, second = GetHeroLastSeenInfo(5), false, false
            assert(#seen == 2)
            for _, entry in ipairs(seen) do
                assert(entry.time_since_seen == 0)
                if entry.location == Vector(100,200,0) then first = true
                elseif entry.location == Vector(300,400,0) then second = true
                else error('unexpected received location') end
            end
            assert(first and second)
            GetBot():Action_MoveToLocation(Vector(123,456,0))
        end
    "#,
    )
    .expect("script");
    assert_move(&lua.think(&sessions[0]).expect("think"), 9339, 9672);
}

#[test]
fn player_id_rejects_an_expired_creep_handle() {
    use bota_client_core::IssuedOrder;
    let (mut local, mut sessions, target) = support::durable_creep_encounter(HeroId::SHADOW_FIEND);
    let mut lua = team(Team::Radiant, SlotId(0));
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "expired_player_id",
        br#"
        local held, turns = nil, 0
        function Think()
            turns = turns + 1
            if turns == 1 then
                held = GetBot():GetNearbyCreeps(1600,true)[1]
                assert(held and held:GetPlayerID() == -1)
            else
                local ok, reason = pcall(GetBot().GetPlayerID, held)
                assert(not ok and string.find(tostring(reason), 'unit is no longer known', 1, true))
            end
            GetBot():Action_MoveToLocation(Vector(turns,222,0))
        end
    "#,
    )
    .expect("script");
    assert_move(
        &lua.think(&sessions[0]).expect("observed creep"),
        9217,
        9438,
    );
    let fountain = bota_core::fountain_pos(sessions[0].map(), Team::Radiant);
    for _ in 0..3600 {
        if sessions[0].entity(target).is_none() {
            break;
        }
        let order = sessions[0]
            .issue(IssuedOrder::hero(Order::Move {
                target: Target::Pos(fountain),
            }))
            .expect("move");
        let streams = local.step(&[&[order], &[]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            support::fold(session, stream);
        }
    }
    assert!(
        sessions[0].entity(target).is_none(),
        "the received creep observation expired"
    );
    assert_move(&lua.think(&sessions[0]).expect("expired creep"), 9218, 9438);
}
