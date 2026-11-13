//! What a neutral's passives do on its own attacks: chances rolled at the
//! attack point, and what a blow that landed leaves on its target.

use bota_core::{
    AttackClass, MAX_ITEM_STRIKES, ModifierKind, NeutralAbility, Odds, Spell, Strike,
    ability_cooldown, at_level, carried_strikes, class_pct, rules, ticks_of_ms,
};
use bota_proto::{DamageKind, Fixed, Inflictor, ItemId, UnitKind, Vec2};

use crate::game::{
    Entity, Landed, Modifier, Orphan, PseudoRandom, Purpose, RiderKind, Rolls, Stats, World,
    wire_id,
};

use super::hitting::{BLOCK_PLACE, entity_slot};
use super::modifiers::resisted_ticks;

/// What a unit's passives add to one swing, rolled at its attack point.
#[derive(Clone, Copy)]
pub(super) struct Boost {
    /// The percent of its damage the swing deals: 100 for a plain one.
    pub crit_pct: i32,
    /// A critical strike landed as magic on top of the swing: the percent of its damage, and the
    /// item (Revenant's Brooch).
    pub phantom: Option<(i32, ItemId)>,
    /// The item whose chain lightning the swing lets loose where it lands.
    pub chain: Option<ItemId>,
    /// The split shot the swing throws besides itself.
    pub split: Option<SplitShot>,
    /// Whether the swing goes through evasion for an accuracy aura's sake (a chain lightning
    /// that went off takes it through too: [`Boost::chain`]).
    pub accurate: bool,
    pub proc: Option<crate::game::AttackProc>,
}

/// A split shot that went off on a swing: [`Strike::Split`]'s numbers.
#[derive(Clone, Copy)]
pub(super) struct SplitShot {
    pub count: u8,
    pub range_bonus: i32,
    pub angle: i32,
    pub base_damage: i32,
    pub pct: i32,
}

/// Where a strike comes from, which keeps its rolls and its wait: a neutral's passive in an
/// ability slot, or an item carried in an inventory slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    Ability {
        slot: usize,
        ability: NeutralAbility,
        level: u8,
    },
    Item {
        slot: usize,
        /// Which of the item's strikes.
        index: usize,
        item: ItemId,
    },
}

/// Where the items' strikes keep their rolls among a unit's chance procs: past every ability
/// slot, [`MAX_ITEM_STRIKES`] places to an inventory slot.
const ITEM_PLACE: usize = 64;
const _: () = assert!(ITEM_PLACE + rules::INVENTORY_SLOTS * MAX_ITEM_STRIKES <= BLOCK_PLACE);
const _: () = assert!(BLOCK_PLACE <= u8::MAX as usize);

impl Origin {
    /// Its place among the unit's chance procs, which keys its sequence.
    const fn place(self) -> usize {
        match self {
            Origin::Ability { slot, .. } => slot,
            Origin::Item { slot, index, .. } => ITEM_PLACE + slot * MAX_ITEM_STRIKES + index,
        }
    }

    /// The level its numbers are read at: an item's have one.
    const fn level(self) -> u8 {
        match self {
            Origin::Ability { level, .. } => level,
            Origin::Item { .. } => 1,
        }
    }

    /// What the damage and the modifiers it deals are laid to.
    fn inflictor(self) -> Inflictor {
        match self {
            Origin::Ability { ability, .. } => Inflictor::Ability(ability.id()),
            Origin::Item { item, .. } => Inflictor::Item(item),
        }
    }
}

/// One strike a unit holds, and where it comes from.
#[derive(Clone, Copy)]
struct Source {
    origin: Origin,
    strike: Strike,
}

