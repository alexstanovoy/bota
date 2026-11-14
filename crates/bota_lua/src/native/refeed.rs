//! Re-executes a recorded seat stream without sockets, deadlines or a game simulation.
use crate::{BotSeat, Keys, ModuleBank, TeamTurn};
use bota_client_core::{ClientSession, Ingested, MatchStatic, ReplayPlayer, SessionOptions};
use bota_proto::{ServerMsg, SlotId, Team};
use std::io;

pub struct RefeedOutcome {
    pub statistics: crate::statistics::RunStatistics,
    pub winner: Option<Team>,
    pub ticks: u32,
    pub orders: u64,
}
/// The existing replay reader bounds decoding and decompression. Only a recorded seat's filtered
/// views are accepted: a fogless recording cannot reconstruct the facts that seat originally knew.
pub fn refeed(
    mut player: ReplayPlayer,
    slot: SlotId,
    modules: ModuleBank,
    keys: Keys,
    selection: super::HeroSelection,
    mut orders: impl FnMut(u32, &TeamTurn),
) -> io::Result<RefeedOutcome> {
    let mut outcome = RefeedOutcome {
        statistics: Default::default(),
        winner: None,
        ticks: 0,
        orders: 0,
    };
    let mut session: Option<ClientSession> = None;
    let mut lua = None;
    let mut modules = Some(modules);
    let mut last = None;
    while !player.finished() {
        player.advance_ticks(f64::from(u32::MAX));
        for message in player.poll(0.0) {
            if let ServerMsg::MatchStart { info } = &message {
                if session.is_some() {
                    return Err(io::Error::other("repeated match start"));
                }
                let pick = info
                    .picks
                    .iter()
                    .find(|pick| pick.slot == slot)
                    .ok_or_else(|| io::Error::other("recording has no selected seat"))?;
                session = Some(
                    ClientSession::seat(
                        MatchStatic::new(info)
                            .map_err(|error| io::Error::other(error.to_string()))?,
                        slot,
                        Some(pick.hero),
                        SessionOptions::default(),
                    )
                    .map_err(|error| io::Error::other(error.to_string()))?,
                );
                let mut config = super::NativeConfig::new(
                    pick.team,
                    vec![BotSeat {
                        slot,
                        hero: pick.hero,
                    }],
                    keys,
                );
                config.statistics = true;
                config.hero_selection = selection;
                let (host, seats) =
                    super::worker::build(config, modules.take().expect("first start"), None)
                        .map_err(io::Error::other)?;
                if seats[0].hero != pick.hero {
                    return Err(io::Error::other(
                        "script selection differs from recorded seat",
                    ));
                }
                lua = Some(host);
                continue;
            }
            if let ServerMsg::Snapshot { view } = &message
                && view.viewer != session.as_ref().map(ClientSession::team)
            {
                return Err(io::Error::other(
                    "re-feed requires recorded filtered seat views; fogless replay is not a seat recording",
                ));
            }
            let session = session
                .as_mut()
                .ok_or_else(|| io::Error::other("recording needs match start before views"))?;
            match session
                .ingest(message, &mut ())
                .map_err(|error| io::Error::other(error.to_string()))?
            {
                Ingested::TickComplete { tick, .. } if last != Some(tick) => {
                    last = Some(tick);
                    let began = crate::statistics::cpu_now();
                    let snapshot = crate::state::Snapshot::capture(session)
                        .map_err(|error| io::Error::other(error.to_string()))?;
                    let capture = crate::statistics::cpu_now().saturating_sub(began);
                    let mut turn = lua
                        .as_mut()
                        .expect("started host")
                        .think_snapshot(snapshot)
                        .map_err(|error| io::Error::other(error.to_string()))?;
                    if let Some(data) = &mut turn.statistics {
                        data.capture_ns = capture;
                    }
                    super::worker::record_turn(&mut outcome.statistics, &turn);
                    outcome.ticks = tick;
                    outcome.orders = outcome.orders.saturating_add(
                        turn.bots
                            .iter()
                            .map(|bot| bot.orders.len() as u64)
                            .sum::<u64>(),
                    );
                    orders(tick, &turn);
                    session.end_turn();
                }
                Ingested::Over { winner, .. } => outcome.winner = Some(winner),
                _ => {}
            }
        }
        if let Some(error) = player.error() {
            return Err(io::Error::other(error.to_string()));
        }
    }
    if last.is_none() {
        return Err(io::Error::other(
            "recording contains no complete seat turns",
        ));
    }
    Ok(outcome)
}
