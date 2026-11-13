#![cfg(all(feature = "native", target_os = "linux"))]
mod support;

use bota_lua::{
    BotSeat, Keys, Limits, ModuleBank,
    native::{HeroSelection, NativeConfig, NativeTeam},
};
use bota_proto::{HeroId, SlotId, Team};

#[test]
fn a_fixed_hero_runs_its_pack_without_executing_the_draft_or_foreign_hero() {
    let mut modules = ModuleBank::new(Limits::DEFAULT);
    modules
        .insert(
            "hero_selection",
            b"error('draft hook must not run for a fixed hero')",
        )
        .expect("selection");
    modules
        .insert("bot_pudge", b"error('foreign hero must not run')")
        .expect("foreign");
    modules.insert("bot_nevermore", b"function Think() local b=GetBot() assert(b:GetUnitName()=='npc_dota_hero_nevermore') b:Action_MoveToLocation(Vector(0,0,0)) end").expect("configured");
    let seat = BotSeat {
        slot: SlotId(0),
        hero: HeroId::SHADOW_FIEND,
    };
    let mut config = NativeConfig::new(
        Team::Radiant,
        vec![seat],
        Keys {
            hash: [17, 19],
            random: 23,
        },
    );
    config.hero_selection = HeroSelection::Fixed;
    config.think_timeout = std::time::Duration::from_secs(10);
    let mut worker = NativeTeam::new(config, modules).expect("fixed setup bypasses the draft");
    assert_eq!(worker.seats(), &[seat]);
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let turn = worker.think(&sessions[0]).expect("turn");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
    assert_eq!(turn.bots[0].orders.len(), 1);
}
