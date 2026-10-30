//! Runes: the schedule that puts them out, the hero that takes one, what each
//! does, and the Bottle that keeps one.

use bota_core::{
    BOTTLE_CHARGES, BOTTLE_LESSER_RUNE_CHARGES, BOTTLE_REFILL_PERIOD_TICKS, BOTTLE_RUNE_TICKS,
    BOTTLE_TICKS, Drink, ModifierKind, POWER_RUNES, RUNE_PICKUP_RANGE, RuneSpot, RuneSpotKind,
    SHIELD_BARRIER_PCT, WATER_RUNE_HP, WATER_RUNE_MANA, bounty_rune_gold, rules, rune_ticks,
    runes_due, side_index,
};
use bota_proto::{
    EntityId, EventKind, Fixed, Inflictor, ItemId, RuneKind, RuneSet, Target, Team, Vec2,
};

use crate::game::{
    Entity, Event, EventVisibility, HeldRune, Modifier, Purpose, Rune, Transform, Visibility,
    World, wire_id,
};

impl World {
    /// Sets which kinds of rune the match spawns.
    pub(crate) fn stand_up_runes(&mut self, runes: RuneSet) {
        self.runes = runes;
    }

    /// Puts out the runes the game clock owes this tick, after taking away
    /// the river's untaken ones when the river turns over.
    pub(crate) fn tick_runes(&mut self) {
        let due = runes_due(self.clock(), self.schedule());
        if !due.any() || self.runes.is_empty() {
            return;
        }
        if due.river_turns() {
            self.clear_river_runes();
        }
        let spots = self.map.rune_spots;
        if due.opening_bounty && self.map.opening_runes {
            for at in 0..spots.len() {
                self.put_rune(RuneKind::Bounty, at);
            }
        }
        if due.jungle_bounty {
            for at in jungle_spots(spots) {
                self.put_rune(RuneKind::Bounty, at);
            }
        }
        if due.water {
            for at in river_spots(spots) {
                self.put_rune(RuneKind::Water, at);
            }
        }
        if due.power {
            self.put_power_rune();
        }
    }

    /// The rune spot of the map nearest a point: what a cheat's rune appears
    /// at. Of two as near, the earlier.
    pub(crate) fn nearest_rune_spot(&self, near: Vec2) -> Option<usize> {
        self.map
            .rune_spots
            .iter()
            .enumerate()
            .min_by_key(|(_, spot)| spot.at.distance_squared(near))
            .map(|(at, _)| at)
    }

    /// Takes away every water and power rune still lying anywhere: the next
    /// river spawn has come. Bounty runes stay until they are taken.
    fn clear_river_runes(&mut self) {
        let gone: Vec<Entity> = self
            .entities
            .iter()
            .filter(|&entity| {
                self.rune
                    .get(entity)
                    .is_some_and(|rune| rune.kind != RuneKind::Bounty)
            })
            .collect();
        for entity in gone {
            self.rune.remove(entity);
            self.despawn(entity);
        }
    }

    /// Puts a power rune of the next kind of the cycle at a river spot drawn
    /// at random, both from a stream of this spawn's own.
    fn put_power_rune(&mut self) {
        let river: Vec<usize> = river_spots(self.map.rune_spots).collect();
        if river.is_empty() || self.runes.and(POWER_RUNES).is_empty() {
            return;
        }
        let key = EntityId {
            idx: self.rune_draws,
            generation: 0,
        };
        self.rune_draws += 1;
        let mut stream = self.rng.for_unit(Purpose::Rune, key, 0);
        let mut cycle = self.rune_cycle;
        let Some(kind) = cycle.draw(self.runes, |n| stream.below(n)) else {
            return;
        };
        self.rune_cycle = cycle;
        let at = river[stream.below(river.len() as u32) as usize];
        self.put_rune(kind, at);
    }

