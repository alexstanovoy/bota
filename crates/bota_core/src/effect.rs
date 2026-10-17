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
}

/// How many effect ids the protocol has ever handed out, retired ones
/// included: every table indexed by [`EffectId`] is this long.
pub const EFFECT_COUNT: usize = 22;

const fn buff(name: &'static str) -> EffectDef {
    EffectDef {
        name,
        harmful: false,
        retired: false,
    }
}

const fn debuff(name: &'static str) -> EffectDef {
    EffectDef {
        name,
        harmful: true,
        retired: false,
    }
}

/// Every effect, indexed by [`EffectId`].
pub const EFFECTS: [EffectDef; EFFECT_COUNT] = [
    EffectDef {
        name: "Haste",
        harmful: false,
        retired: true,
    },
    buff("Mending"),
    buff("Clarity"),
    buff("Fountain"),
    debuff("Stunned"),
    debuff("Slowed"),
    debuff("Burning"),
    buff("Hastened"),
    buff("Shielded"),
    buff("Phased"),
    buff("FleshHeap"),
    buff("Souls"),
    debuff("ArmorBroken"),
    buff("Guarded"),
    buff("Inspired"),
    debuff("Shadowraze"),
    buff("Rot"),
    debuff("Feared"),
    debuff("MagicBroken"),
    buff("Feasting"),
    buff("FeastLinger"),
    debuff("BloodGrenade"),
];

/// What one effect is, or nothing if the id is retired or was never handed
/// out.
#[inline]
pub fn effect_def(id: EffectId) -> Option<&'static EffectDef> {
    EFFECTS.get(usize::from(id.0)).filter(|def| !def.retired)
}
