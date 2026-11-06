//! Taking blows off the health they landed on.

use bota_core::{
    ModifierKind, Odds, amplify, bite, bite_share, cooldown_after, item_def, kind_def, mitigate,
    rules,
};
use bota_proto::{DamageKind, Fixed, Inflictor, Team, UnitKind, Vec2};

use crate::game::{
    Entity, Fear, Health, Hit, HitEffect, Illusion, Inventory, MatchRng, Modifier, Modifiers,
    PseudoRandom, Purpose, Stats, Table, Transform, wire_id,
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
    /// Whether a spell dealt it, see [`Hit::spell`].
    pub spell: bool,
    /// Whether it removed health rather than dealt damage, see
    /// [`Hit::removal`].
    pub removal: bool,
    /// What lifesteal heals a share of: the blow after armor and resistance,
    /// before what the target alone takes off or adds (a block, a barrier,
    /// the more an illusion takes) and not cut to the health it had left
    /// (Liquipedia, *Lifesteal*: the heal is "based on the attack damage
    /// value the dying target would have received"; Dota 7.41: increased
    /// damage against illusions is not counted).
    pub stolen: i32,
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
    /// The tick the blows are felt on.
    pub tick: u32,
    pub abilities: &'a mut Table<crate::game::AbilityBook>,
    pub blocks: &'a mut Vec<super::spell_block::BlockActivation>,
    pub free_spells: bool,
    /// What is carried: a magic block's charges and cooldown are spent by the blows it blocks.
    pub inventory: &'a mut Table<Inventory>,
    /// Which units are illusions: an illusion's items block nothing.
    pub illusion: &'a Table<Illusion>,
    /// Whether every cooldown comes to nothing (`-wtf`).
    pub wtf: bool,
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
        tick,
        abilities,
        blocks,
        free_spells,
        inventory,
        illusion,
        wtf,
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
        // Physical damage passes an ethereal unit by (Dota's Ghost Form: "immune to physical
        // damage").
        let passes_by = stat.ethereal && blow.kind == DamageKind::Physical;
        if !standing || stat.invulnerable || shielded || passes_by {
            continue;
        }
        if let Some(side) = blow.blockable_source_team
            && let Some(blocked) = super::spell_block::absorb(
                blow.target,
                side,
                abilities,
                stats,
                modifiers,
                team,
                free_spells,
            )
        {
            blocks.push(blocked);
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
        // What the attack lays is on before its own damage lands (Dota: "The attacks first apply
        // the debuff, then their own attack damage"): armor it takes off that the target's stats
        // do not hold yet counts against this blow.
        let struck = Struck {
            stat: &stat,
            kind: kind.get(blow.target).copied(),
            foe: blow.source.and_then(|by| team.get(by)) != Some(&side),
        };
        let fresh_cut = lay(blow, struck, modifiers.get_mut(blow.target), tick);
        let stat = Stats {
            armor: stat.armor - fresh_cut,
            ..stat
        };
        if let Some(proc) = blow.proc {
            apply_attack_proc(blow, proc, &stat, kind, stats, modifiers);
        }
        if let HitEffect::FountainDamage { .. } = blow.effect
            && let Some(on_it) = modifiers.get_mut(blow.target)
        {
            on_it.stack_fountain_damage(blow.source);
        }
        let dealt = stacked_damage(blow, modifiers.get(blow.target));
        // A blow that spares stops one short of the last point after every amplification and
        // cut, as an immortal unit does.
        let spared = blow.spares
            || kind.get(blow.target).is_some_and(|k| {
                (immortal_structures && kind_def(*k).structure)
                    || (immortal_heroes && *k == UnitKind::Hero)
            });
        let Some(hp) = health.get(blow.target).map(|pool| pool.hp) else {
            continue;
        };
        // Health counted in hits loses its striker's share, a fraction of a
        // hit as readily as a whole one; the blow tells the health it took,
        // a part of one as one.
        let (lost, applied, stolen) = match stat.hit_count {
            Some(count) => {
                let lost = bite_share(hp, count.taken(blow.striker, stat.max_hp), spared);
                let applied = (lost.raw + Fixed::from_int(1).raw - 1) >> Fixed::FRAC_BITS;
                (lost, applied, applied)
            }
            None => {
                // A magic block answers to an enemy's blow alone, carried by no illusion.
                let enemy = blow.source.is_some_and(|by| {
                    by != blow.target && team.get(by).is_some_and(|&theirs| theirs != side)
                });
                let guard = MagicGuard {
                    bag: inventory
                        .get_mut(blow.target)
                        .filter(|_| enemy && illusion.get(blow.target).is_none()),
                    wtf,
                };
                let (taken, stolen) =
                    damage_taken::<AMPLIFIED>(blow, &stat, modifiers.get_mut(blow.target), guard);
                let applied = bite(hp, taken, spared);
                (Fixed::from_int(applied), applied, stolen)
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
            spell: blow.spell,
            removal: blow.removal,
            stolen,
            dealt,
        });
    }
}

