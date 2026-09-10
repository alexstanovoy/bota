//! Where a hero's skill points go.
//!
//! A plan is the abilities in the order points are spent on them, one entry
//! per point. The place of an entry is the hero level that point is spent at,
//! so the tenth entry is the last point a hero gets.

use bota_proto::{AbilityId, AbilitySlot, HeroId};

use crate::{
    BOUNCE, CRIT, FRENZY, Field, NECROMASTERY, RAZE_NEAR, REQUIEM, SHADOW_FIEND, SYLLA, VOLLEY,
};

/// Shadow Fiend's points: the razes first, the souls beside them, the
/// requiem whenever it opens.
pub const FIEND_PLAN: [AbilityId; 10] = [
    RAZE_NEAR,
    NECROMASTERY,
    RAZE_NEAR,
    NECROMASTERY,
    RAZE_NEAR,
    REQUIEM,
    RAZE_NEAR,
    REQUIEM,
    NECROMASTERY,
    REQUIEM,
];

/// Sylla's points: the bolt first, the crit beside it, the volley whenever it
/// opens.
pub const SYLLA_PLAN: [AbilityId; 10] = [
    BOUNCE, CRIT, BOUNCE, CRIT, BOUNCE, VOLLEY, BOUNCE, VOLLEY, FRENZY, VOLLEY,
];

/// The plan a hero follows. Empty for one the bot has no plan for.
pub fn plan_of(hero: HeroId) -> &'static [AbilityId] {
    match hero {
        SHADOW_FIEND => &FIEND_PLAN,
        SYLLA => &SYLLA_PLAN,
        _ => &[],
    }
}

/// The slot a waiting skill point should go into.
///
/// The plan is followed while it can be; a point the plan has nowhere to put
/// goes into the first slot of the plan that will take one, and then into the
/// first slot at all. `None` when no point is waiting.
pub fn next_point(field: &Field) -> Option<AbilitySlot> {
    let plan = plan_of(field.hero);
    let mut spent: Vec<(AbilityId, u8)> = Vec::new();
    for id in plan {
        let level = match spent.iter_mut().find(|(had, _)| had == id) {
            Some((_, count)) => {
                *count += 1;
                *count
            }
            None => {
                spent.push((*id, 1));
                1
            }
        };
        let Some((slot, held)) = field.ability(*id) else {
            continue;
        };
        if held.level < level {
            return held.can_level.then_some(slot);
        }
    }
    plan.iter()
        .find_map(|id| field.ability(*id).filter(|(_, held)| held.can_level))
        .map(|(slot, _)| slot)
        .or_else(|| {
            field
                .abilities()
                .iter()
                .position(|held| held.can_level)
                .map(|at| AbilitySlot(at as u8))
        })
}
