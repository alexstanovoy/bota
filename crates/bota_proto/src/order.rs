//! What a participant asks the units of its seat to do.
//!
//! Every unit a seat drives takes orders of its own, and keeps a queue of
//! orders to take up one after another, as Dota's Shift-queue: see
//! [`ClientMsg::Order::queue`](crate::ClientMsg::Order) and the
//! [`contract`](crate::contract).

use crate::{AbilitySlot, EntityId, HeroId, ItemId, ItemSlot, TalentSlot, Vec2};
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

/// A scale at nominal, in basis points.
const NOMINAL_RATE: i32 = 10_000;

/// Bounded stat changes a cheat may put on one unit.
///
/// Scales are in basis points of the nominal 10_000. Several sources add as
/// deltas of the nominal and never compound. Resistances are additive
/// hundredths of a percentage point.
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
    Item { item: ItemId },
    /// A stat change put on one unit for a time, replacing whatever this
    /// cheat put there before.
    ApplyModifier {
        /// Which unit it lands on. Nothing means the unit the order is for.
        target: Target,
        /// What it changes. Outside its bounds it is rejected with
        /// [`RejectReason::BadCheat`](crate::RejectReason::BadCheat).
        spec: ModifierSpec,
        /// Ticks it runs, `1..=MAX_MODIFIER_TICKS`.
        ticks: u32,
    },
    /// Takes the stat change this cheat put on one unit away.
    ClearModifiers {
        /// Which unit it is taken off. Nothing means the unit the order is
        /// for.
        target: Target,
    },
    /// Every hero no player drives (one a cheat created) goes up so many
    /// levels, to the cap at most (Dota's `-levelbots`).
    LevelBots {
        /// How many.
        count: u8,
    },
    /// Abilities and items cost no mana and leave no cooldown, for every
    /// unit (Dota's `-wtf`, with a refresh), or do again (`-unwtf`).
    Wtf {
        /// Whether they are free.
        on: bool,
    },
    /// The seat's hero stands up at its fountain at once with full pools,
    /// dead or alive (Dota's `-respawn`).
    Respawn,
    /// A hero created where told, driven by no player: on the seat's side,
    /// or the other (Dota's `-createhero NAME [enemy]`).
    CreateHero {
        hero: HeroId,
        /// On the other side.
        enemy: bool,
        /// Where it stands up: the open ground nearest this spot.
        at: Vec2,
    },
    /// A wave in every lane for both sides now (Dota's `-spawncreeps`).
    SpawnCreeps,
    /// Lane waves and neutral camps spawn on their clocks, or not at all
    /// (Dota's `-enablecreepspawn` / `-disablecreepspawn`).
    CreepSpawning {
        /// Whether they spawn.
        on: bool,
    },
    /// Every neutral camp with nothing in its spawn box fills now (Dota's
    /// `-spawnneutrals`).
    SpawnNeutrals,
    /// Every creep of a kind is taken away, with no bounty and nothing done
    /// on its death (Dota's `-killcreeps`).
    KillCreeps {
        /// Whose.
        which: CreepSide,
    },
    /// The pregame ends now: the game clock jumps to the horn (Dota's
    /// `-startgame`). Nothing once the game has begun.
    StartGame,
    /// Each side sees what either sees (Dota's `-allvision`), or its own
    /// again (`-normalvision`).
    AllVision {
        /// Whether the sides share it.
        on: bool,
    },
    /// The seat's hero takes no damage, dead and back again, until turned
    /// off (Dota's Demo Hero's Invulnerable).
    Invulnerable {
        /// Whether it is.
        on: bool,
    },
    /// The seat's hero moves to the open ground nearest a spot at once
    /// (Dota's `-teleport`).
    Teleport {
        /// Where to.
        at: Vec2,
    },
    /// The seat's hero starts over at its fountain: level one, the starting
    /// gold, nothing carried or stashed (Dota's Demo Hero's Reset Hero).
    ResetHero,
    /// Day breaks or night falls now, the cycle going on from there (Dota's
    /// Demo Hero's night toggle).
    TimeOfDay {
        /// Whether it is night from now on.
        night: bool,
    },
    /// A creep camp's worth of lane creeps of the seat's side, or the
    /// other's, at a spot, marching the nearest lane from there (Dota's
    /// Demo Hero's creeps spawned at a spot).
    CreateCreeps {
        /// On the other side.
        enemy: bool,
        /// Where they stand up.
        at: Vec2,
    },
}

