//! What a participant asks its hero to do.
//!
//! An order is an intent. The server validates it, may reject it, and decides
//! what actually happens; the result shows up in the next snapshot and in
//! [`EventKind`](crate::EventKind).
//!
//! At most one order per seat survives per tick, and the last one submitted
//! wins. There is no shift-queue in v0.1.

use crate::{AbilitySlot, EntityId, ItemId, ItemSlot, Vec2};
use serde::{Deserialize, Serialize};

/// Where an order is aimed.
///
/// Which variant is legal depends on the order carrying it. A mismatch is
/// rejected with
/// [`RejectReason::WrongTargetKind`](crate::RejectReason::WrongTargetKind).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// At nothing: the order works on the unit itself, or where it stands.
    None,
    /// At a position on the ground.
    Pos(Vec2),
    /// At a live entity. Must be visible to the issuing team.
    Unit(EntityId),
}

/// The longest an applied modifier may be put on for, in ticks.
pub const MAX_MODIFIER_TICKS: u32 = 1_000_000;

/// Whether a tick count is one an applied modifier may run for.
pub const fn modifier_ticks_bounded(ticks: u32) -> bool {
    ticks >= 1 && ticks <= MAX_MODIFIER_TICKS
}

/// What a scale is worth at nominal, in basis points.
const NOMINAL_RATE: i32 = 10_000;

/// Bounded stat changes a cheat may put on one unit.
///
/// Rates, amplifications and magnitudes are scales in basis points of the
/// nominal 10_000, so 100 is one percent and a value below nominal reduces.
/// Scales add as deltas of the nominal, so two sources never compound.
/// Resistances are additive hundredths of a percentage point. Every field at
/// its neutral value changes nothing.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModifierSpec {
    /// Magic resistance added, in hundredths of a percentage point.
    pub magic_resist: i32,
    /// Share taken off the duration of stuns, fears and slows, in basis
    /// points: 1_000 makes a five-tick stun four ticks long.
    pub status_resist: i32,
    /// Scale on physical damage dealt before mitigation, 10_000 nominal.
    pub physical_damage: i32,
    /// Scale on magical damage dealt before mitigation, 10_000 nominal.
    pub magic_damage: i32,
    /// Scale on pure damage dealt before mitigation, 10_000 nominal.
    pub pure_damage: i32,
    /// Scale on every cooldown set on the unit, 10_000 nominal.
    pub cooldown_rate: i32,
    /// Scale on every mana cost the unit pays, 10_000 nominal.
    pub mana_cost_rate: i32,
    /// Scale on the unit's movement speed, 10_000 nominal.
    pub move_speed: i32,
    /// Scale on the maximum health the unit is raised with, 10_000 nominal.
    pub max_hp: i32,
    /// Scale on the maximum mana the unit is raised with, 10_000 nominal.
    pub max_mana: i32,
    /// Scale on the gold the unit's seat earns, 10_000 nominal.
    pub gold_income: i32,
}

impl ModifierSpec {
    /// Every field neutral: no stat changes at all.
    pub const NOMINAL: Self = Self {
        magic_resist: 0,
        status_resist: 0,
        physical_damage: NOMINAL_RATE,
        magic_damage: NOMINAL_RATE,
        pure_damage: NOMINAL_RATE,
        cooldown_rate: NOMINAL_RATE,
        mana_cost_rate: NOMINAL_RATE,
        move_speed: NOMINAL_RATE,
        max_hp: NOMINAL_RATE,
        max_mana: NOMINAL_RATE,
        gold_income: NOMINAL_RATE,
    };

    /// The widest magic resistance delta accepted, either way.
    pub const MAX_RESIST: i32 = 10_000;
    /// The most status resistance accepted.
    pub const MAX_STATUS_RESIST: i32 = 9_000;
    /// The lowest scale accepted for every scale field.
    pub const MIN_SCALE: i32 = 2_500;
    /// The highest scale accepted for every scale field.
    pub const MAX_SCALE: i32 = 40_000;

    /// Whether every field lies within the accepted bounds.
    pub const fn is_bounded(&self) -> bool {
        self.magic_resist >= -Self::MAX_RESIST
            && self.magic_resist <= Self::MAX_RESIST
            && self.status_resist >= 0
            && self.status_resist <= Self::MAX_STATUS_RESIST
            && bounded_scale(self.physical_damage)
            && bounded_scale(self.magic_damage)
            && bounded_scale(self.pure_damage)
            && bounded_scale(self.cooldown_rate)
            && bounded_scale(self.mana_cost_rate)
            && bounded_scale(self.move_speed)
            && bounded_scale(self.max_hp)
            && bounded_scale(self.max_mana)
            && bounded_scale(self.gold_income)
    }

    /// Whether every field is neutral.
    pub const fn is_nominal(&self) -> bool {
        self.magic_resist == 0
            && self.status_resist == 0
            && self.physical_damage == NOMINAL_RATE
            && self.magic_damage == NOMINAL_RATE
            && self.pure_damage == NOMINAL_RATE
            && self.cooldown_rate == NOMINAL_RATE
            && self.mana_cost_rate == NOMINAL_RATE
            && self.move_speed == NOMINAL_RATE
            && self.max_hp == NOMINAL_RATE
            && self.max_mana == NOMINAL_RATE
            && self.gold_income == NOMINAL_RATE
    }
}

