//! Where a hero's skill points go.
//!
//! A plan is the abilities in the order points are spent on them, one entry
//! per point. The place of an entry is the hero level that point is spent at.

use bota_proto::{AbilityId, AbilitySlot, HeroId, TalentSlot};

use crate::{
    BOUNCE, CRIT, FEAST_OF_SOULS, FRENZY, Field, PRESENCE, RAZE_NEAR, REQUIEM, SHADOW_FIEND, SYLLA,
    VOLLEY,
};

/// Shadow Fiend's points: the razes first, the feast beside them, the
/// requiem when it opens at hero level 6; its later levels come through the
/// fallback once the hero level allows. The souls ask for none.
pub const FIEND_PLAN: [AbilityId; 9] = [
    RAZE_NEAR,
    FEAST_OF_SOULS,
    RAZE_NEAR,
    FEAST_OF_SOULS,
    RAZE_NEAR,
    REQUIEM,
    RAZE_NEAR,
    PRESENCE,
    PRESENCE,
];

/// Sylla's points: the bolt first, the crit beside it, the volley when it
/// opens at hero level 6; its later levels come through the fallback.
pub const SYLLA_PLAN: [AbilityId; 10] = [
    BOUNCE, CRIT, BOUNCE, CRIT, BOUNCE, VOLLEY, BOUNCE, CRIT, FRENZY, FRENZY,
];

/// The plan a hero follows. Empty for one the bot has no plan for.
pub fn plan_of(hero: HeroId) -> &'static [AbilityId] {
    match hero {
        SHADOW_FIEND => &FIEND_PLAN,
        SYLLA => &SYLLA_PLAN,
        _ => &[],
    }
}

/// Shadow Fiend's talents, one a tier: the raze stack damage, the
/// presence's armor, the souls held and the razes' attack damage. Each is
/// worth something every fight; the feast's and the requiem's are worth
/// something only while they run.
pub const FIEND_TALENTS: [TalentSlot; 4] =
    [TalentSlot(1), TalentSlot(3), TalentSlot(5), TalentSlot(6)];

/// The talents a hero takes, one a tier. Empty for one without a tree.
pub fn talents_of(hero: HeroId) -> &'static [TalentSlot] {
    match hero {
        SHADOW_FIEND => &FIEND_TALENTS,
        _ => &[],
    }
}

/// The talent a waiting skill point should go into: the first of the hero's
/// picks the view says a point could go into now. A talent comes before an
/// ability, since a tier opens on a level an ability does not wait on.
pub fn next_talent(field: &Field) -> Option<TalentSlot> {
    let own = field
        .view
        .talents
        .iter()
        .find(|talents| talents.slot == field.seat.slot)?;
    talents_of(field.hero)
        .iter()
        .copied()
        .find(|pick| own.learnable.has(*pick))
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