/// Whose creeps a [`Cheat::KillCreeps`] takes away.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CreepSide {
    Radiant,
    Dire,
    Neutral,
    All,
}

/// A single instruction to one unit a seat drives.
///
/// A target the issuing team cannot currently see is rejected with
/// [`RejectReason::UnknownTarget`](crate::RejectReason::UnknownTarget).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Order {
    /// Go somewhere, ignoring enemies on the way.
    ///
    /// Aimed at nothing it cancels the current order and stands still,
    /// attacking nothing. Aimed at a position it walks there. Aimed at a unit
    /// it follows that unit.
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
    /// turning into a deny once the unit is low enough to allow one.
    ///
    /// A hero's attack order at an enemy hero draws nearby enemy creeps and
    /// towers onto the hero; one at a friendly unit calls them off.
    Attack {
        /// What to fight: nothing to hold position, a position to
        /// attack-move to, a unit to attack.
        target: Target,
    },
    /// Cast one of the unit's abilities. A hero's cast at an enemy hero draws
    /// creeps and towers as an attack order does.
    Cast {
        /// Which ability slot to cast.
        slot: AbilitySlot,
        /// What the ability is aimed at.
        target: Target,
    },
    /// Activate an item in the inventory. Backpack and stash items cannot be
    /// used.
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
    /// Buy an item, with any of its parts not already held. It goes into the
    /// hero's bag while at the home shop and there is room, into the stash
    /// otherwise. Needs no body: a seat whose hero lies dead buys into the
    /// stash, what the hero carried counting as held, as in Dota.
    Buy {
        /// What to buy.
        item: ItemId,
    },
    /// Sell an item for part of its cost.
    ///
    /// The stash sells from anywhere. A bag away from the shop marks the stack
    /// for sale instead, and a second order on the same slot unmarks it; a
    /// marked stack is sold once it reaches the shop. While the hero lies
    /// dead only what was bought since it fell sells, from the stash, as in
    /// Dota; anything else is refused with
    /// [`RejectReason::HeroDead`](crate::RejectReason::HeroDead).
    Sell {
        /// Which slot to empty.
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
        /// Which ability slot to level.
        slot: AbilitySlot,
    },
    /// Take a shortcut round the rules. Interrupts nothing the body is doing.
    Cheat {
        /// Which one.
        cheat: Cheat,
    },
    /// Take a talent of the hero's tree. Talents spend no skill point: each
    /// opening level brings a talent point of its own. Interrupts nothing
    /// the body is doing.
    ///
    /// A tier opens at its hero level and takes one of its two talents; the
    /// other opens at levels 27 to 30, a tier a level. A talent is never
    /// unlearned. Refused with
    /// [`RejectReason::CannotLearn`](crate::RejectReason::CannotLearn)
    /// otherwise.
    LearnTalent { talent: TalentSlot },
    /// Spend a skill point on the Attribute Bonus of a hero with a talent
    /// tree: two of every attribute a level, seven levels, level `k` from
    /// hero level `2k`. A level up whose point no ability could take puts
    /// it there by itself. Interrupts nothing the body is doing. Refused
    /// with [`RejectReason::CannotLearn`](crate::RejectReason::CannotLearn)
    /// otherwise.
    LearnAttributeBonus,
    /// Activate the side's Glyph of Fortification: its structures take no
    /// damage for seven seconds, its lane creeps for three, and its towers
    /// of tier two and above shoot five more targets with each shot. One
    /// cooldown serves the whole side, so any seat's order spends it.
    /// Needs no body: a seat whose hero lies dead may give it. Refused with
    /// [`RejectReason::OnCooldown`](crate::RejectReason::OnCooldown) while
    /// the Glyph recharges.
    Glyph,
    /// Switch which kind of ward a Ward Dispenser plants next, an Observer
    /// Ward or a Sentry Ward: Dota's double tap on the item. It costs no
    /// charge and starts no wait, and interrupts nothing the body is doing.
    /// Refused with [`RejectReason::EmptySlot`](crate::RejectReason::EmptySlot)
    /// for a slot holding nothing and
    /// [`RejectReason::NotCastable`](crate::RejectReason::NotCastable) for one
    /// holding anything but a Ward Dispenser.
    SwitchWard {
        /// The bag slot holding the dispenser.
        slot: ItemSlot,
    },
}