/// The health a blow takes of a target whose health damage takes: the blow
/// amplified, then armor or resistance, the Glyph or backdoor protection,
/// the extra magic it takes and what soaks magic, and what blocks a share of
/// every blow.
/// Also what lifesteal takes its share of: the same before what the target
/// alone takes off or adds.
fn damage_taken<const AMPLIFIED: bool>(
    blow: Hit,
    stat: &Stats,
    mut on_it: Option<&mut Modifiers>,
    guard: MagicGuard<'_>,
) -> (i32, i32) {
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
        // The block comes first of every negation and ignores what the target takes more of
        // (Liquipedia, Infused Raindrops: "the highest priority in the group of damage negating
        // abilities", "Ignores every form of incoming damage amplification").
        taken = guard.block(taken, amount, stat.cooldown_rate_bp);
        taken = more_magic(taken, stat.magic_taken_pct);
        if let Some(on_it) = on_it.as_deref_mut() {
            taken = soak(on_it, taken);
        }
    }
    let stolen = taken;
    let mut taken = blocked(taken, on_it.as_deref());
    if blow.attack && blow.striker == Some(bota_core::Striker::Illusion) {
        taken = (i64::from(taken) * i64::from(stat.illusion_attack_taken_bp)
            / i64::from(rules::NOMINAL_BP)) as i32;
    }
    // What the target takes more or less of than lands on others, as an
    // illusion takes more.
    let taken = (i64::from(taken) * i64::from(stat.damage_taken_bp) / i64::from(rules::NOMINAL_BP))
        .clamp(0, i64::from(i32::MAX)) as i32;
    let taken = match on_it {
        Some(on_it) => barrier(on_it, taken),
        None => taken,
    };
    (taken, stolen)
}

/// Puts on the struck what the blow lays ([`Hit::laid`]), each item's
/// modifier freshened if it is on already; nothing that spell immunity keeps
/// off, where the item's does not go through it. The armor taken off by what
/// was not on before.
fn lay(blow: Hit, struck: Struck<'_>, on_it: Option<&mut Modifiers>, tick: u32) -> Fixed {
    let Some(on_it) = on_it else {
        return Fixed::ZERO;
    };
    let mut fresh_cut = Fixed::ZERO;
    for item in blow.laid.items() {
        let Some(laid) = bota_core::item_def(item).and_then(|def| def.on_attack) else {
            continue;
        };
        if !struck.takes(laid) {
            continue;
        }
        let worn = on_it.active().find_map(|held| match held.kind {
            ModifierKind::Laid { item: had, since } if had == item => Some(since),
            _ => None,
        });
        if worn.is_none() {
            fresh_cut += laid.armor;
        }
        on_it.put(Modifier::new(
            ModifierKind::Laid {
                item,
                since: worn.unwrap_or(tick),
            },
            blow.source,
            Some(Inflictor::Item(item)),
            // One that beats holds the tick of its last beat too, so that beat lands before it
            // lifts: three beats in three seconds (Liquipedia, Orb of Venom: "resulting in 3
            // instances").
            Some(laid.ticks + u32::from(laid.dps > 0)),
        ));
    }
    fresh_cut
}

