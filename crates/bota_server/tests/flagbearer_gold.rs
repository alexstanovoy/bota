//! Independent flagbearer bounty and team reward through a real attack order.

use bota_core::{Catalogue, passive_gold_by};
use bota_proto::{
    Cheat, Fixed, HeroId, MapId, Order, OutgoingOrder, Pick, ServerMsg, SlotId, Target, Team,
    TickMode, UnitKind, Vec2,
};
use bota_server::{
    game::{MatchConfig, OrderDelay},
    local::LocalMatch,
};

fn game() -> LocalMatch {
    let config = MatchConfig {
        match_id: 314,
        master_key: [7; 32],
        stream: 314,
        picks: [Team::Radiant, Team::Dire, Team::Radiant]
            .into_iter()
            .enumerate()
            .map(|(slot, team)| Pick {
                slot: SlotId(slot as u8),
                team,
                hero: HeroId::SHADOW_FIEND,
            })
            .collect(),
        map: MapId(0),
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 150,
        cheats: true,
        spawn_modifiers: Vec::new(),
        immortal_structures: true,
        immortal_heroes: true,
        starting_gold_lost: Vec::new(),
        objectives: bota_proto::ObjectiveSet::NONE,
        runes: bota_proto::RuneSet::NONE,
        catalogue: Catalogue::FULL,
        economy_events: Vec::new(),
    };
    LocalMatch::new(&config, &[OrderDelay::NONE])
        .expect("a match")
        .0
}

fn order(game: &mut LocalMatch, order: Order) -> Vec<Vec<ServerMsg>> {
    let tick = game.tick();
    let issued = OutgoingOrder {
        seq: tick,
        unit: None,
        order,
        seen: tick,
        queue: false,
    };
    let streams = game.step(&[&[issued], &[], &[]]).expect("order step");
    assert!(
        !streams
            .iter()
            .flatten()
            .any(|message| matches!(message, ServerMsg::OrderRejected { .. })),
        "the real order is accepted"
    );
    streams
}

fn arranged_flagbearer() -> (
    LocalMatch,
    bota_server::game::Entity,
    bota_server::game::Entity,
    i32,
    bota_core::GoldRange,
) {
    let mut game = game();
    order(
        &mut game,
        Order::Cheat {
            cheat: Cheat::StartGame,
        },
    );
    let flag = (0..4000)
        .find_map(|_| {
            let flag = game.world().entities.iter().find(|entity| {
                game.world().kind.get(*entity) == Some(&UnitKind::CreepFlagbearer)
                    && game.world().team.get(*entity) == Some(&Team::Dire)
            });
            if flag.is_none() {
                game.step(&[&[], &[], &[]]).expect("clock step");
            }
            flag
        })
        .expect("the first real flagbearer wave");
    let hero = game.hero_of(0).expect("killer");
    let ally = game.hero_of(2).expect("nearby ally");
    let world = game.world_mut();
    let at = world.transform.get(flag).expect("flag position").pos;
    let promised = world.bounty.get(flag).expect("flag bounty").bonus;
    let range = world.bounty.get(flag).expect("flag bounty").gold;
    assert_eq!(
        (range.min, range.max),
        (34, 39),
        "7.41f normal flagbearer bounty"
    );
    assert!((44..=49).contains(&promised));
    world.creeps_halted = true;
    world.all_vision = true;
    world.transform.get_mut(hero).expect("hero placed").pos = at + Vec2::from_ints(-100, 0);
    world.transform.get_mut(ally).expect("ally placed").pos = at + Vec2::from_ints(-900, 0);
    world.health.get_mut(flag).expect("flag alive").hp = Fixed::from_int(1);
    game.step(&[&[], &[], &[]])
        .expect("the arranged positions become visible");
    (game, flag, ally, promised, range)
}

