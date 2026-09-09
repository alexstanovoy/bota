//! The numbers a model is shown.
//!
//! One place, one layout, and a table that says what is in it. A block is
//! added by putting it in [`LAYOUT`] and writing the piece that fills it;
//! nothing counts offsets by hand, and a test holds the table to the length it
//! claims. The names are there so that a weight can be traced back to a thing
//! in the game when a trained model does something strange.
//!
//! Everything is brought to about the same size before it goes in. A model fed
//! gold in thousands and health in fractions spends its first thousand steps
//! learning the scale rather than the game. Distances are in thousands of
//! world units, because a lane is fifteen of those and a swing reaches half of
//! one.
//!
//! Nothing here is absolute. Where a body is, is where it is from the bot and
//! which way the bot is facing up the map, so one set of weights serves both
//! ends of it. What is missing — a creep that is not there, a hero out of
//! sight — is a nought in its place and a nought in the flag that says it is
//! there, never a made-up number.

use crate::{CREEPS, Field, HEROES, OWN_CREEPS, part, span};

/// Numbers describing the bot itself.
pub const SELF_NUMBERS: usize = 16;
/// Numbers describing the match as a whole.
pub const MATCH_NUMBERS: usize = 4;
/// Numbers describing the role and the lane it belongs in.
pub const ROLE_NUMBERS: usize = crate::ROLES + 3;
/// Numbers describing one body of the other side's wave.
pub const CREEP_NUMBERS: usize = 6;
/// Numbers describing one of its own.
pub const OWN_CREEP_NUMBERS: usize = 5;
/// Numbers describing one hero of the other side.
pub const HERO_NUMBERS: usize = 7;
/// Numbers describing the towers on hand.
pub const TOWER_NUMBERS: usize = 8;
/// Numbers describing the courier and the stash.
pub const ERRAND_NUMBERS: usize = 6;
/// Numbers describing one ability slot.
pub const ABILITY_NUMBERS: usize = 4;
/// Numbers describing one inventory slot.
pub const ITEM_NUMBERS: usize = 4;
/// Numbers describing one spare slot: the backpack, then the stash.
pub const SPARE_NUMBERS: usize = 2;

/// What the vector is made of, in order.
pub const LAYOUT: [(&str, usize); 11] = [
    ("itself", SELF_NUMBERS),
    ("the match", MATCH_NUMBERS),
    ("its role and lane", ROLE_NUMBERS),
    ("their creeps", CREEPS * CREEP_NUMBERS),
    ("its own creeps", OWN_CREEPS * OWN_CREEP_NUMBERS),
    ("their heroes", HEROES * HERO_NUMBERS),
    ("towers", TOWER_NUMBERS),
    ("the courier", ERRAND_NUMBERS),
    ("abilities", crate::ABILITIES * ABILITY_NUMBERS),
    ("items", crate::ITEMS * ITEM_NUMBERS),
    ("spare items", crate::SPARES * SPARE_NUMBERS),
];

/// How many numbers a model is shown.
pub const NUMBERS: usize = {
    let mut total = 0;
    let mut at = 0;
    while at < LAYOUT.len() {
        total += LAYOUT[at].1;
        at += 1;
    }
    total
};

/// The whole of what a model is given for one tick.
///
/// The numbers and, beside them, which deeds could be done. The second half is
/// as much a part of the tick as the first: a model that is shown the world
/// but not what it may do about it has to guess at legality, and guessing is
/// what the flags are for.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    /// Which tick this is.
    pub at: u32,
    /// The numbers, in the order [`LAYOUT`] gives.
    pub numbers: Vec<f32>,
    /// One flag per deed, in the order the deeds are numbered.
    pub allowed: Vec<bool>,
}

impl Shown {
    /// Whether there is anything at all to choose this tick.
    pub fn anything_to_do(&self) -> bool {
        self.allowed.iter().any(|may| *may)
    }
}

/// Everything a model is shown for one tick.
pub fn shown(field: &Field) -> Shown {
    Shown {
        at: field.view.tick,
        numbers: sight(field),
        allowed: crate::allowed(field),
    }
}

/// The numbers alone.
pub fn sight(field: &Field) -> Vec<f32> {
    let mut out = Vec::with_capacity(NUMBERS);
    itself(field, &mut out);
    the_match(field, &mut out);
    its_role(field, &mut out);
    their_creeps(field, &mut out);
    its_own_creeps(field, &mut out);
    their_heroes(field, &mut out);
    towers(field, &mut out);
    the_courier(field, &mut out);
    abilities(field, &mut out);
    items(field, &mut out);
    spare_items(field, &mut out);
    debug_assert_eq!(out.len(), NUMBERS, "the layout and the filling disagree");
    out.resize(NUMBERS, 0.0);
    out
}

