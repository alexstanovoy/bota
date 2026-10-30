//! How the types of other crates go into the world fingerprint, see [`StateHash`]. The types of this
//! crate say it next to their definitions.

use bota_core::{GoldRange, MapDef, ModifierKind, NeutralKind, Striker};
use bota_proto::{
    AbilityId, AbilitySlot, Attribute, ClockShift, DamageKind, EntityId, HeroId, Inflictor, ItemId,
    ItemSlot, ModifierSpec, NeutralId, ObjectiveSet, Order, ProjectileModel, RejectReason, SlotId,
    Talents, Target,
};

use crate::engine::{Fnv, StateHash, state_hash};

state_hash!(AbilityId(0));
state_hash!(ItemId(0));
state_hash!(SlotId(0));
state_hash!(HeroId(0));
state_hash!(AbilitySlot(0));
state_hash!(ItemSlot(0));
state_hash!(Talents { bits });
state_hash!(ClockShift { game, day });
state_hash!(GoldRange { min, max });
state_hash!(ModifierSpec {
    magic_resist,
    status_resist,
    physical_damage,
    magic_damage,
    pure_damage,
    cooldown_rate,
    mana_cost_rate,
    move_speed,
    max_hp,
    max_mana,
    gold_income
});
state_hash!(DamageKind as u8: DamageKind::Physical, DamageKind::Magical);
state_hash!(Attribute as u8: Attribute::Strength, Attribute::Agility);
state_hash!(NeutralId(0));
state_hash!(Striker as u8: Striker::MeleeHero, Striker::RangedHero);
state_hash!(RejectReason as u8: RejectReason::HeroDead, RejectReason::UnknownTarget);
state_hash!(NeutralKind as u8: NeutralKind::Kobold, NeutralKind::KoboldTunneler);

impl<T: StateHash + ?Sized> StateHash for &T {
    fn state_hash(&self, fnv: &mut Fnv) {
        (**self).state_hash(fnv);
    }
}

/// A map is told apart by its number; the rest of it is the rules'.
impl StateHash for MapDef {
    fn state_hash(&self, fnv: &mut Fnv) {
        self.id.0.state_hash(fnv);
    }
}

impl StateHash for ObjectiveSet {
    fn state_hash(&self, fnv: &mut Fnv) {
        self.bits.state_hash(fnv);
    }
}

impl StateHash for Target {
    fn state_hash(&self, fnv: &mut Fnv) {
        match self {
            Target::None => fnv.u8(0),
            Target::Pos(pos) => {
                fnv.u8(1);
                pos.state_hash(fnv);
            }
            Target::Unit(EntityId { idx, generation }) => {
                fnv.u8(2);
                idx.state_hash(fnv);
                generation.state_hash(fnv);
            }
        }
    }
}

impl StateHash for Inflictor {
    fn state_hash(&self, fnv: &mut Fnv) {
        match self {
            Inflictor::Ability(id) => {
                fnv.u8(1);
                id.state_hash(fnv);
            }
            Inflictor::Item(id) => {
                fnv.u8(2);
                id.state_hash(fnv);
            }
        }
    }
}

impl StateHash for ProjectileModel {
    fn state_hash(&self, fnv: &mut Fnv) {
        match self {
            ProjectileModel::RangedCreep => fnv.u8(0),
            ProjectileModel::SiegeCreep => fnv.u8(1),
            ProjectileModel::Tower => fnv.u8(2),
            ProjectileModel::Fountain => fnv.u8(3),
            ProjectileModel::Hero(hero) => {
                fnv.u8(4);
                hero.state_hash(fnv);
            }
            ProjectileModel::Neutral(neutral) => {
                fnv.u8(5);
                neutral.state_hash(fnv);
            }
        }
    }
}

/// An order is eaten as it goes over the wire, which tells any two apart.
impl StateHash for Order {
    fn state_hash(&self, fnv: &mut Fnv) {
        let mut bytes = Vec::new();
        bota_proto::encode_frame(self, &mut bytes).expect("an order always encodes");
        bytes.state_hash(fnv);
    }
}

