//! What the model may choose, and what a choice means.
//!
//! A flat, numbered list. Every number means one thing and means it on every
//! tick: number nineteen is always "step forward and left", never "swing at
//! whatever happens to be nearest". Where a deed names a body it names it by
//! its place in [`Field`], which is settled in one place for exactly this
//! reason.
//!
//! The list is laid out as blocks, and a block is added by putting it in
//! [`BLOCKS`] and giving it an arm. Nothing counts indices by hand: the
//! offsets are worked out from the table, and a test walks every index there
//! and back again.
//!
//! **Legality is decided here, not by the model.** Every tick comes with a
//! flag per deed saying whether it could be carried out, and the model is
//! never allowed to pick one that could not. Letting it choose a deed that
//! cannot happen and taking the points off afterwards was considered and
//! dropped: an order is one a tick, so a wasted pick is a lost creep, and the
//! rules of what is legal are known to us for nothing. What the flags do earn
//! is a model that never has to spend its capacity learning them.

use crate::{CREEPS, HEROES, OWN_CREEPS};

/// Ability slots a hero carries.
pub const ABILITIES: usize = 6;
/// Inventory slots an item may be used from.
pub const ITEMS: usize = 6;
/// Directions a step may be taken in.
pub const STEPS: usize = 8;
/// How far one step goes, in world units.
pub const STEP: f32 = 400.0;
/// The named places a deed may send the bot to.
pub const PLACES: usize = 3;
/// Ways a cast may be aimed.
pub const AIMS: usize = 4;
/// Errands a courier may be sent on.
pub const ERRANDS: usize = 4;

/// The blocks the list is made of, in order.
///
/// The order is the numbering. Adding to the end never moves what is already
/// there, which matters once a set of weights has been trained: the model has
/// learned what each number means.
pub const BLOCKS: [(&str, usize); 12] = [
    ("stand", 1),
    ("swing at a creep", CREEPS),
    ("put out a creep of its own", OWN_CREEPS),
    ("swing at a hero", HEROES),
    ("step", STEPS),
    ("go to a place", PLACES),
    ("cast", ABILITIES * AIMS),
    ("use an item", ITEMS),
    ("buy", 1),
    ("spend a skill point", ABILITIES),
    ("send the courier", ERRANDS),
    ("sell an item", ITEMS),
];

/// How many deeds there are altogether.
pub const DEEDS: usize = {
    let mut total = 0;
    let mut at = 0;
    while at < BLOCKS.len() {
        total += BLOCKS[at].1;
        at += 1;
    }
    total
};

/// Where each block starts.
const fn start_of(block: usize) -> usize {
    let mut total = 0;
    let mut at = 0;
    while at < block {
        total += BLOCKS[at].1;
        at += 1;
    }
    total
}

/// One thing the model may choose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deed {
    /// Stand still and pay attention to nobody.
    Stand,
    /// Swing at the creep of the other side in this place of the list.
    Swing(usize),
    /// Swing at one of its own, to put it out.
    PutOut(usize),
    /// Swing at the hero of the other side in this seat.
    Fight(usize),
    /// Take a step: nought is forward, and round to the left from there.
    Step(usize),
    /// Walk to somewhere named.
    GoTo(Place),
    /// Cast an ability, aimed the given way.
    Cast(usize, Aim),
    /// Use what is in an inventory slot.
    Use(usize),
    /// Buy the next thing wanted.
    Buy,
    /// Put the spare skill point into a slot.
    Learn(usize),
    /// Send the courier on an errand.
    Errand(Errand),
    /// Sell what is in an inventory slot. At the shop it sells at once;
    /// anywhere else it marks the stack to be sold, and asked again unmarks
    /// it.
    Sell(usize),
}

/// Somewhere a deed may send the bot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Its own fountain.
    Home,
    /// The nearest tower of its own side.
    OwnTower,
    /// The nearest tower of the other side.
    TheirTower,
}

/// How a cast is aimed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aim {
    /// At nobody: on itself, or wherever it stands.
    Own,
    /// At the nearest hero of the other side.
    Hero,
    /// At the nearest creep of the other side.
    Creep,
    /// At a spot a little way forward.
    Ahead,
}

/// An errand a courier may be sent on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Errand {
    /// Fetch what waits in the stash.
    TakeStash,
    /// Bring what it holds.
    Deliver,
    /// Fly faster for a while.
    Burst,
    /// Go home and stay there.
    GoHome,
}

impl Errand {
    /// The courier ability this errand is, by its number.
    pub fn ability(self) -> u16 {
        match self {
            Errand::Burst => crate::BURST,
            Errand::TakeStash => crate::TAKE_STASH,
            Errand::Deliver => crate::DELIVER,
            Errand::GoHome => crate::GO_HOME,
        }
    }
}

impl Deed {
    /// What the number means.
    pub fn at(index: usize) -> Option<Deed> {
        let mut left = index;
        for (block, (_, size)) in BLOCKS.iter().enumerate() {
            if left < *size {
                return Some(Deed::in_block(block, left));
            }
            left -= size;
        }
        None
    }

    /// Which number it is.
    pub fn index(self) -> usize {
        match self {
            Deed::Stand => start_of(0),
            Deed::Swing(at) => start_of(1) + at,
            Deed::PutOut(at) => start_of(2) + at,
            Deed::Fight(at) => start_of(3) + at,
            Deed::Step(at) => start_of(4) + at,
            Deed::GoTo(place) => {
                start_of(5)
                    + match place {
                        Place::Home => 0,
                        Place::OwnTower => 1,
                        Place::TheirTower => 2,
                    }
            }
            Deed::Cast(slot, aim) => {
                start_of(6)
                    + slot * AIMS
                    + match aim {
                        Aim::Own => 0,
                        Aim::Hero => 1,
                        Aim::Creep => 2,
                        Aim::Ahead => 3,
                    }
            }
            Deed::Use(slot) => start_of(7) + slot,
            Deed::Buy => start_of(8),
            Deed::Learn(slot) => start_of(9) + slot,
            Deed::Errand(errand) => {
                start_of(10)
                    + match errand {
                        Errand::TakeStash => 0,
                        Errand::Deliver => 1,
                        Errand::Burst => 2,
                        Errand::GoHome => 3,
                    }
            }
            Deed::Sell(at) => start_of(11) + at,
        }
    }

    /// The deed at a place within one block.
    fn in_block(block: usize, at: usize) -> Deed {
        match block {
            0 => Deed::Stand,
            1 => Deed::Swing(at),
            2 => Deed::PutOut(at),
            3 => Deed::Fight(at),
            4 => Deed::Step(at),
            5 => Deed::GoTo(match at {
                0 => Place::Home,
                1 => Place::OwnTower,
                _ => Place::TheirTower,
            }),
            6 => Deed::Cast(
                at / AIMS,
                match at % AIMS {
                    0 => Aim::Own,
                    1 => Aim::Hero,
                    2 => Aim::Creep,
                    _ => Aim::Ahead,
                },
            ),
            7 => Deed::Use(at),
            8 => Deed::Buy,
            9 => Deed::Learn(at),
            10 => Deed::Errand(match at {
                0 => Errand::TakeStash,
                1 => Errand::Deliver,
                2 => Errand::Burst,
                _ => Errand::GoHome,
            }),
            _ => Deed::Sell(at),
        }
    }
}
