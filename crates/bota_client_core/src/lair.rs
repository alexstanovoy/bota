//! Boss state inferred from public deaths, team evidence and the declared respawn window.
use crate::ClientSession;
use bota_proto::{ObjectiveKind, Team, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LairState {
    Alive,
    Dead { remaining: u32 },
    Possible { remaining: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LairStatus {
    pub objective: ObjectiveKind,
    pub state: LairState,
    /// The publicly scheduled pit; a journey's hidden progress is not inferred.
    pub home: Vec2,
}

impl ClientSession {
    /// Uses one team's evidence, or both teams' union for a full spectator.
    /// Time advances only with authoritative snapshots, including through pauses.
    pub fn lair_status(&self, index: u8, viewer: Option<Team>) -> Option<LairStatus> {
        let view = self.current()?;
        let known = view.lairs.iter().find(|l| l.lair == index)?;
        let def = self.map().lairs.get(usize::from(index))?;
        let proof = match viewer {
            Some(team) => known.alive_known_by.by(team),
            None => known.alive_known_by.radiant || known.alive_known_by.dire,
        };
        let now = view.game_tick();
        let state = match known.fell_at {
            _ if proof => LairState::Alive,
            None => LairState::Alive,
            Some(fall) => {
                let since = now.saturating_sub(fall);
                let early = def.respawn[0].saturating_mul(bota_core::rules::TICKS_PER_SECOND);
                let late = def.respawn[1].saturating_mul(bota_core::rules::TICKS_PER_SECOND);
                if since < early {
                    LairState::Dead {
                        remaining: early - since,
                    }
                } else if since < late {
                    LairState::Possible {
                        remaining: late - since,
                    }
                } else {
                    LairState::Alive
                }
            }
        };
        Some(LairStatus {
            objective: def.objective,
            state,
            home: def.home_at(
                now,
                bota_core::rules::Schedule::of(self.statics().metadata().pregame_ticks),
            ),
        })
    }
}
