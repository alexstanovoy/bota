//! Taking blows off the health they landed on.

use bota_core::{ModifierKind, Ratio, amplify, bite, kind_def, mitigate, rules};
use bota_proto::{DamageKind, Fixed, Team, UnitKind, Vec2};

use crate::game::{
    Chance, Entity, Fear, Health, Hit, HitEffect, MatchRng, Modifier, Modifiers, Purpose, Stats,
    Table, Transform, wire_id,
};

use super::modifiers::resisted_ticks;

use std::collections::VecDeque;

/// One attack that did not land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Missed {
    /// Who swung. The handle may outlive the body.
    pub source: Option<Entity>,
    pub target: Entity,
    /// Where it happened.
    pub at: Vec2,
    /// The side it was swung at.
    pub side: Team,
}

/// One blow once it has been felt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Landed {
    /// Who dealt it. The handle may outlive the body.
    pub source: Option<Entity>,
    pub target: Entity,
    /// Health taken, after armor and resistance, at most one past what the
    /// target had left.
    pub amount: i32,
    pub kind: DamageKind,
    /// Where it happened.
    pub at: Vec2,
    /// The side that took it.
    pub side: Team,
    /// Whether it brought the target down.
    pub fatal: bool,
}

/// What resolving blows reads and writes.
pub struct HitCx<'a> {
    /// The blows waiting to be felt.
    pub hits: &'a mut VecDeque<Hit>,
    /// Where a blow that was felt is left for whatever answers to it.
    pub landed: &'a mut VecDeque<Landed>,
    pub transform: &'a Table<Transform>,
    pub team: &'a Table<Team>,
    /// Armor and resistance.
    pub stats: &'a Table<Stats>,
    /// What the blow comes off.
    pub health: &'a mut Table<Health>,
    /// What is on each entity, read and added to by successful blows.
    pub modifiers: &'a mut Table<Modifiers>,
    pub kind: &'a Table<UnitKind>,
    /// Whether a blow never takes a structure below 1 hp.
    pub immortal_structures: bool,
    /// Whether a blow never takes a hero below 1 hp.
    pub immortal_heroes: bool,
    /// Hidden streams, for a target's first evasion roll.
    pub rng: &'a MatchRng,
    /// Evasion sequence per target slot.
    pub evasion: &'a mut Vec<Option<Chance>>,
    /// Where an attack that was evaded is left for whatever answers to it.
    pub missed: &'a mut VecDeque<Missed>,
}

/// Takes every waiting blow off the health it landed on.
///
/// A blow at something already down, or at something damage passes by, is
/// given up unfelt. Every blow leaves the queue either way: none survives the
/// tick that resolves it.
///
/// Outgoing amplification is compiled out when every queued blow is nominal.
pub fn hitting_system<const AMPLIFIED: bool>(cx: HitCx<'_>) {
    let HitCx {
        hits,
        landed,
        transform,
        team,
        stats,
        health,
        modifiers,
        kind,
        immortal_structures,
        immortal_heroes,
        rng,
        evasion,
        missed,
    } = cx;
    while let Some(blow) = hits.pop_front() {
        let standing = health
            .get(blow.target)
            .is_some_and(|health| health.hp > Fixed::ZERO);
        let Some(stat) = stats.get(blow.target).copied() else {
            continue;
        };
        let on_it = modifiers.get(blow.target);
        let shielded = on_it.is_some_and(|on_it| {
            on_it
                .active()
                .any(|held| held.kind == ModifierKind::Shielded)
        });
        if !standing || stat.invulnerable || shielded {
            continue;
        }
        let at = transform.get(blow.target).map_or(Vec2::ZERO, |t| t.pos);
        let side = team.get(blow.target).copied().unwrap_or(Team::Neutral);
        if blow.attack && !blow.pierces && evades(blow.target, stat.evasion, rng, evasion) {
            missed.push_back(Missed {
                source: blow.source,
                target: blow.target,
                at,
                side,
            });
            continue;
        }
        let amount = dealt::<AMPLIFIED>(blow, on_it);
        let taken = mitigate(amount, blow.kind, stat.armor, stat.magic_resist_bp);
        let Some(pool) = health.get_mut(blow.target) else {
            continue;
        };
        let spared = kind.get(blow.target).is_some_and(|k| {
            (immortal_structures && kind_def(*k).structure)
                || (immortal_heroes && *k == UnitKind::Hero)
        });
        let applied = bite(pool.hp, taken, spared);
        pool.hp -= Fixed::from_int(applied);
        let fatal = pool.hp <= Fixed::ZERO;
        if applied > 0 && !fatal {
            apply_hit_effect(blow, stats, modifiers);
        }
        landed.push_back(Landed {
            source: blow.source,
            target: blow.target,
            amount: applied,
            kind: blow.kind,
            at,
            side,
            fatal,
        });
    }
}

