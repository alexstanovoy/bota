mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, SlotId, Team};
fn run(source: &[u8]) {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for (index, session) in sessions.iter().enumerate() {
        let mut lua = LuaTeam::new(
            session.team(),
            &[BotSeat {
                slot: SlotId(index as u8),
                hero: HeroId::SHADOW_FIEND,
            }],
            Limits::DEFAULT,
            Keys {
                hash: [1, 2],
                random: 3,
            },
        )
        .unwrap();
        lua.add_script(SlotId(index as u8), ScriptKind::Takeover, "classes", source)
            .unwrap();
        let turn = lua.think(session).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    }
}
#[test]
fn bot_class_overrides_are_looked_up_by_existing_handles() {
    run(br#"
function Think()
 local bot=GetBot(); local base=CDOTA_Bot_Script.GetTeam; local original=bot:GetTeam()
 assert(type(base)=='function')
 CDOTA_Bot_Script.GetTeam=function(self) return base(self)+100 end
 assert(bot:GetTeam()==original+100 and GetBot()==bot)
 CDOTA_Bot_Script.SharedMethod=function(self) return self:GetTeam() end
 assert(bot:SharedMethod()==original+100)
 bot.SharedMethod=function() return 7 end; assert(bot:SharedMethod()==7)
 assert(not pcall(base,{})); CDOTA_Bot_Script.GetTeam=base
end"#);
}
#[test]
fn ability_class_overrides_are_looked_up_by_existing_handles() {
    run(br#"
function Think()
 local a=GetBot():GetAbilityByName("nevermore_shadowraze1"); assert(a)
 local base=CDOTABaseAbility_BotScript.GetLevel; local level=a:GetLevel()
 CDOTABaseAbility_BotScript.GetLevel=function(self) return base(self)+10 end
 assert(a:GetLevel()==level+10)
 assert(not pcall(base,{})); CDOTABaseAbility_BotScript.GetLevel=base
end"#);
}

#[test]
fn class_overrides_are_team_wide_but_never_cross_vm_boundaries() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Radiant, Team::Dire]);
    let keys = Keys {
        hash: [1, 2],
        random: 3,
    };
    let seats = [
        BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        },
        BotSeat {
            slot: SlotId(1),
            hero: HeroId::SHADOW_FIEND,
        },
    ];
    let mut team = LuaTeam::new(Team::Radiant, &seats, Limits::DEFAULT, keys).unwrap();
    team.add_script(SlotId(0),ScriptKind::Takeover,"writer",b"function Think() CDOTA_Bot_Script.GetTeam=function() return 99 end; CDOTABaseAbility_BotScript.GetLevel=function() return 77 end end").unwrap();
    team.add_script(SlotId(1),ScriptKind::Takeover,"reader",b"function Think() assert(GetBot():GetTeam()==99); assert(GetBot():GetAbilityByName('nevermore_shadowraze1'):GetLevel()==77) end").unwrap();
    let turn = team.think(&sessions[0]).unwrap();
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    for (session, slot) in [(&sessions[0], SlotId(0)), (&sessions[2], SlotId(2))] {
        let mut separate = LuaTeam::new(
            session.team(),
            &[BotSeat {
                slot,
                hero: HeroId::SHADOW_FIEND,
            }],
            Limits::DEFAULT,
            keys,
        )
        .unwrap();
        separate.add_script(slot,ScriptKind::Takeover,"separate_pack",b"function Think() assert(GetBot():GetTeam()==GetTeam()); assert(GetBot():GetAbilityByName('nevermore_shadowraze1'):GetLevel()~=77) end").unwrap();
        assert!(separate.think(session).unwrap().failures.is_empty());
    }
}
#[test]
fn replacing_every_script_method_cannot_replace_host_fallback_dispatch() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let execute = |overrides| {
        let mut lua = LuaTeam::new(
            Team::Radiant,
            &[BotSeat {
                slot: SlotId(0),
                hero: HeroId::SHADOW_FIEND,
            }],
            Limits::DEFAULT,
            Keys {
                hash: [1, 2],
                random: 3,
            },
        )
        .unwrap();
        if overrides {
            lua.add_script(SlotId(0),ScriptKind::TeamDesires,"patch_classes",b"function TeamThink() for _,class in ipairs({CDOTA_Bot_Script,CDOTABaseAbility_BotScript}) do for key,value in pairs(class) do if type(value)=='function' then class[key]=function() error('script override reached by host') end end end end end").unwrap();
        }
        lua.think(&sessions[0]).unwrap()
    };
    let plain = execute(false);
    let patched = execute(true);
    assert!(plain.failures.is_empty());
    assert!(patched.failures.is_empty(), "{:?}", patched.failures);
    assert!(!plain.bots[0].orders.is_empty());
    assert_eq!(plain.bots[0].orders, patched.bots[0].orders);
}
#[test]
fn host_identity_metatables_remain_locked_for_units_and_abilities() {
    run(br#"function Think()
      local bot=GetBot(); local ability=bot:GetAbilityByName('nevermore_shadowraze1')
      assert(getmetatable(bot)==false and getmetatable(ability)==false)
      assert(not pcall(setmetatable,bot,{})); assert(not pcall(setmetatable,ability,{}))
    end"#);
}
