//! Immutable catalogue ward definitions. Private item data may select rows, not forge them.
use crate::UnitDef;
use bota_proto::{ItemId, UnitKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WardDef {
    unit: &'static UnitDef,
    item: ItemId,
    level: u8,
}
impl WardDef {
    pub const fn unit(self) -> &'static UnitDef {
        self.unit
    }
    pub const fn kind(self) -> UnitKind {
        self.unit.kind
    }
    pub const fn source_item(self) -> ItemId {
        self.item
    }
    /// Zero identifies a pre-existing, unlevelled ward row.
    pub const fn level(self) -> u8 {
        self.level
    }
}

// All construction lives here; every key is unique and its policy is part of RULES.
static DEFINITIONS: [WardDef; 5] = [
    WardDef {
        unit: &crate::OBSERVER_WARD,
        item: ItemId::OBSERVER_WARD,
        level: 0,
    },
    WardDef {
        unit: &crate::SENTRY_WARD,
        item: ItemId::SENTRY_WARD,
        level: 0,
    },
    WardDef {
        unit: &crate::BANNERS[0],
        item: ItemId::ROSHANS_BANNER,
        level: 1,
    },
    WardDef {
        unit: &crate::BANNERS[1],
        item: ItemId::ROSHANS_BANNER,
        level: 2,
    },
    WardDef {
        unit: &crate::BANNERS[2],
        item: ItemId::ROSHANS_BANNER,
        level: 3,
    },
];
pub static OBSERVER_WARDS: [WardDef; 1] = [DEFINITIONS[0]];
pub static SENTRY_WARDS: [WardDef; 1] = [DEFINITIONS[1]];
pub static BANNER_WARDS: [WardDef; 3] = [DEFINITIONS[2], DEFINITIONS[3], DEFINITIONS[4]];

const _: () = {
    let mut a = 0;
    while a < DEFINITIONS.len() {
        let mut b = a + 1;
        while b < DEFINITIONS.len() {
            assert!(
                DEFINITIONS[a].item.0 != DEFINITIONS[b].item.0
                    || DEFINITIONS[a].level != DEFINITIONS[b].level
            );
            b += 1;
        }
        a += 1;
    }
};

pub fn ward_placement_allowed(map: &crate::MapDef, at: bota_proto::Vec2) -> bool {
    !map.no_wards.iter().any(|zone| zone.contains(at))
}
