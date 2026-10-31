//! What each size of jungle camp may fill with.
//!
//! The dry camps' rosters are Liquipedia's camp articles (read 2026-10-18),
//! in the order of its *Neutral Creeps* tables: a camp whose map entry draws
//! from fewer rosters than its size has takes the first ones. A flooded
//! camp's creeps are not drawn: they evolve with the clock, as
//! [`flooded_creeps`] works out. Which camp stands where is the map's
//! business; this is only what it draws from.

use crate::{CampDef, CampKind, NeutralKind, rules};

/// One camp roster: the creeps a camp of that size may fill with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Roster {
    /// Which size of camp draws it.
    pub kind: CampKind,
    /// The creeps it puts out.
    pub creeps: &'static [NeutralKind],
}

/// The most creeps any roster puts out.
pub const MAX_CAMP_CREEPS: usize = 5;

const _: () = {
    let mut most = 0;
    let mut at = 0;
    while at < ROSTERS.len() {
        if ROSTERS[at].creeps.len() > most {
            most = ROSTERS[at].creeps.len();
        }
        at += 1;
    }
    assert!(most == MAX_CAMP_CREEPS);
};

/// Every roster in the jungle, grouped by camp size.
pub const ROSTERS: [Roster; 22] = [
    // Kobold Camp
    Roster {
        kind: CampKind::Small,
        creeps: &[
            NeutralKind::Kobold,
            NeutralKind::Kobold,
            NeutralKind::Kobold,
            NeutralKind::KoboldTunneler,
            NeutralKind::KoboldTaskmaster,
        ],
    },
    // Hill Troll Camp
    Roster {
        kind: CampKind::Small,
        creeps: &[
            NeutralKind::ForestTrollBerserker,
            NeutralKind::ForestTrollBerserker,
            NeutralKind::ForestTrollHighPriest,
        ],
    },
    // Hill Troll and Kobold Camp
    Roster {
        kind: CampKind::Small,
        creeps: &[
            NeutralKind::ForestTrollBerserker,
            NeutralKind::ForestTrollBerserker,
            NeutralKind::KoboldTaskmaster,
        ],
    },
    // Vhoul Assassin Camp
    Roster {
        kind: CampKind::Small,
        creeps: &[
            NeutralKind::GnollAssassin,
            NeutralKind::GnollAssassin,
            NeutralKind::GnollAssassin,
        ],
    },
    // Ghost Camp
    Roster {
        kind: CampKind::Small,
        creeps: &[
            NeutralKind::FelBeast,
            NeutralKind::FelBeast,
            NeutralKind::Ghost,
        ],
    },
    // Harpy Camp
    Roster {
        kind: CampKind::Small,
        creeps: &[
            NeutralKind::HarpyScout,
            NeutralKind::HarpyScout,
            NeutralKind::HarpyStorm,
        ],
    },
    // Centaur Camp
    Roster {
        kind: CampKind::Medium,
        creeps: &[NeutralKind::CentaurOutrunner, NeutralKind::CentaurKhan],
    },
    // Wolf Camp
    Roster {
        kind: CampKind::Medium,
        creeps: &[
            NeutralKind::GiantWolf,
            NeutralKind::GiantWolf,
            NeutralKind::AlphaWolf,
        ],
    },
    // Satyr Camp
    Roster {
        kind: CampKind::Medium,
        creeps: &[
            NeutralKind::SatyrTrickster,
            NeutralKind::SatyrTrickster,
            NeutralKind::SatyrSoulstealer,
            NeutralKind::SatyrSoulstealer,
        ],
    },
    // Ogre Camp
    Roster {
        kind: CampKind::Medium,
        creeps: &[
            NeutralKind::OgreMauler,
            NeutralKind::OgreMauler,
            NeutralKind::OgreMagi,
        ],
    },
    // Golem Camp
    Roster {
        kind: CampKind::Medium,
        creeps: &[NeutralKind::MudGolem, NeutralKind::MudGolem],
    },
    // Large Centaur Camp
    Roster {
        kind: CampKind::Large,
        creeps: &[
            NeutralKind::CentaurOutrunner,
            NeutralKind::CentaurOutrunner,
            NeutralKind::CentaurKhan,
        ],
    },
    // Large Satyr Camp
    Roster {
        kind: CampKind::Large,
        creeps: &[
            NeutralKind::SatyrTrickster,
            NeutralKind::SatyrSoulstealer,
            NeutralKind::SatyrHellcaller,
        ],
    },
    // Hellbear Camp
    Roster {
        kind: CampKind::Large,
        creeps: &[
            NeutralKind::PolarFurbolgChampion,
            NeutralKind::PolarFurbolgUrsaWarrior,
        ],
    },
    // Wildwing Camp
    Roster {
        kind: CampKind::Large,
        creeps: &[
            NeutralKind::Wildkin,
            NeutralKind::Wildkin,
            NeutralKind::EnragedWildkin,
        ],
    },
    // Troll Camp
    Roster {
        kind: CampKind::Large,
        creeps: &[
            NeutralKind::DarkTroll,
            NeutralKind::DarkTroll,
            NeutralKind::DarkTrollWarlord,
        ],
    },
    // Warpine Camp
    Roster {
        kind: CampKind::Large,
        creeps: &[NeutralKind::WarpineRaider, NeutralKind::WarpineRaider],
    },
    // Dragon Camp
    Roster {
        kind: CampKind::Ancient,
        creeps: &[
            NeutralKind::BlackDrake,
            NeutralKind::BlackDrake,
            NeutralKind::BlackDragon,
        ],
    },
    // Large Golem Camp
    Roster {
        kind: CampKind::Ancient,
        creeps: &[
            NeutralKind::RockGolem,
            NeutralKind::RockGolem,
            NeutralKind::GraniteGolem,
        ],
    },
    // Thunderhide Camp
    Roster {
        kind: CampKind::Ancient,
        creeps: &[
            NeutralKind::SmallThunderLizard,
            NeutralKind::SmallThunderLizard,
            NeutralKind::BigThunderLizard,
        ],
    },
    // Frostbitten Camp
    Roster {
        kind: CampKind::Ancient,
        creeps: &[
            NeutralKind::FrostbittenGolem,
            NeutralKind::FrostbittenGolem,
            NeutralKind::IceShaman,
        ],
    },
    // Prowler Camp
    Roster {
        kind: CampKind::Ancient,
        creeps: &[
            NeutralKind::ProwlerAcolyte,
            NeutralKind::ProwlerAcolyte,
            NeutralKind::ProwlerShaman,
        ],
    },
];

