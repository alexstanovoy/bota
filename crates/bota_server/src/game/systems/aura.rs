//! Auras: what a unit hands out to those standing near it, of its own (a
//! building's, a flagbearer's) or with an ability whose passive it is (the
//! presence, a neutral's aura or storm), all in one pass.

use bota_core::{
    Aura, AuraModifier, NeutralAbility, Reach, Spell, Whom, ability_def, isqrt64, kind_def, rules,
    ticks_of_ms,
};
use bota_proto::{DamageKind, Inflictor, Team, UnitKind};

use crate::game::{Def, Entity, Modifier, Spots, Upgrades, World, disabled_in};

use super::modifiers::resisted_ticks;

/// One aura a unit hands out this tick.
#[derive(Clone, Copy)]
struct Source {
    carrier: Entity,
    aura: Aura,
    /// The ability whose passive it is. Absent for the unit's own.
    inflictor: Option<Inflictor>,
    /// The level the carrier holds that ability at; one for its own.
    level: u8,
}

/// A storm's burn, from [`Spell::Storm`], and the spell it is.
#[derive(Clone, Copy)]
struct Storm {
    ability: NeutralAbility,
    radius: i32,
    near_radius: i32,
    near_dps: i32,
    far_dps: i32,
    period_ms: i32,
}

/// The room a pass of auras works in, kept between passes.
#[derive(Clone, Default)]
pub struct AuraScratch {
    /// Every entity that stands somewhere, by where.
    spots: Spots<Entity>,
    /// Every aura handed out this tick, in the order it goes out.
    sources: Vec<Source>,
    /// What stands within the square about one carrier.
    near: Vec<Entity>,
}

impl World {
    /// Hands out every aura for the tick, carrier by carrier in slot order,
    /// whatever the tick's coin: a unit's own first, then its abilities' in
    /// the order of its slots.
    ///
    /// Each puts its modifier afresh on every unit it reaches, which then
    /// lingers for the aura's [`Aura::linger`] once it stops reaching it. A
    /// broken carrier's auras that a Break silences go out as ever, and what
    /// they put on does nothing while the break holds ([`World::disabled`]).
    /// A storm also burns what it reaches, on its beat, unless a Break
    /// silences it.
    pub fn tick_auras(&mut self) {
        let mut scratch = std::mem::take(&mut self.aura_scratch);
        scratch.spots.clear();
        for entity in self.entities.iter() {
            if let Some(at) = self.transform.get(entity) {
                scratch.spots.push(at.pos, entity);
            }
        }
        scratch.spots.sort();
        scratch.sources.clear();
        for carrier in self.entities.iter() {
            self.gather_auras(carrier, &mut scratch.sources);
        }
        for source in scratch.sources.iter().copied() {
            self.hand_aura_out(source, &scratch.spots, &mut scratch.near);
        }
        self.aura_scratch = scratch;
    }

    /// The auras one unit hands out this tick: its own, then those of the
    /// abilities it has learned, by slot.
    fn gather_auras(&self, carrier: Entity, into: &mut Vec<Source>) {
        let own: &'static [Aura] = self.def.get(carrier).map_or(&[], |Def(def)| def.auras);
        for aura in own {
            into.push(Source {
                carrier,
                aura: *aura,
                inflictor: None,
                level: 1,
            });
        }
        if let Some(mods) = self.modifiers.get(carrier) {
            for held in mods.active() {
                if let bota_core::ModifierKind::ItemBuff { item, .. } = held.kind
                    && let Some(template) = bota_core::item_modifier(held.kind)
                {
                    let def = bota_core::item_def(item).expect("an emitter's item is registered");
                    for emitter in template.emitters {
                        let modifier = def.modifiers[usize::from(emitter.index)];
                        into.push(Source {
                            carrier,
                            aura: emitter.emitter(
                                modifier.kind(item, emitter.index, self.tick, false),
                                modifier.ticks,
                            ),
                            inflictor: Some(Inflictor::Item(item)),
                            level: 1,
                        });
                    }
                }
                if let bota_core::ModifierKind::AuraEmitter { ability, level } = held.kind
                    && let Spell::Cast {
                        act: bota_core::Act::Roar(spec),
                        ..
                    } = *ability.spell()
                {
                    into.push(Source {
                        carrier,
                        aura: spec.aura,
                        inflictor: Some(Inflictor::Ability(ability.id())),
                        level,
                    });
                }
            }
        }
        self.gather_item_auras(carrier, into);
        let Some(book) = self.abilities.get(carrier) else {
            return;
        };
        for slot in book.slots.iter().filter(|slot| slot.level > 0) {
            let Some(aura) = ability_def(slot.id).and_then(|def| def.aura.as_ref()) else {
                continue;
            };
            into.push(Source {
                carrier,
                aura: *aura,
                inflictor: Some(Inflictor::Ability(slot.id)),
                level: slot.level,
            });
        }
    }