/// What the bot is.
fn itself(field: &Field, out: &mut Vec<f32>) {
    let Some(me) = field.me else {
        out.extend(std::iter::repeat_n(0.0, SELF_NUMBERS));
        return;
    };
    let statuses = me.statuses.bits;
    let flagged = |bit: u16| f32::from(statuses & bit != 0);
    out.push(1.0); // standing
    out.push(part(me.hp, me.max_hp));
    out.push(part(me.mana, me.max_mana));
    out.push(f32::from(me.level) / 10.0);
    out.push(field.seat.gold.unwrap_or(0) as f32 / 1000.0);
    out.push(me.attack_range.to_f32() / 1000.0);
    out.push(me.attack_damage as f32 / 100.0);
    out.push(me.move_speed.to_f32() / 400.0);
    out.push(me.armor.to_f32() / 10.0);
    out.push(flagged(bota_proto::StatusFlags::STUNNED));
    out.push(flagged(bota_proto::StatusFlags::ROOTED));
    out.push(flagged(bota_proto::StatusFlags::SILENCED));
    out.push(flagged(bota_proto::StatusFlags::SLOWED));
    out.push(flagged(bota_proto::StatusFlags::DOT));
    // Whether a point could go somewhere right now: the razes share a level
    // and cost one point together, so levels are not summed here; the
    // snapshot's own answer per slot is.
    out.push(f32::from(
        me.abilities.iter().any(|ability| ability.can_level),
    ));
    out.push(under_fire(field) / 5.0);
}

/// What the match is.
fn the_match(field: &Field, out: &mut Vec<f32>) {
    out.push(field.view.tick as f32 / 18000.0);
    out.push(f32::from(field.team == bota_proto::Team::Radiant));
    out.push(f32::from(field.seat.deaths).min(10.0) / 10.0);
    let (forward, left) = field
        .home
        .map_or((0.0, 0.0), |home| field.seen_from_here(home));
    // How far home is, and which way: one number for each, so that "a long way
    // behind" and "a long way ahead" are not the same to a model.
    out.push(forward / 18.0);
    let _ = left;
}

/// What it is there to do, and how well it is doing it.
///
/// The role is a row of flags rather than a number: roles are not a scale, and
/// a model told that a role is "three" would believe three sits between two
/// and four.
fn its_role(field: &Field, out: &mut Vec<f32>) {
    for at in 0..crate::ROLES {
        out.push(f32::from(at == field.role.at()));
    }
    match field.lane() {
        None => out.extend(std::iter::repeat_n(0.0, 3)),
        Some(lane) => {
            let at = field.at();
            out.push(lane.off_the_line(at) / 1000.0);
            out.push(lane.how_far_along(at) / lane.length().max(1.0));
            // Where the far end of its own wave is, which is what holding a
            // lane means standing with.
            out.push(
                crate::furthest_own_creep(field, &lane)
                    .map_or(0.0, |creep| field.seen_from_here(creep.pos).0),
            );
        }
    }
}

/// The creeps of the other side, nearest first.
fn their_creeps(field: &Field, out: &mut Vec<f32>) {
    for at in 0..CREEPS {
        match field.creeps.get(at) {
            None => out.extend(std::iter::repeat_n(0.0, CREEP_NUMBERS)),
            Some(creep) => {
                let (forward, left) = field.seen_from_here(creep.pos);
                out.push(1.0);
                out.push(forward);
                out.push(left);
                out.push(part(creep.hp, creep.max_hp));
                // What one swing would leave it with, as a part of that swing:
                // below one it falls to the next hit, which is the whole of
                // last hitting and is not worth making a model rediscover.
                out.push(swings_from_death(field, creep).clamp(-4.0, 4.0));
                out.push(f32::from(field.in_reach(creep)));
            }
        }
    }
}

/// The creeps of its own.
fn its_own_creeps(field: &Field, out: &mut Vec<f32>) {
    for at in 0..OWN_CREEPS {
        match field.own_creeps.get(at) {
            None => out.extend(std::iter::repeat_n(0.0, OWN_CREEP_NUMBERS)),
            Some(creep) => {
                let (forward, left) = field.seen_from_here(creep.pos);
                out.push(1.0);
                out.push(forward);
                out.push(left);
                out.push(part(creep.hp, creep.max_hp));
                out.push(swings_from_death(field, creep).clamp(-4.0, 4.0));
            }
        }
    }
}

/// The heroes of the other side, by seat.
fn their_heroes(field: &Field, out: &mut Vec<f32>) {
    for at in 0..HEROES {
        match field.heroes.get(at) {
            None => out.extend(std::iter::repeat_n(0.0, HERO_NUMBERS)),
            Some(hero) => {
                let (forward, left) = field.seen_from_here(hero.pos);
                out.push(1.0);
                out.push(forward);
                out.push(left);
                out.push(part(hero.hp, hero.max_hp));
                out.push(part(hero.mana, hero.max_mana));
                out.push(f32::from(hero.level) / 10.0);
                out.push(f32::from(field.gap_to(hero) <= hero.attack_range.to_f32()));
            }
        }
    }
}