/// The rosters a dry camp draws from, with their numbers: the first
/// [`CampDef::rosters`] of its size.
pub fn rosters_of(camp: &CampDef) -> impl Iterator<Item = (u8, &'static Roster)> {
    ROSTERS
        .iter()
        .enumerate()
        .filter(move |(_, roster)| roster.kind == camp.kind)
        .take(usize::from(camp.rosters))
        .map(|(index, roster)| (index as u8, roster))
}

/// The creeps a flooded camp is made of: three, each climbing one rung of
/// its line at a time.
pub const FLOODED_CREEPS: usize = 3;

/// The rungs a flooded camp's creeps climb, by its place in the camp: two
/// Pollywogs become Boglets, Croakers and Ancient Croakers, the third a
/// Marshmage Apprentice, a Marshmage and an Ancient Marshmage.
const FLOODED_LINES: [[NeutralKind; 4]; FLOODED_CREEPS] = [
    [
        NeutralKind::Tadpole,
        NeutralKind::Froglet,
        NeutralKind::GrownFrog,
        NeutralKind::AncientFrog,
    ],
    [
        NeutralKind::Tadpole,
        NeutralKind::Froglet,
        NeutralKind::GrownFrog,
        NeutralKind::AncientFrog,
    ],
    [
        NeutralKind::Tadpole,
        NeutralKind::FrogletMage,
        NeutralKind::GrownFrogMage,
        NeutralKind::AncientFrogMage,
    ],
];

/// How many creeps of a flooded camp have evolved by `tick`: one every
/// [`rules::FLOODED_EVOLUTION_PERIOD_TICKS`] from the horn, three to each
/// whole size it may grow by.
pub const fn flooded_evolutions(camp: &CampDef, tick: u32, schedule: rules::Schedule) -> u32 {
    let since = tick.saturating_sub(schedule.horn);
    let done = since / rules::FLOODED_EVOLUTION_PERIOD_TICKS;
    let most = camp.evolutions as u32 * FLOODED_CREEPS as u32;
    if done < most { done } else { most }
}

/// What a flooded camp puts out after `evolutions` evolutions: each one
/// takes the next creep, in camp order, a rung up its line. A small camp
/// starts on the first rung, a medium one on the second.
pub const fn flooded_creeps(camp: &CampDef, evolutions: u32) -> [NeutralKind; FLOODED_CREEPS] {
    let start = match camp.kind {
        CampKind::Small => 0,
        CampKind::Medium => 1,
        CampKind::Large => 2,
        CampKind::Ancient => 3,
    };
    let whole = evolutions / FLOODED_CREEPS as u32;
    let part = evolutions % FLOODED_CREEPS as u32;
    let mut creeps = [NeutralKind::Tadpole; FLOODED_CREEPS];
    let mut at = 0;
    while at < FLOODED_CREEPS {
        let mut rung = start + whole as usize + if (at as u32) < part { 1 } else { 0 };
        if rung > 3 {
            rung = 3;
        }
        creeps[at] = FLOODED_LINES[at][rung];
        at += 1;
    }
    creeps
}
