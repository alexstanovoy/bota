//! Sylla's spellwork.
//!
//! The bolt is thrown at a body and jumps from it to the ones around; the
//! volley and the frenzy both work on the caster and are weighed by what
//! stands near.

use bota_proto::{Target, UnitView};

use crate::{Ask, BOUNCE, FIGHT_RANGE, FRENZY, Field, VOLLEY, span};

/// Enemy heroes within a volley's reach that make it worth loosing.
pub const VOLLEY_CROWD: usize = 1;

/// Creeps within a volley's reach that make it worth loosing at the wave.
pub const VOLLEY_WAVE: usize = 4;

/// How near two creeps must stand for a bolt to jump between them.
pub const BOUNCE_SPREAD: f32 = 500.0;

/// Creeps a bolt must be able to jump between to be worth throwing at a wave.
pub const BOUNCE_CHAIN: usize = 3;

/// Mana, as a part of the whole, kept back from throwing bolts at a wave.
pub const BOLT_SPARE_MANA: f32 = 0.55;

/// What Sylla would cast this tick.
pub fn sylla_spell(field: &Field, _beat: &crate::Beat) -> Option<Ask> {
    volley(field)
        .or_else(|| frenzy(field))
        .or_else(|| bolt(field))
}

/// The volley, when there is a crowd for it.
fn volley(field: &Field) -> Option<Ask> {
    let me = field.me?;
    let (slot, held) = field.ability(VOLLEY)?;
    if held.level == 0 || held.cooldown_left > 0 || me.mana < held.mana_cost {
        return None;
    }
    let reach = held.range as f32;
    let heroes = field.foes_within(held.range).count();
    let creeps = field
        .creeps
        .iter()
        .filter(|creep| span(creep.pos, me.pos) <= reach)
        .count();
    (heroes >= VOLLEY_CROWD || creeps >= VOLLEY_WAVE).then(|| Ask::cast(slot, Target::None))
}

/// The frenzy, when there is something in front worth swinging faster at.
fn frenzy(field: &Field) -> Option<Ask> {
    let me = field.me?;
    let (slot, held) = field.ability(FRENZY)?;
    if held.level == 0 || held.cooldown_left > 0 || me.mana < held.mana_cost {
        return None;
    }
    let fighting = field.foes_within(FIGHT_RANGE).next().is_some();
    let sieging = field
        .enemy_works
        .first()
        .is_some_and(|works| field.in_reach(works));
    (fighting || sieging).then(|| Ask::cast(slot, Target::None))
}

/// The bolt, thrown at a hero when one is in range and at the wave when the
/// mana is spare.
fn bolt(field: &Field) -> Option<Ask> {
    let me = field.me?;
    let (slot, held) = field.ability(BOUNCE)?;
    if held.level == 0 || held.cooldown_left > 0 || me.mana < held.mana_cost {
        return None;
    }
    let reach = held.range as f32;
    if let Some(mark) = field
        .foes_within(held.range)
        .min_by(|one, other| crate::order_by(span(one.pos, me.pos), span(other.pos, me.pos)))
    {
        return Some(Ask::cast(slot, Target::Unit(mark.id)));
    }
    if field.mana() < BOLT_SPARE_MANA {
        return None;
    }
    let mark = field.creeps.iter().copied().find(|creep| {
        span(creep.pos, me.pos) <= reach && chain_from(field, creep) >= BOUNCE_CHAIN
    })?;
    Some(Ask::cast(slot, Target::Unit(mark.id)))
}

/// How many creeps a bolt thrown at one would find, itself counted.
fn chain_from(field: &Field, first: &UnitView) -> usize {
    1 + field
        .creeps
        .iter()
        .filter(|creep| creep.id != first.id && span(creep.pos, first.pos) <= BOUNCE_SPREAD)
        .count()
}
