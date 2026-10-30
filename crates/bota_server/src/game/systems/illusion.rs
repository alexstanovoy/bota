//! What making an illusion of a hero, or handing a unit to another side,
//! does to the aims at it.

use bota_core::{
    GoldRange, ILLUSION_DEALT_BP, ILLUSION_MELEE_TAKEN_BP, ILLUSION_RANGED_TAKEN_BP,
    ILLUSION_RUNE_COUNT, ILLUSION_RUNE_TICKS, ModifierKind, effect_def, point_ahead,
};
use bota_proto::{Angle, Fixed, Team};

use crate::game::{Bounty, Entity, Illusion, Modifier, PendingCast, UnitOrder, World, wire_id};

impl World {
    /// An Illusion Rune: two illusions of the hero stand beside it, dealing a
    /// share of its damage and taking more of what is dealt to them, for
    /// 75 s; the hero is stopped and loses its dispellable buffs, and the
    /// illusions a rune of the hero made before are gone [LIQ Runes].
    ///
    /// The illusions stand round the hero, evenly, one body-length-and-a-bit
    /// off for each illusion and the hero: Dota's formation puts the hero in
    /// any of its places at random, which only changes who stands where.
    pub(crate) fn make_rune_illusions(&mut self, hero: Entity) {
        let (Some(team), Some(slot), Some(kind), Some(from)) = (
            self.team.get(hero).copied(),
            self.owner.get(hero).copied(),
            self.hero.get(hero).copied(),
            self.transform.get(hero).copied(),
        ) else {
            return;
        };
        let old: Vec<Entity> = self
            .entities
            .iter()
            .filter(|&other| self.illusion.get(other).is_some_and(|made| made.of == hero))
            .collect();
        for other in old {
            self.despawn(other);
        }
        self.dispel_basic(hero);
        self.set_order(hero, UnitOrder::Stand);
        let ranged = self
            .stats
            .get(hero)
            .is_some_and(|stats| stats.projectile_speed.is_some());
        let taken_bp = if ranged {
            ILLUSION_RANGED_TAKEN_BP
        } else {
            ILLUSION_MELEE_TAKEN_BP
        };
        let reach = self
            .hull
            .get(hero)
            .map_or(Fixed::ZERO, |hull| hull.collision)
            * Fixed::from_int(ILLUSION_RUNE_COUNT as i32 + 2);
        let level = self
            .level
            .get(hero)
            .copied()
            .unwrap_or(crate::game::Level(1));
        for nth in 0..ILLUSION_RUNE_COUNT {
            let facing = Angle {
                brads: from
                    .facing
                    .brads
                    .wrapping_add(((nth + 1) * 65_536 / (ILLUSION_RUNE_COUNT + 1)) as u16),
            };
            let mut at = point_ahead(from.pos, facing, reach);
            if !self.clearance.stands_clear(at) {
                at = from.pos;
            }
            let illusion = self.spawn_hero(team, at, slot, kind);
            let book = self.abilities.get(hero).cloned();
            let mut bag = self.inventory.get(hero).cloned();
            for stack in bag.iter_mut().flat_map(|bag| bag.every_stack_mut()) {
                stack.rune = None;
            }
            self.level.insert(illusion, level);
            if let Some(book) = book {
                self.abilities.insert(illusion, book);
            }
            if let Some(bag) = bag {
                self.inventory.insert(illusion, bag);
            }
            // What a hero keeps through its deaths (its souls, its heap)
            // shows on its illusions; nothing else of what it wears does.
            let kept: Vec<Modifier> = self
                .modifiers
                .get(hero)
                .into_iter()
                .flat_map(|on_it| on_it.active().copied())
                .filter(|held| {
                    effect_def(held.kind.effect_id()).is_some_and(|def| def.kept_through_death)
                })
                .collect();
            for held in kept {
                self.put_modifier(illusion, held);
            }
            if let Some(transform) = self.transform.get_mut(illusion) {
                transform.facing = from.facing;
            }
            self.set_order(illusion, UnitOrder::Stand);
            self.illusion.insert(
                illusion,
                Illusion {
                    of: hero,
                    made: self.tick,
                    dealt_bp: ILLUSION_DEALT_BP,
                    taken_bp,
                },
            );
            let price = 2 * i32::from(level.0);
            self.bounty.insert(
                illusion,
                Bounty {
                    gold: GoldRange::exactly(price),
                    bonus: 0,
                    team: 0,
                    xp: price,
                },
            );
            self.put_modifier(
                illusion,
                Modifier::new(
                    ModifierKind::Illusion,
                    Some(hero),
                    None,
                    Some(ILLUSION_RUNE_TICKS),
                ),
            );
        }
        self.call_off_aims_at(hero);
        self.settle();
    }

    /// Takes the buffs a basic dispel takes off a unit.
    pub(crate) fn dispel_basic(&mut self, unit: Entity) {
        if let Some(on_it) = self.modifiers.get_mut(unit) {
            on_it.0.retain(|held| {
                !effect_def(held.kind.effect_id()).is_some_and(|def| def.dispellable)
            });
        }
    }

    /// Calls off what the other side aims at a hero an illusion of it was
    /// just made of, as Dota's illusion makers do: attack and follow orders
    /// and casts waiting on it, whom its units are set on, and the queued
    /// orders that name it. None of them follows the hero under the name
    /// the other side is told for it afresh.
    pub fn call_off_aims_at(&mut self, hero: Entity) {
        let Some(&side) = self.team.get(hero) else {
            return;
        };
        self.call_off_aims(hero, |team| team != side);
    }

    /// Calls off every aim at a unit of the units whose side `of` takes: the
    /// orders and casts waiting on it, whom they are set on, and the queued
    /// orders that name it.
    pub(crate) fn call_off_aims(&mut self, mark: Entity, of: impl Fn(Team) -> bool) {
        let named = wire_id(mark);
        let entities = self.take_entity_snapshot();
        for unit in entities.iter().copied() {
            if self.team.get(unit).is_none_or(|team| !of(*team)) {
                continue;
            }
            if let Some(orders) = self.orders.get_mut(unit) {
                if matches!(
                    orders.current,
                    UnitOrder::Attack { target, .. } | UnitOrder::Follow { target, .. }
                        if target == mark
                ) {
                    orders.current = UnitOrder::Idle;
                }
                let aimed = |cast: &PendingCast| cast.target() == bota_proto::Target::Unit(named);
                if orders.pending.as_ref().is_some_and(aimed) {
                    orders.pending = None;
                }
            }
            if self.target_of(unit) == Some(mark) {
                self.target.remove(unit);
            }
            for queue in [self.queue.get_mut(unit), self.waiting.get_mut(unit)]
                .into_iter()
                .flatten()
            {
                queue.drop_aimed_at(named);
            }
        }
        self.recycle_entity_snapshot(entities);
    }
}
