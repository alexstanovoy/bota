//! Re-executes recorded seat views without sockets, deadlines or a game simulation.
use crate::{BotSeat, Keys, LuaTeam, ModuleBank, TeamTurn};
use bota_client_core::{ClientSession, Ingested, MatchStatic, ReplayPlayer, SessionOptions};
use bota_proto::{MatchInfo, ServerMsg, SlotId, Team};
use std::io;
#[derive(Default)]
pub struct RefeedOutcome {
    pub statistics: crate::statistics::RunStatistics,
    pub winner: Option<Team>,
    pub ticks: u32,
    pub orders: u64,
}
struct Refeed {
    session: Option<ClientSession>,
    host: Option<LuaTeam>,
    modules: Option<ModuleBank>,
    slot: SlotId,
    keys: Keys,
    selection: super::HeroSelection,
    last: Option<u32>,
    outcome: RefeedOutcome,
}
/// Reads this build's recorded filtered seat views through the existing bounded replay reader.
/// Fogless recordings cannot reconstruct the facts a seat originally knew and are refused.
pub fn refeed(
    mut player: ReplayPlayer,
    slot: SlotId,
    modules: ModuleBank,
    keys: Keys,
    selection: super::HeroSelection,
    mut orders: impl FnMut(u32, &TeamTurn),
) -> io::Result<RefeedOutcome> {
    player.require_current_wire()?;
    let mut run = Refeed {
        session: None,
        host: None,
        modules: Some(modules),
        slot,
        keys,
        selection,
        last: None,
        outcome: Default::default(),
    };
    while !player.finished() {
        player.advance_ticks(f64::from(u32::MAX));
        for message in player.poll(0.0) {
            run.ingest(message, &mut orders)?;
        }
        if let Some(error) = player.error() {
            return Err(io::Error::other(error.to_string()));
        }
    }
    if run.last.is_none() {
        return Err(io::Error::other(
            "recording contains no complete seat turns",
        ));
    }
    Ok(run.outcome)
}
impl Refeed {
    fn start(&mut self, info: &MatchInfo) -> io::Result<()> {
        if self.session.is_some() {
            return Err(io::Error::other("repeated match start"));
        }
        let pick = info
            .picks
            .iter()
            .find(|pick| pick.slot == self.slot)
            .ok_or_else(|| io::Error::other("recording has no selected seat"))?;
        self.session = Some(
            ClientSession::seat(
                MatchStatic::new(info).map_err(failed)?,
                self.slot,
                Some(pick.hero),
                SessionOptions::default(),
            )
            .map_err(failed)?,
        );
        let mut config = super::NativeConfig::new(
            pick.team,
            vec![BotSeat {
                slot: self.slot,
                hero: pick.hero,
            }],
            self.keys,
        );
        config.statistics = true;
        config.hero_selection = self.selection;
        let (host, seats) =
            super::worker::build(config, self.modules.take().expect("first start"), None)
                .map_err(failed)?;
        if seats[0].hero != pick.hero {
            return Err(io::Error::other(
                "script selection differs from recorded seat",
            ));
        }
        self.host = Some(host);
        Ok(())
    }
    fn ingest(
        &mut self,
        message: ServerMsg,
        orders: &mut impl FnMut(u32, &TeamTurn),
    ) -> io::Result<()> {
        if let ServerMsg::MatchStart { info } = &message {
            return self.start(info);
        }
        if let ServerMsg::Snapshot { view } = &message
            && view.viewer != self.session.as_ref().map(ClientSession::team)
        {
            return Err(io::Error::other(
                "re-feed requires recorded filtered seat views; fogless replay is not a seat recording",
            ));
        }
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| io::Error::other("recording needs match start before views"))?;
        match session.ingest(message, &mut ()).map_err(failed)? {
            Ingested::TickComplete { tick, .. } if self.last != Some(tick) => {
                self.think(tick, orders)?
            }
            Ingested::Over { winner, .. } => self.outcome.winner = Some(winner),
            _ => {}
        }
        Ok(())
    }
    fn think(&mut self, tick: u32, orders: &mut impl FnMut(u32, &TeamTurn)) -> io::Result<()> {
        self.last = Some(tick);
        let session = self.session.as_mut().expect("started session");
        let began = crate::statistics::cpu_now();
        let snapshot = crate::state::Snapshot::capture(session).map_err(failed)?;
        let capture = crate::statistics::cpu_now().saturating_sub(began);
        let host = self.host.as_mut().expect("started host");
        let mut turn =
            super::worker::guarded_call(|| host.think_snapshot(snapshot)).map_err(failed)?;
        if let Some(data) = &mut turn.statistics {
            data.capture_ns = capture;
        }
        super::worker::record_turn(&mut self.outcome.statistics, &turn);
        self.outcome.ticks = tick;
        self.outcome.orders = self.outcome.orders.saturating_add(
            turn.bots
                .iter()
                .map(|bot| bot.orders.len() as u64)
                .sum::<u64>(),
        );
        orders(tick, &turn);
        session.end_turn();
        Ok(())
    }
}
fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}
