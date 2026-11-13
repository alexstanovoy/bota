mod support;
use bota_client_core::IssuedOrder;
use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{Cheat, HeroId, Order, SlotId, Target, Team, Vec2};

fn lua(side: Team, slot: SlotId, source: &[u8]) -> LuaTeam {
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
    lua.add_script(slot, ScriptKind::Takeover, "glyph_queries", source)
        .expect("script");
    lua
}

#[test]
fn glyph_queries_and_action_follow_the_real_cooldown_without_observing_the_other_side() {
    for (index, side) in [(0, Team::Radiant), (1, Team::Dire)] {
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
        {
            let world = local.world_mut();
            world.cheats = true;
            world.cheat(SlotId(0), None, Cheat::Refresh, &mut Vec::new());
            world.cheat(
                SlotId(0),
                None,
                Cheat::AllVision { on: true },
                &mut Vec::new(),
            );
        }
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[&[], &[]]).expect("refresh"))
        {
            support::fold(session, stream);
        }
        let slot = SlotId(index as u8);
        let mut active=lua(side,slot,br#"
            local phase=0
            function Think()
                phase=phase+1
                local bot=GetBot()
                if phase==1 then
                    assert(GetGlyphCooldown()==0)
                    local enemy=GetUnitList(UNIT_LIST_ENEMY_HEROES)[1]
                    assert(enemy)
                    assert(not pcall(bot.ActionImmediate_Glyph,{}))
                    local ok,reason=pcall(bot.ActionImmediate_Glyph,enemy)
                    assert(not ok and string.find(tostring(reason),'action requires the active bot',1,true))
                    bot:ActionImmediate_Glyph()
                else
                    local cooldown=GetGlyphCooldown()
                    assert(cooldown>0)
                    bot:Action_MoveToLocation(Vector(math.floor(cooldown*30+0.5),200,0))
                end
            end
        "#);
        let first = active.think(&sessions[index]).expect("ready think");
        assert!(first.failures.is_empty(), "{:?}", first.failures);
        assert_eq!(first.bots[0].orders, vec![IssuedOrder::hero(Order::Glyph)]);
        let outgoing = sessions[index]
            .issue(first.bots[0].orders[0])
            .expect("glyph order");
        let mut commands = [Vec::new(), Vec::new()];
        commands[index].push(outgoing);
        for (session, stream) in sessions
            .iter_mut()
            .zip(local.step(&[&commands[0], &commands[1]]).expect("activate"))
        {
            assert!(
                !stream
                    .iter()
                    .any(|message| matches!(message, bota_proto::ServerMsg::OrderRejected { .. })),
                "glyph accepted"
            );
            support::fold(session, stream);
        }
        let own = sessions[index]
            .current()
            .expect("view")
            .glyphs
            .iter()
            .find(|glyph| glyph.team == side)
            .expect("own glyph");
        assert!(own.active_for > 0);
        let ticks = own.ready_after.expect("observed cooldown");
        assert!(ticks > 0);
        let second = active.think(&sessions[index]).expect("cooling think");
        assert!(second.failures.is_empty(), "{:?}", second.failures);
        assert_eq!(
            second.bots[0].orders,
            vec![IssuedOrder::hero(Order::Move {
                target: Target::Pos(Vec2::from_ints(9216 + ticks as i32, 9416))
            })]
        );
        let foreign = sessions[1 - index]
            .current()
            .expect("other view")
            .glyphs
            .iter()
            .find(|glyph| glyph.team == side)
            .expect("foreign glyph");
        assert!(foreign.active_for > 0 && foreign.ready_after.is_none());
        let mut other=lua(side.enemy(),SlotId((1-index) as u8),br#"function Think() assert(GetGlyphCooldown()==0) GetBot():Action_MoveToLocation(Vector(0,200,0)) end"#);
        let observed = other
            .think(&sessions[1 - index])
            .expect("own other cooldown");
        assert!(observed.failures.is_empty(), "{:?}", observed.failures);
        assert_eq!(
            observed.bots[0].orders,
            vec![IssuedOrder::hero(Order::Move {
                target: Target::Pos(Vec2::from_ints(9216, 9416))
            })]
        );
    }
}
