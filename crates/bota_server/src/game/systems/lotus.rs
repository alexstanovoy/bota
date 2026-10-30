//! The Lotus Pools: what they grow and when, the row of plucks a hero makes
//! standing in one, and a lotus eaten.

use bota_core::{PLUCK_TICKS, POOL_GROWTH, POOL_MAX_LOTUSES, grown_at, lotus_item, rounded_up};
use bota_proto::{EventKind, Inflictor, LotusTier, Target, Team, UnitKind};

use crate::game::{Entity, Event, EventVisibility, ItemStack, Purpose, World, wire_id};

impl World {
    /// Grows each pool's lotus on the minute marks of the game clock, rounds
    /// what it holds up into the tier it grows from the time of the game,
    /// and runs the plucking in it. The three-minute clock is the game's, so
    /// a pool that is full waits for the next mark.
    pub(crate) fn tick_pools(&mut self) {
        if self.pool.is_empty() {
            return;
        }
        let clock = self.clock();
        let grows = POOL_GROWTH.firing_at(clock, self.schedule()).is_some();
        let pools: Vec<Entity> = self
            .entities
            .iter()
            .filter(|&entity| self.pool.get(entity).is_some())
            .collect();
        for entity in pools {
            let Some(mut pool) = self.pool.get(entity).copied() else {
                continue;
            };
            let tier = grown_at(clock, self.schedule());
            if pool.tier < tier {
                pool.lotuses = rounded_up(pool.lotuses, pool.tier, tier);
                pool.tier = tier;
            }
            if grows && pool.lotuses < POOL_MAX_LOTUSES {
                pool.lotuses += 1;
                self.events.push(Event {
                    kind: EventKind::LotusGrown {
                        pool: wire_id(entity),
                        tier: pool.tier,
                    },
                    visible_to: EventVisibility::Everyone,
                    gold_hidden_from: None,
                    source_told_to: EventVisibility::Everyone,
                });
            }
            self.pool.insert(entity, pool);
            self.pluck_in(entity);
        }
    }

    /// Whether a hero could pluck a pool's lotus now: a real hero that
    /// stands and has room for it in its inventory or backpack.
    fn may_pluck(&self, hero: Entity, tier: LotusTier) -> bool {
        self.is_real_hero(hero)
            && self.alive(hero)
            && self
                .inventory
                .get(hero)
                .is_some_and(|bag| bag.receiving_slot(&self.lotus_stack(hero, tier)).is_some())
    }

    /// One lotus of a tier as a hero would be handed it.
    fn lotus_stack(&self, hero: Entity, tier: LotusTier) -> ItemStack {
        let owner = self
            .owner
            .get(hero)
            .copied()
            .unwrap_or(bota_proto::SlotId(0));
        ItemStack::granted(lotus_item(tier), owner, self.tick, 1)
    }

    /// Runs a pool's plucking one tick: the heroes of one side alone in its
    /// radius, with room for a lotus, run its countdown; an enemy hero that
    /// could pluck as well runs it back; and a pool holding nothing is not
    /// plucked. A countdown that comes to its end hands one of those heroes
    /// the lotus, and the next of the row takes less time.
    fn pluck_in(&mut self, pool_at: Entity) {
        let Some(mut pool) = self.pool.get(pool_at).copied() else {
            return;
        };
        let Some(mut zone) = self.zone.get(pool_at).copied() else {
            return;
        };
        if pool.lotuses == 0 {
            zone.side = None;
            zone.progress = 0;
            pool.streak = 0;
            zone.ticks = PLUCK_TICKS[0];
            self.zone.insert(pool_at, zone);
            self.pool.insert(pool_at, pool);
            return;
        }
        let tier = pool.tier;
        let done = self.run_zone_of(pool_at, &|world, hero| world.may_pluck(hero, tier));
        let Some(mut zone) = self.zone.get(pool_at).copied() else {
            return;
        };
        match done {
            Some(side) => {
                self.hand_lotus(pool_at, &mut pool, side);
                pool.streak = (pool.streak + 1).min(PLUCK_TICKS.len() as u8 - 1);
            }
            // Nobody who could pluck stands there: the row is over, though the
            // countdown only rewinds.
            None if !self.pluckers_in(pool_at, tier) => pool.streak = 0,
            None => {}
        }
        zone.ticks = PLUCK_TICKS[usize::from(pool.streak)];
        self.zone.insert(pool_at, zone);
        self.pool.insert(pool_at, pool);
    }

