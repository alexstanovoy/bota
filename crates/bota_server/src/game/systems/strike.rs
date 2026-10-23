//! What a neutral's passives do on its own attacks: chances rolled at the
//! attack point, and what a blow that landed leaves on its target.

use bota_core::{
    AttackClass, ModifierKind, NeutralAbility, Ratio, Spell, Strike, ability_cooldown, at_level,
    class_pct, rules, ticks_of_ms,
};
use bota_proto::{DamageKind, Inflictor, UnitKind};

use crate::game::{Chance, Entity, Landed, Modifier, Purpose, Stats, World, wire_id};

use super::hitting::entity_slot;
use super::modifiers::resisted_ticks;

/// What a unit's passives add to one swing, rolled at its attack point.
#[derive(Clone, Copy)]
pub(super) struct Boost {
    /// The percent of its damage the swing deals: 100 for a plain one.
    pub crit_pct: i32,
    /// Whether the swing goes through evasion.
    pub accurate: bool,
}

/// One strike of one unit, aimed at what its blow landed on.
#[derive(Clone, Copy)]
struct Proc {
    by: Entity,
    on: Entity,
    slot: usize,
    ability: NeutralAbility,
    level: u8,
}

impl Proc {
    /// The passive, as what it does names it.
    fn inflictor(self) -> Option<Inflictor> {
        Some(Inflictor::Ability(self.ability.id()))
    }
}

/// Ticks of a petrify's damage beat: a tenth of a second.
const PETRIFY_BEAT: u32 = ticks_of_ms(100);

impl World {
    /// The strikes and chances a unit's own passives hold, by slot: none
    /// for a unit that is not a neutral, and none while it is broken.
    fn passives(&self, entity: Entity) -> Vec<(usize, NeutralAbility, u8)> {
        if self.neutral.get(entity).is_none() || self.broken(entity) {
            return Vec::new();
        }
        let Some(book) = self.abilities.get(entity) else {
            return Vec::new();
        };
        book.slots
            .iter()
            .enumerate()
            .filter_map(|(at, state)| Some((at, NeutralAbility::of(state.id)?, state.level)))
            .collect()
    }

    /// What a swing is worth in damage class: a neutral with the Piercing
    /// ability attacks as a piercing creep, any other unit as its kind.
    pub(super) fn class_pct_of(&self, attacker: Entity, on: Entity) -> i32 {
        let (Some(&from), Some(&to)) = (self.kind.get(attacker), self.kind.get(on)) else {
            return 100;
        };
        let piercing = self.neutral.get(attacker).is_some()
            && self.abilities.get(attacker).is_some_and(|book| {
                book.slots
                    .iter()
                    .any(|state| NeutralAbility::of(state.id) == Some(NeutralAbility::Piercing))
            });
        if piercing {
            class_pct(AttackClass::Piercing, to)
        } else {
            bota_core::attack_class_pct(from, to)
        }
    }

    /// Rolls what a swing of `attacker` adds at its attack point: a
    /// critical strike, whose chance is held exactly over every run of
    /// swings like a pierce, and the accuracy an aura gave it.
    pub(super) fn boost_of(&mut self, attacker: Entity, stats: &Stats) -> Boost {
        let mut boost = Boost {
            crit_pct: 100,
            accurate: false,
        };
        if stats.accuracy_pct > 0 {
            let pct = u32::try_from(stats.accuracy_pct).unwrap_or(0);
            boost.accurate = self.rng.global(Purpose::Accuracy).below(100) < pct;
        }
        for (_, ability, level) in self.passives(attacker) {
            if let Spell::Strike(Strike::Crit {
                chance_pct,
                mult_pct,
            }) = *ability.spell()
            {
                let ratio = Ratio::percent(u8::try_from(chance_pct).unwrap_or(0));
                let rng = &self.rng;
                let chance = entity_slot(&mut self.crit, attacker, || {
                    Chance::new(rng.for_unit(Purpose::Crit, wire_id(attacker), 0), ratio)
                });
                if chance.roll(ratio) {
                    boost.crit_pct = boost.crit_pct.max(at_level(&mult_pct, level));
                }
            }
        }
        boost
    }

