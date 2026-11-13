mod support;

use bota_client_core::{ClientSession, IssuedOrder};
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, Order, SlotId, Target, Team, UnitKind, Vec2};
use bota_server::local::LocalMatch;

fn assert_script_ran(turn: &bota_lua::TeamTurn, x: i32, y: i32) {
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 1);
    assert_eq!(
        turn.bots[0].orders[0].order,
        Order::Move {
            target: Target::Pos(Vec2::from_ints(x, y))
        }
    );
}

fn team(side: Team, slot: SlotId) -> LuaTeam {
    LuaTeam::new(
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
    .expect("team")
}

fn move_both(local: &mut LocalMatch, sessions: &mut [ClientSession], positions: [Vec2; 2]) {
    let orders: Vec<_> = sessions
        .iter_mut()
        .zip(positions)
        .map(|(session, position)| {
            session
                .issue(IssuedOrder::hero(Order::Move {
                    target: Target::Pos(position),
                }))
                .expect("move")
        })
        .collect();
    let streams = local.step(&[&[orders[0]], &[orders[1]]]).expect("step");
    for (session, stream) in sessions.iter_mut().zip(streams) {
        support::fold(session, stream);
    }
}

fn sees_enemy(session: &ClientSession) -> bool {
    session
        .current()
        .expect("view")
        .units
        .iter()
        .any(|unit| unit.kind == UnitKind::Hero && unit.team == session.enemy())
}

#[test]
fn global_unit_lists_include_visible_units_and_drop_remembered_enemies_from_both_sides() {
    for (index, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let slot = SlotId(index as u8);
        let mut lua = team(side, slot);
        lua.add_script(slot, ScriptKind::Takeover, "visible_lists", br#"
            local phase, enemy = 0, nil
            local function same(left, right)
                return left.x == right.x and left.y == right.y and left.z == right.z
            end
            function Think()
                phase = phase + 1
                local bot = GetBot()
                assert(type(bot.GetOrigin) == 'function')
                assert(same(bot:GetOrigin(), bot:GetLocation()))
                local ok, reason = pcall(bot.GetOrigin, {})
                assert(not ok and string.find(tostring(reason), 'unit handle expected', 1, true))
                assert(not pcall(bot.GetOrigin, setmetatable({}, {__index=bot})))
                local own = GetUnitList(UNIT_LIST_ALLIED_HEROES)
                assert(#own == 1 and own[1] == GetBot())
                local enemies = GetUnitList(UNIT_LIST_ENEMY_HEROES)
                if phase == 2 then
                    assert(#enemies == 1 and enemies[1]:GetTeam() == GetOpposingTeam())
                    enemy = enemies[1]
                    assert(same(enemy:GetOrigin(), enemy:GetLocation()))
                else
                    assert(#enemies == 0)
                    if phase >= 3 then
                        assert(enemy and not enemy:CanBeSeen())
                        if phase == 3 then assert(same(enemy:GetOrigin(), enemy:GetLocation()))
                        else
                            for _, name in ipairs({'GetOrigin','GetLocation'}) do
                                local ok, reason = pcall(enemy[name],enemy)
                                assert(not ok and string.find(tostring(reason),'unit is no longer known',1,true))
                            end
                        end
                    end
                end
                for _, unit in ipairs(GetUnitList(UNIT_LIST_ALL)) do assert(unit:CanBeSeen()) end
                for _, unit in ipairs(GetUnitList(UNIT_LIST_ALLIES)) do assert(unit:GetTeam() == GetTeam()) end
                for _, unit in ipairs(GetUnitList(UNIT_LIST_ENEMIES)) do assert(unit:GetTeam() == GetOpposingTeam()) end
                local buildings = GetUnitList(UNIT_LIST_ALLIED_BUILDINGS)
                local tower, found = GetTower(GetTeam(), TOWER_MID_1), false
                for _, unit in ipairs(buildings) do
                    assert(not unit:IsHero() and unit:GetTeam() == GetTeam())
                    if unit == tower then found = true end
                end
                assert(tower and found)
                GetBot():Action_MoveToLocation(Vector(100+phase,200,0))
            end
        "#).expect("script");
        let turn = lua.think(&sessions[index]).expect("initial view");
        assert_script_ran(&turn, 9317, 9416);
        let center = Vec2::from_ints(9216, 9216);
        for _ in 0..3600 {
            if sees_enemy(&sessions[index]) {
                break;
            }
            move_both(&mut local, &mut sessions, [center; 2]);
        }
        assert!(
            sees_enemy(&sessions[index]),
            "the ordinary moves reached enemy vision"
        );
        let turn = lua.think(&sessions[index]).expect("visible enemy");
        assert_script_ran(&turn, 9318, 9416);
        let observed_enemy = sessions[index]
            .current()
            .expect("view")
            .units
            .iter()
            .find(|unit| unit.kind == UnitKind::Hero && unit.team == sessions[index].enemy())
            .expect("visible enemy")
            .id;
        let mut positions = [center; 2];
        positions[index] = bota_core::fountain_pos(sessions[index].map(), side);
        for _ in 0..1800 {
            if !sees_enemy(&sessions[index]) {
                break;
            }
            move_both(&mut local, &mut sessions, positions);
        }
        assert!(
            !sees_enemy(&sessions[index]),
            "the enemy left this side's sight"
        );
        assert!(
            sessions[index].entity(observed_enemy).is_some(),
            "the hidden enemy is still remembered"
        );
        let turn = lua.think(&sessions[index]).expect("hidden enemy");
        assert_script_ran(&turn, 9319, 9416);
        let mut view = sessions[index].current().expect("view").clone();
        view.tick += 481;
        let tick = view.tick;
        sessions[index]
            .ingest(bota_proto::ServerMsg::Snapshot { view }, &mut ())
            .expect("expired track view");
        sessions[index]
            .ingest(
                bota_proto::ServerMsg::Events {
                    tick,
                    events: Vec::new(),
                },
                &mut (),
            )
            .expect("events");
        assert!(sessions[index].entity(observed_enemy).is_none());
        let turn = lua.think(&sessions[index]).expect("forgotten enemy");
        assert_script_ran(&turn, 9320, 9416);
    }
}

#[test]
fn unit_lists_classify_visible_creeps_wards_buildings_and_neutrals() {
    use bota_core::NeutralKind;
    use bota_core::{FILLER, MELEE_CREEP, OBSERVER_WARD, SENTRY_WARD};
    use bota_server::game::Place;

    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let position = sessions[0].own_hero().expect("hero").pos;
    {
        let world = local.world_mut();
        world.spawn_creep(
            &MELEE_CREEP,
            Team::Radiant,
            position,
            bota_core::rules::LANE_MID,
            0,
        );
        world.spawn_creep(
            &MELEE_CREEP,
            Team::Dire,
            position + Vec2::from_ints(100, 0),
            bota_core::rules::LANE_MID,
            0,
        );
        world.spawn_neutral(
            NeutralKind::Kobold,
            position + Vec2::from_ints(200, 0),
            0,
            0,
        );
        world.spawn_ward(
            &OBSERVER_WARD,
            (Team::Radiant, position + Vec2::from_ints(0, 100)),
            300,
            (None, None),
        );
        world.spawn_ward(
            &SENTRY_WARD,
            (Team::Dire, position + Vec2::from_ints(100, 100)),
            300,
            (None, None),
        );
        world.spawn_building(
            &FILLER,
            Team::Dire,
            position + Vec2::from_ints(200, 100),
            Place::Filler,
        );
    }
    let streams = local.step(&[&[], &[]]).expect("project units");
    for (session, stream) in sessions.iter_mut().zip(streams) {
        support::fold(session, stream);
    }
    assert!(
        sessions[0]
            .current()
            .expect("view")
            .units
            .iter()
            .any(|unit| unit.kind == SENTRY_WARD.kind && unit.team == Team::Dire),
        "the fixture reveals the enemy ward"
    );
    let mut lua = team(Team::Radiant, SlotId(0));
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "unit_categories",
        br#"
        function Think()
            assert(#GetUnitList(UNIT_LIST_ALLIED_CREEPS) == 1)
            assert(#GetUnitList(UNIT_LIST_ENEMY_CREEPS) == 1)
            assert(#GetUnitList(UNIT_LIST_NEUTRAL_CREEPS) == 1)
            assert(#GetUnitList(UNIT_LIST_ALLIED_WARDS) == 1)
            assert(#GetUnitList(UNIT_LIST_ENEMY_WARDS) == 1)
            local found = false
            for _, unit in ipairs(GetUnitList(UNIT_LIST_ENEMY_BUILDINGS)) do
                if unit:GetUnitName() == 'npc_dota_badguys_fillers' then found = true end
            end
            assert(found)
            for _, unit in ipairs(GetUnitList(UNIT_LIST_ENEMIES)) do
                assert(unit:GetTeam() == TEAM_DIRE)
            end
            GetBot():Action_MoveToLocation(Vector(123,456,0))
        end
    "#,
    )
    .expect("script");
    assert_script_ran(&lua.think(&sessions[0]).expect("think"), 9339, 9672);
}

#[test]
fn global_unit_lists_reject_invalid_categories_and_return_independent_tables() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team(Team::Radiant, SlotId(0));
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "list_boundaries",
        br#"
        function Think()
            assert(type(GetUnitList) == 'function')
            for _, value in ipairs({-1,12,0.5,1e300,-1e300,0/0,math.huge,-math.huge,{}}) do
                assert(not pcall(GetUnitList, value))
            end
            local first = GetUnitList(UNIT_LIST_ALLIED_HEROES)
            assert(#first == 1 and first[1] == GetBot())
            first[1] = {}
            local second = GetUnitList(UNIT_LIST_ALLIED_HEROES)
            assert(second ~= first and second[1] == GetBot())
            assert(#GetUnitList(UNIT_LIST_NEUTRAL_CREEPS) == 0)
            GetBot():Action_MoveToLocation(Vector(123,456,0))
        end
    "#,
    )
    .expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert_script_ran(&turn, 9339, 9672);
}
