//! Mending health and mana over time.

use bota_proto::Fixed;

use crate::game::{EntityAllocator, Health, Mana, Modifiers, Stats, Table};

/// Adds what each entity mends to its health and mana, up to its maximum.
/// An entity already dead, a hero lying dead among them, mends nothing.
pub fn regenerate(
    entities: &EntityAllocator,
    stats: &Table<Stats>,
    health: &mut Table<Health>,
    mana: &mut Table<Mana>,
    modifiers: &mut Table<Modifiers>,
) {
    for entity in entities.iter() {
        let Some(stat) = stats.get(entity) else {
            continue;
        };
        if health
            .get(entity)
            .is_some_and(|pool| pool.hp <= Fixed::ZERO)
        {
            continue;
        }
        super::restoration::heal_scaled(
            entity,
            super::restoration::Gain {
                amount: stat.hp_regen,
                redirected: stat.hp_regen_redirect,
            },
            None,
            stats,
            health,
            modifiers,
        );
        if let Some(mp) = mana.get_mut(entity) {
            mp.mana = (mp.mana + stat.mana_regen).min(stat.max_mana);
        }
    }
}