    /// Stands a rune on a spot, if its kind is in the match's rules and the
    /// spot has room, and tells both sides.
    pub(crate) fn put_rune(&mut self, kind: RuneKind, spot: usize) {
        if !self.runes.has(kind) {
            return;
        }
        let at = self.map.rune_spots[spot].at;
        let lying = self
            .entities
            .iter()
            .filter(|&entity| {
                self.rune
                    .get(entity)
                    .is_some_and(|rune| usize::from(rune.spot) == spot)
            })
            .count();
        if lying >= bota_proto::MAX_RUNES_AT_SPOT {
            return;
        }
        let gold = if kind == RuneKind::Bounty {
            bounty_rune_gold(self.clock(), self.schedule())
        } else {
            0
        };
        let entity = self.spawn();
        self.transform.insert(
            entity,
            Transform {
                pos: at,
                facing: bota_proto::Angle::default(),
            },
        );
        // A row in the sight table is what makes it something sides can see.
        self.visibility.insert(entity, Visibility::NONE);
        self.rune.insert(
            entity,
            Rune {
                kind,
                spot: spot as u8,
                gold,
            },
        );
        self.events.push(Event {
            kind: EventKind::RuneSpawned { kind, at },
            visible_to: EventVisibility::Everyone,
            gold_hidden_from: None,
            source_told_to: EventVisibility::Everyone,
        });
    }

    /// Hands every rune within reach of a hero to the hero, the first hero of
    /// the tick's order to stand within reach of it. A courier, a creep or
    /// an illusion takes none. A hero with a Bottle in hand that holds no
    /// rune keeps it there; any other takes effect at once.
    pub(crate) fn take_runes(&mut self) {
        if self.rune.is_empty() {
            return;
        }
        // A copy: taking a rune may need the snapshot itself (the illusions'
        // do).
        let snapshot = self.take_entity_snapshot();
        let entities = snapshot.clone();
        self.recycle_entity_snapshot(snapshot);
        let heroes: Vec<(Entity, Vec2)> = entities
            .iter()
            .copied()
            .filter(|&entity| self.is_real_hero(entity) && self.alive(entity))
            .filter_map(|hero| Some((hero, self.pos(hero)?)))
            .collect();
        let reach = bota_core::rules::units(RUNE_PICKUP_RANGE);
        for rune in entities.iter().copied() {
            let (Some(held), Some(at)) = (self.rune.get(rune).copied(), self.pos(rune)) else {
                continue;
            };
            let Some(&(taker, _)) = heroes.iter().find(|(_, pos)| pos.within(at, reach)) else {
                continue;
            };
            self.rune.remove(rune);
            self.despawn(rune);
            self.take_rune(taker, held);
        }
    }

    /// A hero takes a rune: into its Bottle, or into effect.
    fn take_rune(&mut self, hero: Entity, rune: Rune) {
        let Rune { kind, gold, .. } = rune;
        if let Some(slot) = self.empty_bottle_of(hero)
            && let Some(stack) = self
                .inventory
                .get_mut(hero)
                .and_then(|bag| bag.slot_mut(slot))
                .and_then(Option::as_mut)
        {
            stack.touched = true;
            stack.rune = Some(HeldRune {
                kind,
                ticks_left: BOTTLE_RUNE_TICKS,
                gold,
                was_full: stack.charges >= BOTTLE_CHARGES,
            });
            let told = self.who_may_know(hero, self.team.get(hero).copied());
            self.events.push(Event {
                kind: EventKind::RuneBottled {
                    unit: wire_id(hero),
                    kind,
                },
                visible_to: told,
                gold_hidden_from: None,
                source_told_to: told,
            });
            return;
        }
        self.activate_rune(hero, kind, gold, false);
    }

    /// The first slot of the inventory proper holding a Bottle with no rune
    /// in it. A Bottle in the backpack stores nothing.
    fn empty_bottle_of(&self, hero: Entity) -> Option<usize> {
        let bag = self.inventory.get(hero)?;
        (0..bota_core::rules::INVENTORY_SLOTS).find(|&slot| {
            bag.slot(slot)
                .copied()
                .flatten()
                .is_some_and(|stack| stack.rune.is_none() && stores_runes(stack.id))
        })
    }

