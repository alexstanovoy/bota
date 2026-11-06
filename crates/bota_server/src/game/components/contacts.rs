//! Contact cooldowns are controller state, independent of the bounded, evictable buff store.
use crate::engine::state_hash;
use crate::game::Entity;
use bota_proto::{AbilityId, EffectId};

pub const MAX_CONTACT_SOURCES: usize = 16;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContactClock {
    pub source: Entity,
    pub ability: AbilityId,
    pub effect: EffectId,
    pub until: u32,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contacts(pub Vec<ContactClock>);
state_hash!(ContactClock {
    source,
    ability,
    effect,
    until
});
state_hash!(Contacts(0));