    /// Answers to the blows that landed this tick: what their attackers'
    /// lifesteal mends, and what their strikes lay on the struck. A swing
    /// that missed is not among them; a strike needs a target that lives
    /// and is a unit the attacker's side fights.
    pub fn land_strikes(&mut self, felt: &[Landed]) {
        for blow in felt.iter().filter(|blow| blow.attack) {
            let Some(by) = blow.source.filter(|by| self.entities.contains(*by)) else {
                continue;
            };
            let on = blow.target;
            if !self.foes(by, on) {
                continue;
            }
            self.lifesteal(by, blow.amount);
            for (slot, ability, level) in self.passives(by) {
                let Spell::Strike(strike) = *ability.spell() else {
                    continue;
                };
                let given = Proc {
                    by,
                    on,
                    slot,
                    ability,
                    level,
                };
                match strike {
                    Strike::Splash { radius, pct } => self.splash(given, blow, (radius, pct)),
                    _ if blow.fatal => {}
                    strike => self.lay_strike(given, strike),
                }
            }
        }
    }

    /// Whether `by` fights `on` and `on` is a unit its blows leave
    /// something on: another side's, alive, no building and no ward.
    fn foes(&self, by: Entity, on: Entity) -> bool {
        self.team.get(by) != self.team.get(on)
            && self.team.get(on).is_some()
            && self.kind.get(on).is_some_and(|kind| {
                !bota_core::kind_def(*kind).structure && *kind != UnitKind::Ward
            })
    }

    /// Mends an attacker by its lifesteal share of the damage it dealt.
    fn lifesteal(&mut self, by: Entity, dealt: i32) {
        let pct = self.stats.get(by).map_or(0, |stats| stats.lifesteal_pct);
        if pct > 0 && dealt > 0 && self.living_unit(by) {
            self.mend(by, dealt * pct / 100);
        }
    }

    /// One strike besides the splash, on a target that lives.
    fn lay_strike(&mut self, given: Proc, strike: Strike) {
        match strike {
            Strike::Disarm { hits, ms, .. } => {
                self.steal_weapon(given, hits, at_level(&ms, given.level));
            }
            Strike::Break { ms } => {
                if self.cooled_down(given) {
                    self.start_cooldown(given);
                    let ticks = ticks_of_ms(ms);
                    self.put_modifier(
                        given.on,
                        Modifier::new(
                            ModifierKind::Broken,
                            Some(given.by),
                            given.inflictor(),
                            Some(ticks),
                        ),
                    );
                }
            }
            Strike::Lay {
                hero_ms, other_ms, ..
            } => {
                let hero = self.kind.get(given.on) == Some(&UnitKind::Hero);
                self.lay_debuff(given, ticks_of_ms(if hero { hero_ms } else { other_ms }));
            }
            Strike::Petrify { below_pct, ms, .. } => {
                self.petrify(given, below_pct, at_level(&ms, given.level))
            }
            Strike::Crit { .. } | Strike::Splash { .. } => {}
        }
    }

    /// Counts a blow towards a disarm; the `hits`th, with the ability off
    /// its wait, disarms the target for `ms` and starts the wait. The count
    /// is only kept while the ability is ready.
    fn steal_weapon(&mut self, given: Proc, hits: u8, ms: i32) {
        if !self.cooled_down(given) {
            return;
        }
        let counted = self
            .modifiers
            .get(given.on)
            .and_then(|on_it| on_it.spell(given.ability, Some(given.by)))
            .map_or(0, |held| match held.kind {
                ModifierKind::Spell { amount, .. } => amount,
                _ => 0,
            })
            + 1;
        if counted >= u16::from(hits) {
            self.take_spell(given.on, given.ability, given.by);
            self.start_cooldown(given);
            self.put_modifier(
                given.on,
                Modifier::new(
                    ModifierKind::Disarmed,
                    Some(given.by),
                    given.inflictor(),
                    Some(ticks_of_ms(ms)),
                ),
            );
            return;
        }
        let kind = ModifierKind::Spell {
            ability: given.ability,
            level: given.level,
            amount: counted,
        };
        let held = Modifier::new(
            kind,
            Some(given.by),
            given.inflictor(),
            Some(rules::STRIKE_COUNTER_TICKS),
        );
        if let Some(on_it) = self.modifiers.get_mut(given.on) {
            on_it.put(held);
        }
    }

    /// Takes a spell's modifier from one source off a unit.
    fn take_spell(&mut self, on: Entity, ability: NeutralAbility, source: Entity) {
        if let Some(on_it) = self.modifiers.get_mut(on) {
            on_it.0.retain(|held| {
                !(matches!(held.kind, ModifierKind::Spell { ability: it, .. } if it == ability)
                    && held.source == Some(source))
            });
        }
    }

