//! Taking blows off the health they landed on.

use bota_core::{ModifierKind, Odds, amplify, bite, bite_share, kind_def, mitigate, rules};
use bota_proto::{DamageKind, Fixed, Inflictor, Team, UnitKind, Vec2};

use crate::game::{
    Entity, Fear, Health, Hit, HitEffect, MatchRng, Modifier, Modifiers, PseudoRandom, Purpose,
    Stats, Table, Transform, wire_id,
};

use super::bubble::soak;
use super::modifiers::resisted_ticks;

use std::collections::VecDeque;

/// One attack that did not land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Missed {
    /// Who swung. The handle may outlive the body.
    pub source: Option<Entity>,
    pub target: Entity,
}

/// One blow once it has been felt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Landed {
    /// Who dealt it. The handle may outlive the body.
    pub source: Option<Entity>,
    /// The ability or item that dealt it. Absent for an attack's own blow.
    pub inflictor: Option<Inflictor>,
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
    /// Whether it was a swing of the source's weapon, rather than a spell,
    /// a burn or the magic riding with a swing.
    pub attack: bool,
    /// The blow as it was dealt, before amplification, armor and
    /// resistance: what a splash takes its share of.
    pub dealt: i32,
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
    pub evasion: &'a mut Vec<Option<PseudoRandom>>,
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
        if stat.hit_count.is_some() && blow.striker.is_none() {
            // Health counted in hits takes nothing from what no attack dealt.
            continue;
        }
        let odds = if blow.uphill {
            stat.evasion.or(rules::UPHILL_MISS)
        } else {
            stat.evasion
        };
        let surely_struck = kind
            .get(blow.target)
            .is_some_and(|kind| kind_def(*kind).surely_struck);
        if !surely_struck && blow.attack && !blow.pierces && evades(blow.target, odds, rng, evasion)
        {
            missed.push_back(Missed {
                source: blow.source,
                target: blow.target,
            });
            continue;
        }
        if let HitEffect::FountainDamage { .. } = blow.effect
            && let Some(on_it) = modifiers.get_mut(blow.target)
        {
            on_it.stack_fountain_damage(blow.source);
        }
        let dealt = stacked_damage(blow, modifiers.get(blow.target));
        let spared = kind.get(blow.target).is_some_and(|k| {
            (immortal_structures && kind_def(*k).structure)
                || (immortal_heroes && *k == UnitKind::Hero)
        });
        let Some(hp) = health.get(blow.target).map(|pool| pool.hp) else {
            continue;
        };
        // Health counted in hits loses its striker's share, a fraction of a
        // hit as readily as a whole one; the blow tells the health it took,
        // a part of one as one.
        let (lost, applied) = match stat.hit_count {
            Some(count) => {
                let lost = bite_share(hp, count.taken(blow.striker, stat.max_hp), spared);
                (
                    lost,
                    (lost.raw + Fixed::from_int(1).raw - 1) >> Fixed::FRAC_BITS,
                )
            }
            None => {
                let taken = damage_taken::<AMPLIFIED>(blow, &stat, modifiers.get_mut(blow.target));
                let applied = bite(hp, taken, spared);
                (Fixed::from_int(applied), applied)
            }
        };
        let Some(pool) = health.get_mut(blow.target) else {
            continue;
        };
        pool.hp -= lost;
        let fatal = pool.hp <= Fixed::ZERO;
        if blow.effect == HitEffect::Dismember {
            heal_source(blow.source, applied, health, stats);
        }
        if !fatal && (applied > 0 || blow.effect.holds()) {
            apply_hit_effect(blow, stats, modifiers);
        }
        landed.push_back(Landed {
            source: blow.source,
            inflictor: blow.inflictor,
            target: blow.target,
            amount: applied,
            kind: blow.kind,
            at,
            side,
            fatal,
            attack: blow.attack,
            dealt,
        });
    }
}

/// The health a blow takes of a target whose health damage takes: the blow
/// amplified, then armor or resistance, the Glyph or backdoor protection,
/// the extra magic it takes and what soaks magic, and what blocks a share of
/// every blow.
fn damage_taken<const AMPLIFIED: bool>(
    blow: Hit,
    stat: &Stats,
    mut on_it: Option<&mut Modifiers>,
) -> i32 {
    let amount = dealt::<AMPLIFIED>(blow, on_it.as_deref());
    let protected = on_it.as_deref().is_some_and(|on_it| {
        on_it
            .active()
            .any(|held| held.kind == ModifierKind::BackdoorProtection)
    });
    let mut taken = mitigate(amount, blow.kind, stat.armor, stat.magic_resist_bp);
    if stat.glyph {
        taken = 0;
    } else if protected {
        taken = taken * rules::BACKDOOR_DAMAGE_PCT / 100;
    }
    if blow.kind == DamageKind::Magical {
        taken = more_magic(taken, stat.magic_taken_pct);
        if let Some(on_it) = on_it.as_deref_mut() {
            taken = soak(on_it, taken);
        }
    }
    let taken = blocked(taken, on_it.as_deref());
    // What the target takes more or less of than lands on others, as an
    // illusion takes more.
    let taken = (i64::from(taken) * i64::from(stat.damage_taken_bp) / i64::from(rules::NOMINAL_BP))
        .clamp(0, i64::from(i32::MAX)) as i32;
    match on_it {
        Some(on_it) => barrier(on_it, taken),
        None => taken,
    }
}