    /// A rune takes effect on a hero, and both sides that see it are told.
    pub(crate) fn activate_rune(&mut self, hero: Entity, kind: RuneKind, gold: i32, bottled: bool) {
        let told = self.who_may_know(hero, self.team.get(hero).copied());
        self.events.push(Event {
            kind: EventKind::RuneActivated {
                unit: wire_id(hero),
                kind,
                bottled,
                gold: (kind == RuneKind::Bounty).then_some(gold),
            },
            visible_to: told,
            gold_hidden_from: None,
            source_told_to: told,
        });
        match kind {
            RuneKind::Water => {
                let (hp, mana) = self.refill_pools(hero, WATER_RUNE_HP, WATER_RUNE_MANA);
                if hp > 0 || mana > 0 {
                    self.tell_refilled(hero, hp, mana, None);
                }
            }
            RuneKind::Bounty => self.pay_bounty_rune(hero, gold),
            RuneKind::Haste => self.wear_rune(hero, ModifierKind::RuneHaste, kind),
            RuneKind::AmplifyDamage => self.wear_rune(hero, ModifierKind::RuneAmplifyDamage, kind),
            RuneKind::Arcane => self.wear_rune(hero, ModifierKind::RuneArcane, kind),
            RuneKind::Regeneration => {
                self.wear_rune(hero, ModifierKind::RuneRegeneration { damped: 0 }, kind);
            }
            RuneKind::Invisibility => self.wear_rune(hero, ModifierKind::RuneInvisibility, kind),
            RuneKind::Shield => self.wear_shield(hero),
            RuneKind::Illusion => self.make_rune_illusions(hero),
        }
    }

    /// Puts a power rune's modifier on a hero for its time, refreshing the
    /// one it wore of the kind.
    fn wear_rune(&mut self, hero: Entity, modifier: ModifierKind, kind: RuneKind) {
        self.put_modifier(
            hero,
            Modifier::new(modifier, Some(hero), None, Some(rune_ticks(kind))),
        );
    }

    /// A Shield Rune: a barrier of half the hero's maximum health. A hero
    /// that wears one already has its time renewed and its barrier left
    /// as it is [LIQ Runes: "Reapplying the buff refreshes the duration, but
    /// does not refresh the damage barrier values"].
    fn wear_shield(&mut self, hero: Entity) {
        let ticks = rune_ticks(RuneKind::Shield);
        let held = self.modifiers.get(hero).and_then(|on_it| {
            on_it.active().find_map(|held| match held.kind {
                ModifierKind::RuneShield { left } => Some(left),
                _ => None,
            })
        });
        let left = held.unwrap_or_else(|| {
            let max = self
                .stats
                .get(hero)
                .map_or(Fixed::ZERO, |stats| stats.max_hp);
            (i64::from(max.to_int()) * i64::from(SHIELD_BARRIER_PCT) / 100) as i32
        });
        self.put_modifier(
            hero,
            Modifier::new(
                ModifierKind::RuneShield { left },
                Some(hero),
                None,
                Some(ticks),
            ),
        );
    }

    /// A Bounty Rune pays every seat of the hero's side, living or fallen,
    /// through its hero's gold income.
    fn pay_bounty_rune(&mut self, hero: Entity, gold: i32) {
        let Some(side) = self.team.get(hero).copied() else {
            return;
        };
        let seats: Vec<usize> = (0..self.seats.len())
            .filter(|&index| self.seats[index].team == side)
            .collect();
        self.pay_seats(seats, gold, None);
    }

    /// Uses a Bottle: it lets out the rune it holds on its user, or it is
    /// drunk by its user or by an allied hero in reach.
    pub(crate) fn use_bottle(
        &mut self,
        user: Entity,
        slot: usize,
        target: Target,
        drink: Drink,
    ) -> bool {
        let Some(stack) = self.held_in_bag(user, slot) else {
            return false;
        };
        if let Some(rune) = stack.rune {
            // A stored rune cannot be shared [LOC
            // DOTA_Tooltip_ability_item_bottle_Note0].
            if !matches!(target, Target::None) && target != Target::Unit(wire_id(user)) {
                return false;
            }
            self.let_out_rune(user, slot, rune);
            return true;
        }
        self.drink_from_bottle(user, target, drink)
    }

    /// The rune in a Bottle takes effect, and the Bottle is refilled: whole
    /// for a power rune, and for a Bounty or Water Rune whole if the Bottle
    /// was full when the rune was stored, two charges more otherwise [LOC
    /// DOTA_Tooltip_ability_item_bottle_Description and _Note1].
    fn let_out_rune(&mut self, user: Entity, slot: usize, rune: HeldRune) {
        if let Some(stack) = self
            .inventory
            .get_mut(user)
            .and_then(|bag| bag.slot_mut(slot))
            .and_then(Option::as_mut)
        {
            stack.rune = None;
            stack.touched = true;
            stack.charges = match rune.kind {
                RuneKind::Water | RuneKind::Bounty if !rune.was_full => stack
                    .charges
                    .saturating_add(BOTTLE_LESSER_RUNE_CHARGES)
                    .min(BOTTLE_CHARGES),
                _ => BOTTLE_CHARGES,
            };
        }
        self.activate_rune(user, rune.kind, rune.gold, true);
    }