/// Whether one scale field lies within the accepted bounds.
const fn bounded_scale(value: i32) -> bool {
    value >= ModifierSpec::MIN_SCALE && value <= ModifierSpec::MAX_SCALE
}

/// A shortcut round the rules, honoured only in a match started with
/// cheats on. Anywhere else it is rejected with
/// [`RejectReason::NoCheats`](crate::RejectReason::NoCheats).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cheat {
    /// Gold put into the seat's hand. Negative takes gold away, never
    /// below nothing.
    Gold {
        /// How much.
        amount: i32,
    },
    /// Levels the hero goes up at once, to the cap at most.
    Levels {
        /// How many.
        count: u8,
    },
    /// Health and mana back to full, every cooldown and wait cleared.
    Refresh,
    /// An item put into the hero's bag for nothing, or into the stash when
    /// the bag has no room.
    Item {
        /// Which item.
        item: ItemId,
    },
    /// A stat change put on one unit for a time, replacing whatever this
    /// cheat put there before.
    ApplyModifier {
        /// Which unit it lands on. Nothing means the hero the order is for.
        target: Target,
        /// What it changes. Outside its bounds it is rejected with
        /// [`RejectReason::BadCheat`](crate::RejectReason::BadCheat).
        spec: ModifierSpec,
        /// Ticks it runs, `1..=MAX_MODIFIER_TICKS`.
        ticks: u32,
    },
    /// Takes the stat change this cheat put on one unit away.
    ClearModifiers {
        /// Which unit it is taken off. Nothing means the hero the order is
        /// for.
        target: Target,
    },
}

/// A single instruction from a participant to its own hero.
///
/// A target the issuing team cannot currently see is rejected with
/// [`RejectReason::UnknownTarget`](crate::RejectReason::UnknownTarget).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Order {
    /// Go somewhere, ignoring enemies on the way.
    ///
    /// Aimed at nothing it cancels the current order and stands still. Aimed
    /// at a position it walks there. Aimed at a unit it follows that unit; a
    /// plain follow calls no enemy creeps or towers on or off.
    Move {
        /// Where to go: nothing to stand still, a position to walk to, a
        /// unit to follow.
        target: Target,
    },
    /// Fight whatever the order is aimed at.
    ///
    /// Aimed at nothing it stands still, but attacks anything that comes into
    /// range. Aimed at a position it walks there, stopping to attack enemies
    /// encountered on the way. Aimed at a unit it attacks that unit, following
    /// it if it moves out of range; against a friendly unit this is a follow,
    /// turning into a deny once the unit is low enough to allow one, and
    /// either way an order aimed at a unit calls off any enemy creeps and
    /// towers currently aggroed on the issuer.
    Attack {
        /// What to fight: nothing to hold position, a position to
        /// attack-move to, a unit to attack.
        target: Target,
    },
    /// Cast one of the hero's abilities.
    Cast {
        /// Which of the four ability slots to cast.
        slot: AbilitySlot,
        /// What the ability is aimed at.
        target: Target,
    },
    /// Activate an item in the inventory.
    Use {
        /// Which inventory slot holds the item.
        slot: ItemSlot,
        /// What the item is aimed at.
        target: Target,
    },
    /// Lay an item out of the bag: on the ground, or into an ally's hands.
    ///
    /// Aimed at a position it lands there, aimed at nothing it lands
    /// underfoot, and aimed at an allied unit with a bag it goes into that
    /// bag's first free slot. The unit walks into reach first when it has to.
    Put {
        /// Which bag slot gives the item up. Stash slots take no part.
        slot: ItemSlot,
        /// Where the item goes.
        target: Target,
    },
    /// Take an item lying on the ground into the first free bag slot.
    ///
    /// The unit walks over to it first when it has to. Any unit with a bag may
    /// take any ground item, whoever dropped it.
    Take {
        /// The ground item to take. Must be [`Target::Unit`]; anything else
        /// is rejected with
        /// [`RejectReason::WrongTargetKind`](crate::RejectReason::WrongTargetKind).
        target: Target,
    },
    /// Buy an item. Legal only while standing in the fountain area.
    Buy {
        /// What to buy.
        item: ItemId,
    },
    /// Sell an item from the inventory for part of its cost.
    ///
    /// Away from the shop this marks the stack for sale instead, and a second
    /// order on the same slot unmarks it. A marked stack is sold the moment it
    /// reaches the shop — carried there, delivered by courier, or put in the
    /// stash.
    Sell {
        /// Which inventory slot to empty.
        slot: ItemSlot,
    },
    /// Move an item between two slots, swapping whatever is in the way.
    ///
    /// Stash slots take part only while standing in the home shop area.
    Swap {
        /// The slot being moved from.
        from: ItemSlot,
        /// The slot being moved to.
        to: ItemSlot,
    },
    /// Spend a skill point on an ability.
    Learn {
        /// Which of the four ability slots to level.
        slot: AbilitySlot,
    },
    /// Take a shortcut round the rules. Interrupts nothing the body is doing.
    Cheat {
        /// Which one.
        cheat: Cheat,
    },
}
