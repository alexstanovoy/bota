//! Item effects use the same buffs and aura emitters as abilities.

use bota_proto::{DamageKind, EffectId, ItemId};

use crate::{Aura, AuraModifier, Buff, ModifierKind, Reach, Whom, rules};

/// A passive lost after receiving an attack or landing an attack on a hero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttackBreak {
    pub ticks: u32,
    pub broken: crate::Carried,
}

/// A periodic damage effect; illusion carriers may have a different rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamagePulse {
    pub damage: i32,
    pub illusion_damage: i32,
    pub kind: DamageKind,
    pub period: u32,
}

/// A damage barrier uses the same ordered soak path as a Water Bubble.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemBarrier {
    pub kind: DamageKind,
    pub amount: i32,
}

/// A receiver's restoration callback changes while this source's active template is present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestorationAura {
    pub passive_cut_pct: i32,
    pub active_redirect_pct: i32,
    pub active: u8,
}

/// One modifier template. Runtime modifiers hold its item and index, not a copy of its stats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemModifier {
    pub effect: EffectId,
    pub buff: Buff,
    pub ticks: u32,
    pub priority: i32,
    pub pierces_immunity: bool,
    pub barrier: Option<ItemBarrier>,
    pub pulse: Option<DamagePulse>,
    pub restoration: Option<RestorationAura>,
    /// Ticks between payouts of restoration collected in the runtime modifier's amount.
    pub collection_period: Option<u32>,
    /// Recovery history shared by different effects of one family.
    pub lockout_family: Option<EffectId>,
    pub emitters: &'static [ItemAura],
    pub ethereal_pct: Option<i32>,
    pub movement_steal: Option<MovementSteal>,
    pub stacking: bool,
    /// Only the strongest live member contributes stats; weaker instances keep their timers.
    pub buff_family: Option<EffectId>,
}

impl ItemModifier {
    pub const fn buff(effect: EffectId, buff: Buff) -> Self {
        Self {
            effect,
            buff,
            ticks: rules::AURA_LINGER_TICKS,
            priority: 0,
            pierces_immunity: false,
            barrier: None,
            pulse: None,
            restoration: None,
            collection_period: None,
            lockout_family: None,
            emitters: &[],
            ethereal_pct: None,
            movement_steal: None,
            stacking: false,
            buff_family: None,
        }
    }

    pub const fn kind(self, item: ItemId, index: u8, since: u32, illusion: bool) -> ModifierKind {
        if let Some(pct) = self.ethereal_pct {
            return ModifierKind::Ethereal { pct };
        }
        let amount = match (self.barrier, self.pulse) {
            (Some(barrier), _) => barrier.amount,
            (_, Some(pulse)) if illusion => pulse.illusion_damage,
            (_, Some(pulse)) => pulse.damage,
            _ if self.stacking => 1,
            _ => 0,
        };
        ModifierKind::ItemBuff {
            item,
            index,
            amount,
            since,
        }
    }
}

/// A periodic transfer of movement speed from enemy heroes to the emitter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MovementSteal {
    pub period: u32,
    pub radius: i32,
    pub debuff: u8,
    pub buff: u8,
}

/// An emission policy bound to one of an item's modifier templates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemAura {
    pub index: u8,
    pub radius: i32,
    pub whom: Whom,
    pub reaches: Reach,
    pub health_below_pct: Option<i32>,
}

impl ItemAura {
    pub const fn allies(index: u8, radius: i32) -> Self {
        Self {
            index,
            radius,
            whom: Whom::Allies,
            reaches: Reach::Combatants,
            health_below_pct: None,
        }
    }

    pub const fn enemies(index: u8, radius: i32) -> Self {
        Self {
            whom: Whom::Enemies,
            ..Self::allies(index, radius)
        }
    }

    pub const fn emitter(self, modifier: ModifierKind, linger: u32) -> Aura {
        Aura {
            growth_clock: None,
            whom: self.whom,
            reaches: self.reaches,
            radius: self.radius,
            linger,
            breakable: false,
            invulnerable: false,
            water: false,
            modifier: AuraModifier::Kind(modifier),
        }
    }
}

/// Which part of an area support cast a receiver timer suppresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupportLockout {
    All,
    Health,
}

/// Recovery and timed modifier grants to nearby allies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AreaSupport {
    pub radius: i32,
    pub health: i32,
    pub mana: i32,
    pub grants: &'static [u8],
    pub lockout: Option<u8>,
    /// The margin subtracted from the successful caster's cooldown when its receiver timer is created.
    pub lockout_margin: u32,
    pub dispel_self: bool,
    pub reaches: Reach,
    pub lockout_scope: SupportLockout,
}

impl AreaSupport {
    pub const fn recovery(radius: i32, health: i32, mana: i32) -> Self {
        Self {
            radius,
            health,
            mana,
            grants: &[],
            lockout: None,
            lockout_margin: 0,
            dispel_self: false,
            reaches: Reach::Combatants,
            lockout_scope: SupportLockout::All,
        }
    }
}

/// A circular wave that follows its caster and reaches each enemy once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AreaWaveSpec {
    pub pierces_immunity: bool,
    pub radius: i32,
    pub speed: i32,
    pub damage: i32,
    pub debuff: u8,
}

/// The item data behind a live modifier, if this is one.
pub const fn item_modifier(kind: ModifierKind) -> Option<ItemModifier> {
    match kind {
        ModifierKind::ItemBuff { item, index, .. } => {
            Some(crate::ITEMS[item.0 as usize].modifiers[index as usize])
        }
        _ => None,
    }
}
