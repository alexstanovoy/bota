//! What sits in a slot of the panel: how pressing it is answered, and how it
//! is drawn.
//!
//! Nothing here asks whether an order would succeed. A press is sent whatever
//! state the slot is in, and the server names what was wrong with it; what is
//! worked out here is only which order a press means and what the slot should
//! look like.

use bota_proto::{
    AbilitySlot, AbilityView, Aim, EntityId, ItemSlot, ItemView, Order, OrderTarget, UnitView,
};

use crate::state::App;

/// How many item slots a hero carries on itself: inventory and backpack.
pub const BAG_SLOTS: u8 = 9;

/// How many of those are the inventory proper, where items work.
pub const INVENTORY_SLOTS: u8 = 6;

/// One slot of the bottom panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// An ability slot of whatever is commanded.
    Ability(u8),
    /// An item slot of this seat: inventory, backpack, then stash.
    Item(u8),
}

/// What pressing a slot asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Press {
    /// Send this order now.
    Send(Order),
    /// Take the slot up to be aimed; the next click in the world spends it.
    Aim(Slot),
    /// Nothing to press: the slot holds nothing, or holds it somewhere a
    /// press cannot reach.
    Nothing,
}

/// How a slot is drawn.
///
/// The states are not exclusive and are not meant to be: what has no points
/// in it is unusable as well as unlearned, and a passive may sit on a
/// cooldown of its own. Each field answers one question and the drawing
/// decides what to do with all of them together.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    /// Whether anything is in the slot at all.
    pub filled: bool,
    /// Whether it has been learned. Always true for an item.
    pub learned: bool,
    /// Whether pressing it right now would be carried out.
    pub usable: bool,
    /// Whether it is a toggle that is currently on.
    pub toggled: bool,
    /// Whether it works on its own and is never used.
    pub passive: bool,
    /// Ticks before it may be used again. Zero when nothing is owed.
    pub cooldown_left: u32,
}

impl App {
    /// The unit whose ability slots the panel is about.
    pub fn slot_unit(&self) -> Option<&UnitView> {
        let commanded = self.commanded()?;
        self.view.as_ref()?.units.iter().find(|u| u.id == commanded)
    }

    /// What sits in one item slot of the panel.
    ///
    /// The bag slots are the commanded unit's — the same bag the panel
    /// draws, a courier's as readily as the hero's. The stash is the seat's
    /// own whatever is commanded.
    pub fn item_in(&self, slot: u8) -> Option<ItemView> {
        if slot < BAG_SLOTS {
            let unit = self.slot_unit()?;
            unit.items.get(usize::from(slot)).copied().flatten()
        } else {
            let view = self.view.as_ref()?;
            let mine = self.my_slot?;
            let player = view.players.iter().find(|p| p.slot == mine)?;
            player
                .stash
                .as_ref()?
                .get(usize::from(slot - BAG_SLOTS))
                .copied()
                .flatten()
        }
    }

    /// How one slot is aimed, and nothing for one that cannot be used at all.
    pub fn aim_in(&self, slot: Slot) -> Option<Aim> {
        match slot {
            Slot::Ability(at) => self
                .slot_unit()?
                .abilities
                .get(usize::from(at))
                .map(|held| held.aim),
            Slot::Item(at) => self.item_in(at)?.aim,
        }
    }

    /// How far one slot reaches, in world units.
    pub fn reach_of(&self, slot: Slot) -> i32 {
        match slot {
            Slot::Ability(at) => self.slot_unit().map_or(0, |unit| {
                unit.abilities
                    .get(usize::from(at))
                    .map_or(0, |held| held.range)
            }),
            Slot::Item(at) => self.item_in(at).map_or(0, |item| item.range),
        }
    }
}

/// Which order pressing a slot means.
///
/// With `ctrl` held an ability slot spends a skill point instead of casting.
/// A slot that is aimed is taken up rather than sent, and reaching for one
/// already taken up aims it at whoever holds it, so a salve is drunk with two
/// presses of its own key and no click at all.
pub fn press(app: &App, slot: Slot, ctrl: bool) -> Press {
    decide(slot, ctrl, filled(app, slot), app.aiming, app.commanded())
}