    /// Whether a hero that could pluck a pool's lotus stands in its radius.
    fn pluckers_in(&self, pool_at: Entity, tier: LotusTier) -> bool {
        let (Some(at), Some(zone)) = (self.pos(pool_at), self.zone.get(pool_at)) else {
            return false;
        };
        self.entities.iter().any(|hero| {
            self.may_pluck(hero, tier)
                && self
                    .pos(hero)
                    .is_some_and(|pos| pos.within(at, zone.radius))
        })
    }

    /// Hands the lotus a pool let go of to one of `side`'s heroes in its
    /// radius that has room for it, drawn at random.
    fn hand_lotus(&mut self, pool_at: Entity, pool: &mut crate::game::LotusPool, side: Team) {
        let (Some(at), Some(zone)) = (self.pos(pool_at), self.zone.get(pool_at).copied()) else {
            return;
        };
        let tier = pool.tier;
        let candidates: Vec<Entity> = self
            .entities
            .iter()
            .filter(|&hero| {
                self.team.get(hero) == Some(&side)
                    && self.kind.get(hero) == Some(&UnitKind::Hero)
                    && self.may_pluck(hero, tier)
                    && self
                        .pos(hero)
                        .is_some_and(|pos| pos.within(at, zone.radius))
            })
            .collect();
        if candidates.is_empty() {
            return;
        }
        let mut draw = self
            .rng
            .for_unit(Purpose::Lotus, wire_id(pool_at), pool.plucked as u8);
        let hero = candidates[draw.below(candidates.len() as u32) as usize];
        let stack = self.lotus_stack(hero, tier);
        if !self
            .inventory
            .get_mut(hero)
            .is_some_and(|bag| bag.receive(stack))
        {
            return;
        }
        pool.lotuses -= 1;
        pool.plucked += 1;
        let seen_by = self.who_may_name(Some(hero));
        self.events.push(Event {
            kind: EventKind::LotusPlucked {
                pool: wire_id(pool_at),
                unit: Some(wire_id(hero)),
                tier,
            },
            visible_to: EventVisibility::Everyone,
            gold_hidden_from: None,
            source_told_to: seen_by,
        });
    }

    /// Eats a lotus, Cheese or the like: the user, or an allied hero in reach
    /// when the item may be aimed at one, is mended by `restore` health and
    /// mana at once. Any hero eats one: an illusion included [LIQ Healing
    /// Lotus].
    pub(crate) fn eat_item(
        &mut self,
        user: Entity,
        target: Target,
        restore: (i32, i32),
        with: Inflictor,
        reach: i32,
    ) -> bool {
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
        if !from.within(at, bota_core::rules::units(reach)) {
            return false;
        }
        let (hp, mana) = self.refill_pools(on, restore.0, restore.1);
        self.tell_refilled(on, hp, mana, Some((user, with)));
        true
    }

    /// What every pool holds, for a view.
    pub(crate) fn project_pools(&self) -> Vec<bota_proto::PoolView> {
        let clock = self.clock();
        let mut pools: Vec<bota_proto::PoolView> = self
            .entities
            .iter()
            .filter_map(|entity| {
                let pool = self.pool.get(entity)?;
                Some(bota_proto::PoolView {
                    id: wire_id(entity),
                    tier: pool.tier,
                    lotuses: pool.lotuses,
                    next_in: (pool.lotuses < POOL_MAX_LOTUSES)
                        .then(|| bota_core::next_growth_in(clock, self.schedule())),
                })
            })
            .collect();
        pools.sort_unstable_by_key(|pool: &bota_proto::PoolView| pool.id);
        pools
    }
}