/// The nearest tower of each side.
fn towers(field: &Field, out: &mut Vec<f32>) {
    for tower in [field.towers.0, field.towers.1] {
        match tower {
            None => out.extend(std::iter::repeat_n(0.0, TOWER_NUMBERS / 2)),
            Some(tower) => {
                let (forward, left) = field.seen_from_here(tower.pos);
                out.push(1.0);
                out.push(forward);
                out.push(left);
                out.push(f32::from(
                    span(field.at(), tower.pos) <= tower.attack_range.to_f32(),
                ));
            }
        }
    }
}

/// The courier, and what waits at home.
fn the_courier(field: &Field, out: &mut Vec<f32>) {
    match field.courier {
        None => out.extend(std::iter::repeat_n(0.0, 4)),
        Some(bird) => {
            let (forward, left) = field.seen_from_here(bird.pos);
            out.push(1.0);
            out.push(forward / 18.0);
            out.push(left / 18.0);
            out.push(f32::from(bird.items.iter().any(Option::is_some)));
        }
    }
    let waiting = field
        .seat
        .stash
        .as_ref()
        .map_or(0, |slots| slots.iter().flatten().count());
    out.push(waiting as f32 / 6.0);
    out.push(f32::from(waiting > 0));
}

/// What it can cast.
fn abilities(field: &Field, out: &mut Vec<f32>) {
    for at in 0..crate::ABILITIES {
        match field.me.and_then(|me| me.abilities.get(at)) {
            None => out.extend(std::iter::repeat_n(0.0, ABILITY_NUMBERS)),
            Some(ability) => {
                out.push(f32::from(ability.level) / 4.0);
                out.push((ability.cooldown_left as f32).min(900.0) / 900.0);
                out.push(f32::from(
                    field.me.is_some_and(|me| me.mana >= ability.mana_cost),
                ));
                out.push(f32::from(ability.cooldown_left == 0 && ability.level > 0));
            }
        }
    }
}

/// What it wears.
fn items(field: &Field, out: &mut Vec<f32>) {
    for at in 0..crate::ITEMS {
        match field
            .me
            .and_then(|me| me.items.get(at))
            .and_then(|slot| slot.as_ref())
        {
            None => out.extend(std::iter::repeat_n(0.0, ITEM_NUMBERS)),
            Some(held) => {
                out.push(1.0);
                out.push(f32::from(held.charges.unwrap_or(0)) / 3.0);
                out.push(f32::from(held.cooldown_left == 0));
                out.push(crate::cost_of(held.id.0) as f32 / 1000.0);
            }
        }
    }
}

/// What it carries but does not wear: the backpack, then the stash, in the
/// order the swap deeds count them.
fn spare_items(field: &Field, out: &mut Vec<f32>) {
    for spare in 0..crate::SPARES {
        let outside = crate::WORN_SLOTS + spare;
        let held = if outside < crate::BAG_SLOTS {
            field
                .me
                .and_then(|me| me.items.get(outside))
                .and_then(|slot| slot.as_ref())
        } else {
            field
                .seat
                .stash
                .as_ref()
                .and_then(|slots| slots.get(outside - crate::BAG_SLOTS))
                .and_then(|slot| slot.as_ref())
        };
        match held {
            None => out.extend(std::iter::repeat_n(0.0, SPARE_NUMBERS)),
            Some(held) => {
                out.push(1.0);
                out.push(crate::cost_of(held.id.0) as f32 / 1000.0);
            }
        }
    }
}

/// Damage a tick landing on the bot right now.
pub fn under_fire(field: &Field) -> f32 {
    let Some(me) = field.me else {
        return 0.0;
    };
    field
        .view
        .units
        .iter()
        .filter(|unit| unit.team != field.team && unit.hp > 0)
        .filter(|unit| unit.attack_damage > 0 && unit.attack_interval > 0)
        .filter(|unit| field.gap_to(unit) <= unit.attack_range.to_f32())
        .map(|unit| unit.attack_damage as f32 / unit.attack_interval as f32)
        .sum::<f32>()
        / f32::from(1 + me.armor.to_f32() as u16)
}

/// How many of the bot's swings a body is from falling.
///
/// Below one it falls to the next: the whole of a last hit in one number.
/// Armor is taken off here because a model would otherwise have to learn what
/// armor does from a reward that arrives a minute later.
pub fn swings_from_death(field: &Field, body: &bota_proto::UnitView) -> f32 {
    let Some(me) = field.me else {
        return 0.0;
    };
    let armor = body.armor.to_f32().max(0.0);
    let blow = me.attack_damage as f32 * 100.0 / (100.0 + ARMOR_SCALE * armor);
    if blow <= 0.0 {
        return 4.0;
    }
    body.hp as f32 / blow
}

/// What one point of armor adds to the hundred a blow is divided by.
const ARMOR_SCALE: f32 = 6.0;
