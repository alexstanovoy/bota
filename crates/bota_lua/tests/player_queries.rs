mod support;

use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, SlotId, Team};

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
                assert(GetOpposingTeam() ~= GetTeam())
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
            end
        "#
            .replace("__PLAYER__", &player.to_string())
            .replace("__OPPONENT__", &opponent.to_string());
            lua.add_script(
                slot,
                ScriptKind::Takeover,
                "player_queries",
                source.as_bytes(),
            )
            .expect("script");
            let turn = lua.think(&sessions[index]).expect("think");
            assert!(turn.failures.is_empty(), "{:?}", turn.failures);
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
            assert(bot:GetPlayerID() == 0 and IsPlayerBot(0))
        end
    "#).expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}

#[test]
fn host_bot_declarations_are_bounded_and_distinct_and_frozen_before_execution() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(Team::Radiant, SlotId(0));
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
        function Think() assert(IsPlayerBot(0) and IsPlayerBot(5)) end
    "#,
    )
    .expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
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
            b"function Think() assert(IsPlayerBot(0) and IsPlayerBot(5)) end".as_slice()
        } else {
            b"function Think() assert(IsPlayerBot(0) and not IsPlayerBot(5)) end".as_slice()
        };
        modules.insert("bot_nevermore", source).expect("script");
        let mut worker = NativeTeam::new(config, modules).expect("worker");
        let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let turn = worker.think(&sessions[0]).expect("think");
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    }
}

#[test]
fn last_seen_info_uses_received_body_memory_after_entity_tracking_expires() {
    use bota_proto::{ServerMsg, Vec2};
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let enemy = sessions[0].current().expect("view").players[1]
        .unit
        .expect("enemy identity");
    let mut shown = sessions[1].own_hero().expect("enemy body").clone();
    shown.id = enemy;
    shown.pos = Vec2::from_ints(9316, 9416);
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
    shown.queue.clear();
    let mut view = sessions[0].current().expect("view").clone();
    view.tick += 1;
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
        end
    "#,
    )
    .expect("script");
    let turn = lua.think(&sessions[0]).expect("visible frame");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
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
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}
