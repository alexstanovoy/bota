//! What using an item does to a world: each [`UseSpec`] of the catalog
//! ([`bota_core::ITEMS`]) turned into the change it makes.

use bota_core::{ItemDef, UseSpec, item_def};
use bota_proto::{ItemId, ItemView, Target};

use crate::game::{Entity, Inventory, ItemStack, World};

impl World {
    /// Uses an item at the moment its cast point lands. `false`: nothing
    /// happened and nothing is spent.
    pub fn use_effect(&mut self, user: Entity, slot: usize, def: &ItemDef, target: Target) -> bool {
        match def.effect {
            UseSpec::None => false,
            UseSpec::Mend(drink) => self.mend_with(user, target, drink),
            UseSpec::PlantTree { ticks, range } => self.plant_a_tree(user, target, ticks, range),
            UseSpec::FellTree { range } => self.fell_a_tree(user, target, range),
            UseSpec::Ward { unit, ticks, range } => {
                self.stand_ward(user, target, unit, ticks, range)
            }
            UseSpec::Teleport => self.begin_teleport(user, target),
            UseSpec::SwitchMode => self.switch_mode(user, slot),
            UseSpec::Phase {
                pct,
                ranged_pct,
                ticks,
            } => {
                let ranged = self
                    .stats
                    .get(user)
                    .is_some_and(|stats| stats.projectile_speed.is_some());
                self.walk_through(user, if ranged { ranged_pct } else { pct }, ticks)
            }
            UseSpec::Blink { range, overshoot } => self.blink_to(user, target, range, overshoot),
            UseSpec::Restore {
                hp_per_charge,
                mana_per_charge,
            } => {
                let charges = i32::from(self.charges_in(user, slot));
                self.restore_with(user, charges * hp_per_charge, charges * mana_per_charge)
            }
            UseSpec::Replenish { mana } => self.replenish_mana(user, target, mana),
            UseSpec::Grenade => self.throw_grenade(user, target),
        }
    }

    /// Every tick a use runs on. `false`: it breaks off.
    pub fn use_goes_on(&mut self, user: Entity, def: &ItemDef) -> bool {
        match def.effect {
            UseSpec::Teleport => self.teleport_holds(user),
            _ => true,
        }
    }

    /// Breaks a use off before its end.
    pub fn use_breaks_off(&mut self, user: Entity, def: &ItemDef) {
        if matches!(def.effect, UseSpec::Teleport) {
            self.break_teleport(user);
        }
    }

    /// Runs a use to the end of its duration.
    pub fn use_completes(&mut self, user: Entity, slot: usize, def: &ItemDef) {
        if matches!(def.effect, UseSpec::Teleport) {
            self.teleport_to(user, slot);
        }
    }
}

/// What a bag looks like on the wire, an empty slot keeping its place; the
/// town portal slot is [`item_view`]'s alone.
///
/// Mana costs carry the holder's mana cost rate; `mana_rate_bp` is
/// [`bota_core::rules::NOMINAL_BP`] for a bag with no body behind it. `clocks` are the
/// waits its seat owes on kinds of item, which a stack shows as its own.
pub fn item_views(
    bag: &Inventory,
    mana_rate_bp: i32,
    clocks: &[(ItemId, u32)],
) -> Vec<Option<ItemView>> {
    bag.slots
        .iter()
        .map(|slot| slot.map(|stack| item_view(&stack, mana_rate_bp, clocks)))
        .collect()
}

/// What one stack looks like on the wire, on the terms of [`item_views`].
pub fn item_view(stack: &ItemStack, mana_rate_bp: i32, clocks: &[(ItemId, u32)]) -> ItemView {
    let def = item_def(stack.id);
    let owed = clocks
        .iter()
        .filter(|(item, _)| *item == stack.id)
        .map(|(_, left)| *left)
        .max()
        .unwrap_or(0);
    ItemView {
        id: stack.id,
        charges: def
            .filter(|def| def.charges > 0 || def.cast_charges > 0)
            .map(|_| stack.charges),
        cooldown_left: stack.cooldown.max(owed),
        mute_left: stack.mute,
        mode: stack.mode,
        mana_cost: def.map_or(0, |def| {
            crate::game::cost_after(def.mana_cost, mana_rate_bp)
        }),
        range: def.map_or(0, |def| def.range),
        aim: def.and_then(|def| def.aim),
        for_sale: stack.for_sale,
        owner: stack.owner,
    }
}
