//! Shortcuts round the rules, for trying things out in a match that allows
//! them.

use bota_proto::{Cheat, EventKind, ItemId, SlotId};

use crate::game::{Entity, Event, EventVisibility, ItemStack, World, rules};

impl World {
    /// Carries out a cheat for a seat and the hero it drives.
    pub fn cheat(&mut self, slot: SlotId, unit: Entity, cheat: Cheat, events: &mut Vec<Event>) {
        let Some(seat) = self.seats.iter().position(|seat| seat.slot == slot) else {
            return;
        };
        match cheat {
            Cheat::Gold { amount } => {
                let gold = &mut self.seats[seat].gold;
                *gold = gold.saturating_add(amount).max(0);
            }
            Cheat::Levels { count } => self.raise_levels(seat, count, events),
            Cheat::Refresh => self.refresh(seat, unit),
            Cheat::Item { item } => self.hand_out(seat, item, events),
        }
    }

    /// Takes a seat's hero up so many levels, to the cap at most, by
    /// granting the experience the levels are worth.
    fn raise_levels(&mut self, seat: usize, count: u8, events: &mut Vec<Event>) {
        let level = self.seats[seat].level;
        let wanted = level.saturating_add(count).min(rules::HERO_MAX_LEVEL);
        if wanted <= level {
            return;
        }
        let needed = rules::XP_THRESHOLDS[usize::from(wanted - 1)] - self.seats[seat].xp;
        self.grant_xp(seat, needed.max(0), events);
    }

    /// Fills the hero's pools and clears every wait on it: ability
    /// cooldowns, item cooldowns and mutes, and the waits the seat owes on
    /// kinds of item.
    fn refresh(&mut self, seat: usize, unit: Entity) {
        self.fill_pools(unit);
        if let Some(book) = self.abilities.get_mut(unit) {
            for slot in book.slots.iter_mut() {
                slot.cooldown = 0;
            }
        }
        if let Some(bag) = self.inventory.get_mut(unit) {
            for stack in bag.slots.iter_mut().flatten() {
                stack.cooldown = 0;
                stack.mute = 0;
            }
        }
        self.seats[seat].item_clocks.clear();
    }

    /// Puts an item into the hero's bag for nothing, or into the stash when
    /// the bag has no room. Told of as a purchase.
    fn hand_out(&mut self, seat: usize, item: ItemId, events: &mut Vec<Event>) {
        let owner = self.seats[seat].slot;
        let Some(stack) = ItemStack::bought(item, owner, self.tick) else {
            return;
        };
        let in_hand = self.seats[seat]
            .unit
            .and_then(|unit| self.inventory.get_mut(unit))
            .is_some_and(|bag| bag.receive(stack));
        if !in_hand && !self.seats[seat].stash.receive(stack) {
            return;
        }
        events.push(Event {
            kind: EventKind::ItemBought { slot: owner, item },
            visible_to: EventVisibility::OneTeam(self.seats[seat].team),
        });
    }
}
