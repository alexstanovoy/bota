//! What sits in a slot of the panel: how pressing it is answered, and how it
//! is drawn.
//!
//! Nothing here asks whether an order would succeed: a press of what can be
//! used is sent whatever state the slot is in, and the server names what was
//! wrong with it. What works on its own is never used, so pressing it is
//! nothing at all, as in Dota.

use bota_proto::{
    AbilitySlot, AbilityView, Aim, EntityId, ItemSlot, ItemView, Order, Target, UnitView,
};

use crate::ui::app::App;

/// How many item slots a hero carries on itself: inventory and backpack.
pub const BAG_SLOTS: u8 = crate::limits::HERO_ITEM_SLOTS as u8;

/// How many of those are the inventory proper, where items work.
pub const INVENTORY_SLOTS: u8 = bota_core::rules::INVENTORY_SLOTS as u8;

/// The hero's town portal slot, Dota's own slot for scrolls beside the
/// inventory.
pub const TOWN_PORTAL_SLOT: u8 = ItemSlot::TOWN_PORTAL.0;

/// One slot of the bottom panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// An ability slot of whatever is commanded.
    Ability(u8),
    /// An item slot of this seat: inventory, backpack, stash, then the town
    /// portal slot.
    Item(u8),
}

/// What pressing a slot asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Press {
    /// Send this order now.
    Send(Order),
    /// Take the slot up to be aimed; the next click in the world spends it.
    Aim(Slot),
    /// Nothing to press: the slot holds nothing, holds a passive, or holds
    /// it somewhere a press cannot reach.
    Nothing,
}

/// How a slot is drawn.
///
/// The states are not exclusive: what has no points in it is unusable as well
/// as unlearned, and a passive may sit on a cooldown of its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    /// Whether anything is in the slot at all.
    pub filled: bool,
    /// Whether it has been learned. Always true for an item.
    pub learned: bool,
    /// Whether it looks ready: learned, not passive, off cooldown and
    /// affordable; an item also in the inventory with charges left.
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
        self.view()?.unit(commanded)
    }

    /// Whether a Disassemble of what sits in an item slot would be carried
    /// out: the item can be taken apart now, and a stash slot is in reach of
    /// the commanded unit, as the server asks.
    pub fn can_disassemble(&self, slot: u8) -> bool {
        let in_reach = slot < BAG_SLOTS
            || self
                .slot_unit()
                .zip(self.my_team())
                .is_some_and(|(unit, team)| {
                    self.session()
                        .is_some_and(|s| bota_core::at_home_shop(s.map(), team, unit.pos))
                });
        in_reach
            && self
                .item_in(slot)
                .is_some_and(|item| item.disassembly.is_some())
    }

    /// Whether what sits in an item slot is locked against combining, when
    /// it can be locked at all: `None` for an empty slot, an item that is
    /// part of no recipe, and a unit of another side.
    pub fn combine_lock_of(&self, slot: u8) -> Option<bool> {
        let item = self.item_in(slot)?;
        bota_core::is_part_of_a_recipe(item.id)
            .then_some(item.locked)
            .flatten()
    }

    /// What one's own hero carries while it lies dead and is the one
    /// commanded: its kit, as the server tells it.
    fn dead_kit(&self) -> Option<&bota_proto::Kit> {
        let player = self.player(self.my_slot?)?;
        let commanded = self.commanded()?;
        (player.respawn_left > 0 && player.unit == Some(commanded))
            .then_some(player.kit.as_ref())
            .flatten()
    }

    /// What sits in one item slot of the panel.
    ///
    /// The bag slots are the commanded unit's, a courier's as readily as the
    /// hero's, a dead hero's from its kit (it may still swap them, as in
    /// Dota). The stash is the seat's own whatever is commanded, and the
    /// town portal slot is a hero's alone.
    pub fn item_in(&self, slot: u8) -> Option<ItemView> {
        if slot == TOWN_PORTAL_SLOT {
            return match self.slot_unit() {
                Some(unit) => unit.town_portal,
                None => self.dead_kit()?.town_portal,
            };
        }
        if slot < BAG_SLOTS {
            let items = match self.slot_unit() {
                Some(unit) => &unit.items,
                None => &self.dead_kit()?.items,
            };
            items.get(usize::from(slot)).copied().flatten()
        } else {
            self.player(self.my_slot?)?
                .stash
                .as_ref()?
                .get(usize::from(slot - BAG_SLOTS))
                .copied()
                .flatten()
        }
    }

    /// How one slot is aimed, and nothing for one that cannot be used at all. A slot taken up
    /// for an ally is aimed at a unit.
    pub fn aim_in(&self, slot: Slot) -> Option<Aim> {
        let aim = filled(self, slot).flatten();
        if aim.is_some() && self.aiming_ally(slot) {
            Some(Aim::Unit)
        } else {
            aim
        }
    }

    /// What an item in a slot does for an allied unit, where Dota lets it (Control held).
    pub fn ally_use_in(&self, slot: Slot) -> Option<bota_core::AllyUse> {
        let Slot::Item(at) = slot else {
            return None;
        };
        bota_core::item_def(self.item_in(at)?.id)?.ally
    }

    /// Whether a slot is taken up for an ally.
    pub fn aiming_ally(&self, slot: Slot) -> bool {
        self.aiming == Some(slot) && self.ally_aim == Some(slot)
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

    /// The radius of the area one slot's use at a point takes in, in world units; zero for
    /// one that takes in none. No ability aimed at a point takes in an area yet: an
    /// ability's area joins its data when one does.
    pub fn area_of(&self, slot: Slot) -> i32 {
        match slot {
            Slot::Ability(_) => 0,
            Slot::Item(at) => self
                .item_in(at)
                .and_then(|item| bota_core::item_def(item.id))
                .and_then(bota_core::ItemDef::radius)
                .unwrap_or(0),
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
/// for a passive, which is never used. `aiming` is the slot already taken up,
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
        return Press::Send(Order::Learn {
            slot: AbilitySlot(at),
        });
    }
    // The backpack and the stash are reached by dragging, not by pressing.
    if let Slot::Item(at) = slot
        && at >= INVENTORY_SLOTS
        && at != TOWN_PORTAL_SLOT
    {
        return Press::Nothing;
    }
    // An empty slot, and a passive: Dota answers a press of either with
    // nothing, no error and no sound.
    let Some(Some(aim)) = held else {
        return Press::Nothing;
    };
    // Taken up already, a scroll pressed again goes to the fountain: Dota's
    // double tap.
    if aiming == Some(slot) && aim == Aim::Building {
        return Press::Send(order_for(slot, Target::None));
    }
    // Taken up already and aimed at a unit, a second press aims it at the one
    // holding it.
    if aiming == Some(slot) && aim == Aim::Unit {
        return match commanded {
            Some(target) => Press::Send(order_for(slot, Target::Unit(target))),
            None => Press::Nothing,
        };
    }
    match aim {
        Aim::Own => Press::Send(order_for(slot, Target::None)),
        Aim::Point | Aim::Unit | Aim::Tree | Aim::Building => Press::Aim(slot),
    }
}

/// What one slot holds, in the shape [`decide`] takes as `held`.
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
pub fn order_for(slot: Slot, target: Target) -> Order {
    match slot {
        Slot::Ability(at) => Order::Cast {
            slot: AbilitySlot(at),
            target,
        },
        Slot::Item(at) => Order::Use {
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
        usable: (at < INVENTORY_SLOTS || at == TOWN_PORTAL_SLOT)
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
