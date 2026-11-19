//! Permanent hero upgrades from consumed items, and the installed ability values they affect.

use bota_proto::{AbilityId, HeroUpgrades, ItemId};

pub const SCEPTER_ITEM: ItemId = upgrade_row("item_ultimate_scepter").id;
const SHARD_COST: i32 = upgrade_row("item_aghanims_shard").cost;
const BLESSING_COST: i32 = upgrade_row("item_ultimate_scepter_2").cost;

const fn upgrade_row(key: &str) -> &'static crate::ItemDef {
    let mut index = 0;
    while index < crate::data::ITEM_ROWS.len() {
        let row = &crate::data::ITEM_ROWS[index];
        if crate::item_data::str_eq(row.key, key) {
            return row;
        }
        index += 1;
    }
    panic!("missing upgrade row")
}

pub const fn consumed_upgrade_value(upgrades: HeroUpgrades) -> i32 {
    let shard = if upgrades.shard { SHARD_COST } else { 0 };
    let scepter = if upgrades.scepter_blessing {
        BLESSING_COST
    } else {
        0
    };
    shard + scepter
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeroUpgrade {
    Shard,
    Scepter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpgradeOn {
    Purchase,
    Assemble,
    RecipePart,
    Use,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PermanentItem {
    pub upgrade: HeroUpgrade,
    pub on: UpgradeOn,
}

impl HeroUpgrade {
    pub const fn held(self, upgrades: HeroUpgrades) -> bool {
        match self {
            Self::Shard => upgrades.shard,
            Self::Scepter => upgrades.scepter_blessing,
        }
    }

    pub fn grant(self, upgrades: &mut HeroUpgrades) -> bool {
        let held = match self {
            Self::Shard => &mut upgrades.shard,
            Self::Scepter => &mut upgrades.scepter_blessing,
        };
        !std::mem::replace(held, true)
    }
}

pub fn ability_range(id: AbilityId, upgrades: HeroUpgrades) -> i32 {
    let base = crate::ability_def(id).map_or(0, |def| def.range);
    if id == AbilityId::MEAT_HOOK && upgrades.shard {
        base + crate::rules::HOOK_SHARD_RANGE
    } else {
        base
    }
}
