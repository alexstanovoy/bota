//! Shadow Fiend's spellwork.
//!
//! A raze takes no aim. It lands at its own reach along the line the caster
//! is already looking down, so casting one is two decisions: which way to
//! look, and whether what stands where it would land is worth the mana.

use bota_proto::{DamageKind, Target, UnitView, Vec2};

use crate::{
    Ask, Beat, Field, RAZE_DAMAGE, RAZE_RADIUS, RAZES, REQUIEM, REQUIEM_DAMAGE_PER_SOUL,
    after_mitigation, facing_gap, facing_towards, span, spot_ahead, within,
};

/// Souls a requiem is worth letting go for a lone enemy that it would not
/// kill.
pub const REQUIEM_SOULS: u32 = 10;

/// Enemy heroes within a requiem's reach that make it worth casting whatever
/// is gathered.
pub const REQUIEM_CROWD: usize = 2;

/// Creeps a raze must cover to be worth casting for the wave alone.
pub const RAZE_CROWD: usize = 3;

/// Mana, as a part of the whole, kept back from razing a wave.
pub const RAZE_SPARE_MANA: f32 = 0.5;

/// What Shadow Fiend would cast this tick.
pub fn fiend_spell(field: &Field, beat: &Beat) -> Option<Ask> {
    requiem(field).or_else(|| raze(field, beat))
}

/// Where Shadow Fiend should look so that a raze would land on somebody.
///
/// `None` when nobody is within reach of any raze, or when the hero is
/// already looking near enough at the nearest one.
pub fn fiend_aim(field: &Field) -> Option<Vec2> {
    let me = field.me?;
    let reaches = ready_reaches(field);
    if reaches.is_empty() {
        return None;
    }
    let furthest = reaches.iter().copied().max()? + RAZE_RADIUS;
    let mark = field
        .enemies
        .iter()
        .find(|foe| span(foe.pos, me.pos) <= furthest as f32)?;
    let already = reaches
        .iter()
        .any(|reach| within(mark, spot_ahead(me, *reach), RAZE_RADIUS));
    if already {
        return None;
    }
    // Half a raze's width at the nearest reach it could be cast from, as an
    // angle: looking within it, the burn already covers the mark.
    let nearest = reaches.iter().copied().min()?;
    let slack = (RAZE_RADIUS as f32 / nearest.max(1) as f32).atan();
    let wanted = facing_towards(me.pos, mark.pos);
    let off = f32::from(facing_gap(me.facing, wanted)) / 65536.0 * std::f32::consts::TAU;
    (off > slack).then_some(mark.pos)
}

/// The requiem, when there is a crowd for it.
fn requiem(field: &Field) -> Option<Ask> {
    let me = field.me?;
    let (slot, held) = field.ability(REQUIEM)?;
    if held.level == 0 || held.cooldown_left > 0 || me.mana < held.mana_cost {
        return None;
    }
    let souls = field.souls();
    if souls == 0 {
        return None;
    }
    let each = REQUIEM_DAMAGE_PER_SOUL
        .get(usize::from(held.level - 1))
        .copied()
        .unwrap_or(0);
    let caught: Vec<&UnitView> = field.foes_within(held.range).collect();
    let kills = caught
        .iter()
        .any(|foe| after_mitigation(each * souls as i32, DamageKind::Magical, foe) >= foe.hp);
    let worth =
        caught.len() >= REQUIEM_CROWD || kills || (!caught.is_empty() && souls >= REQUIEM_SOULS);
    worth.then(|| Ask::cast(slot, Target::None))
}

/// The best raze the hero is already looking down the line of.
///
/// Between two that cover the same mark, the one whose burn lands nearest it
/// is taken: a mark at the edge of one is a mark a step out of it.
fn raze(field: &Field, beat: &Beat) -> Option<Ask> {
    let me = field.me?;
    let mut best: Option<((usize, usize, usize, i64), bota_proto::AbilitySlot)> = None;
    for id in RAZES {
        let Some((slot, held)) = field.ability(id) else {
            continue;
        };
        if held.level == 0 || held.cooldown_left > 0 || me.mana < held.mana_cost {
            continue;
        }
        let damage = RAZE_DAMAGE
            .get(usize::from(held.level - 1))
            .copied()
            .unwrap_or(0);
        let lands = spot_ahead(me, held.range);
        let struck: Vec<&UnitView> = field
            .enemies
            .iter()
            .copied()
            .filter(|foe| within(foe, lands, RAZE_RADIUS))
            .collect();
        let heroes = struck.len();
        let caught: Vec<&UnitView> = field
            .creeps
            .iter()
            .copied()
            .filter(|creep| within(creep, lands, RAZE_RADIUS))
            .collect();
        let killed = caught
            .iter()
            .filter(|creep| {
                after_mitigation(damage, DamageKind::Magical, creep) >= beat.health_in(creep, 1)
            })
            .count();
        let spare = field.mana() >= RAZE_SPARE_MANA;
        let worth =
            heroes > 0 || killed >= 2 || (spare && killed >= 1 && caught.len() >= RAZE_CROWD);
        if !worth {
            continue;
        }
        let centred = struck
            .iter()
            .chain(caught.iter())
            .map(|mark| mark.pos.distance_squared(lands))
            .min()
            .unwrap_or(i64::MAX);
        let score = (heroes, killed, caught.len(), -centred);
        if best.is_none_or(|(had, _)| score > had) {
            best = Some((score, slot));
        }
    }
    best.map(|(_, slot)| Ask::cast(slot, Target::None))
}

/// The reaches of every raze that could be cast right now.
fn ready_reaches(field: &Field) -> Vec<i32> {
    let Some(me) = field.me else {
        return Vec::new();
    };
    RAZES
        .iter()
        .filter_map(|id| field.ability(*id))
        .filter(|(_, held)| held.level > 0 && held.cooldown_left == 0 && me.mana >= held.mana_cost)
        .map(|(_, held)| held.range)
        .collect()
}
