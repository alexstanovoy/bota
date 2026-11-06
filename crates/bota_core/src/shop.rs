//! Shop access: the footprints of Dota's secret-shop trigger volumes.

use bota_proto::{Fixed, ShopZoneView, Vec2};

use crate::{MapDef, ShopAccess, at_home_shop, segment_distance_squared};

/// A shopkeeper and the counter-clockwise footprint of its trigger volume.
#[derive(Clone, Copy, Debug)]
pub struct ShopZone {
    pub pos: Vec2,
    pub corners: &'static [Vec2],
}

impl ShopZone {
    pub fn view(&self) -> ShopZoneView {
        ShopZoneView {
            pos: self.pos,
            corners: self.corners.to_vec(),
        }
    }
}

/// A carrier touches a shop if its hull overlaps the trigger's footprint.
pub fn shop_zone_touches(corners: &[Vec2], pos: Vec2, bound: Fixed) -> bool {
    if corners.len() < 3 || corners.len() > bota_proto::MAX_SHOP_ZONE_CORNERS || bound.raw < 0 {
        return false;
    }
    let edges = || (0..corners.len()).map(|at| (corners[at], corners[(at + 1) % corners.len()]));
    edges().all(|(a, b)| cross(a, b, pos) >= 0)
        || edges().any(|(a, b)| segment_distance_squared(pos, a, b) <= bound.squared_raw())
}

/// A strict convex counter-clockwise footprint, without repeated corners.
pub const fn shop_zone_is_convex(corners: &[Vec2]) -> bool {
    if corners.len() < 3 || corners.len() > bota_proto::MAX_SHOP_ZONE_CORNERS {
        return false;
    }
    let mut at = 0;
    while at < corners.len() {
        if cross(
            corners[at],
            corners[(at + 1) % corners.len()],
            corners[(at + 2) % corners.len()],
        ) <= 0
        {
            return false;
        }
        let mut point = 0;
        while point < corners.len() {
            if cross(
                corners[at],
                corners[(at + 1) % corners.len()],
                corners[point],
            ) < 0
            {
                return false;
            }
            point += 1;
        }
        at += 1;
    }
    true
}

const fn cross(a: Vec2, b: Vec2, p: Vec2) -> i128 {
    (b.x.raw as i128 - a.x.raw as i128) * (p.y.raw as i128 - a.y.raw as i128)
        - (b.y.raw as i128 - a.y.raw as i128) * (p.x.raw as i128 - a.x.raw as i128)
}

pub fn at_secret_shop(map: &MapDef, pos: Vec2, bound: Fixed) -> bool {
    map.secret_shops
        .iter()
        .any(|shop| shop_zone_touches(shop.corners, pos, bound))
}

pub fn shop_access(map: &MapDef, team: bota_proto::Team, pos: Vec2, bound: Fixed) -> ShopAccess {
    ShopAccess {
        base: at_home_shop(map, team, pos),
        secret: at_secret_shop(map, pos, bound),
    }
}

const fn point(x: i32, y: i32) -> Vec2 {
    Vec2 {
        x: Fixed { raw: x },
        y: Fixed { raw: y },
    }
}

// Dota 7.41f, maps/dota/entities/unnamed_3010_2431_58.vmdl_c and
// unnamed_3010_4090_43.vmdl_c, PHYS convex hulls, transformed by their
// trigger_shop entities in the default entity lump (shoptype=2).
// Positions are in the simulator's frame, Q16.16; no circular approximation.
pub const DOTA_SECRET_SHOPS: [ShopZone; 2] = [
    ShopZone {
        pos: point(271050048, 731640824),
        corners: &[
            point(239994861, 726534663),
            point(240634981, 716457405),
            point(245371291, 707651417),
            point(264109244, 695208337),
            point(278123363, 695935633),
            point(296322715, 707137205),
            point(302111520, 718612887),
            point(299436415, 735969813),
            point(292825101, 747712552),
            point(278818706, 755250112),
            point(259230514, 755070925),
            point(243667418, 748720584),
        ],
    },
    ShopZone {
        pos: point(924197952, 524834464),
        corners: &[
            point(881058100, 516940337),
            point(897128081, 488601273),
            point(929045995, 482074899),
            point(957385058, 498144888),
            point(963911405, 530062794),
            point(947841455, 558401863),
            point(915923541, 564928237),
            point(887584478, 548858248),
        ],
    },
];

const _: () = {
    assert!(DOTA_SECRET_SHOPS.len() <= bota_proto::MAX_SECRET_SHOPS);
    let mut at = 0;
    while at < DOTA_SECRET_SHOPS.len() {
        assert!(DOTA_SECRET_SHOPS[at].corners.len() <= bota_proto::MAX_SHOP_ZONE_CORNERS);
        assert!(shop_zone_is_convex(DOTA_SECRET_SHOPS[at].corners));
        at += 1;
    }
};

/// Every base and secret shop grants the backpack exemption.
pub fn at_any_shop(map: &MapDef, pos: Vec2, bound: Fixed) -> bool {
    map.fountains
        .iter()
        .any(|shop| pos.within(*shop, crate::rules::units(crate::rules::SHOP_RANGE)))
        || at_secret_shop(map, pos, bound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shop_distance_handles_the_full_signed_wire_coordinate_range() {
        let corners = [
            point(0, 0),
            point(65536, 0),
            point(65536, 65536),
            point(0, 65536),
        ];
        for pos in [point(i32::MAX, i32::MAX), point(i32::MIN, i32::MIN)] {
            assert!(!shop_zone_touches(&corners, pos, Fixed::from_int(32)));
        }
        assert_eq!(
            segment_distance_squared(
                point(i32::MAX, i32::MAX),
                point(i32::MIN, i32::MIN),
                point(i32::MIN + 1, i32::MIN)
            ),
            i64::MAX
        );
        assert!(shop_zone_touches(
            &corners,
            point(-1, 1),
            Fixed::from_int(1)
        ));
    }
}
