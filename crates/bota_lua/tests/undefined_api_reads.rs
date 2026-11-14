mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind, statistics::RunStatistics};
use bota_proto::{HeroId, SlotId, Team};

fn run(measured: bool, source: &[u8]) -> bota_lua::TeamTurn {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut team = LuaTeam::new_measured(
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
        measured,
    )
    .unwrap();
    team.add_script(SlotId(0), ScriptKind::Takeover, "undefined_reads", source)
        .unwrap();
    team.think(&sessions[0]).unwrap()
}
#[test]
fn undefined_api_reads_keep_nil_and_collect_each_name_even_when_calls_are_caught() {
    let source = br#"
local initialized = BOT_UNDEFINED_DURING_INIT
function Think()
 assert(initialized==nil and BOT_UNDEFINED_DURING_TURN==nil)
 assert(ordinary_optional_variable==nil)
 assert(not pcall(function() GetMissingGlobal() end))
 assert(not pcall(function() GetBot():GetMissingUnit() end))
 assert(not pcall(function() GetBot():GetAbilityByName('nevermore_shadowraze1'):GetMissingAbility() end))
 assert(GetMissingGlobal==nil)
 assert(rawget(_G,"GetRawMissing")==nil)
 assert(rawget(CDOTA_Bot_Script,"GetRawMissingUnit")==nil)
 assert(rawget(CDOTABaseAbility_BotScript,"GetRawMissingAbility")==nil)
 GetBot():Action_MoveToLocation(Vector(100,0,0))
end"#;
    let plain = run(false, source);
    let measured = run(true, source);
    assert!(plain.failures.is_empty() && measured.failures.is_empty());
    assert_eq!(plain.bots[0].orders, measured.bots[0].orders);
    let stats = measured.statistics.as_ref().unwrap();
    let entries = &stats.undefined_api_reads.names;
    for name in [
        "BOT_UNDEFINED_DURING_INIT",
        "BOT_UNDEFINED_DURING_TURN",
        "GetMissingGlobal",
        "GetMissingUnit",
        "GetMissingAbility",
        "GetRawMissing",
        "GetRawMissingUnit",
        "GetRawMissingAbility",
    ] {
        assert!(
            entries.iter().any(|entry| entry.name == name),
            "{name}: {entries:?}"
        );
    }
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry.name == "GetMissingGlobal")
            .unwrap()
            .reads,
        2
    );
    assert!(
        !entries
            .iter()
            .any(|entry| entry.name == "ordinary_optional_variable")
    );
    let mut summary = RunStatistics::default();
    summary.record(stats);
    summary.record(stats);
    assert_eq!(
        summary.undefined_api_reads.names,
        stats.undefined_api_reads.names
    );
    assert!(
        serde_json::to_string(&summary)
            .unwrap()
            .contains("GetMissingAbility")
    );
}
#[test]
fn hostile_undefined_names_have_bounded_storage_and_escaped_summary_output() {
    let turn = run(
        true,
        br#"function Think()
      assert(_G['BOT_'..string.rep('x',600)]==nil)
      assert(_G['BOT_BAD\"\\\nNAME']==nil)
      for i=1,150 do local value=_G['BOT_MISSING_'..i]; assert(value==nil) end
    end"#,
    );
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    let names = &turn.statistics.as_ref().unwrap().undefined_api_reads;
    assert_eq!(names.names.len(), 128);
    assert_eq!(names.other_reads, 24);
    assert!(names.names.iter().all(|entry| entry.name.len() <= 64));
    assert!(serde_json::to_vec(names).unwrap().len() < 16384);
}

#[test]
fn an_uncaught_missing_call_is_reported_without_replacing_the_nil_error() {
    let turn = run(true, b"function Think() GetUncaughtMissing() end");
    assert_eq!(turn.failures.len(), 1);
    assert!(
        turn.failures[0]
            .error
            .to_string()
            .contains("GetUncaughtMissing")
    );
    assert!(turn.bots[0].orders.is_empty());
    assert!(
        turn.statistics
            .unwrap()
            .undefined_api_reads
            .names
            .iter()
            .any(|entry| entry.name == "GetUncaughtMissing" && entry.reads == 1)
    );
}

#[test]
fn scripts_keep_their_own_global_metatable_and_identical_orders_when_measured() {
    let source = br#"
        assert(getmetatable(_G)==nil)
        local reads,writes=0,0
        local meta={__index=function(t,k) reads=reads+1; if k=='GetLazyValue' then return 19 end end,
                    __newindex=function(t,k,v) writes=writes+v end}
        assert(setmetatable(_G,meta)==_G and getmetatable(_G)==meta)
        function Think()
            assert(GetLazyValue==19)
            _G.NewUserValue=3
            assert(writes==3 and rawget(_G,'NewUserValue')==nil)
            assert(BOT_MISSING_WITH_USER_METATABLE==nil)
            local count=0; for k,v in pairs(_G) do count=count+1 end
            GetBot():Action_MoveToLocation(Vector(reads,writes+count,0))
        end"#;
    let plain = run(false, source);
    let measured = run(true, source);
    assert!(plain.failures.is_empty(), "{:?}", plain.failures);
    assert!(measured.failures.is_empty(), "{:?}", measured.failures);
    assert_eq!(plain.bots[0].orders, measured.bots[0].orders);
    let names = measured.statistics.unwrap().undefined_api_reads.names;
    assert!(!names.iter().any(|entry| entry.name == "GetLazyValue"));
    assert!(
        names
            .iter()
            .any(|entry| entry.name == "BOT_MISSING_WITH_USER_METATABLE")
    );
}
