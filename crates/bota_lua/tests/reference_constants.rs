// Reference snapshot: numeric bot-API globals exported from the installed game, build 6975.
// An installation-local exporter enumerated every numeric registration, preserving integer values
// and the exact promoted single-precision desire values. To regenerate, export that installation's
// numeric bot-API names and values again, sort the names, and replace data/reference_constants.json
// with {"build":6975,"constants":{name:number,...}}. Keep recovery metadata outside the repository.
// BOT_MODE_NONE (0) is missing from the first export, yet the installed game registers it with the other
// BOT_MODE_ constants: it was added after a targeted check of that registration, so the table holds 357 names.
mod support;
use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, SlotId, Team};

#[test]
fn every_installed_reference_constant_is_defined_with_its_exact_value() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("data/reference_constants.json")).unwrap();
    assert_eq!(reference["build"], 6975);
    let constants = reference["constants"].as_object().unwrap();
    assert_eq!(constants.len(), 357);
    let mut source = String::from("function Think()\n");
    for (name, value) in constants {
        source.push_str(&format!("assert({name} == {value}, '{name}')\n"));
    }
    source.push_str("end");
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for (index, session) in sessions.iter().enumerate() {
        let slot = SlotId(index as u8);
        let mut team = LuaTeam::new(
            session.team(),
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
        .unwrap();
        team.add_script(
            slot,
            ScriptKind::Takeover,
            "reference_constants",
            source.as_bytes(),
        )
        .unwrap();
        let turn = team.think(session).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    }
}

#[test]
fn public_mode_ids_are_not_private_scheduler_indices() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut team = LuaTeam::new(
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
    .unwrap();
    team.add_script(SlotId(0), ScriptKind::Mode(8), "mid_push", b"function GetDesire() return 1 end; function Think() assert(GetBot():GetActiveMode()==BOT_MODE_PUSH_TOWER_MID and BOT_MODE_PUSH_TOWER_MID==9) end").unwrap();
    let turn = team.think(&sessions[0]).unwrap();
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}

#[test]
fn reference_constants_are_writable_globals_without_changing_host_facts() {
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    for (index, session) in sessions.iter().enumerate() {
        let slot = SlotId(index as u8);
        let mut team = LuaTeam::new(
            session.team(),
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
        .unwrap();
        team.add_script(
            slot,
            ScriptKind::Takeover,
            "constant_assignment",
            br#"function Think()
            local actual=GetTeam()
            _G.BOT_MODE_WARD=99; assert(BOT_MODE_WARD==99)
            _G.BOT_ACTION_DESIRE_HIGH=-7; assert(BOT_ACTION_DESIRE_HIGH==-7)
            rawset(_G,'TEAM_RADIANT',77); assert(TEAM_RADIANT==77)
            assert(GetTeam()==actual)
        end"#,
        )
        .unwrap();
        let turn = team.think(session).unwrap();
        assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    }
}
