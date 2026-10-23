//! Every timed effect a unit may show on the wire, and how a client treats
//! it.

use bota_proto::EffectId;

/// One entry of the effect list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectDef {
    /// What it is called.
    pub name: &'static str,
    /// Whether it hurts whoever wears it: a debuff, apart from the buffs.
    pub harmful: bool,
    /// Whether the id is out of use for good: kept so that no other effect
    /// takes its number.
    pub retired: bool,
    /// Whether a basic dispel takes it off.
    pub dispellable: bool,
    /// Whether a hero keeps it through death, as Dota keeps a modifier that is
    /// not removed on death: what it has gathered.
    pub kept_through_death: bool,
}

/// How many effect ids the protocol has ever handed out, retired ones
/// included: every table indexed by [`EffectId`] is this long.
pub const EFFECT_COUNT: usize = EFFECTS.len();

const fn buff(name: &'static str) -> EffectDef {
    EffectDef {
        name,
        harmful: false,
        retired: false,
        dispellable: false,
        kept_through_death: false,
    }
}

const fn debuff(name: &'static str) -> EffectDef {
    EffectDef {
        harmful: true,
        ..buff(name)
    }
}

/// A count of what a hero has gathered, kept through its deaths.
const fn gathered(name: &'static str) -> EffectDef {
    EffectDef {
        kept_through_death: true,
        ..buff(name)
    }
}

/// A buff a basic dispel takes off.
const fn fleeting(name: &'static str) -> EffectDef {
    EffectDef {
        dispellable: true,
        ..buff(name)
    }
}

/// Every effect, indexed by [`EffectId`].
pub const EFFECTS: [EffectDef; 61] = [
    EffectDef {
        retired: true,
        ..buff("Haste")
    },
    fleeting("Mending"),
    fleeting("Clarity"),
    buff("Fountain"),
    debuff("Stunned"),
    debuff("Slowed"),
    debuff("Burning"),
    fleeting("Hastened"),
    buff("Shielded"),
    fleeting("Phased"),
    gathered("FleshHeap"),
    gathered("Souls"),
    debuff("ArmorBroken"),
    buff("Guarded"),
    buff("Inspired"),
    debuff("Shadowraze"),
    buff("Rot"),
    debuff("Feared"),
    debuff("MagicBroken"),
    fleeting("Feasting"),
    buff("FeastLinger"),
    debuff("BloodGrenade"),
    debuff("Rooted"),
    debuff("Disarmed"),
    debuff("Broken"),
    debuff("StealWeapon"),
    buff("SpeedAura"),
    debuff("EnvenomedWeapon"),
    debuff("FrostAttack"),
    buff("CloakAura"),
    debuff("Intimidate"),
    buff("PackleadersAura"),
    buff("ManaAura"),
    buff("UnholyAura"),
    debuff("ShardSplit"),
    buff("SwiftnessAura"),
    fleeting("DeathThroeRush"),
    debuff("ThunderClap"),
    fleeting("DeathThroePower"),
    buff("ToughnessAura"),
    debuff("MagicAmplificationAura"),
    buff("DragonhideAura"),
    debuff("WeakeningAura"),
    buff("GraniteAura"),
    buff("WarDrumsAura"),
    debuff("Slam"),
    fleeting("Frenzy"),
    buff("TimeWarpAura"),
    buff("ProwlerAura"),
    debuff("Desecrate"),
    debuff("Petrify"),
    buff("RiverbornAura"),
    buff("Rally"),
    debuff("Tempest"),
    fleeting("WaterBubble"),
    debuff("Purge"),
    buff("Fortified"),
    buff("MeatShield"),
    debuff("Dismembered"),
    buff("BackdoorProtection"),
    buff("Illusion"),
];

/// What one effect is, or nothing if the id is retired or was never handed
/// out.
#[inline]
pub fn effect_def(id: EffectId) -> Option<&'static EffectDef> {
    EFFECTS.get(usize::from(id.0)).filter(|def| !def.retired)
}
