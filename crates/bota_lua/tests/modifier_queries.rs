mod support;

use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{
    AbilityId, CastView, EffectId, EffectView, HeroId, Order, ServerMsg, SlotId, StatusFlags,
    Target, Team, Vec2,
};

fn team() -> LuaTeam {
    LuaTeam::new(
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
    .expect("team")
}

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
fn reported_modifiers_supply_names_indices_stacks_remaining_time_and_conditions() {
    let (_, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut view = sessions[0].current().expect("view").clone();
    view.tick += 1;
    let own = sessions[0].own_hero().expect("hero").id;
    let unit = view
        .units
        .iter_mut()
        .find(|unit| unit.id == own)
        .expect("hero");
    unit.statuses.bits |= StatusFlags::STUNNED | StatusFlags::INVISIBLE | StatusFlags::CASTING;
    unit.cast = Some(CastView {
        ability: Some(AbilityId::RAZE_NEAR),
        goes_off_in: 3,
    });
    unit.effects = vec![
        EffectView {
            id: EffectId::STUNNED,
            ticks_left: Some(75),
            stacks: None,
            caster: None,
            inflictor: None,
            ticks_total: Some(150),
        },
        EffectView {
            id: EffectId::ILLUSION,
            ticks_left: None,
            stacks: Some(7),
            caster: Some(own),
            inflictor: None,
            ticks_total: None,
        },
    ];
    let tick = view.tick;
    sessions[0]
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .expect("reported effects");
    sessions[0]
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
    let mut lua = team();
    lua.add_script(SlotId(0), ScriptKind::Takeover, "modifier_data", br#"
        function Think()
            local unit = GetBot()
            assert(unit:NumModifiers() == 2)
            assert(unit:HasModifier('modifier_stunned') and unit:HasModifier('modifier_illusion'))
            assert(not unit:HasModifier('modifier_not_reported'))
            assert(unit:GetModifierByName('modifier_stunned') == 0)
            assert(unit:GetModifierByName('modifier_illusion') == 1)
            assert(unit:GetModifierByName('modifier_not_reported') == -1)
            assert(unit:GetModifierName(0) == 'modifier_stunned' and unit:GetModifierName(1) == 'modifier_illusion')
            assert(unit:GetModifierRemainingDuration(0) == 2.5 and unit:GetModifierRemainingDuration(1) == -1)
            assert(unit:GetModifierStackCount(0) == 0 and unit:GetModifierStackCount(1) == 7)
            assert(unit:IsIllusion() and unit:IsStunned() and unit:IsInvisible() and unit:IsCastingAbility())
            assert(not unit:IsCreep() and not unit:IsTower())
            unit:Action_MoveToLocation(Vector(123,456,0))
        end
    "#).expect("script");
    assert_script_ran(&lua.think(&sessions[0]).expect("think"));
}

#[test]
fn modifier_queries_reject_forged_receivers_and_malformed_names_and_indices() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = team();
    lua.add_script(SlotId(0), ScriptKind::Takeover, "modifier_boundaries", br#"
        function Think()
            local unit = GetBot()
            for _, name in ipairs({'HasModifier','NumModifiers','GetModifierByName','GetModifierName',
                'GetModifierRemainingDuration','GetModifierStackCount','IsIllusion','IsStunned',
                'IsInvisible','IsCastingAbility','IsCreep','IsTower'}) do
                local method = unit[name]
                assert(type(method) == 'function' and not pcall(method, {}, 0))
            end
            for _, value in ipairs({-1,0.5,1e300,-1e300,0/0,math.huge,-math.huge,{}}) do
                for _, name in ipairs({'GetModifierName','GetModifierRemainingDuration','GetModifierStackCount'}) do
                    assert(not pcall(unit[name], unit, value))
                end
            end
            local ok, reason = pcall(unit.GetModifierName, unit, -1)
            assert(not ok and string.find(tostring(reason), 'integer within bounds expected', 1, true))
            for _, value in ipairs({0,1e300,0/0,math.huge,{}}) do
                assert(not pcall(unit.HasModifier, unit, value))
                assert(not pcall(unit.GetModifierByName, unit, value))
            end
            assert(not unit:IsIllusion() and not unit:IsStunned() and not unit:IsCastingAbility())
            unit:Action_MoveToLocation(Vector(123,456,0))
        end
    "#).expect("script");
    assert_script_ran(&lua.think(&sessions[0]).expect("think"));
}

#[test]
fn visible_creeps_and_towers_have_their_received_unit_kinds() {
    let (_, sessions, _) = support::durable_creep_encounter(HeroId::SHADOW_FIEND);
    let mut lua = team();
    lua.add_script(
        SlotId(0),
        ScriptKind::Takeover,
        "unit_kinds",
        br#"
        function Think()
            local creeps = GetBot():GetNearbyCreeps(1600,true)
            assert(#creeps > 0)
            for _, creep in ipairs(creeps) do assert(creep:IsCreep() and not creep:IsTower()) end
            local tower = GetTower(GetTeam(), TOWER_MID_1)
            assert(tower and tower:IsTower() and not tower:IsCreep())
            GetBot():Action_MoveToLocation(Vector(123,456,0))
        end
    "#,
    )
    .expect("script");
    assert_script_ran(&lua.think(&sessions[0]).expect("think"));
}
