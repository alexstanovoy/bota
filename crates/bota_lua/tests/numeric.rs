mod support;

use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{Fixed, HeroId, ServerMsg, SlotId, Team};

#[test]
fn attack_estimates_reject_out_of_range_observations_and_recover() {
    let (_, mut sessions, _) = support::durable_creep_encounter(HeroId::SHADOW_FIEND);
    let session = &mut sessions[0];
    let original = session.current().expect("view").clone();
    let source = session.own_hero().expect("hero").id;
    let mut view = original.clone();
    view.tick += 1;
    let tick = view.tick;
    let own = view
        .units
        .iter_mut()
        .find(|unit| unit.id == source)
        .expect("hero");
    own.attack_damage = i32::MAX;
    own.attack_damage_min = i32::MAX;
    own.attack_damage_max = i32::MAX;
    own.armor = Fixed::MIN;
    session
        .ingest(ServerMsg::Snapshot { view }, &mut ())
        .expect("received statistics");
    session
        .ingest(
            ServerMsg::Events {
                tick,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
    let mut lua = LuaTeam::new(
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
    .expect("team");
    lua.add_script(SlotId(0), ScriptKind::Takeover, "projection", b"function Think() local b=GetBot() b:Action_MoveToLocation(Vector(10,20,0)) assert(b:GetEstimatedDamageToTarget(false,b,1,DAMAGE_TYPE_PHYSICAL)>=0) end").expect("script");
    let rejected = lua.think(session).expect("bounded projection failure");
    assert!(rejected.bots[0].orders.is_empty());
    assert_eq!(rejected.failures.len(), 1);
    assert!(
        matches!(&rejected.failures[0].error, luagee::Error::Runtime { message, .. } if message.contains("statistics"))
    );
    let mut restored = original;
    restored.tick = tick + 1;
    session
        .ingest(ServerMsg::Snapshot { view: restored }, &mut ())
        .expect("valid statistics");
    session
        .ingest(
            ServerMsg::Events {
                tick: tick + 1,
                events: Vec::new(),
            },
            &mut (),
        )
        .expect("events");
    let recovered = lua.think(session).expect("recovery");
    assert!(recovered.failures.is_empty(), "{:?}", recovered.failures);
    assert_eq!(recovered.bots[0].orders.len(), 1);
}
