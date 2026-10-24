//! Mending health and mana over time.

use bota_proto::Fixed;

use crate::game::{EntityAllocator, Health, Mana, Stats, Table};

/// Adds what each entity mends to its health and mana, up to its maximum.
/// An entity already dead, a hero lying dead among them, mends nothing.
pub fn regenerate(
    entities: &EntityAllocator,
    stats: &Table<Stats>,
    health: &mut Table<Health>,
    mana: &mut Table<Mana>,
) {
    for entity in entities.iter() {
        let Some(stat) = stats.get(entity) else {
            continue;
        };
        if let Some(hp) = health.get_mut(entity) {
            if hp.hp <= Fixed::ZERO {
                continue;
            }
            hp.hp = (hp.hp + stat.hp_regen).min(stat.max_hp);
        }
        if let Some(mp) = mana.get_mut(entity) {
            mp.mana = (mp.mana + stat.mana_regen).min(stat.max_mana);
        }
    }
}