    fn gather_item_auras(&self, carrier: Entity, into: &mut Vec<Source>) {
        if !self.alive(carrier) || self.def.get(carrier).is_some_and(|Def(def)| def.porter) {
            return;
        }
        let Some(bag) = self.inventory.get(carrier) else {
            return;
        };
        for stack in bag
            .slots
            .iter()
            .take(rules::INVENTORY_SLOTS)
            .flatten()
            .filter(|stack| stack.mute == 0)
        {
            let Some(def) = bota_core::item_def(stack.id) else {
                continue;
            };
            if matches!(def.effect, bota_core::UseSpec::Toggle) && !stack.active {
                continue;
            }
            for spec in def.auras {
                if let Some(threshold) = spec.health_below_pct {
                    let (Some(health), Some(stats)) =
                        (self.health.get(carrier), self.stats.get(carrier))
                    else {
                        continue;
                    };
                    if i64::from(health.hp.raw) * 100
                        >= i64::from(stats.max_hp.raw) * i64::from(threshold)
                    {
                        continue;
                    }
                }
                into.push(Source {
                    carrier,
                    aura: spec.emitter(
                        def.modifiers[usize::from(spec.index)].kind(
                            stack.id,
                            spec.index,
                            self.tick,
                            self.illusion.get(carrier).is_some(),
                        ),
                        def.modifiers[usize::from(spec.index)].ticks,
                    ),
                    inflictor: Some(Inflictor::Item(stack.id)),
                    level: 1,
                });
            }
        }
    }

    /// One aura of one carrier, on every unit it reaches. A neutral's reaches
    /// no further than [`rules::NEUTRAL_AURA_RADIUS`]; one that works only in
    /// the river does nothing while its carrier stands out of it.
    fn hand_aura_out(&mut self, source: Source, spots: &Spots<Entity>, near: &mut Vec<Entity>) {
        let Source {
            carrier,
            aura,
            inflictor,
            level,
        } = source;
        let (Some(from), Some(side)) = (self.pos(carrier), self.team.get(carrier).copied()) else {
            return;
        };
        if aura.water && !self.ground.water(from) {
            return;
        }
        let radius = if side == Team::Neutral {
            aura.radius.min(rules::NEUTRAL_AURA_RADIUS)
        } else {
            aura.radius
        };
        // An AoE bonus widens every aura (Liquipedia, Area of Effect: "AoE Bonuses affect all
        // Auras"; Presence of the Dark Lord's `presence_radius` is marked).
        let reach = self.widened_of(carrier, radius);
        let steps = self
            .upgrades
            .get(carrier)
            .map_or(0, |Upgrades(steps)| *steps);
        let steps = aura
            .growth_clock
            .map_or(steps, |clock| clock.count(self.clock(), self.schedule()));
        let kind = aura
            .modifier
            .modifier(level, |talent| self.has_talent(carrier, talent), steps);
        let with = inflictor;
        let storm = storm_of(&aura).filter(|_| !(aura.breakable && self.broken(carrier)));
        near.clear();
        near.extend(spots.around(from, i64::from(reach.raw).abs()));
        for target in near.iter().copied() {
            let Some(there) = self.pos(target).filter(|at| at.within(from, reach)) else {
                continue;
            };
            if !self.reaches(&aura, side, carrier, target) {
                continue;
            }
            let ticks = resisted_ticks(&self.stats, target, kind, aura.linger);
            let fresh = Modifier {
                breakable: aura.breakable,
                ..Modifier::new(kind, Some(carrier), with, Some(ticks))
            };
            let outranked = self.modifiers.get(target).is_some_and(|on_it| {
                on_it.outranks(&fresh, |held| disabled_in(&self.modifiers, held))
            });
            if !outranked {
                self.store_modifier(target, fresh);
            }
            if let Some(storm) = storm {
                self.rain_on(
                    carrier,
                    target,
                    storm,
                    isqrt64(from.distance_squared(there)),
                );
            }
        }
    }