impl StateHash for ModifierKind {
    fn state_hash(&self, fnv: &mut Fnv) {
        match *self {
            ModifierKind::Phased => fnv.u8(9),
            ModifierKind::Flying => fnv.u8(35),
            ModifierKind::FountainInvulnerable => fnv.u8(36),
            ModifierKind::FountainPassive => fnv.u8(37),
            ModifierKind::FountainDamage { stacks } => {
                fnv.u8(38);
                fnv.u32(stacks);
            }
            ModifierKind::Mending { per_tick, breaks } => {
                fnv.u8(1);
                fnv.i32(per_tick.raw);
                fnv.some(breaks);
            }
            ModifierKind::Clarity { per_tick, breaks } => {
                fnv.u8(2);
                fnv.i32(per_tick.raw);
                fnv.some(breaks);
            }
            ModifierKind::Fountain { hp_pct, mana_pct } => hash_tagged(fnv, 3, &[hp_pct, mana_pct]),
            ModifierKind::Stunned => fnv.u8(4),
            ModifierKind::Shielded => fnv.u8(8),
            ModifierKind::Slowed { pct } => hash_tagged(fnv, 5, &[pct]),
            ModifierKind::Hastened { pct } => hash_tagged(fnv, 7, &[pct]),
            ModifierKind::Haste { pct } => hash_tagged(fnv, 34, &[pct]),
            ModifierKind::Guarded {
                armor,
                hp_per_second,
            } => hash_tagged(fnv, 12, &[armor, hp_per_second]),
            ModifierKind::Inspired {
                hp_per_second,
                magic_resist_pct,
            } => hash_tagged(fnv, 13, &[hp_per_second, magic_resist_pct]),
            ModifierKind::ArmorBroken { armor } => hash_tagged(fnv, 10, &[armor.raw]),
            ModifierKind::MagicBroken { pct } => hash_tagged(fnv, 16, &[pct]),
            ModifierKind::Burning {
                amount,
                kind,
                lethal,
            } => {
                fnv.u8(6);
                fnv.i32(amount);
                fnv.u8(kind as u8);
                fnv.some(lethal);
            }
            ModifierKind::Shadowraze { stacks } => {
                fnv.u8(11);
                fnv.u8(stacks);
            }
            ModifierKind::Rot { level } => {
                fnv.u8(14);
                fnv.u8(level);
            }
            ModifierKind::Rotting { level } => {
                fnv.u8(32);
                fnv.u8(level);
            }
            ModifierKind::Feared => fnv.u8(15),
            ModifierKind::BloodGrenade { since } => {
                fnv.u8(17);
                fnv.u32(since);
            }
            ModifierKind::Rooted => fnv.u8(18),
            ModifierKind::Disarmed => fnv.u8(19),
            ModifierKind::Broken => fnv.u8(20),
            ModifierKind::BackdoorProtection => fnv.u8(24),
            ModifierKind::Fortified => fnv.u8(25),
            ModifierKind::TimedLife => fnv.u8(30),
            ModifierKind::Illusion => fnv.u8(33),
            ModifierKind::NeutralUpgrade { count } => {
                fnv.u8(31);
                fnv.u32(count);
            }
            ModifierKind::FleshHeap { count } => {
                fnv.u8(26);
                fnv.u32(count);
            }
            ModifierKind::Souls { count } => {
                fnv.u8(27);
                fnv.u32(count);
            }
            ModifierKind::Feasting { level, souls } => {
                fnv.u8(28);
                fnv.u8(level);
                fnv.u32(u32::from(souls));
            }
            ModifierKind::FeastLinger { souls } => {
                fnv.u8(29);
                fnv.u32(u32::from(souls));
            }
            ModifierKind::Spell {
                ability,
                level,
                amount,
            } => {
                fnv.u8(21);
                fnv.u32(u32::from(ability.id().0));
                fnv.u8(level);
                fnv.u32(u32::from(amount));
            }
            ModifierKind::Dismembered { level } => {
                fnv.u8(22);
                fnv.u8(level);
            }
            ModifierKind::MeatShield { amount } => hash_tagged(fnv, 23, &[amount]),
            ModifierKind::VisionGrant { side, day, night } => {
                fnv.u8(39);
                fnv.team(side);
                fnv.i32(day);
                fnv.i32(night);
            }
            ModifierKind::LampOff => fnv.u8(40),
            ModifierKind::CaptureGuard => fnv.u8(41),
            ModifierKind::ShrineReady => fnv.u8(42),
        }
    }
}

/// A tag and its numbers.
fn hash_tagged(fnv: &mut Fnv, tag: u8, values: &[i32]) {
    fnv.u8(tag);
    for &value in values {
        fnv.i32(value);
    }
}

#[cfg(test)]
mod specimens {
    use super::*;
    use crate::engine::Specimen;

    impl Specimen for &'static MapDef {
        fn low() -> Self {
            bota_core::map_of(bota_proto::MapId(0))
        }

        fn high() -> Self {
            bota_core::map_of(bota_proto::MapId(1))
        }
    }

    impl Specimen for ObjectiveSet {
        fn low() -> Self {
            ObjectiveSet::NONE
        }

        fn high() -> Self {
            ObjectiveSet::of(bota_proto::ObjectiveKind::Outposts)
        }
    }

    impl Specimen for Target {
        fn low() -> Self {
            Target::None
        }

        fn high() -> Self {
            Target::Pos(Specimen::high())
        }
    }

    impl Specimen for Inflictor {
        fn low() -> Self {
            Inflictor::Ability(Specimen::low())
        }

        fn high() -> Self {
            Inflictor::Item(Specimen::high())
        }
    }

    impl Specimen for ProjectileModel {
        fn low() -> Self {
            ProjectileModel::Tower
        }

        fn high() -> Self {
            ProjectileModel::Hero(Specimen::high())
        }
    }

    impl Specimen for Order {
        fn low() -> Self {
            Order::Move {
                target: Specimen::low(),
            }
        }

        fn high() -> Self {
            Order::Move {
                target: Specimen::high(),
            }
        }
    }

    impl Specimen for ModifierKind {
        fn low() -> Self {
            ModifierKind::Phased
        }

        fn high() -> Self {
            ModifierKind::Stunned
        }
    }
}
