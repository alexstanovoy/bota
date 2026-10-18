//! The names each side knows the entities it is told of by.
//!
//! The simulation names an entity by a slot of one allocator the whole world
//! shares and that slot's generation. Slots are reused and generations bumped
//! whenever anything anywhere comes or goes, so those numbers on the wire
//! would let a side count what the other side spawned out of its sight: a
//! missile, a raze, a ward. A side is told its own names instead. A name is
//! given the first time the side is told of an entity, counts up from zero,
//! is never given again and says nothing of the slot. Of the entities a side
//! is first told of on one tick, those the match was set up with (the map's
//! buildings, the heroes and couriers of the picks) are named first, in slot
//! order: that order is the setup's, the same in every match of it. The rest
//! are named in an order drawn from a hidden stream of the side's own, so not
//! even their order follows the slots. A view of everything keeps the
//! simulation's handles.

use bota_core::rules::TICKS_PER_SECOND;
use bota_proto::{Cheat, EntityId, EventKind, Order, SlotOrder, Target, Team, WorldView};
use rustc_hash::FxHashMap;

use crate::game::{MatchConfig, Stream, World};

/// The generation of every name. The simulation's handles never have
/// generation zero, so a name is never mistaken for one.
const NAME_GENERATION: u32 = 0;

/// What a name a side was never given stands for: generation zero, so no
/// entity, and every order naming it is refused as one naming the unseen.
const NOBODY: EntityId = EntityId {
    idx: u32::MAX,
    generation: 0,
};

/// Ticks a name outlives both its entity and the last time its side was told
/// of it: an event may still name a unit that died while its missile flew.
const KEPT_TICKS: u32 = 30 * TICKS_PER_SECOND;

/// How often names past their keep are let go.
const SWEEP_TICKS: u32 = 64;

/// The names of every side that has seats.
#[derive(Clone, Debug)]
pub struct Handles {
    sides: [Names; 2],
    /// Slots below this hold, in their first generation, what the match was
    /// set up with.
    founding: u32,
}

/// One side's names.
#[derive(Clone, Debug)]
struct Names {
    team: Team,
    /// The name of every simulation handle the side was told of, kept until
    /// its entity is gone and [`KEPT_TICKS`] passed without telling.
    given: FxHashMap<EntityId, Given>,
    /// The simulation handle behind every name in `given`.
    behind: FxHashMap<u32, EntityId>,
    next: u32,
    /// Orders the names first given on one tick.
    order: Stream,
    /// Handles to name on the tick being told; empty between calls.
    fresh: Vec<EntityId>,
}

#[derive(Clone, Copy, Debug)]
struct Given {
    name: u32,
    /// The last tick the side was told of it.
    told: u32,
}

impl Handles {
    /// No names given yet, for a match whose world is as it was set up,
    /// before its first tick.
    pub fn new(cfg: &MatchConfig, world: &World) -> Handles {
        let rng = cfg.rng();
        let side = |team| Names {
            team,
            given: FxHashMap::default(),
            behind: FxHashMap::default(),
            next: 0,
            order: rng.for_handles(team),
            fresh: Vec::new(),
        };
        let founding = world
            .entities
            .iter()
            .map(|entity| entity.index().0 + 1)
            .max();
        Handles {
            sides: [side(Team::Radiant), side(Team::Dire)],
            founding: founding.unwrap_or(0),
        }
    }

    fn side(&self, team: Team) -> Option<&Names> {
        self.sides.iter().find(|side| side.team == team)
    }

    fn side_mut(&mut self, team: Team) -> Option<&mut Names> {
        self.sides.iter_mut().find(|side| side.team == team)
    }

    /// Puts a side's names into a view through its eyes, sorted again by
    /// them. A view of everything is left as it is.
    pub fn tell_view(&mut self, world: &World, view: &mut WorldView) {
        let founding = self.founding;
        let Some(side) = view.viewer.and_then(|team| self.side_mut(team)) else {
            return;
        };
        if view.tick.is_multiple_of(SWEEP_TICKS) {
            side.sweep(world, view.tick);
        }
        side.rename(view.tick, founding, |visit| view_ids(view, visit));
        view.units.sort_unstable_by_key(|unit| unit.id);
        view.projectiles.sort_unstable_by_key(|shot| shot.id);
        view.loot.sort_unstable_by_key(|loot| loot.id);
        view.aggro.sort_unstable_by_key(|entry| entry.unit);
    }

    /// Puts a side's names into the events it is told of on a tick.
    pub fn tell_events(&mut self, team: Team, tick: u32, events: &mut [EventKind]) {
        let founding = self.founding;
        if let Some(side) = self.side_mut(team) {
            side.rename(tick, founding, |visit| {
                events.iter_mut().for_each(|e| event_ids(e, visit))
            });
        }
    }

    /// Puts a side's names into the orders it is told of on a tick.
    pub fn tell_orders(&mut self, team: Team, tick: u32, orders: &mut [SlotOrder]) {
        let founding = self.founding;
        if let Some(side) = self.side_mut(team) {
            side.rename(tick, founding, |visit| {
                for given in orders.iter_mut() {
                    given.unit.iter_mut().for_each(&mut *visit);
                    order_ids(&mut given.order, visit);
                }
            });
        }
    }

