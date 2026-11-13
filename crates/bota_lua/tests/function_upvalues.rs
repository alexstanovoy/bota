//! How many outer locals one script function may capture in the host: up to 120, as the reference game's script runtime
//! accepts (game build 6975) and scripts written for it rely on, and a function that captures more is refused with the
//! interpreter's own diagnostic. The host's fuel and memory budgets hold for such scripts as for any other.
#![cfg(all(feature = "native", not(target_arch = "wasm32")))]

mod support;

use std::time::Duration;

use bota_lua::native::{NativeConfig, NativeError, NativeTeam};
use bota_lua::{BotSeat, Keys, Limits, ModuleBank};
use bota_proto::{HeroId, SlotId, Team};

fn config() -> NativeConfig {
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
    config.think_timeout = Duration::from_secs(10);
    config
}

/// A bot script whose `Think` captures `captured` locals of its chunk, adds them up and checks the sum.
fn capturing(captured: usize) -> String {
    let names: Vec<String> = (1..=captured).map(|i| format!("u{i}")).collect();
    let values: Vec<String> = (1..=captured).map(|i| i.to_string()).collect();
    let adds: Vec<String> = names
        .iter()
        .map(|name| format!("total = total + {name}"))
        .collect();
    let declaration = format!("{} = {}", names.join(", "), values.join(", "));
    format!(
        "local {declaration}\nfunction Think()\nlocal total = 0\n{}\nassert(total == {})\nGetBot():Action_MoveToLocation(Vector(0, 0, 0))\nend\n",
        adds.join("\n"),
        captured * (captured + 1) / 2
    )
}

fn modules(script: &str) -> ModuleBank {
    let mut modules = ModuleBank::new(Limits::DEFAULT);
    modules
        .insert("bots/bot_nevermore", script.as_bytes())
        .expect("script");
    modules
}

#[test]
fn a_script_function_capturing_from_61_to_120_outer_locals_loads_and_runs_in_the_host() {
    for captured in [60, 61, 90, 120] {
        let mut team = NativeTeam::new(config(), modules(&capturing(captured)))
            .unwrap_or_else(|error| panic!("{captured} captured locals: {error}"));
        let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
        let turn = team.think(&sessions[0]).expect("think");
        assert!(turn.failures.is_empty(), "{captured}: {:?}", turn.failures);
        assert!(
            !turn.bots[0].orders.is_empty(),
            "{captured}: the function ran to its order"
        );
    }
}

#[test]
fn a_script_function_capturing_121_outer_locals_is_refused_with_the_interpreters_diagnostic() {
    let refused = NativeTeam::new(config(), modules(&capturing(121))).err();
    let message = match refused {
        Some(NativeError::Setup(message)) => message,
        other => panic!("expected a setup error, got {other:?}"),
    };
    assert!(
        message.contains("function at line 2 has more than 120 upvalues"),
        "{message}"
    );
}

#[test]
fn fuel_still_ends_a_script_that_makes_closures_of_120_upvalues_without_end() {
    let names: Vec<String> = (1..=120).map(|i| format!("u{i}")).collect();
    let uses: Vec<String> = names
        .iter()
        .map(|name| format!("{name} = {name}"))
        .collect();
    let script = format!(
        "local {} = {}\nfunction Think()\nwhile true do local f = function() {} end end\nend\n",
        names.join(", "),
        vec!["0"; 120].join(", "),
        uses.join(" ")
    );
    let mut team = NativeTeam::new(config(), modules(&script)).expect("worker");
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let turn = team.think(&sessions[0]).expect("think");
    assert_eq!(turn.failures.len(), 1, "{:?}", turn.failures);
    assert!(
        format!("{:?}", turn.failures[0]).contains("Limit(FuelPerCall)"),
        "{:?}",
        turn.failures[0]
    );
    assert!(!team.retired(), "a bot out of fuel does not lose the team");
}

#[test]
fn keeping_closures_of_120_upvalues_ends_at_a_host_budget_and_the_team_survives() {
    let names: Vec<String> = (1..=120).map(|i| format!("u{i}")).collect();
    let uses: Vec<String> = names
        .iter()
        .map(|name| format!("{name} = {name}"))
        .collect();
    // Each call keeps 500 more closures of some 2 KB each in a global table, well inside the call's fuel; what ends the
    // script is the budget the growing set exhausts first: its memory cap, or the fuel the collector's walks over the
    // closures' cells take (a cell is a quarter fuel).
    let script = format!(
        "local {} = {}\nkept = {{}}\nfunction Think()\nfor i = 1, 500 do kept[#kept + 1] = function() {} end end\nend\n",
        names.join(", "),
        vec!["0"; 120].join(", "),
        uses.join(" ")
    );
    // A team's own setup takes some 24 MB; the cap leaves a few calls' worth of closures.
    let mut config = config();
    config.limits = config.limits.with_memory(32 << 20);
    let mut team = NativeTeam::new(config, modules(&script)).expect("worker");
    let (mut local, mut sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut ended = None;
    for call in 0..60 {
        let turn = team.think(&sessions[0]).expect("think");
        if let Some(failure) = turn.failures.first() {
            ended = Some((call, format!("{failure:?}")));
            break;
        }
        let streams = local.step(&[&[], &[]]).expect("step");
        for (session, stream) in sessions.iter_mut().zip(streams) {
            support::fold(session, stream);
        }
    }
    let (call, failure) = ended.expect("a host budget ends the script");
    assert!(call > 0, "the first call fits the budgets: {failure}");
    assert!(
        failure.contains("Limit(Memory)") || failure.contains("Limit(FuelPerCall)"),
        "{failure}"
    );
    assert!(
        !team.retired(),
        "a bot out of budget does not lose the team"
    );
}