    /// Whether an aura of a carrier of a side reaches a unit standing within
    /// its radius: one of the side it is for, of the units it reaches, not
    /// invulnerable unless the aura reaches the invulnerable too.
    fn reaches(&self, aura: &Aura, side: Team, carrier: Entity, target: Entity) -> bool {
        let Some(theirs) = self.team.get(target).copied() else {
            return false;
        };
        let sided = match aura.whom {
            Whom::Allies => theirs == side,
            Whom::Enemies => theirs != side,
        };
        let invulnerable = self
            .stats
            .get(target)
            .is_some_and(|stats| stats.invulnerable);
        sided
            && (aura.reaches != Reach::Bearer || target == carrier)
            && self.reach_takes(aura.reaches, target)
            && (aura.invulnerable || !invulnerable)
    }

    /// Whether a unit is of those an aura's [`Reach`] takes.
    pub(crate) fn reach_takes(&self, reach: Reach, target: Entity) -> bool {
        let kind = self.kind.get(target).copied();
        match reach {
            Reach::All | Reach::Bearer => true,
            Reach::Combatants => {
                kind.is_some_and(|kind| kind == UnitKind::Hero || kind_def(kind).creep)
            }
            Reach::CombatantsAndBuildings => kind.is_some_and(|kind| {
                kind == UnitKind::Hero || kind_def(kind).creep || kind_def(kind).structure
            }),
            Reach::LaneCreeps => kind.is_some_and(|kind| kind_def(kind).lane_creep),
            Reach::Heroes => kind == Some(UnitKind::Hero),
            Reach::Units => {
                kind.is_some_and(|kind| kind != UnitKind::Hero && !kind_def(kind).fixed)
            }
            Reach::Living => self.living_unit(target),
        }
    }

    /// A storm's damage on a unit `distance` (in raw units) from its eye,
    /// on the beat: `far_dps` a second at the rim, rising evenly to
    /// `near_dps` within `near_radius`. The 0.25 s beat is eight ticks.
    fn rain_on(&mut self, source: Entity, target: Entity, storm: Storm, distance: i64) {
        let beat = ticks_of_ms(storm.period_ms);
        if !self.tick.is_multiple_of(beat) {
            return;
        }
        let near = i64::from(rules::units(storm.near_radius).raw);
        let rim = i64::from(rules::units(storm.radius).raw);
        let (low, high) = (i64::from(storm.far_dps), i64::from(storm.near_dps));
        let dps = if distance <= near {
            high
        } else {
            low + (high - low) * (rim - distance).max(0) / (rim - near)
        };
        let amount = dps * i64::from(beat) / i64::from(rules::TICKS_PER_SECOND);
        self.push_hit(
            Some(source),
            target,
            amount as i32,
            DamageKind::Magical,
            Some(Inflictor::Ability(storm.ability.id())),
        );
    }
}

/// The burn of a storm: what a neutral's [`Spell::Storm`] adds to its aura.
/// Absent for every other aura.
fn storm_of(aura: &Aura) -> Option<Storm> {
    let AuraModifier::Spell(ability) = aura.modifier else {
        return None;
    };
    match *ability.spell() {
        Spell::Storm {
            radius,
            near_radius,
            near_dps,
            far_dps,
            period_ms,
            ..
        } => Some(Storm {
            ability,
            radius,
            near_radius,
            near_dps,
            far_dps,
            period_ms,
        }),
        _ => None,
    }
}
