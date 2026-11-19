//! Buyback: a seat pays to have its dead hero stand up at once, then waits
//! out a cooldown, and the hero's next death lasts longer.

use bota_core::{buyback_cost, buyback_refusal, rules};
use bota_proto::{BuybackView, EntityId, EventKind, RejectReason, SlotId};

use crate::game::{Event, EventVisibility, Seat, World};

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
                // A hero the seat controls is still another seat's to buy back.
                return Err(if self.controlled_seat(slot, named).is_some() {
                    RejectReason::NotYourUnit
                } else {
                    RejectReason::NotAHero
                });
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
    /// where a respawn stands it (its fountain, or where it fell on a map that
    /// raises heroes so), the cooldown starts and the hero's next death owes
    /// its penalty. Everyone is told.
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
        self.seats[index].spend_gold(cost);
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
        buyback_cost(self.owned_net_worth(seat))
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
