//! What each size of jungle camp may fill with.
//!
//! Generated. Which camp stands where is the map's business; this is only what
//! it draws from.

use crate::game::CampKind;
use crate::game::NeutralKind;

/// One camp roster: the creeps a camp of that size may fill with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Roster {
    /// Which size of camp draws it.
    pub kind: CampKind,
    /// The creeps it puts out.
    pub creeps: &'static [NeutralKind],
}

/// Every roster in the jungle, grouped by camp size.
pub const ROSTERS: [Roster; 21] = [
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
];

/// The rosters a camp of this size draws from, with their numbers.
pub fn rosters_of(kind: CampKind) -> impl Iterator<Item = (u8, &'static Roster)> {
    ROSTERS
        .iter()
        .enumerate()
        .filter(move |(_, roster)| roster.kind == kind)
        .map(|(index, roster)| (index as u8, roster))
}