/// What a blow lands on, for what an attack lays.
#[derive(Clone, Copy)]
struct Struck<'a> {
    stat: &'a Stats,
    kind: Option<UnitKind>,
    /// Whether it is of another side than the blow's dealer.
    foe: bool,
}

impl Struck<'_> {
    /// Whether what an attack passive lays lands on it: what spell immunity
    /// keeps off unless it goes through, and only enemy units for one that
    /// lands on them alone.
    fn takes(self, laid: bota_core::OnAttack) -> bool {
        if self.stat.spell_immune && !laid.pierces_immunity {
            return false;
        }
        match laid.affects {
            bota_core::Affects::Everything => true,
            bota_core::Affects::EnemyUnits => {
                self.foe
                    && self.kind.is_some_and(|kind| {
                        !kind_def(kind).structure
                            && !matches!(kind, UnitKind::Ward | UnitKind::Courier)
                    })
            }
        }
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

/// The disable of a conditional attack occurs before its combined damage instance.
fn apply_attack_proc(
    blow: Hit,
    proc: crate::game::AttackProc,
    target: &Stats,
    kinds: &Table<UnitKind>,
    stats: &Table<Stats>,
    modifiers: &mut Table<Modifiers>,
) {
    let Some(modifier) = proc.modifier else {
        return;
    };
    if kinds
        .get(blow.target)
        .is_some_and(|kind| kind_def(*kind).target_class == bota_core::TargetClass::Ward)
    {
        return;
    }
    if target.spell_immune
        && !match proc.pierces {
            bota_core::Pierces::No => false,
            bota_core::Pierces::Yes => true,
            bota_core::Pierces::NotCouriers => kinds.get(blow.target) != Some(&UnitKind::Courier),
        }
    {
        return;
    }
    let ticks = resisted_ticks(stats, blow.target, modifier, proc.ticks);
    if let Some(held) = modifiers.get_mut(blow.target) {
        held.put(Modifier::new(
            modifier,
            blow.source,
            Some(proc.inflictor),
            Some(ticks),
        ));
    }
}

/// The carried magic blocks of a blow's target, where they may answer to it.
struct MagicGuard<'a> {
    /// The target's inventory; absent where no block answers to the blow.
    bag: Option<&'a mut Inventory>,
    wtf: bool,
}

impl MagicGuard<'_> {
    /// What is left of magical damage `taken` (after resistance) once a carried magic block that
    /// answers to a blow that `dealt` this much before reductions has blocked its share: the
    /// first in the inventory proper, off cooldown and not muted, whose least damage the blow
    /// reaches, which spends a charge and goes on its cooldown; any other carried keeps its own
    /// (Dota 7.41e: "Multiple instances of Infused Raindrops no longer lose Magical Damage Block
    /// charges simultaneously"; Liquipedia: "Neither consumes charges nor blocks any damage while
    /// on cooldown"). One whose last charge is spent is gone.
    fn block(self, taken: i32, dealt: i32, cooldown_rate_bp: i32) -> i32 {
        let Some(bag) = self.bag else {
            return taken;
        };
        for held in bag.slots.iter_mut().take(rules::INVENTORY_SLOTS) {
            let Some(stack) = held.as_mut() else {
                continue;
            };
            let Some(def) = item_def(stack.id) else {
                continue;
            };
            let Some(guard) = def.magic_block else {
                continue;
            };
            if stack.mute != 0 || stack.cooldown != 0 || dealt < guard.min_damage {
                continue;
            }
            stack.cooldown = if self.wtf {
                0
            } else {
                cooldown_after(def.cooldown, cooldown_rate_bp.max(1))
            };
            stack.touched = true;
            stack.disassemble_until = 0;
            stack.spend_one();
            if stack.charges == 0 {
                *held = None;
            }
            return taken - guard.amount.clamp(0, taken.max(0));
        }
        taken
    }
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
        let amount = super::bubble::restored(Fixed::from_int(amount), stat.restoration_bp);
        pool.hp = (pool.hp + amount).min(stat.max_hp);
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
    let amount = amount.saturating_add(blow.proc.map_or(0, |proc| proc.bonus_damage));
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
