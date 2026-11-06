//! Passive reincarnation and item-expiry recovery, selected by catalogue rules.
use crate::game::{Entity, Modifier, World};
use bota_core::{ModifierKind, item_def, rules};
use bota_proto::ItemId;

impl World {
    pub(crate) fn begin_reincarnation(&mut self, unit: Entity) -> bool {
        let Some(seat) = self.seat_of(unit) else {
            return false;
        };
        if self.wears_illusion(unit) {
            return false;
        }
        let Some(at) = self.pos(unit) else {
            return false;
        };
        let Some((slot, delay)) = self.inventory.get(unit).and_then(|bag| {
            bag.slots
                .iter()
                .take(rules::INVENTORY_SLOTS)
                .enumerate()
                .find_map(|(slot, stack)| {
                    let stack = stack.as_ref()?;
                    Some((slot, item_def(stack.id)?.reincarnation?))
                })
        }) else {
            return false;
        };
        let item = self.inventory.get_mut(unit).expect("bag").slots[slot]
            .take()
            .expect("reincarnation item")
            .id;
        self.tell_item_used(unit, item, bota_proto::Target::Pos(at), Some(at));
        self.cancel_action(unit);
        self.rot_goes_out(unit);
        self.carry_fights_on(unit);
        self.seats[seat].reincarnating_at = Some(at);
        self.lay_hero_down(seat, delay);
        true
    }

    pub(crate) fn recover_expired_item(&mut self, unit: Entity, item: ItemId) {
        let Some(recovery) = item_def(item).and_then(|def| def.expiry_recovery) else {
            return;
        };
        if !self.alive(unit) || self.wears_illusion(unit) {
            return;
        }
        self.put_modifier(
            unit,
            Modifier::new(
                ModifierKind::Recovery {
                    health_pct: recovery.health_pct,
                    mana_pct: recovery.mana_pct,
                },
                Some(unit),
                Some(bota_proto::Inflictor::Item(item)),
                Some(recovery.ticks),
            ),
        );
    }

    pub(crate) fn refresh_cooldowns(&mut self, unit: Entity, items: bool) -> bool {
        if let Some(book) = self.abilities.get_mut(unit) {
            for state in &mut book.slots {
                state.cooldown = 0;
            }
        }
        if items && let Some(bag) = self.inventory.get_mut(unit) {
            for stack in bag.every_stack_mut() {
                stack.cooldown = 0;
            }
        }
        true
    }
}

impl World {
    /// All temporary vision, with its own ground/flying rule: portal landing or reincarnation.
    pub(crate) fn temporary_sights(
        &self,
    ) -> impl Iterator<Item = (bota_proto::Team, bota_proto::Vec2, i32, bool)> + '_ {
        let night = self.time_of_day().night;
        self.teleport_sights()
            .map(|(team, at, radius)| (team, at, radius, true))
            .chain(self.seats.iter().filter_map(move |seat| {
                Some((
                    seat.team,
                    seat.reincarnating_at?,
                    if night { 800 } else { 1800 },
                    false,
                ))
            }))
    }
}
