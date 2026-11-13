//! Buyback: a seat pays to have its dead hero stand up at once, then waits
//! out a cooldown, and the hero's next death lasts longer.

use bota_core::{buyback_cost, buyback_refusal, item_def, rules, stack_worth};
use bota_proto::{BuybackView, EntityId, EventKind, ItemId, RejectReason, SlotId};

use crate::game::{Event, EventVisibility, ItemStack, Seat, World};

impl World {
    /// Whether a seat may buy its hero back now. The order is the seat's: it
    /// names nobody, or the seat's own hero.
    pub fn check_buyback(&self, slot: SlotId, named: Option<EntityId>) -> Result<(), RejectReason> {
        let seat = self.seat(slot).ok_or(RejectReason::HeroDead)?;
        if let Some(named) = named {
            let unit = self
                .driven_by(slot, Some(named))
                .ok_or(RejectReason::NotYourUnit)?;
            if seat.unit != Some(unit) {
                return Err(RejectReason::NotAHero);
            }
        }
        buyback_refusal(
            seat.dead(),
            seat.gold,
            self.buyback_cost_of(seat),
            self.tick,
            seat.buyback.ready_at,
            seat.reincarnating_at.is_some(),
        )
    }

    /// Buys a seat's hero back, if it may: the seat pays, the hero stands up
    /// at its fountain as at a respawn, the cooldown starts and the hero's
    /// next death owes its penalty. Everyone is told.
    pub(crate) fn buy_back(
        &mut self,
        slot: SlotId,
        named: Option<EntityId>,
        events: &mut Vec<Event>,
    ) {
        if self.check_buyback(slot, named).is_err() {
            return;
        }
        let Some(index) = self.seats.iter().position(|seat| seat.slot == slot) else {
            return;
        };
        let cost = self.buyback_cost_of(&self.seats[index]);
        let now = self.tick;
        self.seats[index].gold -= cost;
        assert!(self.seats[index].gold >= 0, "a buyback is paid in full");
        self.respawn(index);
        let ledger = &mut self.seats[index].buyback;
        ledger.ready_at = now + rules::BUYBACK_COOLDOWN_TICKS;
        ledger.respawn_penalty = rules::BUYBACK_RESPAWN_PENALTY_TICKS;
        ledger.last = Some(now);
        events.push(Event::Fact {
            kind: EventKind::BoughtBack { slot },
            visible_to: EventVisibility::Everyone,
            gold_hidden_from: None,
            source_told_to: EventVisibility::Everyone,
        });
    }

    /// What a buyback costs a seat now.
    pub fn buyback_cost_of(&self, seat: &Seat) -> i32 {
        buyback_cost(self.holdings_worth(seat))
    }

    /// A seat's net worth as Dota prices a buyback by it: its gold and the
    /// worth of every stack it owns in its hero's bag, its stash and its
    /// courier's bag. What lies on the ground counts for nothing.
    fn holdings_worth(&self, seat: &Seat) -> i32 {
        let carried = [seat.unit, seat.courier]
            .into_iter()
            .flatten()
            .filter_map(|unit| self.inventory.get(unit));
        let worth: i64 = carried
            .chain([&seat.stash])
            .flat_map(|bag| bag.slots.iter().chain(bag.town_portal.iter()).flatten())
            .filter(|stack| stack.owner == seat.slot)
            .map(|stack| i64::from(worth_of(stack)))
            .sum();
        i32::try_from(i64::from(seat.gold) + worth).unwrap_or(i32::MAX)
    }

    /// A seat's buyback as its own side is told it.
    pub(crate) fn buyback_view(&self, seat: &Seat) -> BuybackView {
        BuybackView {
            cost: self.buyback_cost_of(seat),
            ready_at: seat.buyback.ready_at,
            respawn_penalty: seat.buyback.respawn_penalty,
        }
    }
}

/// What one stack adds to its owner's net worth: a Ward Dispenser the wards
/// it holds, anything else by its charges.
fn worth_of(stack: &ItemStack) -> i32 {
    let cost = |id: ItemId| item_def(id).map_or(0, |def| def.cost);
    if stack.id == ItemId::WARD_DISPENSER {
        let (observers, sentries) = stack.ward_counts().unwrap_or_default();
        return cost(ItemId::OBSERVER_WARD) * i32::from(observers)
            + cost(ItemId::SENTRY_WARD) * i32::from(sentries);
    }
    let initial = item_def(stack.id).map_or(0, |def| def.charges);
    stack_worth(cost(stack.id), initial, stack.charges, stack.free_charges)
}
