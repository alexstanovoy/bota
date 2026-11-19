//! The planted Banner: hit-count health and a protection aura, by its item level.
const ITEM_ROSHANS_BANNER: crate::ScriptData = crate::ScriptData {
    key: "item_roshans_banner",
    values: crate::item_table::script_values(bota_proto::ItemId::ROSHANS_BANNER),
};

/// Each upgrade of the planted ward uses the same script's health and aura rows.
static BANNER_AURAS: [[crate::Aura; 1]; 3] = [[banner_aura(0)], [banner_aura(1)], [banner_aura(2)]];
pub static BANNERS: [crate::UnitDef; 3] = [banner(0), banner(1), banner(2)];

const fn banner(level: usize) -> crate::UnitDef {
    use crate::roshan_data::BANNER_UNIT as U;
    use crate::{HitCount, QUARTERS_A_PIP};
    // 7.42: "Now has fragile pip health". Its `health` is its pips, its health counted in
    // quarter pips ([`HitCount::pips`]): a ranged hero's, a tower's or the fountain's attack
    // takes one, a melee hero's or Roshan's two (Fragile), anything else's a quarter.
    let pips = ITEM_ROSHANS_BANNER.value("health")[level].int();
    assert!(ITEM_ROSHANS_BANNER.value("health_tooltip")[level].int() == pips);
    crate::UnitDef {
        kind: bota_proto::UnitKind::Banner,
        base_level: U.first("Level").int() as u8,
        max_hp: QUARTERS_A_PIP * pips,
        hit_count: Some(HitCount::pips(true)),
        collision: 18, // Measured NPC hull; the script names HULL_SIZE_REGULAR.
        bound: 8,
        armor: U.first("ArmorPhysical").int(),
        magic_resist_pct: U.first("MagicalResistance").int(),
        ancient: U.first("IsAncient").int() != 0,
        spell_immune: true,
        day_vision: U.first("VisionDaytimeRange").int(),
        night_vision: U.first("VisionNighttimeRange").int(),
        bounty_gold: crate::GoldRange {
            min: U.first("BountyGoldMin").int(),
            max: U.first("BountyGoldMax").int(),
        },
        auras: &BANNER_AURAS[level],
        ..crate::unit::BARE_UNIT
    }
}
const fn banner_aura(level: usize) -> crate::Aura {
    crate::Aura {
        growth_clock: None,
        whom: crate::Whom::Allies,
        reaches: crate::Reach::LaneCreeps,
        radius: ITEM_ROSHANS_BANNER.value("radius")[level].int(),
        linger: ITEM_ROSHANS_BANNER
            .first("buff_linger_duration")
            .secs()
            .ticks(),
        breakable: false,
        invulnerable: true,
        water: false,
        modifier: crate::AuraModifier::Kind(crate::ModifierKind::Fortified),
    }
}
const _: () = assert!(ITEM_ROSHANS_BANNER.first("max_level").int() == BANNERS.len() as i32);

// The client may draw hit-count health by kind; this combat quantity is level-independent.
const _: () = {
    let mut level = 0;
    let block = BANNERS[0].hit_count.expect("hits").ranged_hero.raw;
    while level < BANNERS.len() {
        let share = BANNERS[level].hit_count.expect("hits").ranged_hero;
        assert!(share.raw == block);
        assert!(share.raw & ((1 << bota_proto::Fixed::FRAC_BITS) - 1) == 0);
        level += 1;
    }
};
