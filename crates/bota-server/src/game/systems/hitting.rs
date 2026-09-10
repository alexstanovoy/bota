//! Taking blows off the health they landed on.

use bota_proto::{DamageKind, Fixed, Team, Vec2};

use crate::game::rules;
use std::collections::VecDeque;

use crate::game::{Entity, Health, Hit, HitEffect, Stats, StatusKind, Statuses, Table, Transform};

/// One blow once it has been felt.
///
/// What the world does with it afterwards — the event it sends, the bounty it
/// pays — is not this system's business.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Landed {
    /// Who dealt it, while that one still stands.
    pub source: Option<Entity>,
    /// Who took it.
    pub target: Entity,
    /// After armor and resistance.
    pub amount: i32,
    /// Which reduction applied.
    pub kind: DamageKind,
    /// Whether the blow was a critical strike.
    pub crit: bool,
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
    /// Where each entity stands.
    pub transform: &'a Table<Transform>,
    /// Which side each entity is on.
    pub team: &'a Table<Team>,
    /// Armor and resistance.
    pub stats: &'a Table<Stats>,
    /// What the blow comes off.
    pub health: &'a mut Table<Health>,
    /// Timed statuses read and applied by successful blows.
    pub statuses: &'a mut Table<Statuses>,
}

/// Takes every waiting blow off the health it landed on.
///
/// A blow at something already down, or at something damage passes by, is
/// given up unfelt. Every blow leaves the queue either way: none survives the
/// tick that resolves it.
pub fn hitting_system(cx: HitCx<'_>) {
    let HitCx {
        hits,
        landed,
        transform,
        team,
        stats,
        health,
        statuses,
    } = cx;
    while let Some(blow) = hits.pop_front() {
        let standing = health
            .get(blow.target)
            .is_some_and(|health| health.hp > Fixed::ZERO);
        let Some(stat) = stats.get(blow.target).copied() else {
            continue;
        };
        let on_it = statuses.get(blow.target);
        let shielded = on_it.is_some_and(|statuses| {
            statuses
                .active()
                .any(|status| status.kind == StatusKind::Shielded)
        });
        if !standing || stat.invulnerable || shielded {
            continue;
        }
        let amount = amplified_damage(blow, on_it);
        let taken = mitigate(amount, blow.kind, stat.armor, stat.magic_resist_pct);
        let Some(pool) = health.get_mut(blow.target) else {
            continue;
        };
        let applied = taken.min(pool.hp.to_int().max(0) + 1);
        pool.hp -= Fixed::from_int(applied);
        let fatal = pool.hp <= Fixed::ZERO;
        if applied > 0 && !fatal {
            apply_hit_effect(blow, statuses);
        }
        landed.push_back(Landed {
            source: blow.source,
            target: blow.target,
            amount: applied,
            kind: blow.kind,
            crit: blow.crit,
            at: transform.get(blow.target).map_or(Vec2::ZERO, |t| t.pos),
            side: team.get(blow.target).copied().unwrap_or(Team::Neutral),
            fatal,
        });
    }
}

/// Pre-mitigation damage including the current valid same-caster stack count.
fn amplified_damage(blow: Hit, statuses: Option<&Statuses>) -> i32 {
    let HitEffect::Shadowraze { level } = blow.effect else {
        return blow.amount;
    };
    assert_eq!(blow.kind, DamageKind::Magical);
    assert!(usize::from(level) < rules::RAZE_STACK_DAMAGE.len());
    let caster = blow.source.expect("a Shadowraze hit has a caster");
    let stacks = statuses.map_or(0, |statuses| statuses.raze_stacks(caster));
    blow.amount + i32::from(stacks) * rules::RAZE_STACK_DAMAGE[usize::from(level)]
}

/// Applies a damaging hit's status to a surviving target.
fn apply_hit_effect(blow: Hit, statuses: &mut Table<Statuses>) {
    if let HitEffect::Shadowraze { .. } = blow.effect {
        assert_eq!(blow.kind, DamageKind::Magical);
        assert!(blow.amount > 0);
        let caster = blow.source.expect("a Shadowraze hit has a caster");
        if let Some(on_it) = statuses.get_mut(blow.target) {
            on_it.stack_raze(caster);
        } else {
            let mut on_it = Statuses::default();
            on_it.stack_raze(caster);
            statuses.insert(blow.target, on_it);
        }
    }
}

/// Damage after armor or magic resistance.
fn mitigate(amount: i32, kind: DamageKind, armor: Fixed, magic_resist_pct: i32) -> i32 {
    match kind {
        DamageKind::Physical => {
            let armor = armor.max(Fixed::ZERO);
            let whole = i64::from(Fixed::ONE.raw);
            let den = 100 * whole + i64::from(rules::ARMOR_SCALE) * i64::from(armor.raw);
            (i64::from(amount) * 100 * whole / den) as i32
        }
        DamageKind::Magical => {
            let kept = (100 - magic_resist_pct).clamp(0, 100);
            (i64::from(amount) * i64::from(kept) / 100) as i32
        }
        DamageKind::Pure => amount,
    }
}
