//! Dota's "Require Hold/Stop" options: while the hero channels, an order to
//! its body waits for the channel to end instead of breaking it, and only a
//! stop or a hold breaks it. As Dota has it, only the last order given while
//! the channel runs is carried out then; orders given queued, and casts and
//! uses that leave a channel be, go on as they would.

use bota_proto::{Order, StatusFlags, Target};

use crate::orders::IssuedOrder;
use crate::ui::app::App;

/// Which of the hero's channels hold the orders back (all off in Dota until
/// the player turns them on).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChannelStops {
    /// A teleport: the Town Portal Scroll and the like.
    pub teleport: bool,
    /// Any other channelled ability or item.
    pub channels: bool,
}

impl App {
    /// Whether one of the hero's channels is running that the options keep
    /// orders back for.
    fn channel_holds_orders(&self) -> bool {
        let (Some(view), Some(slot)) = (self.view(), self.my_slot) else {
            return false;
        };
        let Some(hero) = self.my_hero().and_then(|id| view.unit(id)) else {
            return false;
        };
        if hero.statuses.bits & StatusFlags::CHANNELLING == 0 {
            return false;
        }
        let teleporting = view.teleports.iter().any(|teleport| teleport.slot == slot);
        if teleporting {
            self.channel_stops.teleport
        } else {
            self.channel_stops.channels
        }
    }

    /// Whether an order of the hero is kept back until the channel ends: one
    /// given unqueued to the body that is neither a stop or a hold nor a cast
    /// or use that leaves a channel be.
    pub(crate) fn held_by_channel(&self, issued: &IssuedOrder) -> bool {
        if issued.queue || issued.unit.is_some() || !bota_core::order_queues(issued.order) {
            return false;
        }
        if matches!(
            issued.order,
            Order::Move {
                target: Target::None
            } | Order::Attack {
                target: Target::None
            }
        ) {
            return false;
        }
        self.channel_holds_orders() && !self.leaves_channel_be(issued.order)
    }

    /// Whether a cast or a use goes off beside a channel.
    fn leaves_channel_be(&self, order: Order) -> bool {
        let Some(hero) = self.view().and_then(|view| view.unit(self.my_hero()?)) else {
            return false;
        };
        match order {
            Order::Cast { slot, .. } => hero
                .abilities
                .get(usize::from(slot.0))
                .and_then(|held| bota_core::ability_def(held.id))
                .is_some_and(|def| def.immediate || def.ignores_channel),
            Order::Use { slot, .. } => crate::orders::item_in(hero, slot)
                .and_then(|held| bota_core::item_def(held.id))
                .is_some_and(|def| def.immediate || def.ignores_channel),
            _ => false,
        }
    }

    /// Carries out the order kept back, once the channel is over.
    pub(crate) fn release_held_order(&mut self) {
        if self.held_order.is_none() || self.channel_holds_orders() {
            return;
        }
        if let Some(held) = self.held_order.take() {
            let queueing = std::mem::take(&mut self.queueing);
            self.send_order_to(held.unit, held.order);
            self.queueing = queueing;
        }
    }
}
