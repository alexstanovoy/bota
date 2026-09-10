//! What a bot answers a tick with.

use bota_proto::{AbilitySlot, EntityId, ItemId, ItemSlot, Order, Target};

/// One order and the unit it is for.
///
/// A unit of `None` means the seat's own hero, which is what most orders are
/// for; a courier errand names the courier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ask {
    /// Which unit it is for. Absent means the seat's own hero.
    pub unit: Option<EntityId>,
    /// The order itself.
    pub order: Order,
}

impl Ask {
    /// An order for the seat's own hero.
    pub fn mine(order: Order) -> Ask {
        Ask { unit: None, order }
    }

    /// An order for one of the other units the seat drives.
    pub fn of(unit: EntityId, order: Order) -> Ask {
        Ask {
            unit: Some(unit),
            order,
        }
    }

    /// Walk to a spot.
    pub fn walk_to(pos: bota_proto::Vec2) -> Ask {
        Ask::mine(Order::Move {
            target: Target::Pos(pos),
        })
    }

    /// Swing at a unit, following it while it lives.
    pub fn swing_at(unit: EntityId) -> Ask {
        Ask::mine(Order::Attack {
            target: Target::Unit(unit),
        })
    }

    /// Walk to a spot, stopping to fight whatever is met on the way.
    pub fn fight_towards(pos: bota_proto::Vec2) -> Ask {
        Ask::mine(Order::Attack {
            target: Target::Pos(pos),
        })
    }

    /// Cast an ability from one of the hero's slots.
    pub fn cast(slot: AbilitySlot, target: Target) -> Ask {
        Ask::mine(Order::Cast { slot, target })
    }

    /// Use an item from one of the hero's inventory slots.
    pub fn use_item(slot: ItemSlot, target: Target) -> Ask {
        Ask::mine(Order::Use { slot, target })
    }

    /// Spend a skill point.
    pub fn learn(slot: AbilitySlot) -> Ask {
        Ask::mine(Order::Learn { slot })
    }

    /// Buy an item.
    pub fn buy(item: ItemId) -> Ask {
        Ask::mine(Order::Buy { item })
    }
}
