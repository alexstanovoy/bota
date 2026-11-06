//! A map boss's life, held across the entities each respawn stands up.
use crate::{engine::state_hash, game::Entity};
use bota_proto::Vec2;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lair {
    pub unit: Option<Entity>,
    pub falls: u32,
    pub last_killer: Option<bota_proto::Team>,
    pub respawn_at: u32,
    pub home: Vec2,
    /// A natural pit change is under way; damage never calls it off.
    pub travelling: bool,
    pub journey: Option<Journey>,
    pub fell_at: Option<u32>,
    pub known_alive: [bool; 2],
}
state_hash!(Lair {
    unit,
    falls,
    last_killer,
    respawn_at,
    home,
    travelling,
    journey,
    fell_at,
    known_alive
});

/// Authoritative movement state is independent of the bounded effect store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Journey {
    pub ability: bota_core::NeutralAbility,
    pub stops: u8,
    pub throws: u8,
}
impl crate::engine::StateHash for Journey {
    fn state_hash(&self, fnv: &mut crate::engine::Fnv) {
        let Self {
            ability,
            stops,
            throws,
        } = self;
        ability.id().0.state_hash(fnv);
        stops.state_hash(fnv);
        throws.state_hash(fnv);
    }
}
impl Journey {
    pub fn modifier(self, unit: Entity) -> crate::game::Modifier {
        crate::game::Modifier::new(
            bota_core::ModifierKind::Journey {
                ability: self.ability,
                stops: self.stops,
                throws: self.throws,
            },
            Some(unit),
            Some(bota_proto::Inflictor::Ability(self.ability.id())),
            None,
        )
    }
}
#[cfg(test)]
impl crate::engine::Specimen for Journey {
    fn low() -> Self {
        Self {
            ability: bota_core::NeutralAbility::StrengthOfTheImmortal,
            stops: 0,
            throws: 0,
        }
    }
    fn high() -> Self {
        Self {
            ability: bota_core::NeutralAbility::GrabAndThrow,
            stops: 5,
            throws: 5,
        }
    }
}
