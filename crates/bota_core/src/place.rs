//! Where things stand on a map, and the lines its lanes run along.
//!
//! Everything here follows from the map alone, so it is the same for every
//! match played on it.

use bota_proto::{Team, Vec2};

use crate::{MapDef, rules, segment_distance_squared};

/// The fountain position of a team. The jungle's is the map center.
#[inline]
pub fn fountain_pos(map: &MapDef, team: Team) -> Vec2 {
    match team {
        Team::Radiant => map.fountains[0],
        Team::Dire => map.fountains[1],
        Team::Neutral => Vec2::from_ints(rules::MAP_SIZE / 2, rules::MAP_SIZE / 2),
    }
}

/// Whether a unit of `team` standing at `pos` is in its home shop: within the
/// shop range of its own fountain.
#[inline]
pub fn at_home_shop(map: &MapDef, team: Team, pos: Vec2) -> bool {
    pos.within(fountain_pos(map, team), rules::units(rules::SHOP_RANGE))
}

/// Where a team's hero appears, beside the fountain rather than inside it.
pub fn hero_spawn_pos(map: &MapDef, team: Team) -> Vec2 {
    let offset = match team {
        Team::Radiant => Vec2::from_ints(rules::HERO_SPAWN_OFFSET, rules::HERO_SPAWN_OFFSET),
        Team::Dire => Vec2::from_ints(-rules::HERO_SPAWN_OFFSET, -rules::HERO_SPAWN_OFFSET),
        Team::Neutral => Vec2::ZERO,
    };
    fountain_pos(map, team) + offset
}

/// The tier of the tower of a team standing exactly at a position, if one
/// does. A tower's health is not a way to tell, it varies by match.
pub fn tower_tier_at(map: &MapDef, team: Team, pos: Vec2) -> Option<u8> {
    let table = match team {
        Team::Radiant => map.radiant_towers,
        Team::Dire => map.dire_towers,
        Team::Neutral => return None,
    };
    table
        .iter()
        .find(|&&(_, _, at)| at == pos)
        .map(|&(_, tier, _)| tier)
}

/// Whether the barracks of a team standing exactly at a position, if one
/// does, is the one that feeds the ranged creeps. A barracks' health is not a
/// way to tell either, a match may raise it.
pub fn barracks_ranged_at(map: &MapDef, team: Team, pos: Vec2) -> Option<bool> {
    let table = match team {
        Team::Radiant => map.barracks[0],
        Team::Dire => map.barracks[1],
        Team::Neutral => return None,
    };
    table
        .iter()
        .find(|&&(_, _, at)| at == pos)
        .map(|&(_, ranged, _)| ranged)
}

/// The physical centerline of a lane, Radiant base first: from the Radiant
/// Ancient through the lane's corners to the Dire Ancient, and through the
/// lane's towers too on a map whose lanes run through them.
///
/// A side with no Ancient anchors its end at its own wave spawner instead.
pub fn lane_polyline(map: &MapDef, lane: u8) -> Vec<Vec2> {
    let tower_of = |table: &[(u8, u8, Vec2)], tier: u8| {
        table
            .iter()
            .find(|&&(tl, tt, _)| tl == lane && tt == tier)
            .map(|&(_, _, pos)| pos)
    };
    let anchor =
        |side: usize| map.ancients[side].unwrap_or(map.creep_spawns[side][usize::from(lane)]);
    let mut line = vec![anchor(0)];
    if map.lane_through_towers {
        for tier in [3u8, 2, 1] {
            if let Some(pos) = tower_of(map.radiant_towers, tier) {
                line.push(pos);
            }
        }
    }
    if let Some(corners) = map.lane_corners.get(usize::from(lane)) {
        line.extend_from_slice(corners);
    }
    if map.lane_through_towers {
        for tier in [1u8, 2, 3] {
            if let Some(pos) = tower_of(map.dire_towers, tier) {
                line.push(pos);
            }
        }
    }
    line.push(anchor(1));
    line
}

/// The waypoints a team's creeps push through on a lane, enemy base last.
///
/// Only what lies past the segment of the line nearest the wave's spawner
/// is kept, so a fresh wave never walks back towards its base first.
pub fn lane_route(map: &MapDef, team: Team, lane: u8) -> Vec<Vec2> {
    let mut line = lane_polyline(map, lane);
    if team == Team::Dire {
        line.reverse();
    }
    let spawn = creep_spawn_pos(map, team, lane);
    let mut nearest = (0usize, i64::MAX);
    for (at, seg) in line.windows(2).enumerate() {
        let d = segment_distance_squared(spawn, seg[0], seg[1]);
        if d < nearest.1 {
            nearest = (at, d);
        }
    }
    line.split_off(nearest.0 + 1)
}

/// The creep spawn position of a team on a lane. The jungle runs no lanes.
#[inline]
pub fn creep_spawn_pos(map: &MapDef, team: Team, lane: u8) -> Vec2 {
    match team {
        Team::Radiant => map.creep_spawns[0][usize::from(lane)],
        Team::Dire => map.creep_spawns[1][usize::from(lane)],
        Team::Neutral => Vec2::from_ints(rules::MAP_SIZE / 2, rules::MAP_SIZE / 2),
    }
}

/// Where a team sits in the per-team route tables. The jungle marches
/// nowhere and answers zero.
#[inline]
pub fn team_index(team: Team) -> usize {
    match team {
        Team::Radiant | Team::Neutral => 0,
        Team::Dire => 1,
    }
}

/// The place of one of the two sides in a pair kept per side, Radiant's
/// first; nothing for the jungle, which keeps no such pair.
pub fn side_index(team: Team) -> Option<usize> {
    match team {
        Team::Radiant => Some(0),
        Team::Dire => Some(1),
        Team::Neutral => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MAP2_ID, map_of};

    #[test]
    fn a_tower_is_found_by_where_it_stands() {
        let map = map_of(MAP2_ID);
        for &(_, tier, pos) in map.radiant_towers {
            assert_eq!(tower_tier_at(map, Team::Radiant, pos), Some(tier));
            assert_eq!(tower_tier_at(map, Team::Dire, pos), None);
        }
        for &(_, tier, pos) in map.dire_towers {
            assert_eq!(tower_tier_at(map, Team::Dire, pos), Some(tier));
        }
        assert_eq!(
            tower_tier_at(map, Team::Radiant, Vec2::from_ints(1, 1)),
            None
        );
    }
}