/// What is left of a blow's damage once the barriers on the target have
/// taken theirs, after armor, resistance and every cut: a Shield Rune's
/// barrier takes damage of every kind, the first put on first, and is gone
/// when it is spent [LIQ Runes: "absorbs damage instances (after
/// reductions)"].
fn barrier(on_it: &mut Modifiers, mut taken: i32) -> i32 {
    if taken == 0 {
        return 0;
    }
    for held in on_it.0.iter_mut() {
        if taken == 0 {
            break;
        }
        if held.ticks_left == Some(0) {
            continue;
        }
        if let ModifierKind::RuneShield { left } = &mut held.kind {
            let soaked = taken.min((*left).max(0));
            *left -= soaked;
            taken -= soaked;
        }
    }
    on_it
        .0
        .retain(|held| !matches!(held.kind, ModifierKind::RuneShield { left } if left <= 0));
    taken
}

/// Magical damage after the extra a target takes, in percent.
fn more_magic(taken: i32, extra_pct: i32) -> i32 {
    if extra_pct == 0 {
        return taken;
    }
    let scaled = i64::from(taken) * i64::from((100 + extra_pct).max(0)) / 100;
    scaled.clamp(0, i64::from(i32::MAX)) as i32
}

/// What is left of a blow's damage once what the target wears takes a fixed
/// amount off it, after armor and resistance.
fn blocked(taken: i32, on_it: Option<&Modifiers>) -> i32 {
    if taken == 0 {
        return 0;
    }
    let block: i32 = on_it.map_or(0, |on_it| {
        on_it
            .active()
            .map(|held| match held.kind {
                ModifierKind::MeatShield { amount } => amount,
                _ => 0,
            })
            .sum()
    });
    (taken - block).max(0)
}

/// Mends the source of a blow by the health it took, never past its maximum.
fn heal_source(
    source: Option<Entity>,
    amount: i32,
    health: &mut Table<Health>,
    stats: &Table<Stats>,
) {
    let (Some(source), true) = (source, amount > 0) else {
        return;
    };
    let (Some(pool), Some(stat)) = (health.get_mut(source), stats.get(source)) else {
        return;
    };
    if pool.hp > Fixed::ZERO {
        pool.hp = (pool.hp + Fixed::from_int(amount)).min(stat.max_hp);
    }
}

/// Whether an attack at a target is evaded at `odds`: what the target
/// evades with and, thrown uphill, the uphill miss, stacked as Dota stacks
/// evasion. It rolls on the target's own pseudo-random sequence; nothing is
/// rolled for an attack that cannot miss.
pub fn evades(
    target: Entity,
    odds: Odds,
    rng: &MatchRng,
    evasion: &mut Vec<Option<PseudoRandom>>,
) -> bool {
    if odds == Odds::NEVER {
        return false;
    }
    let chance = entity_slot(evasion, target, || {
        PseudoRandom::new(rng.for_unit(Purpose::Evasion, wire_id(target), 1))
    });
    chance.roll(odds)
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

/// Pre-mitigation damage including the current valid same-caster stack count:
/// a Shadowraze's stacks from earlier razes, a fountain's shot's own stacks
/// with the one it has just added.
fn stacked_damage(blow: Hit, on_it: Option<&Modifiers>) -> i32 {
    if let HitEffect::FountainDamage { stack_damage } = blow.effect {
        let stacks = on_it.map_or(0, Modifiers::fountain_damage_stacks);
        let bonus = i64::from(stacks) * i64::from(stack_damage);
        return (i64::from(blow.amount) + bonus).min(i64::from(i32::MAX)) as i32;
    }
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
        // A fountain's shot has stacked before it dealt its damage.
        HitEffect::None | HitEffect::Dismember | HitEffect::FountainDamage { .. } => {}
        HitEffect::Shadowraze { .. } => {
            assert_eq!(blow.kind, DamageKind::Magical);
            assert!(blow.amount > 0);
            let caster = blow.source.expect("a Shadowraze hit has a caster");
            if let Some(on_it) = modifiers.get_mut(blow.target) {
                on_it.stack_raze(caster, blow.inflictor);
            }
        }
        HitEffect::Requiem { level, fear } => {
            frighten(blow, usize::from(level), fear, stats, modifiers);
        }
        HitEffect::Stun { ticks } => hold(blow, ModifierKind::Stunned, ticks, stats, modifiers),
        HitEffect::Root { ticks } => hold(blow, ModifierKind::Rooted, ticks, stats, modifiers),
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
        on_it.extend(
            Modifier::new(kind, blow.source, blow.inflictor, Some(ticks)),
            fear.most,
        );
    }
}

/// Lays a disable on what a blow struck, shortened by its status resistance.
fn hold(
    blow: Hit,
    kind: ModifierKind,
    ticks: u32,
    stats: &Table<Stats>,
    modifiers: &mut Table<Modifiers>,
) {
    let ticks = resisted_ticks(stats, blow.target, kind, ticks);
    if let Some(on_it) = modifiers.get_mut(blow.target) {
        on_it.put(Modifier::new(
            kind,
            blow.source,
            blow.inflictor,
            Some(ticks),
        ));
    }
}