/// Which order a press means, from the few facts that settle it.
///
/// `held` says whether the slot holds anything and how it is aimed: the outer
/// answer is whether anything is there, the inner one how it is aimed, absent
/// for something that is never used. `aiming` is the slot already taken up,
/// and `commanded` whoever the panel is about.
pub fn decide(
    slot: Slot,
    ctrl: bool,
    held: Option<Option<Aim>>,
    aiming: Option<Slot>,
    commanded: Option<EntityId>,
) -> Press {
    if let Slot::Ability(at) = slot
        && ctrl
    {
        return Press::Send(Order::LevelUpAbility {
            slot: AbilitySlot(at),
        });
    }
    // The backpack and the stash are reached by dragging, not by pressing:
    // there is nothing there a press could mean. This is about where the slot
    // is, not about whether what sits in it would work.
    if let Slot::Item(at) = slot
        && at >= INVENTORY_SLOTS
    {
        return Press::Nothing;
    }
    let Some(aim) = held else {
        return Press::Nothing;
    };
    // Taken up already and aimed at a unit, a second press aims it at the one
    // holding it.
    if aiming == Some(slot) && aim == Some(Aim::Unit) {
        return match commanded {
            Some(target) => Press::Send(order_for(slot, OrderTarget::Unit { target })),
            None => Press::Nothing,
        };
    }
    match aim {
        // What works on its own, and what cannot be used at all, is still
        // sent: the server is the one that says so.
        None | Some(Aim::Own) => Press::Send(order_for(slot, OrderTarget::None)),
        Some(Aim::Point | Aim::Unit | Aim::Tree | Aim::Building) => Press::Aim(slot),
    }
}

/// How a slot is aimed, and whether there is anything in it at all.
///
/// The outer answer says whether the slot holds something; the inner one how
/// that something is aimed, absent for one that is never used.
fn filled(app: &App, slot: Slot) -> Option<Option<Aim>> {
    match slot {
        Slot::Ability(at) => app
            .slot_unit()?
            .abilities
            .get(usize::from(at))
            .map(|held| Some(held.aim)),
        Slot::Item(at) => app.item_in(at).map(|item| item.aim),
    }
}

/// The order one slot sends at a target.
pub fn order_for(slot: Slot, target: OrderTarget) -> Order {
    match slot {
        Slot::Ability(at) => Order::CastAbility {
            slot: AbilitySlot(at),
            target,
        },
        Slot::Item(at) => Order::UseItem {
            slot: ItemSlot(at),
            target,
        },
    }
}

/// How one ability slot should be drawn, given the mana its holder has.
pub fn ability_look(held: Option<&AbilityView>, mana: i32) -> Look {
    let Some(held) = held else {
        return Look::default();
    };
    Look {
        filled: true,
        learned: held.level > 0,
        usable: held.level > 0
            && held.cooldown_left == 0
            && !held.passive
            && mana >= held.mana_cost,
        toggled: held.on,
        passive: held.passive,
        cooldown_left: held.cooldown_left,
    }
}

/// How one item slot should be drawn.
///
/// `at` says which slot it is, since one carried in the backpack or waiting in
/// the stash is held rather than used, and `mana` is what its holder has.
pub fn item_look(item: Option<&ItemView>, at: u8, mana: i32) -> Look {
    let Some(item) = item else {
        return Look::default();
    };
    Look {
        filled: true,
        learned: true,
        usable: at < INVENTORY_SLOTS
            && item.aim.is_some()
            && item.cooldown_left == 0
            && item.charges.is_none_or(|left| left > 0)
            && mana >= item.mana_cost,
        toggled: false,
        // An item with nothing to activate is worn rather than used, which is
        // what a passive ability is.
        passive: item.aim.is_none(),
        cooldown_left: item.cooldown_left,
    }
}
