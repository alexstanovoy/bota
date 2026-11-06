//! The planted Banner: hit-count health and a protection aura, by its item level.
use crate::data::items::ITEM_ROSHANS_BANNER;

/// Each upgrade of the planted ward uses the same script's health and aura rows.
const BANNER_AURAS: [[crate::Aura; 1]; 3] = [[banner_aura(0)], [banner_aura(1)], [banner_aura(2)]];
pub const BANNERS: [crate::UnitDef; 3] = [banner(0), banner(1), banner(2)];

const fn banner(level: usize) -> crate::UnitDef {
    use crate::HitCount;
    use crate::data::BANNER_UNIT as U;
    let hp = ITEM_ROSHANS_BANNER.value("health")[level].int();
    let hits = ITEM_ROSHANS_BANNER.value("health_tooltip")[level].int();
    crate::UnitDef {
        kind: bota_proto::UnitKind::Banner,
        max_hp: hp,
        hit_count: Some(HitCount {
            melee_hero: hits as u8,
            ranged_hero: (hits * 2) as u8,
            other: hp as u8,
        }),
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
    let block = BANNERS[0].max_hp / BANNERS[0].hit_count.expect("hits").ranged_hero as i32;
    while level < BANNERS.len() {
        let hits = BANNERS[level].hit_count.expect("hits").ranged_hero as i32;
        assert!(hits > 0);
        assert!(BANNERS[level].max_hp % hits == 0);
        assert!(BANNERS[level].max_hp / hits == block);
        level += 1;
    }
};
