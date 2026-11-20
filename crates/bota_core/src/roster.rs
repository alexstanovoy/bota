//! Dry-camp rosters for maps that use the legacy spawn ranges.
//!
//! The compositions for side-specific draws are defined separately and held until
//! the matching map ranges and the new unit art are available.

use crate::{CampDef, CampKind, NeutralKind, rules};

/// One camp roster: the creeps a camp of that size may fill with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Roster {
    /// Which size of camp draws it.
    pub kind: CampKind,
    /// The creeps it puts out.
    pub creeps: &'static [NeutralKind],
    pub offered: bool,
}

/// Capacity for a camp roster, including the five-creep compositions used by older maps.
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
    assert!(most <= MAX_CAMP_CREEPS);
};

/// Every roster in the jungle, grouped by camp size.
pub const ROSTERS: [Roster; 22] = [
    // Kobold Camp
    Roster {
        offered: true,
        kind: CampKind::Small,
        creeps: &[
            NeutralKind::Kobold,
            NeutralKind::Kobold,
            NeutralKind::KoboldTunneler,
            NeutralKind::KoboldTaskmaster,
        ],
    },
    // Hill Troll Camp
    Roster {
        offered: true,
        kind: CampKind::Small,
        creeps: &[
            NeutralKind::ForestTrollBerserker,
            NeutralKind::ForestTrollBerserker,
            NeutralKind::ForestTrollHighPriest,
        ],
    },
    // Hill Troll and Kobold Camp
    Roster {
        offered: true,
        kind: CampKind::Small,
        creeps: &[
            NeutralKind::ForestTrollBerserker,
            NeutralKind::ForestTrollBerserker,
            NeutralKind::KoboldTaskmaster,
        ],
    },
    // Vhoul Assassin Camp
    Roster {
        offered: true,
        kind: CampKind::Small,
        creeps: &[
            NeutralKind::GnollAssassin,
            NeutralKind::GnollAssassin,
            NeutralKind::GnollAssassin,
        ],
    },
    // Ghost Camp
    Roster {
        offered: true,
        kind: CampKind::Small,
        creeps: &[
            NeutralKind::FelBeast,
            NeutralKind::FelBeast,
            NeutralKind::Ghost,
        ],
    },
    // Harpy Camp
    Roster {
        offered: true,
        kind: CampKind::Small,
        creeps: &[
            NeutralKind::HarpyScout,
            NeutralKind::HarpyScout,
            NeutralKind::HarpyStorm,
        ],
    },
    // Centaur Camp
    Roster {
        offered: false,
        kind: CampKind::Medium,
        creeps: &[NeutralKind::CentaurOutrunner, NeutralKind::CentaurKhan],
    },
    // Wolf Camp
    Roster {
        offered: true,
        kind: CampKind::Medium,
        creeps: &[
            NeutralKind::GiantWolf,
            NeutralKind::GiantWolf,
            NeutralKind::AlphaWolf,
        ],
    },
    // Satyr Camp
    Roster {
        offered: true,
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
        offered: true,
        kind: CampKind::Medium,
        creeps: &[
            NeutralKind::OgreMauler,
            NeutralKind::OgreMauler,
            NeutralKind::OgreMagi,
        ],
    },
    // Golem Camp
    Roster {
        offered: true,
        kind: CampKind::Medium,
        creeps: &[NeutralKind::MudGolem, NeutralKind::MudGolem],
    },
    // Large Centaur Camp
    Roster {
        offered: true,
        kind: CampKind::Large,
        creeps: &[
            NeutralKind::CentaurOutrunner,
            NeutralKind::CentaurOutrunner,
            NeutralKind::CentaurKhan,
        ],
    },
    // Large Satyr Camp
    Roster {
        offered: true,
        kind: CampKind::Large,
        creeps: &[
            NeutralKind::SatyrSoulstealer,
            NeutralKind::SatyrSoulstealer,
            NeutralKind::SatyrHellcaller,
        ],
    },
    // Hellbear Camp
    Roster {
        offered: true,
        kind: CampKind::Large,
        creeps: &[
            NeutralKind::PolarFurbolgChampion,
            NeutralKind::PolarFurbolgUrsaWarrior,
        ],
    },
    // Wildwing Camp
    Roster {
        offered: true,
        kind: CampKind::Large,
        creeps: &[
            NeutralKind::Wildkin,
            NeutralKind::Wildkin,
            NeutralKind::EnragedWildkin,
        ],
    },
    // Troll Camp
    Roster {
        offered: true,
        kind: CampKind::Large,
        creeps: &[
            NeutralKind::DarkTroll,
            NeutralKind::DarkTroll,
            NeutralKind::DarkTrollWarlord,
        ],
    },
    // Warpine Camp
    Roster {
        offered: false,
        kind: CampKind::Large,
        creeps: &[NeutralKind::WarpineRaider, NeutralKind::WarpineRaider],
    },
    // Dragon Camp
    Roster {
        offered: true,
        kind: CampKind::Ancient,
        creeps: &[
            NeutralKind::BlackDrake,
            NeutralKind::BlackDrake,
            NeutralKind::BlackDragon,
        ],
    },
    // Large Golem Camp
    Roster {
        offered: true,
        kind: CampKind::Ancient,
        creeps: &[
            NeutralKind::RockGolem,
            NeutralKind::RockGolem,
            NeutralKind::GraniteGolem,
        ],
    },
    // Thunderhide Camp
    Roster {
        offered: false,
        kind: CampKind::Ancient,
        creeps: &[
            NeutralKind::SmallThunderLizard,
            NeutralKind::SmallThunderLizard,
            NeutralKind::BigThunderLizard,
        ],
    },
    // Frostbitten Camp
    Roster {
        offered: true,
        kind: CampKind::Ancient,
        creeps: &[
            NeutralKind::FrostbittenGolem,
            NeutralKind::FrostbittenGolem,
            NeutralKind::IceShaman,
        ],
    },
    // Prowler Camp
    Roster {
        offered: true,
        kind: CampKind::Ancient,
        creeps: &[
            NeutralKind::ProwlerAcolyte,
            NeutralKind::ProwlerAcolyte,
            NeutralKind::ProwlerShaman,
        ],
    },
];

