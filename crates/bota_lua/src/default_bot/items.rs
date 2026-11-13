use super::{distance, enemy, mode_is};
use crate::state::{Snapshot, State};
use bota_proto::{Aim, Inflictor, ItemSlot, Order, StatusFlags, Target, UnitKind, UnitView};

pub(super) fn use_item(state: &State, snapshot: &Snapshot, own: &UnitView) -> Option<Order> {
    use bota_core::{Pool, UseSpec};
    if own.statuses.bits
        & (StatusFlags::STUNNED
            | StatusFlags::FEARED
            | StatusFlags::CASTING
            | StatusFlags::CHANNELLING)
        != 0
        || state.orders.iter().any(|order| {
            order.unit.is_none() && matches!(order.order, Order::Cast { .. } | Order::Use { .. })
        })
    {
        return None;
    }
    let hp = (i64::from(own.max_hp) - i64::from(own.hp)).max(0);
    let mana = (i64::from(own.max_mana) - i64::from(own.mana)).max(0);
    let threatened = enemy(snapshot, own, UnitKind::Hero)
        .is_some_and(|unit| distance(own.pos, unit.pos) <= i128::from(1000 * 65536i64).pow(2));
    for slot in 0..bota_proto::limits::INVENTORY_SLOTS {
        let Some(item) = state.item(own.id, ItemSlot(u8::try_from(slot).ok()?)) else {
            continue;
        };
        if item.owner != state.current_slot
            || item.charges == Some(0)
            || item.cooldown_left > 0
            || item.mute_left > 0
            || item.mana_cost.max(0) > own.mana
        {
            continue;
        }
        let Some(definition) = bota_core::item_def(item.id) else {
            continue;
        };
        let active = own.effects.iter().any(|effect| {
            effect.inflictor == Some(Inflictor::Item(item.id))
                && effect.ticks_left.is_none_or(|left| left > 0)
        });
        let useful = match definition.effect {
            UseSpec::Mend(mend)
                if !mend.eats_a_tree && !active && (!mend.breaks || !threatened) =>
            {
                let missing = match mend.pool {
                    Pool::Health => hp,
                    Pool::Mana => mana,
                };
                mend.total > 0 && missing * 2 >= i64::from(mend.total)
            }
            UseSpec::Bottle(drink) if !active && !threatened && item.rune.is_none() => {
                (drink.health > 0 && hp * 2 >= i64::from(drink.health))
                    || (drink.mana > 0 && mana * 2 >= i64::from(drink.mana))
            }
            UseSpec::Restore {
                hp_per_charge,
                mana_per_charge,
                ..
            } => {
                let charges = i64::from(item.charges.unwrap_or(0));
                charges > 0
                    && ((hp_per_charge > 0 && hp * 2 >= i64::from(hp_per_charge) * charges)
                        || (mana_per_charge > 0
                            && mana * 2 >= i64::from(mana_per_charge) * charges))
            }
            UseSpec::Replenish { mana: amount } => amount > 0 && mana * 2 >= i64::from(amount),
            UseSpec::Eat {
                health,
                mana: amount,
            } => {
                (health > 0 && hp * 2 >= i64::from(health))
                    || (amount > 0 && mana * 2 >= i64::from(amount))
            }
            UseSpec::Phase { .. } => mode_is(state, "retreat") && !active,
            _ => false,
        };
        if useful {
            let target = match item.aim {
                Some(Aim::Unit) => Target::Unit(own.id),
                Some(Aim::Own) => Target::None,
                _ => continue,
            };
            return Some(Order::Use {
                slot: ItemSlot(u8::try_from(slot).ok()?),
                target,
            });
        }
    }
    None
}