    /// An order a seat of a side gave, in the simulation's handles. A name
    /// the side was never given, or no longer holds, names nothing.
    pub fn heard(
        &self,
        team: Team,
        mut unit: Option<EntityId>,
        mut order: Order,
    ) -> (Option<EntityId>, Order) {
        let Some(side) = self.side(team) else {
            return (unit, order);
        };
        let mut behind = |id: &mut EntityId| *id = side.behind(*id);
        unit.iter_mut().for_each(&mut behind);
        order_ids(&mut order, &mut behind);
        (unit, order)
    }

    /// The name a side was given for an entity, if it was told of it.
    pub fn name_of(&self, team: Team, entity: EntityId) -> Option<EntityId> {
        let given = self.side(team)?.given.get(&entity)?;
        Some(name(given.name))
    }
}

impl Names {
    /// Names every handle `visit` walks, giving a name to each the side is
    /// told of for the first time; see the module documentation for their
    /// order. Slots below `founding` hold, in generation one, what the match
    /// was set up with.
    fn rename(
        &mut self,
        tick: u32,
        founding: u32,
        mut visit: impl FnMut(&mut dyn FnMut(&mut EntityId)),
    ) {
        visit(&mut |id: &mut EntityId| {
            if !self.given.contains_key(id) {
                self.fresh.push(*id);
            }
        });
        let set_up = |id: &EntityId| id.idx < founding && id.generation == 1;
        self.fresh.sort_unstable_by_key(|id| (!set_up(id), *id));
        self.fresh.dedup();
        let since = self.fresh.partition_point(set_up);
        let later = &mut self.fresh[since..];
        for last in (1..later.len()).rev() {
            let pick = self.order.below(last as u32 + 1) as usize;
            later.swap(last, pick);
        }
        for entity in self.fresh.drain(..) {
            let fresh = self.next;
            self.next = fresh
                .checked_add(1)
                .expect("a match tells a side of fewer than 2^32 entities");
            self.given.insert(
                entity,
                Given {
                    name: fresh,
                    told: tick,
                },
            );
            self.behind.insert(fresh, entity);
        }
        visit(&mut |id: &mut EntityId| {
            let given = self
                .given
                .get_mut(id)
                .expect("every handle was named above");
            given.told = tick;
            *id = name(given.name);
        });
    }

    /// The simulation handle behind a name, or [`NOBODY`].
    fn behind(&self, id: EntityId) -> EntityId {
        if id.generation != NAME_GENERATION {
            return NOBODY;
        }
        self.behind.get(&id.idx).copied().unwrap_or(NOBODY)
    }

    /// Lets go of the names of entities that are gone and that the side was
    /// not told of for [`KEPT_TICKS`].
    fn sweep(&mut self, world: &World, tick: u32) {
        let behind = &mut self.behind;
        self.given.retain(|entity, given| {
            let keep =
                world.of_wire(*entity).is_some() || tick.saturating_sub(given.told) < KEPT_TICKS;
            if !keep {
                behind.remove(&given.name);
            }
            keep
        });
    }
}

/// A name on the wire.
const fn name(name: u32) -> EntityId {
    EntityId {
        idx: name,
        generation: NAME_GENERATION,
    }
}

/// Walks every entity handle a view carries.
fn view_ids(view: &mut WorldView, visit: &mut dyn FnMut(&mut EntityId)) {
    for unit in &mut view.units {
        visit(&mut unit.id);
    }
    for shot in &mut view.projectiles {
        visit(&mut shot.id);
    }
    for player in &mut view.players {
        player.unit.iter_mut().for_each(&mut *visit);
    }
    for loot in &mut view.loot {
        visit(&mut loot.id);
    }
    for entry in &mut view.aggro {
        visit(&mut entry.unit);
        entry.target.iter_mut().for_each(&mut *visit);
        if let Some(swing) = &mut entry.swing {
            visit(&mut swing.target);
        }
    }
    for teleport in &mut view.teleports {
        visit(&mut teleport.building);
    }
}

/// Walks every entity handle an event carries.
fn event_ids(event: &mut EventKind, visit: &mut dyn FnMut(&mut EntityId)) {
    match event {
        EventKind::Damaged { source, target, .. }
        | EventKind::Missed { source, target }
        | EventKind::Healed { source, target, .. } => {
            source.iter_mut().for_each(&mut *visit);
            visit(target);
        }
        EventKind::Died { unit, killer, .. } => {
            visit(unit);
            killer.iter_mut().for_each(visit);
        }
        EventKind::AbilityCast { caster: unit, .. }
        | EventKind::LevelUp { unit, .. }
        | EventKind::StructureDestroyed { unit, .. } => visit(unit),
        EventKind::ItemBought { .. } => {}
    }
}

/// Walks the entity an order names, if it names one.
fn order_ids(order: &mut Order, visit: &mut dyn FnMut(&mut EntityId)) {
    let target = match order {
        Order::Move { target }
        | Order::Attack { target }
        | Order::Cast { target, .. }
        | Order::Use { target, .. }
        | Order::Put { target, .. }
        | Order::Take { target }
        | Order::Cheat {
            cheat: Cheat::ApplyModifier { target, .. } | Cheat::ClearModifiers { target },
        } => target,
        Order::Cheat {
            cheat: Cheat::Gold { .. } | Cheat::Levels { .. } | Cheat::Refresh | Cheat::Item { .. },
        }
        | Order::Buy { .. }
        | Order::Sell { .. }
        | Order::Swap { .. }
        | Order::Learn { .. }
        | Order::LearnTalent { .. }
        | Order::LearnAttributeBonus
        | Order::Glyph => return,
    };
    if let Target::Unit(id) = target {
        visit(id);
    }
}
