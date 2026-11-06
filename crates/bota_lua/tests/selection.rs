use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, SlotId, Team};

fn team() -> LuaTeam {
    LuaTeam::new(Team::Radiant, &[BotSeat { slot: SlotId(0), hero: HeroId::SHADOW_FIEND }], Limits::DEFAULT, Keys { hash: [17, 19], random: 23 }).expect("team")
}

#[test]
fn selection_uses_the_first_owned_pick_and_falls_back_for_unsupported_heroes() {
    let mut lua = team();
    lua.add_script(SlotId(0), ScriptKind::HeroSelection, "selection", br#"
        function Think()
            SelectHero(9,'npc_dota_hero_pudge')
            SelectHero(0,'npc_dota_hero_axe')
            SelectHero(0,'npc_dota_hero_pudge')
            assert(GetSelectedHeroName(0) == 'npc_dota_hero_nevermore')
        end
    "#).expect("script");
    let turn = lua.select_heroes(0).expect("selection turn");
    assert!(turn.complete);
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.seats[0].hero, HeroId::SHADOW_FIEND);
}

#[test]
fn selection_can_choose_a_registered_hero_on_a_later_selection_tick() {
    let mut lua = team();
    lua.add_script(SlotId(0), ScriptKind::HeroSelection, "selection", b"local calls = 0 function Think() calls = calls + 1 if calls > 1 then SelectHero(0,'npc_dota_hero_pudge') end end").expect("script");
    assert!(!lua.select_heroes(0).expect("first turn").complete);
    let turn = lua.select_heroes(1).expect("second turn");
    assert!(turn.complete);
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.seats[0].hero, HeroId::PUDGE);
    assert!(lua.select_heroes(1).expect_err("tick must advance").to_string().contains("increasing selection tick"));
}