impl Order {
    /// Whether the order needs the hero standing when it names no unit: all
    /// but buying, selling (from the stash), the Glyph and a cheat (as
    /// Dota's `-respawn`), which a seat whose hero lies dead may still give.
    pub const fn needs_body(&self) -> bool {
        !matches!(
            self,
            Order::Buy { .. } | Order::Sell { .. } | Order::Glyph | Order::Cheat { .. }
        )
    }

    /// Whether the order waits its turn in its unit's queue when it is given
    /// queued: what the body carries out (a move, an attack, a cast, a use,
    /// laying an item down or taking one up). Learning, talents, buying,
    /// selling, swapping, cheats and the Glyph are carried out at once,
    /// queued or not, and leave the queue as it is.
    pub const fn queues(&self) -> bool {
        match self {
            Order::Move { .. }
            | Order::Attack { .. }
            | Order::Cast { .. }
            | Order::Use { .. }
            | Order::Put { .. }
            | Order::Take { .. } => true,
            Order::Buy { .. }
            | Order::Sell { .. }
            | Order::Swap { .. }
            | Order::Learn { .. }
            | Order::Cheat { .. }
            | Order::LearnTalent { .. }
            | Order::LearnAttributeBonus
            | Order::SwitchWard { .. }
            | Order::Glyph => false,
        }
    }

    /// Calls `visit` on the entity handle the order names, if it names one:
    /// the one walk that puts a side's names into an order, and takes them
    /// out again.
    pub fn visit_ids(&mut self, visit: &mut dyn FnMut(&mut EntityId)) {
        let target = match self {
            Order::Move { target }
            | Order::Attack { target }
            | Order::Cast { target, .. }
            | Order::Use { target, .. }
            | Order::Put { target, .. }
            | Order::Take { target }
            | Order::Cheat {
                cheat: Cheat::ApplyModifier { target, .. } | Cheat::ClearModifiers { target },
            } => target,
            Order::Cheat {
                cheat:
                    Cheat::Gold { .. }
                    | Cheat::Levels { .. }
                    | Cheat::Refresh
                    | Cheat::Item { .. }
                    | Cheat::LevelBots { .. }
                    | Cheat::Wtf { .. }
                    | Cheat::Respawn
                    | Cheat::CreateHero { .. }
                    | Cheat::SpawnCreeps
                    | Cheat::CreepSpawning { .. }
                    | Cheat::SpawnNeutrals
                    | Cheat::KillCreeps { .. }
                    | Cheat::StartGame
                    | Cheat::AllVision { .. }
                    | Cheat::Invulnerable { .. }
                    | Cheat::Teleport { .. }
                    | Cheat::ResetHero
                    | Cheat::TimeOfDay { .. }
                    | Cheat::CreateCreeps { .. },
            }
            | Order::Buy { .. }
            | Order::Sell { .. }
            | Order::Swap { .. }
            | Order::Learn { .. }
            | Order::LearnTalent { .. }
            | Order::LearnAttributeBonus
            | Order::SwitchWard { .. }
            | Order::Glyph => return,
        };
        if let Target::Unit(id) = target {
            visit(id);
        }
    }
}
