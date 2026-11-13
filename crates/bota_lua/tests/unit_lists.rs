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
            function Think()
                phase = phase + 1
                local own = GetUnitList(UNIT_LIST_ALLIED_HEROES)
                assert(#own == 1 and own[1] == GetBot())
                local enemies = GetUnitList(UNIT_LIST_ENEMY_HEROES)
                if phase == 2 then
                    assert(#enemies == 1 and enemies[1]:GetTeam() == GetOpposingTeam())
                    enemy = enemies[1]
                else
                    assert(#enemies == 0)
                    if phase == 3 then assert(enemy and not enemy:CanBeSeen()) end
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
        let turn = lua.think(&sessions[index]).expect("hidden enemy");
        assert_script_ran(&turn, 9319, 9416);
    }
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