    /// A drink: the mending of the Bottle's regeneration on its user, or on
    /// an allied hero within reach.
    fn drink_from_bottle(&mut self, user: Entity, target: Target, drink: Drink) -> bool {
        let on = match target {
            Target::None => user,
            Target::Unit(named) => match self.of_wire(named) {
                Some(on) => on,
                None => return false,
            },
            Target::Pos(..) => return false,
        };
        if !self.is_hero(on) || !self.alive(on) || self.team.get(on) != self.team.get(user) {
            return false;
        }
        let (Some(from), Some(at)) = (self.pos(user), self.pos(on)) else {
            return false;
        };
        if !from.within(at, bota_core::rules::units(drink.range)) {
            return false;
        }
        let per_tick = |total: i32| total * 100 / drink.ticks.max(1) as i32;
        let with = Inflictor::Item(ItemId::BOTTLE);
        self.put_modifier(
            on,
            Modifier::new(
                ModifierKind::BottleRegeneration {
                    hp_per_tick: per_tick(drink.health),
                    mana_per_tick: per_tick(drink.mana),
                },
                Some(user),
                Some(with),
                Some(drink.ticks.min(BOTTLE_TICKS)),
            ),
        );
        let missing = |max: Fixed, held: Fixed| (max - held).to_int().max(0);
        let hp = self
            .stats
            .get(on)
            .zip(self.health.get(on))
            .map_or(0, |(stats, pool)| missing(stats.max_hp, pool.hp));
        let mana = self
            .stats
            .get(on)
            .zip(self.mana.get(on))
            .map_or(0, |(stats, pool)| missing(stats.max_mana, pool.mana));
        self.tell_refilled(
            on,
            drink.health.min(hp),
            drink.mana.min(mana),
            Some((user, with)),
        );
        true
    }

    /// Tells who may know that a hero was mended by `hp` health and `mana`
    /// mana, by `by` when somebody's hand did it.
    pub(crate) fn tell_refilled(
        &mut self,
        on: Entity,
        hp: i32,
        mana: i32,
        by: Option<(Entity, Inflictor)>,
    ) {
        if hp <= 0 && mana <= 0 {
            return;
        }
        self.events.push(Event {
            kind: EventKind::Healed {
                source: by.map(|(source, _)| wire_id(source)),
                target: wire_id(on),
                amount: hp,
                mana,
                inflictor: by.map(|(_, with)| with),
            },
            visible_to: self.who_may_know(on, None),
            gold_hidden_from: None,
            source_told_to: self.who_may_name(by.map(|(source, _)| source)),
        });
    }

    /// Runs the Bottles' clocks one tick: the rune each holds runs down and
    /// takes effect by itself when its time is out (lost if its hero lies
    /// fallen), and a Bottle with no rune in it that its hero carries
    /// inside its fountain's reach is filled on every whole second.
    pub(crate) fn tick_bottles(&mut self) {
        let beat = self.clock().is_multiple_of(BOTTLE_REFILL_PERIOD_TICKS);
        for index in 0..self.seats.len() {
            let Some(hero) = self.seats[index].unit else {
                continue;
            };
            if self.alive(hero) {
                self.tick_bottles_of(hero, index, beat);
            } else if let Some(bag) = self.inventory.get_mut(hero) {
                for stack in bag.every_stack_mut() {
                    let Some(rune) = stack.rune.as_mut() else {
                        continue;
                    };
                    rune.ticks_left = rune.ticks_left.saturating_sub(1);
                    if rune.ticks_left == 0 {
                        stack.rune = None;
                    }
                }
            }
        }
    }

    fn tick_bottles_of(&mut self, hero: Entity, seat: usize, beat: bool) {
        let in_fountain = beat && self.in_own_fountain(hero, self.seats[seat].team);
        for slot in 0..bota_core::rules::INVENTORY_SLOTS {
            let Some(stack) = self.held_in_bag(hero, slot) else {
                continue;
            };
            if !stores_runes(stack.id) {
                continue;
            }
            let Some(rune) = stack.rune else {
                if in_fountain && stack.charges < BOTTLE_CHARGES {
                    self.set_charges(hero, slot, BOTTLE_CHARGES);
                }
                continue;
            };
            if rune.ticks_left <= 1 {
                self.let_out_rune(hero, slot, rune);
            } else if let Some(held) = self
                .inventory
                .get_mut(hero)
                .and_then(|bag| bag.slot_mut(slot))
                .and_then(Option::as_mut)
                .and_then(|stack| stack.rune.as_mut())
            {
                held.ticks_left = rune.ticks_left - 1;
            }
        }
    }

