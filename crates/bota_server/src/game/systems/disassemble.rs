//! Taking a build apart and locking items against combining: the orders
//! [`Order::Disassemble`](bota_proto::Order::Disassemble) and
//! [`Order::SetCombineLock`](bota_proto::Order::SetCombineLock), by the rules
//! `bota_core::disassembly` lays out. A hero that lies dead gives them too,
//! for what it carried and for the stash.

use bota_core::{fountain_pos, item_def};
use bota_proto::{ItemId, RejectReason, SlotId};

use crate::game::{BAG_SLOTS, Entity, Inventory, ItemStack, Seat, World, in_stash};

/// A slot's number in the bag it belongs to: the stash numbers its own from
/// zero.
fn local(at: usize) -> usize {
    if in_stash(at) { at - BAG_SLOTS } else { at }
}

impl World {
    /// The stack in one of a seat's slots: the stash's, else the unit's bag
    /// (a hero that lies dead carries its own still).
    fn stack_in(&self, unit: Entity, seat: &Seat, at: usize) -> Option<ItemStack> {
        if in_stash(at) {
            return seat.stash.slots.get(at - BAG_SLOTS).copied().flatten();
        }
        self.held_in_bag(unit, at)
    }

    /// The bag a slot belongs to, as [`World::stack_in`] reads it.
    fn bag_in(&mut self, seat: usize, unit: Entity, at: usize) -> Option<&mut Inventory> {
        if in_stash(at) {
            return Some(&mut self.seats[seat].stash);
        }
        self.inventory.get_mut(unit)
    }

    /// Why a seat may not take apart what one of its slots holds, or nothing
    /// when it may. The stash wants the unit in reach of it, as swapping
    /// does; a hero that lies dead is in reach of no shop and needs none.
    pub(crate) fn disassembly_refusal(
        &self,
        unit: Entity,
        seat: &Seat,
        at: usize,
    ) -> Result<(), RejectReason> {
        let held = self
            .stack_in(unit, seat, at)
            .ok_or(RejectReason::EmptySlot)?;
        if held.owner != seat.slot {
            return Err(RejectReason::NotYourItem);
        }
        if in_stash(at) && !seat.dead() && !self.at_shop(unit) {
            return Err(RejectReason::NotAtShop);
        }
        bota_core::disassembly(held.id, held.disassemble_until.saturating_sub(self.tick))
            .map(|_| ())
            .ok_or(RejectReason::NotDisassemblable)
    }

    /// Why a seat may not lock what one of its slots holds, or nothing when
    /// it may.
    pub(crate) fn lock_refusal(
        &self,
        unit: Entity,
        seat: &Seat,
        at: usize,
    ) -> Result<(), RejectReason> {
        let held = self
            .stack_in(unit, seat, at)
            .ok_or(RejectReason::EmptySlot)?;
        if held.owner != seat.slot {
            return Err(RejectReason::NotYourItem);
        }
        bota_core::is_part_of_a_recipe(held.id)
            .then_some(())
            .ok_or(RejectReason::CannotLockCombining)
    }

    /// Takes the build in a slot apart into its parts, every one locked. The
    /// first goes where the build was, the rest into the bag's free room, and
    /// what has none on the ground where the unit stands (the fountain for a
    /// hero that lies dead).
    pub fn disassemble_item(&mut self, slot: SlotId, unit: Entity, at: usize) -> bool {
        let Some(seat) = self.seats.iter().position(|s| s.slot == slot) else {
            return false;
        };
        if self
            .disassembly_refusal(unit, &self.seats[seat], at)
            .is_err()
        {
            return false;
        }
        let Some(built) = self.stack_in(unit, &self.seats[seat], at) else {
            return false;
        };
        let mut parts = self.parts_of_build(&built).into_iter();
        let (Some(first), Some(bag)) = (parts.next(), self.bag_in(seat, unit, at)) else {
            return false;
        };
        let Some(held) = bag.slot_mut(local(at)) else {
            return false;
        };
        *held = Some(first);
        let mut left = Vec::new();
        for part in parts {
            if !bag.receive(part) {
                left.push(part);
            }
        }
        let spot = match self.pos(unit) {
            Some(spot) => spot,
            None => fountain_pos(self.map, self.seats[seat].team),
        };
        for part in left {
            self.lay_loot(part, spot);
        }
        true
    }

    /// What a build comes apart into: a stack for every part of the recipe it
    /// was built by, with no wait and no window of its own, locked. A build
    /// that gathered charges gives them to the parts that gather them.
    fn parts_of_build(&self, built: &ItemStack) -> Vec<ItemStack> {
        let mut gathered = item_def(built.id)
            .filter(|def| def.cast_charges > 0)
            .map_or(0, |_| built.charges);
        bota_core::parts_of(built.id, built.mode)
            .iter()
            .filter_map(|part| item_def(*part))
            .map(|def| {
                let mut charges = def.charges;
                if def.cast_charges > 0 {
                    charges = gathered.min(def.cast_charges);
                    gathered -= charges;
                }
                self.part_stack(def.id, def.mode, charges, built)
            })
            .collect()
    }

    /// One part of a build taken apart.
    fn part_stack(
        &self,
        id: ItemId,
        mode: Option<bota_proto::Attribute>,
        charges: u8,
        built: &ItemStack,
    ) -> ItemStack {
        ItemStack {
            id,
            charges,
            cooldown: 0,
            mute: 0,
            mode,
            bought_tick: self.tick,
            touched: true,
            owner: built.owner,
            for_sale: false,
            free_charges: 0,
            sentries: 0,
            sentry_active: false,
            locked: true,
            disassemble_until: 0,
            life: 0,
            rune: None,
        }
    }

    /// Locks what a slot holds against combining, or unlocks it.
    pub fn set_combine_lock(
        &mut self,
        slot: SlotId,
        unit: Entity,
        at: usize,
        locked: bool,
    ) -> bool {
        let Some(seat) = self.seats.iter().position(|s| s.slot == slot) else {
            return false;
        };
        if self.lock_refusal(unit, &self.seats[seat], at).is_err() {
            return false;
        }
        let Some(stack) = self
            .bag_in(seat, unit, at)
            .and_then(|bag| bag.slot_mut(local(at)))
            .and_then(Option::as_mut)
        else {
            return false;
        };
        stack.locked = locked;
        true
    }
}