/// Whether an attack at a target is evaded: the target's share of misses,
/// held exactly over every block of attacks at it. Nothing is rolled for a
/// target that evades nothing. The sequence is the target's own, on source
/// one of the evasion purpose; source zero is the attacker's uphill
/// sequence.
pub fn evades(
    target: Entity,
    ratio: Ratio,
    rng: &MatchRng,
    evasion: &mut Vec<Option<Chance>>,
) -> bool {
    if ratio == Ratio::NEVER {
        return false;
    }
    let chance = entity_slot(evasion, target, || {
        Chance::new(rng.for_unit(Purpose::Evasion, wire_id(target), 1), ratio)
    });
    chance.roll(ratio)
}

/// An entity's own entry in a table indexed by entity slot, opened with
/// `open` the first time it is asked for.
pub(crate) fn entity_slot<T>(
    slots: &mut Vec<Option<T>>,
    entity: Entity,
    open: impl FnOnce() -> T,
) -> &mut T {
    let index = entity.index().0 as usize;
    if slots.len() <= index {
        slots.resize_with(index + 1, || None);
    }
    slots[index].get_or_insert_with(open)
}

/// Pre-mitigation damage including the current valid same-caster stack count.
fn stacked_damage(blow: Hit, on_it: Option<&Modifiers>) -> i32 {
    let HitEffect::Shadowraze { stack_damage } = blow.effect else {
        return blow.amount;
    };
    assert_eq!(blow.kind, DamageKind::Magical);
    assert!(stack_damage > 0);
    assert!(stack_damage <= rules::RAZE_STACK_DAMAGE[3] + rules::TALENT_RAZE_STACK_DAMAGE);
    let caster = blow.source.expect("a Shadowraze hit has a caster");
    let stacks = on_it.map_or(0, |on_it| on_it.raze_stacks(caster));
    blow.amount + i32::from(stacks) * stack_damage
}

/// Pre-mitigation damage, amplified only when `AMPLIFIED`.
fn dealt<const AMPLIFIED: bool>(blow: Hit, on_it: Option<&Modifiers>) -> i32 {
    let amount = stacked_damage(blow, on_it);
    if AMPLIFIED {
        amplify(amount, blow.damage_amp_bp)
    } else {
        debug_assert_eq!(blow.damage_amp_bp, rules::NOMINAL_BP);
        amount
    }
}

/// Applies a damaging hit's modifier to a surviving target.
fn apply_hit_effect(blow: Hit, stats: &Table<Stats>, modifiers: &mut Table<Modifiers>) {
    match blow.effect {
        HitEffect::None => {}
        HitEffect::Shadowraze { .. } => {
            assert_eq!(blow.kind, DamageKind::Magical);
            assert!(blow.amount > 0);
            let caster = blow.source.expect("a Shadowraze hit has a caster");
            if let Some(on_it) = modifiers.get_mut(blow.target) {
                on_it.stack_raze(caster);
            }
        }
        HitEffect::Requiem { level, fear } => {
            frighten(blow, usize::from(level), fear, stats, modifiers);
        }
    }
}

/// Lays a requiem line's fear, slow and torn magic resistance on what it
/// burnt, each lengthened by the line's share and shortened by the target's
/// status resistance.
fn frighten(
    blow: Hit,
    level: usize,
    fear: Fear,
    stats: &Table<Stats>,
    modifiers: &mut Table<Modifiers>,
) {
    assert_eq!(blow.kind, DamageKind::Magical);
    assert!(level < rules::REQUIEM_SLOW_PCT.len());
    assert!(fear.line > 0);
    assert!(fear.most >= fear.line);
    let Some(on_it) = modifiers.get_mut(blow.target) else {
        return;
    };
    for kind in [
        ModifierKind::Slowed {
            pct: rules::REQUIEM_SLOW_PCT[level],
        },
        ModifierKind::MagicBroken {
            pct: rules::REQUIEM_MAGIC_BREAK_PCT[level],
        },
        ModifierKind::Feared,
    ] {
        let ticks = resisted_ticks(stats, blow.target, kind, fear.line);
        on_it.extend(Modifier::new(kind, blow.source, Some(ticks)), fear.most);
    }
}
