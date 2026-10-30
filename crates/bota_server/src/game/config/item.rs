//! What using an item does to a world: each [`UseSpec`] of the catalog
//! ([`bota_core::ITEMS`]) turned into the change it makes.

use bota_core::{ItemDef, UseSpec, item_def};
use bota_proto::{DispenserView, EventKind, Inflictor, ItemId, ItemView, Target, Team, Vec2};

use crate::game::{Entity, Event, EventVisibility, Inventory, ItemStack, World, wire_id};

impl World {
    /// Uses an item at the moment its cast point lands, and tells it.
    /// `false`: nothing happened and nothing is spent. What it does names
    /// the item as its inflictor.
    pub fn use_effect(
        &mut self,
        user: Entity,
        slot: usize,
        used: (ItemId, &ItemDef),
        target: Target,
    ) -> bool {
        let before = self.pos(user);
        if !self.use_effect_of(user, slot, used, target) {
            return false;
        }
        self.tell_item_used(user, used.0, target, before);
        true
    }

    /// Tells a use that took effect to the sides that see the user where it stood or where it
    /// stands now, and to its own.
    fn tell_item_used(&mut self, user: Entity, item: ItemId, target: Target, before: Option<Vec2>) {
        let side = self.team.get(user).copied().unwrap_or(Team::Neutral);
        let after = self.pos(user);
        let told = |team| {
            side == team
                || [before, after]
                    .into_iter()
                    .flatten()
                    .any(|spot| self.can_see_point(team, spot))
        };
        let at = match target {
            Target::None => None,
            Target::Pos(pos) => Some(pos),
            Target::Unit(id) => self.of_wire(id).and_then(|aimed| self.pos(aimed)),
        };
        self.events.push(Event {
            kind: EventKind::ItemUsed {
                caster: Some(wire_id(user)),
                item,
                at,
            },
            visible_to: EventVisibility::of(told(Team::Radiant), told(Team::Dire)),
            gold_hidden_from: None,
            source_told_to: self.who_may_name(Some(user)),
        });
    }

    /// What using an item does to the world.
    fn use_effect_of(
        &mut self,
        user: Entity,
        slot: usize,
        (item, def): (ItemId, &ItemDef),
        target: Target,
    ) -> bool {
        let with = Inflictor::Item(item);
        match def.effect {
            UseSpec::None => false,
            UseSpec::Mend(drink) => self.mend_with(user, target, (drink, def.cast_range), with),
            UseSpec::PlantTree { ticks } => self.plant_a_tree(user, target, ticks, def.cast_range),
            UseSpec::FellTree => self.fell_a_tree(user, target, def.cast_range),
            UseSpec::Ward { unit, ticks } => {
                self.stand_ward(user, target, (item, unit), ticks, def.cast_range)
            }
            UseSpec::Dispense => self.dispense_ward(user, slot, target),
            UseSpec::Teleport(scroll) => self.begin_teleport(user, target, scroll, def.duration),
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
                self.walk_through(user, if ranged { ranged_pct } else { pct }, ticks, with)
            }
            UseSpec::Blink { range, overshoot } => self.blink_to(user, target, range, overshoot),
            UseSpec::Restore {
                hp_per_charge,
                mana_per_charge,
                ..
            } => {
                let charges = i32::from(self.charges_in(user, slot));
                let restores = (charges * hp_per_charge, charges * mana_per_charge);
                self.restore_with(user, restores, with)
            }
            UseSpec::Replenish { mana } => self.replenish_mana(user, target, mana, with),
            UseSpec::Grenade(_) => self.throw_grenade(user, target, item),
        }
    }

    /// Every tick a use runs on. `false`: it breaks off.
    pub fn use_goes_on(&mut self, user: Entity, def: &ItemDef) -> bool {
        match def.effect {
            UseSpec::Teleport(_) => self.teleport_holds(user),
            _ => true,
        }
    }

    /// Breaks a use off before its end.
    pub fn use_breaks_off(&mut self, user: Entity, def: &ItemDef) {
        if matches!(def.effect, UseSpec::Teleport(_)) {
            self.break_teleport(user);
        }
    }

    /// Runs a use to the end of its duration.
    pub fn use_completes(&mut self, user: Entity, slot: usize, def: &ItemDef) {
        if matches!(def.effect, UseSpec::Teleport(_)) {
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
            .map(|_| stack.active_charges()),
        cooldown_left: stack.cooldown.max(owed),
        mute_left: stack.mute,
        mode: stack.mode,
        mana_cost: def.map_or(0, |def| {
            crate::game::cost_after(def.mana_cost, mana_rate_bp)
        }),
        range: def.map_or(0, |def| def.cast_range),
        aim: def.and_then(|def| def.aim),
        for_sale: false,
        owner: stack.owner,
        selling: Some(stack.for_sale),
        dispenser: (stack.id == ItemId::WARD_DISPENSER).then_some(DispenserView {
            observers: stack.charges,
            sentries: stack.sentries,
            sentry_active: stack.sentry_active,
        }),
    }
}