/// The rosters a dry camp draws from, with their numbers: those of its size
/// numbered [`CampDef::rosters`] among them, counted from one.
pub fn rosters_of(camp: &CampDef) -> impl Iterator<Item = (u8, &'static Roster)> {
    let (first, last) = camp.rosters;
    let skipped = usize::from(first.saturating_sub(1));
    ROSTERS
        .iter()
        .enumerate()
        .filter(move |(_, roster)| roster.kind == camp.kind)
        .skip(skipped)
        .take(usize::from(last).saturating_sub(skipped))
        .filter(|(_, roster)| {
            roster.offered && roster.creeps.iter().all(|kind| kind.neutral().offered)
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MAPS, PATCH_CAMP_ROSTERS};

    #[test]
    fn every_legacy_roster_keeps_its_order_composition_and_eligibility() {
        use CampKind::{Ancient, Large, Medium, Small};
        use NeutralKind::*;

        let expected: [(CampKind, bool, &[NeutralKind]); 22] = [
            // Kobold Camp
            (
                Small,
                true,
                &[Kobold, Kobold, KoboldTunneler, KoboldTaskmaster],
            ),
            // Hill Troll Camp
            (
                Small,
                true,
                &[
                    ForestTrollBerserker,
                    ForestTrollBerserker,
                    ForestTrollHighPriest,
                ],
            ),
            // Hill Troll and Kobold Camp
            (
                Small,
                true,
                &[ForestTrollBerserker, ForestTrollBerserker, KoboldTaskmaster],
            ),
            // Vhoul Assassin Camp
            (Small, true, &[GnollAssassin, GnollAssassin, GnollAssassin]),
            // Ghost Camp
            (Small, true, &[FelBeast, FelBeast, Ghost]),
            // Harpy Camp
            (Small, true, &[HarpyScout, HarpyScout, HarpyStorm]),
            // Centaur Camp
            (Medium, false, &[CentaurOutrunner, CentaurKhan]),
            // Wolf Camp
            (Medium, true, &[GiantWolf, GiantWolf, AlphaWolf]),
            // Satyr Camp
            (
                Medium,
                true,
                &[
                    SatyrTrickster,
                    SatyrTrickster,
                    SatyrSoulstealer,
                    SatyrSoulstealer,
                ],
            ),
            // Ogre Camp
            (Medium, true, &[OgreMauler, OgreMauler, OgreMagi]),
            // Golem Camp
            (Medium, true, &[MudGolem, MudGolem]),
            // Large Centaur Camp
            (
                Large,
                true,
                &[CentaurOutrunner, CentaurOutrunner, CentaurKhan],
            ),
            // Large Satyr Camp
            (
                Large,
                true,
                &[SatyrSoulstealer, SatyrSoulstealer, SatyrHellcaller],
            ),
            // Hellbear Camp
            (
                Large,
                true,
                &[PolarFurbolgChampion, PolarFurbolgUrsaWarrior],
            ),
            // Wildwing Camp
            (Large, true, &[Wildkin, Wildkin, EnragedWildkin]),
            // Troll Camp
            (Large, true, &[DarkTroll, DarkTroll, DarkTrollWarlord]),
            // Warpine Camp
            (Large, false, &[WarpineRaider, WarpineRaider]),
            // Dragon Camp
            (Ancient, true, &[BlackDrake, BlackDrake, BlackDragon]),
            // Large Golem Camp
            (Ancient, true, &[RockGolem, RockGolem, GraniteGolem]),
            // Thunderhide Camp
            (
                Ancient,
                false,
                &[SmallThunderLizard, SmallThunderLizard, BigThunderLizard],
            ),
            // Frostbitten Camp
            (
                Ancient,
                true,
                &[FrostbittenGolem, FrostbittenGolem, IceShaman],
            ),
            // Prowler Camp
            (
                Ancient,
                true,
                &[ProwlerAcolyte, ProwlerAcolyte, ProwlerShaman],
            ),
        ];
        for (index, (roster, expected)) in ROSTERS.iter().zip(expected).enumerate() {
            assert_eq!(
                (roster.kind, roster.offered, roster.creeps),
                expected,
                "legacy roster {index}"
            );
        }
    }

    #[test]
    fn retired_legacy_rosters_are_kept_but_never_drawn() {
        use NeutralKind::*;
        let retired: [(CampKind, &[NeutralKind]); 3] = [
            (CampKind::Medium, &[CentaurOutrunner, CentaurKhan]),
            (CampKind::Large, &[WarpineRaider, WarpineRaider]),
            (
                CampKind::Ancient,
                &[SmallThunderLizard, SmallThunderLizard, BigThunderLizard],
            ),
        ];
        let actual: Vec<_> = ROSTERS
            .iter()
            .filter(|row| !row.offered)
            .map(|row| (row.kind, row.creeps))
            .collect();
        assert_eq!(actual, retired);
        for (class, retired_creeps) in retired {
            // The maximal range also checks rows omitted by narrower current maps.
            let camp = CampDef {
                kind: class,
                rosters: (1, u8::MAX),
                ..crate::CAMPS[0]
            };
            assert!(
                rosters_of(&camp).all(|(_, row)| row.creeps != retired_creeps),
                "a camp can draw the retired {class:?} roster {retired_creeps:?}"
            );
        }
    }

    #[test]
    fn offered_legacy_rosters_match_their_side_specific_compositions() {
        // One entry per legacy row, in order. A replacement may move a roster to a different
        // spawn type or list the same creeps in a different order.
        let patch_rows = [
            Some(0),
            Some(2),
            Some(1),
            Some(3),
            Some(4),
            Some(5),
            None,
            Some(1),
            Some(3),
            Some(0),
            Some(4),
            Some(0),
            Some(4),
            Some(3),
            Some(2),
            Some(5),
            None,
            Some(3),
            Some(0),
            None,
            Some(2),
            Some(4),
        ];
        for (index, (legacy, patch_row)) in ROSTERS.iter().zip(patch_rows).enumerate() {
            let Some(patch_row) = patch_row else {
                assert!(!legacy.offered, "legacy roster {index}");
                continue;
            };
            assert!(legacy.offered, "legacy roster {index}");
            let mut actual: Vec<_> = legacy.creeps.iter().map(|kind| kind.id()).collect();
            let mut expected: Vec<_> = PATCH_CAMP_ROSTERS[legacy.kind as usize][patch_row]
                .iter()
                .map(|kind| kind.id())
                .collect();
            actual.sort_unstable();
            expected.sort_unstable();
            assert_eq!(
                actual,
                expected,
                "legacy roster {index}, patch row {}",
                patch_row + 1
            );
        }
    }

    #[test]
    fn known_patch_rows_are_ordered_and_retain_the_flooded_rows_and_ancient_gap() {
        use NeutralKind::*;
        let [small, medium, large, ancient] = PATCH_CAMP_ROSTERS;
        assert_eq!(small[0], [Kobold, Kobold, KoboldTunneler, KoboldTaskmaster]);
        assert_eq!(small[6], [Tadpole, Tadpole, Tadpole]);
        assert_eq!(medium[2], [Radigull, Radigull]);
        assert_eq!(
            medium[5],
            [OdobenusFootman, OdobenusFootman, OdobenusFrostmage]
        );
        assert_eq!(large[1], [EimermoleForeman, Eimermole]);
        assert_eq!(ancient[1], [OgreSealPup, OgreSealPup, OgreSeal]);
        assert!(ancient[5].is_empty());
        assert_eq!(ancient[6], [AncientFrogMage, AncientFrog, AncientFrog]);
    }

    #[test]
    fn every_dry_camp_keeps_a_playable_roster_while_patch_rosters_are_held() {
        for map in MAPS {
            for camp in map.camps.iter().filter(|camp| !camp.flooded) {
                let rosters: Vec<_> = rosters_of(camp).map(|(_, roster)| roster).collect();
                assert!(!rosters.is_empty(), "{camp:?}");
                for roster in rosters {
                    assert_eq!(roster.kind, camp.kind);
                    assert!(roster.offered);
                    assert!(roster.creeps.iter().all(|kind| kind.neutral().offered));
                }
            }
        }
    }
}
