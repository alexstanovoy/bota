mod support;

use bota_lua::{BotSeat, GcStress, Keys, Limits, LuaTeam, ModuleBank, ScriptKind};
use bota_proto::{HeroId, Order, SlotId, Target, Team};

#[test]
fn modules_share_one_team_cache_while_dofile_is_uncached_and_scopes_survive_gc() {
    for stress in [GcStress::Full, GcStress::Incremental] {
        let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire, Team::Radiant]);
        let mut modules = ModuleBank::new(Limits::DEFAULT);
        modules
            .insert(
                "bots/shared",
                b"local count = 0 return {next = function() count = count + 1 return count end}",
            )
            .expect("module");
        modules
            .insert(
                "bots/uncached",
                b"_G.loaded = (_G.loaded or 0) + 1 return _G.loaded",
            )
            .expect("module");
        let seats = [
            BotSeat {
                slot: SlotId(0),
                hero: HeroId::SHADOW_FIEND,
            },
            BotSeat {
                slot: SlotId(2),
                hero: HeroId::SHADOW_FIEND,
            },
        ];
        let mut lua = LuaTeam::new(
            Team::Radiant,
            &seats,
            Limits::DEFAULT,
            Keys {
                hash: [17, 19],
                random: 23,
            },
        )
        .expect("team");
        lua.set_modules(modules).expect("bounded modules");
        for slot in [0, 2] {
            lua.add_script(
                SlotId(slot),
                ScriptKind::Takeover,
                "module_scope",
                br#"
                local shared = require(GetScriptDirectory()..'/shared')
                function Think()
                    assert(shared == require('bots/shared'))
                    local count = shared.next()
                    local loaded = dofile('bots/uncached')
                    collectgarbage()
                    GetBot():Action_MoveToLocation(Vector(count,loaded,0))
                end
            "#,
            )
            .expect("script");
        }
        lua.set_gc_stress(stress);
        for tick in 0..3 {
            let turn = lua.think(&sessions[0]).expect("think");
            assert!(turn.failures.is_empty(), "{:?}", turn.failures);
            for (index, bot) in turn.bots.iter().enumerate() {
                let Order::Move {
                    target: Target::Pos(position),
                } = bot.orders[0].order
                else {
                    panic!("move")
                };
                assert_eq!(position.x.to_int(), 9216 + tick * 2 + index as i32 + 1);
                assert_eq!(position.y.to_int(), position.x.to_int());
            }
            let streams = local.step(&[&[], &[], &[]]).expect("step");
            for (session, stream) in sessions.iter_mut().zip(streams) {
                support::fold(session, stream);
            }
        }
    }
}

#[test]
fn module_bank_rejects_traversal_and_source_resource_overflows() {
    let limits = Limits {
        module_size: 16,
        scripts_size: 32,
        module_count: 2,
        ..Limits::DEFAULT
    };
    let mut modules = ModuleBank::new(limits);
    for name in ["../x", "/x", "x\\y", "x:y", ".hidden", "CON", "x/../../y"] {
        let error = modules
            .insert(name, b"return 1")
            .expect_err("confined name");
        assert!(error.to_string().contains("invalid module name"));
    }
    assert!(
        modules
            .insert("large", &[b' '; 17])
            .expect_err("module size")
            .to_string()
            .contains("module source limit")
    );
    modules.insert("one", &[b' '; 16]).expect("first");
    modules.insert("two", &[b' '; 16]).expect("second");
    assert!(
        modules
            .insert("three", b"")
            .expect_err("module count")
            .to_string()
            .contains("module count limit")
    );
    let mut modules = ModuleBank::new(Limits {
        module_count: 3,
        ..limits
    });
    modules.insert("one", &[b' '; 16]).expect("first");
    modules.insert("two", &[b' '; 16]).expect("second");
    assert!(
        modules
            .insert("three", b"x")
            .expect_err("total source")
            .to_string()
            .contains("total module source limit")
    );
}