/// Of several strikes of one kind that do not stack, the one that counts: the largest `worth`,
/// then the first place.
fn strongest(
    sources: &[Source],
    worth: impl Fn(Strike, u8) -> Option<i32>,
) -> Option<(Source, i32)> {
    sources
        .iter()
        .filter_map(|source| Some((*source, worth(source.strike, source.origin.level())?)))
        .min_by_key(|(source, worth)| (std::cmp::Reverse(*worth), source.origin.place()))
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

/// A cleave's trapezoid: it opens at the attacker towards the struck, `start_radius` to either
/// side of its axis there and `end_radius` at `distance` along it.
struct Cone {
    from: Vec2,
    towards: Vec2,
    start_radius: Fixed,
    end_radius: Fixed,
    distance: Fixed,
}

impl Cone {
    /// Whether a spot lies within it.
    fn holds(&self, at: Vec2) -> bool {
        let axis = self.towards - self.from;
        let length = bota_core::isqrt64(axis.length_squared());
        if length == 0 {
            return false;
        }
        let off = at - self.from;
        let (ax, ay) = (i128::from(axis.x.raw), i128::from(axis.y.raw));
        let (ox, oy) = (i128::from(off.x.raw), i128::from(off.y.raw));
        // Raw units along the axis and away from it.
        let along = (ox * ax + oy * ay) / i128::from(length);
        let aside = (ox * ay - oy * ax).abs() / i128::from(length);
        let distance = i128::from(self.distance.raw);
        if along < 0 || along > distance || distance == 0 {
            return false;
        }
        let (start, end) = (
            i128::from(self.start_radius.raw),
            i128::from(self.end_radius.raw),
        );
        aside <= start + (end - start) * along / distance
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

    /// The strikes a unit holds, by place: its own passives (a neutral's, unless broken), then
    /// those of the items it carries ([`carried_strikes`]), a ranged attacker's as an item gives
    /// them to one, and an illusion's only those it goes through ([`Strike::by_illusions`]).
    fn sources(&self, entity: Entity) -> Vec<Source> {
        let mut sources: Vec<Source> = self
            .passives(entity)
            .into_iter()
            .filter_map(|(slot, ability, level)| match *ability.spell() {
                Spell::Strike(strike) => Some(Source {
                    origin: Origin::Ability {
                        slot,
                        ability,
                        level,
                    },
                    strike,
                }),
                _ => None,
            })
            .collect();
        let ranged = self
            .stats
            .get(entity)
            .is_some_and(|stats| stats.projectile_speed.is_some());
        let illusion = self.wears_illusion(entity);
        let mut index = (usize::MAX, 0);
        for (slot, item, strike) in carried_strikes(self.carried_slots(entity), ranged) {
            index = if index.0 == slot {
                (slot, index.1 + 1)
            } else {
                (slot, 0)
            };
            // The strike keeps its item's index whether or not an illusion goes through it.
            if illusion && !strike.by_illusions() {
                continue;
            }
            sources.push(Source {
                origin: Origin::Item {
                    slot,
                    index: index.1,
                    item,
                },
                strike,
            });
        }
        sources
    }

    /// Whether a source of `by`'s hits at `chance_pct` percent this time, on its own
    /// pseudo-random sequence: a neutral's passive on its ability slot's, an item's on its place's.
    fn rolls(&mut self, by: Entity, origin: Origin, chance_pct: i32) -> bool {
        let odds = Odds::percent(u8::try_from(chance_pct.clamp(0, 100)).unwrap_or(0));
        let place = origin.place();
        let rng = &self.rng;
        let rolls = entity_slot(&mut self.crit, by, Rolls::default);
        let source = rolls.at(place, || {
            let purpose = match origin {
                Origin::Ability { .. } => Purpose::Crit,
                Origin::Item { .. } => Purpose::ItemProc,
            };
            PseudoRandom::new(rng.for_unit(purpose, wire_id(by), place as u8))
        });
        source.roll(odds)
    }

    /// Whether a source is off its wait: an ability's slot, a strike's own timer (a chain's,
    /// which no item shows and nothing cuts or refreshes), or else the item's stack.
    fn strike_ready(&self, by: Entity, on: Entity, source: Source) -> bool {
        match (source.origin, source.strike) {
            (
                Origin::Ability {
                    slot,
                    ability,
                    level,
                },
                _,
            ) => self.cooled_down(Proc {
                by,
                on,
                slot,
                ability,
                level,
            }),
            (origin @ Origin::Item { .. }, Strike::Chain { .. }) => self
                .crit
                .get(by.index().0 as usize)
                .and_then(Option::as_ref)
                .and_then(|rolls| rolls.sources().get(origin.place()))
                .and_then(Option::as_ref)
                .is_none_or(|roll| roll.ready(self.tick)),
            (Origin::Item { slot, .. }, _) => self
                .inventory
                .get(by)
                .and_then(|bag| bag.slots.get(slot).copied().flatten())
                .is_some_and(|stack| stack.cooldown == 0),
        }
    }

    /// Starts a source's wait: an ability's cooldown; a chain's own timer of `wait_ms`, as it is
    /// (Maelstrom's internal wait: neither shown on the item nor touched by cooldown cuts); or
    /// the item's cooldown, shortened as every cooldown of the unit is.
    fn start_wait(&mut self, by: Entity, on: Entity, source: Source) {
        match (source.origin, source.strike) {
            (
                Origin::Ability {
                    slot,
                    ability,
                    level,
                },
                _,
            ) => self.start_cooldown(Proc {
                by,
                on,
                slot,
                ability,
                level,
            }),
            (origin @ Origin::Item { .. }, Strike::Chain { wait_ms, .. }) => {
                let until = self.tick + ticks_of_ms(wait_ms);
                if let Some(roll) = self
                    .crit
                    .get_mut(by.index().0 as usize)
                    .and_then(Option::as_mut)
                    .and_then(|rolls| rolls.at_opened(origin.place()))
                {
                    roll.wait_until(until);
                }
            }
            (Origin::Item { slot, item, .. }, _) => {
                let base = bota_core::item_def(item).map_or(0, |def| def.cooldown);
                let ticks = self.cooldown_after_rate(by, base);
                if let Some(Some(stack)) = self
                    .inventory
                    .get_mut(by)
                    .and_then(|bag| bag.slots.get_mut(slot))
                {
                    stack.cooldown = ticks;
                }
            }
        }
    }

    /// Whether a unit is a building.
    fn structure(&self, entity: Entity) -> bool {
        self.kind
            .get(entity)
            .is_some_and(|kind| bota_core::kind_def(*kind).structure)
    }

    /// Rolls what a swing of `attacker` adds at its attack point, as Dota
    /// rolls it: the accuracy its auras give it, on a pseudo-random sequence
    /// of its own, and a critical strike. Its crits roll from the strongest
    /// down, each on a sequence of its own (its ability's slot, its item's
    /// place), and stop at the first that hits: one that did not roll keeps
    /// its run. A critical strike landed as magic ranks by the share of the
    /// swing the struck takes in all.
    pub(super) fn boost_of(&mut self, attacker: Entity, on: Entity, stats: &Stats) -> Boost {
        let mut boost = Boost {
            crit_pct: 100,
            phantom: None,
            chain: None,
            split: None,
            accurate: false,
            proc: None,
        };
        if stats.accuracy != Odds::NEVER {
            let rng = &self.rng;
            let source = entity_slot(&mut self.accuracy, attacker, || {
                PseudoRandom::new(rng.for_unit(Purpose::Accuracy, wire_id(attacker), 0))
            });
            boost.accurate = source.roll(stats.accuracy);
        }
        let sources = self.sources(attacker);
        let building = self.structure(on);
        let mut crits: Vec<(i32, Source, i32, i32, bool)> = sources
            .iter()
            .filter_map(|source| match source.strike {
                Strike::Crit {
                    chance_pct,
                    mult_pct,
                    magic,
                    buildings,
                } if buildings || !building => {
                    let mult = at_level(&mult_pct, source.origin.level());
                    let strength = if magic { 100 + mult } else { mult };
                    Some((strength, *source, chance_pct, mult, magic))
                }
                _ => None,
            })
            .collect();
        // The strongest first; of two as strong, the earlier place.
        crits.sort_by_key(|&(strength, source, ..)| {
            (std::cmp::Reverse(strength), source.origin.place())
        });
        for (_, source, chance_pct, mult, magic) in crits {
            if self.rolls(attacker, source.origin, chance_pct) {
                match source.origin {
                    Origin::Item { item, .. } if magic => boost.phantom = Some((mult, item)),
                    _ => boost.crit_pct = mult,
                }
                break;
            }
        }
        boost.proc = self.conditional_proc(attacker, on, &sources);
        boost.chain = self.roll_chain(attacker, on, &sources);
        boost.split = self.roll_split(attacker, on, &sources);
        boost
    }

    /// Conditional attack bonuses roll once at the attack point and survive flight: a bash, of a
    /// unit's bashes the one that deals most, rolled while it is off its wait. No illusion bashes
    /// and nothing bashes a ward; an item's bash never a building.
    fn conditional_proc(
        &mut self,
        attacker: Entity,
        on: Entity,
        sources: &[Source],
    ) -> Option<crate::game::AttackProc> {
        if self.wears_illusion(attacker)
            || self.kind.get(on).is_some_and(|k| {
                bota_core::kind_def(*k).target_class == bota_core::TargetClass::Ward
            })
        {
            return None;
        }
        let building = self.structure(on);
        let (source, _) = strongest(sources, |strike, level| match strike {
            Strike::Bash { damage, .. } => Some(at_level(&damage, level)),
            _ => None,
        })
        .filter(|(source, _)| matches!(source.origin, Origin::Ability { .. }) || !building)?;
        let Strike::Bash {
            chance_pct,
            damage,
            stun_ms,
            effect,
        } = source.strike
        else {
            return None;
        };
        if !self.strike_ready(attacker, on, source)
            || !self.rolls(attacker, source.origin, chance_pct)
        {
            return None;
        }
        self.start_wait(attacker, on, source);
        let pierces = match source.origin {
            Origin::Ability { ability, .. } => {
                self.tell_ability_triggered(attacker, ability.id());
                ability.entry().def.pierces
            }
            // Skull Basher's bash goes through spell immunity (`SPELL_IMMUNITY_ENEMIES_YES`, held
            // at its row).
            Origin::Item { .. } => bota_core::Pierces::Yes,
        };
        let level = source.origin.level();
        Some(crate::game::AttackProc {
            inflictor: source.origin.inflictor(),
            bonus_damage: at_level(&damage, level),
            modifier: Some(ModifierKind::Control {
                control: bota_core::Control::Stun,
                effect,
            }),
            ticks: ticks_of_ms(at_level(&stun_ms, level)),
            pierces,
        })
    }

    /// The chain lightning a swing at `on` lets loose where it lands: of the chains a unit
    /// carries, the one that deals most, rolled while off its wait, and only at an enemy unit (no
    /// building, no ward) its bolt reaches (no spell-immune one, for a bolt that does not go
    /// through spell immunity). Its item waits from the roll that hit.
    fn roll_chain(&mut self, attacker: Entity, on: Entity, sources: &[Source]) -> Option<ItemId> {
        if !self.foes(attacker, on) {
            return None;
        }
        let (source, _) = strongest(sources, |strike, _| match strike {
            Strike::Chain { damage, .. } => Some(damage),
            _ => None,
        })?;
        let (
            Strike::Chain {
                chance_pct,
                pierces_immunity,
                ..
            },
            Origin::Item { item, .. },
        ) = (source.strike, source.origin)
        else {
            return None;
        };
        // A bolt that spell immunity keeps off goes off on no swing at a spell-immune target (a
        // courier): nothing is rolled.
        let immune = self.stats.get(on).is_some_and(|stats| stats.spell_immune);
        if immune && !pierces_immunity {
            return None;
        }
        if !self.strike_ready(attacker, on, source)
            || !self.rolls(attacker, source.origin, chance_pct)
        {
            return None;
        }
        self.start_wait(attacker, on, source);
        Some(item)
    }

    /// The split shot a swing at `on` throws besides itself: of the split shots a unit carries,
    /// the one that throws most, rolled only on a swing at an enemy unit (no building, ward or
    /// courier: Splitshot never goes off on one).
    fn roll_split(
        &mut self,
        attacker: Entity,
        on: Entity,
        sources: &[Source],
    ) -> Option<SplitShot> {
        if !self.proc_target(attacker, on) {
            return None;
        }
        let (source, _) = strongest(sources, |strike, _| match strike {
            Strike::Split { count, .. } => Some(i32::from(count)),
            _ => None,
        })?;
        let Strike::Split {
            chance_pct,
            count,
            range_bonus,
            angle,
            base_damage,
            pct,
        } = source.strike
        else {
            return None;
        };
        self.rolls(attacker, source.origin, chance_pct)
            .then_some(SplitShot {
                count,
                range_bonus,
                angle,
                base_damage,
                pct,
            })
    }

    /// The enemies a split shot of `attacker` throws its extra missiles at, besides `on`: up to
    /// its count of the enemy units it may attack (no building, ward or courier), heroes before
    /// the rest and the nearest first, within its attack range and the shot's `range_bonus` and
    /// within `angle` degrees in front of it.
    pub(super) fn split_targets(
        &self,
        attacker: Entity,
        on: Entity,
        stats: &Stats,
        shot: SplitShot,
    ) -> Vec<Entity> {
        let Some(from) = self.transform.get(attacker).copied() else {
            return Vec::new();
        };
        let reach = stats.attack_range + rules::units(shot.range_bonus);
        let half_angle = (i64::from(shot.angle.clamp(0, 360)) * 65_536 / 720) as u16;
        // Heroes first, the nearest first among each.
        let mut found: Vec<(bool, i64, Entity)> = self
            .entities
            .iter()
            .filter(|&other| other != on && self.proc_target(attacker, other))
            .filter(|&other| self.reachable(attacker, reach, other))
            .filter_map(|other| {
                let at = self.pos(other)?;
                let gap =
                    bota_core::facing_gap(from.facing, bota_core::facing_towards(from.pos, at));
                let hero = self.kind.get(other) == Some(&UnitKind::Hero);
                (gap <= half_angle).then_some((!hero, at.distance_squared(from.pos), other))
            })
            .collect();
        found.sort();
        found
            .into_iter()
            .take(usize::from(shot.count))
            .map(|(_, _, other)| other)
            .collect()
    }

    /// Whether an item's attack proc reaches `on`: a unit `by` fights, no building, ward or
    /// courier.
    fn proc_target(&self, by: Entity, on: Entity) -> bool {
        self.foes(by, on) && self.kind.get(on) != Some(&UnitKind::Courier)
    }

    /// Answers to the blows that landed this tick: what their attackers'
    /// lifesteal mends, the chain lightning they let loose, and what their
    /// strikes lay on the struck. A swing that missed is not among them; a
    /// strike needs a target that is a unit the attacker's side fights, and
    /// one that lays something a target that lives.
    pub fn land_strikes(&mut self, felt: &[Landed]) {
        for blow in felt.iter().filter(|blow| blow.attack) {
            let Some(by) = blow.source.filter(|by| self.entities.contains(*by)) else {
                continue;
            };
            let on = blow.target;
            if !self.foes(by, on) {
                continue;
            }
            // An attack's own blow is physical; the magic riding with it is
            // a blow of its own, which lifesteal leaves alone.
            self.lifesteal(by, on, blow.stolen);
            if self.wears_fountain_passive(by) {
                let splash = (rules::FOUNTAIN_SPLASH_RADIUS, rules::FOUNTAIN_SPLASH_PCT);
                self.splash(by, None, blow, splash);
            }
            for rider in blow.riders.iter() {
                if rider.kind == RiderKind::Chain {
                    self.let_loose_chain(by, rider.item, blow.target, blow.at);
                }
            }
            for source in self.sources(by) {
                match (source.strike, source.origin) {
                    (Strike::Splash { radius, pct }, _) => {
                        self.splash(by, Some(source.origin.inflictor()), blow, (radius, pct));
                    }
                    (strike @ Strike::Cleave { .. }, Origin::Item { item, .. }) => {
                        self.cleave(by, item, blow, strike);
                    }
                    (_, _) if blow.fatal => {}
                    (
                        strike,
                        Origin::Ability {
                            slot,
                            ability,
                            level,
                        },
                    ) => self.lay_strike(
                        Proc {
                            by,
                            on,
                            slot,
                            ability,
                            level,
                        },
                        strike,
                    ),
                    // What an item's other strikes do, they did as the swing went off.
                    (_, Origin::Item { .. }) => {}
                }
            }
        }
    }

    /// Lets loose the chain lightning of `item` a swing of `by` carried, where it reached `on`
    /// standing at `at`: it strikes the struck and leaps on from it (its numbers are the item's
    /// [`Strike::Chain`]), and stops as its wielder dies. A dead wielder lets loose none.
    pub(super) fn let_loose_chain(&mut self, by: Entity, item: ItemId, on: Entity, at: Vec2) {
        if !self.alive(by) {
            return;
        }
        let chain = bota_core::item_def(item).and_then(|def| {
            def.carried.strikes.iter().find_map(|strike| match *strike {
                Strike::Chain {
                    damage,
                    strikes,
                    radius,
                    delay_ms,
                    ..
                } => Some(super::spell_strike::Bolt {
                    damage,
                    targets: strikes,
                    radius,
                    delay_ms,
                    keep_pct: 100,
                    life: crate::game::ChainLife::EndsWithSource,
                }),
                _ => None,
            })
        });
        if let Some(bolt) = chain {
            self.chain_from(by, Inflictor::Item(item), on, at, bolt);
        }
    }

    /// Lets loose the chain lightning of attacks whose target was dead before they reached it:
    /// Chain Lightning procs "On already dead enemies (i.e. target died before projectile
    /// impact)" (Liquipedia, Maelstrom), from where the target fell.
    pub(crate) fn let_loose_orphans(&mut self, orphans: Vec<Orphan>) {
        for orphan in orphans {
            for rider in orphan.riders.iter() {
                if rider.kind == RiderKind::Chain {
                    self.let_loose_chain(orphan.source, rider.item, orphan.target, orphan.at);
                }
            }
        }
    }

    /// Cleaves a swing of `by` that landed: every enemy unit but the struck, no building, ward or
    /// courier, in the trapezoid of `strike` takes its share of the blow as dealt as physical
    /// damage, laid to `item`, a cleave a damage block answers to. The trapezoid opens at the
    /// attacker towards where the blow landed, `start_radius` to either side there and
    /// `end_radius` at `distance`, both widened by its AoE bonus.
    fn cleave(&mut self, by: Entity, item: ItemId, blow: &Landed, strike: Strike) {
        let Strike::Cleave {
            hero_pct,
            creep_pct,
            start_radius,
            end_radius,
            distance,
        } = strike
        else {
            return;
        };
        let Some(from) = self.pos(by) else {
            return;
        };
        let cone = Cone {
            from,
            towards: blow.at,
            start_radius: self.widened_of(by, start_radius),
            end_radius: self.widened_of(by, end_radius),
            distance: rules::units(distance),
        };
        for other in self.entities.iter().collect::<Vec<_>>() {
            if other == blow.target
                || !self.proc_target(by, other)
                || !self.alive(other)
                || !self.pos(other).is_some_and(|at| cone.holds(at))
            {
                continue;
            }
            let pct = if self.kind.get(other) == Some(&UnitKind::Hero) {
                hero_pct
            } else {
                creep_pct
            };
            self.push_splash_hit(
                Some(by),
                other,
                blow.dealt * pct / 100,
                Some(Inflictor::Item(item)),
                crate::game::Category::Cleave,
            );
        }
    }

    /// Whether `by` fights `on` and `on` is a unit its blows leave
    /// something on: another side's, alive, no building and no ward.
    fn foes(&self, by: Entity, on: Entity) -> bool {
        self.team.get(by) != self.team.get(on)
            && self.team.get(on).is_some()
            && self.kind.get(on).is_some_and(|kind| {
                !bota_core::kind_def(*kind).structure
                    && bota_core::kind_def(*kind).target_class != bota_core::TargetClass::Ward
            })
    }

    /// Mends an attacker by its lifesteal share of the physical damage it
    /// dealt `on` a foe (see [`Landed::stolen`]). Every lifesteal heals
    /// [`rules::LIFESTEAL_CREEP_PCT`] of itself off what is no hero (an
    /// illusion counts as one) and nothing off a courier (Dota 7.38:
    /// "Lifesteal mechanics are now consistent across every ability", and
    /// "Attack damage against wards, buildings, or couriers" gives none).
    fn lifesteal(&mut self, by: Entity, on: Entity, dealt: i32) {
        let Some(stats) = self.stats.get(by) else {
            return;
        };
        let target = self.kind.get(on).copied();
        if dealt <= 0 || target == Some(UnitKind::Courier) || !self.living_unit(by) {
            return;
        }
        let creep_pct = if target == Some(UnitKind::Hero) {
            100
        } else {
            rules::LIFESTEAL_CREEP_PCT
        };
        self.mend_part(by, share_of(dealt, stats.lifesteal_pct, creep_pct));
    }

    /// Mends the dealers of the blows felt by their spell lifesteal share of
    /// what each took (Dota 7.38): physical and magical damage of spells
    /// ([`Landed::spell`]) and the magic riding with an attack, never pure
    /// damage, an attack's own blow or what splashes off one, and nothing off
    /// an ally, an illusion, a building or a ward (nor a courier, whose
    /// health counted in hits no spell takes). Unlike an
    /// attack's lifesteal it heals off the health the blow took, the overkill
    /// left out (Liquipedia, *Spell Lifesteal*: "it does not heal for
    /// overkill damage dealt to a unit").
    pub fn steal_spell_life(&mut self, felt: &[Landed]) {
        for blow in felt {
            let spell = match blow.kind {
                DamageKind::Pure => false,
                DamageKind::Physical => blow.spell,
                DamageKind::Magical => true,
            };
            let Some(by) = blow.source.filter(|by| self.entities.contains(*by)) else {
                continue;
            };
            let pct = self
                .stats
                .get(by)
                .map_or(0, |stats| stats.spell_lifesteal_pct);
            let on = blow.target;
            let target = self.kind.get(on).copied();
            if !spell
                || blow.removal
                || pct <= 0
                || blow.amount <= 0
                || !self.foes(by, on)
                || self.wears(on, ModifierKind::Illusion)
                || !self.living_unit(by)
            {
                continue;
            }
            let creep_pct = if target == Some(UnitKind::Hero) {
                100
            } else {
                rules::SPELL_LIFESTEAL_CREEP_PCT
            };
            self.mend_part(by, share_of(blow.amount, pct, creep_pct));
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
            Strike::Crit { .. }
            | Strike::Splash { .. }
            | Strike::Bash { .. }
            | Strike::Cleave { .. }
            | Strike::Chain { .. }
            | Strike::Split { .. } => {}
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

    /// Every other enemy of `by` within `radius` of the struck, no building
    /// and no ward, takes `pct` percent of the blow as physical damage, laid
    /// to `inflictor`: a neutral's passive, or nothing for the fountain's own
    /// shot.
    fn splash(
        &mut self,
        by: Entity,
        inflictor: Option<Inflictor>,
        blow: &Landed,
        (radius, pct): (i32, i32),
    ) {
        let reach = rules::units(radius);
        for other in self.entities.iter().collect::<Vec<_>>() {
            if other != blow.target
                && self.foes(by, other)
                && self.alive(other)
                && self.pos(other).is_some_and(|at| at.within(blow.at, reach))
            {
                self.push_splash_hit(
                    Some(by),
                    other,
                    blow.dealt * pct / 100,
                    inflictor,
                    crate::game::Category::Plain,
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

/// `pct` percent of `amount`, and `creep_pct` percent of that, a fraction of a point included
/// (Dota heals the fraction too). Worked out in 64 bits and held to the most health a point can
/// say, so no sum of carried percentages overflows it.
fn share_of(amount: i32, pct: i32, creep_pct: i32) -> Fixed {
    let raw = i64::from(amount)
        * i64::from(pct)
        * i64::from(creep_pct)
        * i64::from(Fixed::from_int(1).raw)
        / 10_000;
    Fixed {
        raw: raw.clamp(0, i64::from(i32::MAX)) as i32,
    }
}