    fn set_charges(&mut self, hero: Entity, slot: usize, charges: u8) {
        if let Some(stack) = self
            .inventory
            .get_mut(hero)
            .and_then(|bag| bag.slot_mut(slot))
            .and_then(Option::as_mut)
        {
            stack.charges = charges;
        }
    }

    /// Whether a unit stands within its own side's fountain: as far as the
    /// Rejuvenation Aura reaches [LIQ Bottle/Changelogs: 7.38].
    fn in_own_fountain(&self, unit: Entity, side: Team) -> bool {
        let Some(fountain) = side_index(side).and_then(|index| self.map.fountains.get(index))
        else {
            return false;
        };
        self.pos(unit)
            .is_some_and(|pos| pos.within(*fountain, rules::units(rules::FOUNTAIN_HEAL_RADIUS)))
    }

    /// Runs the damping of the Regeneration Runes down, and ends the ones
    /// whose wearer is full in both pools: the rune is over "upon
    /// replenishing 100% max health and max mana" [LIQ Runes].
    pub(crate) fn tick_rune_regeneration(&mut self) {
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            let full = self
                .stats
                .get(entity)
                .zip(self.health.get(entity).zip(self.mana.get(entity)))
                .is_none_or(|(stats, (hp, mana))| {
                    hp.hp >= stats.max_hp && mana.mana >= stats.max_mana
                });
            let Some(on_it) = self.modifiers.get_mut(entity) else {
                continue;
            };
            on_it.0.retain_mut(|held| {
                let ModifierKind::RuneRegeneration { damped } = &mut held.kind else {
                    return true;
                };
                *damped = damped.saturating_sub(1);
                !full
            });
        }
        self.recycle_entity_snapshot(entities);
    }

    /// A blow from a hero damps the Regeneration Runes its target wears for
    /// [`bota_core::REGENERATION_DAMP_TICKS`]: damage of a player (a hero
    /// or its illusion), of any side, that took health off after every
    /// reduction; not one's own, not a burn that is not a player's.
    pub(crate) fn damp_rune_regeneration(&mut self, felt: &[crate::game::Landed]) {
        for blow in felt {
            let by_a_player = blow.amount > 0
                && blow.source != Some(blow.target)
                && blow.source.is_some_and(|source| self.is_hero(source));
            if !by_a_player {
                continue;
            }
            if let Some(on_it) = self.modifiers.get_mut(blow.target) {
                for held in on_it.0.iter_mut() {
                    if let ModifierKind::RuneRegeneration { damped } = &mut held.kind {
                        *damped = bota_core::REGENERATION_DAMP_TICKS as u16;
                    }
                }
            }
        }
    }

    /// Takes an Invisibility Rune off a unit: it attacked, cast or used an
    /// item [LOC DOTA_Tooltip_rune_invisibility_description].
    pub(crate) fn reveal(&mut self, unit: Entity) {
        if let Some(on_it) = self.modifiers.get_mut(unit) {
            on_it
                .0
                .retain(|held| held.kind != ModifierKind::RuneInvisibility);
        }
    }
}

/// The indices of the river spots among a map's.
fn river_spots(spots: &'static [RuneSpot]) -> impl Iterator<Item = usize> {
    spots
        .iter()
        .enumerate()
        .filter(|(_, spot)| spot.kind == RuneSpotKind::River)
        .map(|(at, _)| at)
}

/// The indices of the jungle spots among a map's.
fn jungle_spots(spots: &'static [RuneSpot]) -> impl Iterator<Item = usize> {
    spots
        .iter()
        .enumerate()
        .filter(|(_, spot)| matches!(spot.kind, RuneSpotKind::Jungle(_)))
        .map(|(at, _)| at)
}

/// Whether an item keeps runes.
pub(crate) fn stores_runes(id: ItemId) -> bool {
    bota_core::item_def(id).is_some_and(|def| def.stores_runes())
}
