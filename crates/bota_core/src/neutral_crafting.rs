//! Data for the neutral artifact and enchantment choices.

/// An artifact's internal item key and draw weight in its pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeutralArtifact {
    pub key: &'static str,
    pub weight: u32,
}

/// A group drawn independently of the other artifact pools of the tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeutralArtifactPool {
    pub draws: u8,
    pub artifacts: &'static [NeutralArtifact],
}

/// The individual enchantment level offered at a tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeutralEnchantment {
    pub key: &'static str,
    pub level: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeutralEnchantments {
    pub global: &'static [NeutralEnchantment],
    pub strength: &'static [NeutralEnchantment],
    pub agility: &'static [NeutralEnchantment],
    pub intelligence: &'static [NeutralEnchantment],
    pub universal: &'static [NeutralEnchantment],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeutralTier {
    pub start_seconds: u32,
    pub craft_cost: u16,
    pub recraft_increment: Option<u16>,
    pub xp_bonus: u32,
    pub artifact_options: u8,
    pub enchantment_options: u8,
    pub artifact_pools: &'static [NeutralArtifactPool],
    pub enchantments: NeutralEnchantments,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MadstoneAward {
    pub primary: u8,
    pub secondary: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MadstoneBundle {
    pub lifetime_seconds: u32,
    pub normal: &'static [MadstoneAward],
    pub turbo: &'static [MadstoneAward],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeutralCrafting {
    pub unlimited_after_seconds: u32,
    pub tiers: &'static [NeutralTier],
    pub bundle: MadstoneBundle,
}