#[test]
fn a_flagbearer_kill_pays_its_spawn_time_bonus_to_both_nearby_allied_heroes() {
    let (mut game, flag, ally, promised, range) = arranged_flagbearer();
    let target = game.handle_of(0, flag).expect("visible flag");
    let names = [
        target,
        game.handle_of(1, flag)
            .expect("the opposite side's flag name"),
    ];
    let before: Vec<_> = game.world().seats.iter().map(|seat| seat.gold).collect();
    let last_hits = game.world().seats[0].last_hits;
    let clock = game.world().clock();
    let mut streams = order(
        &mut game,
        Order::Attack {
            target: Target::Unit(target),
        },
    );
    for _ in 0..150 {
        if !game.world().alive(flag) {
            break;
        }
        let world = game.world();
        assert!(
            world.transform.get(ally).expect("ally placed").pos.within(
                world.transform.get(flag).expect("flag placed").pos,
                Fixed::from_int(1500)
            ),
            "the ally must stay eligible while the creep moves"
        );
        streams = game.step(&[&[], &[], &[]]).expect("attack progresses");
    }
    assert!(
        !game.world().alive(flag),
        "the ordered attack kills the flagbearer"
    );
    let horn = game.world().schedule().horn;
    let passive = passive_gold_by(game.world().clock().saturating_sub(horn))
        - passive_gold_by(clock.saturating_sub(horn));
    let killer_gain = game.world().seats[0].gold - before[0] - passive;
    let ally_gain = game.world().seats[2].gold - before[2] - passive;
    assert_eq!(
        ally_gain, promised,
        "the nearby ally gets the full spawn-time bonus"
    );
    let kill_bounty = killer_gain - promised;
    assert!(
        (range.min..=range.max).contains(&kill_bounty),
        "the killer also gets its own bounty draw"
    );
    assert_eq!(game.world().seats[0].last_hits, last_hits + 1);
    assert_eq!(
        game.world().seats[1].gold - before[1] - passive,
        0,
        "the flagbearer's own team earns no gold"
    );
    assert_eq!(game.world().seats[2].last_hits, 0);
    assert_award_breakdown(&streams, names, kill_bounty, promised, killer_gain);
    eprintln!(
        "flagbearer: kill={kill_bounty}, spawn-time bonus={promised}, killer gained={killer_gain}, ally gained={ally_gain}"
    );
}

fn assert_award_breakdown(
    streams: &[Vec<ServerMsg>],
    names: [bota_proto::EntityId; 2],
    kill: i32,
    bonus: i32,
    total: i32,
) {
    use bota_proto::{EventKind, GoldAward, GoldAwardKind};
    let expected = [
        GoldAward {
            recipient: SlotId(0),
            team: Team::Radiant,
            kind: GoldAwardKind::KillBounty,
            amount: kill,
        },
        GoldAward {
            recipient: SlotId(0),
            team: Team::Radiant,
            kind: GoldAwardKind::FlagbearerBonus,
            amount: bonus,
        },
        GoldAward {
            recipient: SlotId(2),
            team: Team::Radiant,
            kind: GoldAwardKind::FlagbearerBonus,
            amount: bonus,
        },
    ];
    for (seat, name) in [(0, names[0]), (2, names[0])] {
        let EventKind::Died { bounty, awards, .. } = death(&streams[seat], name) else {
            panic!("death")
        };
        assert_eq!(*bounty, Some(total), "legacy total is unchanged");
        let awards = awards
            .as_deref()
            .expect("the earning side receives the breakdown");
        assert_eq!(awards.len(), expected.len());
        assert!(expected.iter().all(|award| awards.contains(award)));
    }
    assert!(
        matches!(
            death(&streams[1], names[1]),
            EventKind::Died {
                bounty: None,
                awards: None,
                ..
            }
        ),
        "the opposite side receives neither the income total nor the breakdown"
    );
}

fn death(stream: &[ServerMsg], id: bota_proto::EntityId) -> &bota_proto::EventKind {
    stream
        .iter()
        .find_map(|message| match message {
            ServerMsg::Events { events, .. } => events.iter().find(|event| {
                matches!(event,
            bota_proto::EventKind::Died { unit, .. } if *unit == id)
            }),
            _ => None,
        })
        .expect("the ordered flagbearer death")
}