    /// Lays a strike's debuff for `ticks`. Its damage beats on whole
    /// seconds counted from the first blow: a blow that refreshes it keeps
    /// the beat it had.
    fn lay_debuff(&mut self, given: Proc, ticks: u32) {
        let phase = self
            .modifiers
            .get(given.on)
            .and_then(|on_it| on_it.spell(given.ability, Some(given.by)))
            .map_or_else(
                || (self.tick % rules::TICKS_PER_SECOND) as u16,
                |held| match held.kind {
                    ModifierKind::Spell { amount, .. } => amount,
                    _ => 0,
                },
            );
        let kind = ModifierKind::Spell {
            ability: given.ability,
            level: given.level,
            amount: phase,
        };
        let held = Modifier::new(kind, Some(given.by), given.inflictor(), Some(ticks));
        self.put_modifier(given.on, held);
    }

    /// A blow off its wait, begun below `below_pct` of the bearer's health,
    /// roots the target and burns it. Approximation: the health is read as
    /// the blow lands, not as the swing begins, a few ticks earlier.
    fn petrify(&mut self, given: Proc, below_pct: i32, ms: i32) {
        let hurt = self
            .stats
            .get(given.by)
            .zip(self.health.get(given.by))
            .is_some_and(|(stats, health)| {
                i64::from(health.hp.raw) * 100 < i64::from(stats.max_hp.raw) * i64::from(below_pct)
            });
        if !hurt || !self.cooled_down(given) {
            return;
        }
        self.start_cooldown(given);
        let base = ticks_of_ms(ms);
        let ticks = resisted_ticks(&self.stats, given.on, ModifierKind::Rooted, base);
        let burn = ModifierKind::Spell {
            ability: given.ability,
            level: given.level,
            amount: ticks as u16,
        };
        if let Some(on_it) = self.modifiers.get_mut(given.on) {
            on_it.put(Modifier::new(
                ModifierKind::Rooted,
                Some(given.by),
                given.inflictor(),
                Some(ticks),
            ));
            on_it.put(Modifier::new(
                burn,
                Some(given.by),
                given.inflictor(),
                Some(ticks),
            ));
        }
    }

    /// Every other enemy within `radius` of the struck takes `pct` percent
    /// of the blow as physical damage.
    fn splash(&mut self, given: Proc, blow: &Landed, (radius, pct): (i32, i32)) {
        let reach = rules::units(radius);
        for other in self.entities.iter().collect::<Vec<_>>() {
            if other != given.on
                && self.foes(given.by, other)
                && self.alive(other)
                && self.pos(other).is_some_and(|at| at.within(blow.at, reach))
            {
                self.push_hit(
                    Some(given.by),
                    other,
                    blow.dealt * pct / 100,
                    DamageKind::Physical,
                    given.inflictor(),
                );
            }
        }
    }

    /// Whether a strike's ability is off its wait.
    fn cooled_down(&self, given: Proc) -> bool {
        self.abilities
            .get(given.by)
            .and_then(|book| book.slot(given.slot))
            .is_some_and(|state| state.cooldown == 0)
    }

    /// Starts the wait of a strike's ability.
    fn start_cooldown(&mut self, given: Proc) {
        let base = ability_cooldown(given.ability.id(), given.level);
        let ticks = self.cooldown_after_rate(given.by, base);
        if let Some(state) = self
            .abilities
            .get_mut(given.by)
            .and_then(|book| book.slots.get_mut(given.slot))
        {
            state.cooldown = ticks;
        }
    }

    /// The damage a debuff laid by a strike does on this tick, if it is
    /// due: a lay's magical beat each whole second, a petrify's physical
    /// one every tenth.
    pub(super) fn spell_pulse(
        &mut self,
        on: Entity,
        modifier: Modifier,
        (ability, level, amount): (NeutralAbility, u8, u16),
    ) {
        let Spell::Strike(strike) = *ability.spell() else {
            return;
        };
        let left = modifier.ticks_left.unwrap_or(0);
        match strike {
            Strike::Lay { dps, .. } => {
                let damage = at_level(&dps, level);
                let due = self.tick % rules::TICKS_PER_SECOND == u32::from(amount);
                if damage > 0 && due && self.alive(on) {
                    self.push_hit(
                        modifier.source,
                        on,
                        damage,
                        DamageKind::Magical,
                        modifier.inflictor,
                    );
                }
            }
            Strike::Petrify { dps, .. } => {
                let elapsed = u32::from(amount).saturating_sub(left);
                if elapsed > 0 && elapsed.is_multiple_of(PETRIFY_BEAT) && self.alive(on) {
                    let damage = dps * PETRIFY_BEAT as i32 / rules::TICKS_PER_SECOND as i32;
                    self.push_hit(
                        modifier.source,
                        on,
                        damage,
                        DamageKind::Physical,
                        modifier.inflictor,
                    );
                }
            }
            _ => {}
        }
    }
}
