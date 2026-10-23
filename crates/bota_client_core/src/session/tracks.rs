//! Bounded memory for every unit a client has seen, and the facts events
//! tell of them.

use bota_proto::{
    AbilityId, AggroView, DamageKind, EntityId, EventKind, SlotId, StatusFlags, Talents, Team,
    UnitKind, UnitView, Vec2, WorldView,
};

use crate::limits::MAX_TRACKED_ENTITIES;

/// Exact displacement observed between two visible snapshots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VelocityEstimate {
    /// Position difference without division or rounding.
    pub delta: Vec2,
    /// Positive number of game ticks covered by `delta`.
    pub elapsed_ticks: u32,
}

/// One observed damage relation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageObservation {
    pub tick: u32,
    /// Health actually removed.
    pub amount: i32,
    /// Damage reduction category.
    pub kind: DamageKind,
    /// Other entity in the damage relation, when one was reported.
    pub counterpart: Option<EntityId>,
}

/// One manual health-restoration report; an over-time promise may be interrupted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HealObservation {
    pub tick: u32,
    /// Positive reported health amount, not confirmed tick healing.
    pub amount: i32,
    /// Other entity in the healing relation, when one was reported.
    pub counterpart: Option<EntityId>,
}

/// One manual mana-restoration report; an over-time promise may be interrupted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManaRestorationObservation {
    /// Tick reporting the manual restoration, not its completion time.
    pub tick: u32,
    /// Positive reported mana amount, not confirmed tick regeneration.
    pub amount: i32,
    /// Other entity in the report, when supplied; an opaque bookkeeping key.
    pub counterpart: Option<EntityId>,
}

/// One observed death.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeathObservation {
    pub tick: u32,
    /// Entity credited with the kill, when one was reported.
    pub killer: Option<EntityId>,
    pub denied: bool,
    /// Gold paid directly for the death, as this side was told.
    pub gold: i32,
}

/// One observed completed ability cast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbilityCastObservation {
    pub tick: u32,
    pub ability: AbilityId,
}

/// A physical non-critical damage event that might be an attack landing.
///
/// The wire does not identify attack damage. This value is only a candidate and
/// must not be used as proof of an attack phase. A same-tick reported ability
/// cast is excluded, but an unreported physical ability can remain ambiguous.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PossibleAttackLanded {
    pub tick: u32,
    pub target: EntityId,
}

/// Bounded memory for one entity, keyed by its full generational handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityTrack {
    pub id: EntityId,
    /// Most recent visible unit data.
    pub unit: UnitView,
    /// Tick from the preceding visible observation.
    pub previous_seen_tick: u32,
    /// Tick of the most recent visible observation.
    pub last_seen_tick: u32,
    /// Exact displacement and elapsed ticks, absent before a second observation.
    pub velocity: Option<VelocityEstimate>,
    /// Health change since the preceding visible observation.
    pub hp_delta: i64,
    /// Mana change since the preceding visible observation.
    pub mana_delta: i64,
    /// Whether the entity is present in the current snapshot.
    pub visible: bool,
    /// Latest damage dealt by this entity.
    pub last_damage_dealt: Option<DamageObservation>,
    /// Latest damage taken by this entity.
    pub last_damage_taken: Option<DamageObservation>,
    /// Latest positive manual health-restoration report from this entity.
    pub last_heal_dealt: Option<HealObservation>,
    /// Latest positive manual health-restoration report targeting this entity.
    pub last_heal_received: Option<HealObservation>,
    /// Latest positive manual mana-restoration report from this entity.
    pub last_mana_restoration_dealt: Option<ManaRestorationObservation>,
    /// Latest positive manual mana-restoration report targeting this entity.
    pub last_mana_restoration_received: Option<ManaRestorationObservation>,
    /// Latest death reported for this entity.
    pub last_death: Option<DeathObservation>,
    /// Latest completed ability cast reported for this entity.
    pub last_ability_cast: Option<AbilityCastObservation>,
    /// Latest damage event that can only be treated as a possible landed attack.
    pub last_possible_attack_landed: Option<PossibleAttackLanded>,
    /// Whom it fights as the current snapshot tells it; absent while unseen or quiet.
    pub fight: Option<AggroView>,
    /// Units the current snapshot tells set on it: of the viewing side, then of any other.
    pub attackers: [u8; 2],
    /// A hero's learned talents as last told while it was seen; empty for anything else.
    /// Talents are never unlearned, so the last told set stands while the hero is unseen.
    pub talents: Talents,
    /// The tick of the snapshot it was first seen in the cast point it is in
    /// now; absent while it is not seen casting.
    pub casting_since: Option<u32>,
}

/// One event retained with its game tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedEvent {
    pub tick: u32,
    pub kind: EventKind,
}

/// The seat whose own bodies are replaced when they respawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Owner {
    pub slot: SlotId,
    pub team: Team,
}

impl Owner {
    pub(crate) fn owns_courier(self, unit: &UnitView) -> bool {
        unit.kind == UnitKind::Courier && unit.team == self.team && unit.owner == Some(self.slot)
    }
}

/// Every entity seen lately, sorted by full handle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Tracks {
    entities: Vec<EntityTrack>,
    /// Indices of the snapshot's units not tracked yet; empty between snapshots.
    fresh: Vec<usize>,
}

impl Tracks {
    #[inline]
    pub(crate) fn all(&self) -> &[EntityTrack] {
        &self.entities
    }

    #[inline]
    pub(crate) fn get(&self, id: EntityId) -> Option<&EntityTrack> {
        self.entities
            .binary_search_by_key(&id, |entity| entity.id)
            .ok()
            .map(|index| &self.entities[index])
    }

    fn get_mut(&mut self, id: EntityId) -> Option<&mut EntityTrack> {
        let index = self
            .entities
            .binary_search_by_key(&id, |entity| entity.id)
            .ok()?;
        Some(&mut self.entities[index])
    }

    /// Folds one validated snapshot in.
    pub(crate) fn fold_snapshot(
        &mut self,
        view: &WorldView,
        owner: Option<Owner>,
        viewer: Option<Team>,
        forget_after: u32,
    ) {
        if let Some(owner) = owner {
            self.remove_replaced_own_bodies(view, owner);
        }
        self.entities
            .retain(|entity| view.tick.saturating_sub(entity.last_seen_tick) <= forget_after);
        self.make_entity_room(view);
        for entity in &mut self.entities {
            entity.visible = false;
        }
        let mut fresh = std::mem::take(&mut self.fresh);
        for (index, unit, tracked) in pair_tracked(&mut self.entities, &view.units) {
            match tracked {
                Some(entity) => update_entity(entity, view.tick, unit),
                None => fresh.push(index),
            }
        }
        for &index in &fresh {
            let unit = &view.units[index];
            let at = self
                .entities
                .binary_search_by_key(&unit.id, |entity| entity.id)
                .expect_err("new entity cannot already exist");
            self.entities.insert(at, new_entity(view.tick, unit));
        }
        fresh.clear();
        self.fresh = fresh;
        assert!(self.entities.len() <= MAX_TRACKED_ENTITIES);
        for entity in self.entities.iter_mut().filter(|entity| !entity.visible) {
            entity.casting_since = None;
        }
        self.update_fights(view, viewer);
        self.update_talents(view);
    }

    /// Hands each seen hero the talents the snapshot tells of its seat.
    fn update_talents(&mut self, view: &WorldView) {
        for told in &view.talents {
            let Some(hero) = view.player(told.slot).and_then(|player| player.unit) else {
                continue;
            };
            if let Ok(index) = self
                .entities
                .binary_search_by_key(&hero, |entity| entity.id)
                && self.entities[index].visible
            {
                self.entities[index].talents = told.learned;
            }
        }
    }

    /// Hands each entity what the snapshot tells of its fight and counts who is set on it.
    fn update_fights(&mut self, view: &WorldView, viewer: Option<Team>) {
        for entity in &mut self.entities {
            entity.fight = None;
            entity.attackers = [0; 2];
        }
        let index_of = |entities: &[EntityTrack], id| {
            entities
                .binary_search_by_key(&id, |entity: &EntityTrack| entity.id)
                .expect("validated aggro names visible, hence tracked, units")
        };
        for entry in &view.aggro {
            let attacker = index_of(&self.entities, entry.unit);
            self.entities[attacker].fight = Some(*entry);
            let side = usize::from(Some(self.entities[attacker].unit.team) != viewer);
            if let Some(target) = entry.target {
                let target = index_of(&self.entities, target);
                let count = &mut self.entities[target].attackers[side];
                *count = count.saturating_add(1);
            }
        }
    }

    fn remove_replaced_own_bodies(&mut self, view: &WorldView, owner: Owner) {
        let own_hero = view.player(owner.slot).and_then(|player| player.unit);
        let own_courier = view
            .units
            .iter()
            .find(|unit| owner.owns_courier(unit))
            .map(|unit| unit.id);
        self.entities.retain(|track| {
            if track.unit.team != owner.team || track.unit.owner != Some(owner.slot) {
                return true;
            }
            match track.unit.kind {
                UnitKind::Hero => own_hero.is_none_or(|id| id == track.id),
                UnitKind::Courier => own_courier.is_none_or(|id| id == track.id),
                _ => true,
            }
        });
    }

    fn make_entity_room(&mut self, view: &WorldView) {
        let new_count = pair_tracked(&self.entities, &view.units)
            .filter(|(_, _, tracked)| tracked.is_none())
            .count();
        let mut required = self
            .entities
            .len()
            .saturating_add(new_count)
            .saturating_sub(MAX_TRACKED_ENTITIES);
        while required > 0 {
            let oldest = self
                .entities
                .iter()
                .filter(|track| view.unit(track.id).is_none())
                .map(|track| track.last_seen_tick)
                .min()
                .expect("validated visible-unit bound guarantees an invisible eviction cohort");
            let before = self.entities.len();
            self.entities
                .retain(|track| track.last_seen_tick != oldest || view.unit(track.id).is_some());
            let evicted = before - self.entities.len();
            assert!(evicted > 0);
            required = required.saturating_sub(evicted);
        }
    }

    /// Folds one tick's validated events in, casts first.
    pub(crate) fn fold_events(&mut self, tick: u32, events: &[EventKind]) {
        for event in events {
            if let EventKind::AbilityCast { caster, ability } = *event
                && let Some(entity) = self.get_mut(caster)
            {
                entity.last_ability_cast = Some(AbilityCastObservation { tick, ability });
            }
        }
        for event in events {
            self.update_from_event(tick, event);
        }
    }

    fn update_from_event(&mut self, tick: u32, event: &EventKind) {
        match *event {
            EventKind::Damaged {
                source,
                target,
                amount,
                kind,
            } => self.update_damage(tick, source, target, amount, kind),
            EventKind::Healed {
                source,
                target,
                amount,
                mana,
            } => {
                self.update_restoration(tick, source, target, amount, mana);
            }
            EventKind::Died {
                unit,
                killer,
                denied,
                gold,
            } => {
                if let Some(entity) = self.get_mut(unit) {
                    entity.last_death = Some(DeathObservation {
                        tick,
                        killer,
                        denied,
                        gold,
                    });
                    entity.visible = false;
                }
            }
            // A miss is not damage and carries no health, mana or death facts; casts are
            // recorded before any same-tick damage. A teleport's end names no entity.
            EventKind::Missed { .. }
            | EventKind::AbilityCast { .. }
            | EventKind::LevelUp { .. }
            | EventKind::ItemBought { .. }
            | EventKind::StructureDestroyed { .. }
            | EventKind::Teleported { .. } => {}
        }
    }

    fn update_damage(
        &mut self,
        tick: u32,
        source: Option<EntityId>,
        target: EntityId,
        amount: i32,
        kind: DamageKind,
    ) {
        if let Some(source) = source
            && let Some(entity) = self.get_mut(source)
        {
            entity.last_damage_dealt = Some(DamageObservation {
                tick,
                amount,
                kind,
                counterpart: Some(target),
            });
            if kind == DamageKind::Physical
                && entity
                    .last_ability_cast
                    .is_none_or(|cast| cast.tick != tick)
            {
                entity.last_possible_attack_landed = Some(PossibleAttackLanded { tick, target });
            }
        }
        if let Some(entity) = self.get_mut(target) {
            entity.last_damage_taken = Some(DamageObservation {
                tick,
                amount,
                kind,
                counterpart: source,
            });
        }
    }

    fn update_restoration(
        &mut self,
        tick: u32,
        source: Option<EntityId>,
        target: EntityId,
        amount: i32,
        mana: i32,
    ) {
        let heal = |counterpart| {
            (amount > 0).then_some(HealObservation {
                tick,
                amount,
                counterpart,
            })
        };
        let restore = |counterpart| {
            (mana > 0).then_some(ManaRestorationObservation {
                tick,
                amount: mana,
                counterpart,
            })
        };
        if let Some(e) = source.and_then(|id| self.get_mut(id)) {
            e.last_heal_dealt = heal(Some(target)).or(e.last_heal_dealt);
            e.last_mana_restoration_dealt = restore(Some(target)).or(e.last_mana_restoration_dealt);
        }
        if let Some(e) = self.get_mut(target) {
            e.last_heal_received = heal(source).or(e.last_heal_received);
            e.last_mana_restoration_received = restore(source).or(e.last_mana_restoration_received);
        }
    }
}

/// Each snapshot unit with its index and tracked entity, if any, in one merge
/// pass: both lists are sorted by id (units strictly, as validated).
pub(crate) fn pair_tracked<T: std::ops::Deref<Target = EntityTrack>>(
    entities: impl IntoIterator<Item = T>,
    units: &[UnitView],
) -> impl Iterator<Item = (usize, &UnitView, Option<T>)> {
    let mut entities = entities.into_iter().peekable();
    units.iter().enumerate().map(move |(index, unit)| {
        while entities.next_if(|entity| entity.id < unit.id).is_some() {}
        (index, unit, entities.next_if(|entity| entity.id == unit.id))
    })
}

fn casting(unit: &UnitView) -> bool {
    unit.statuses.bits & StatusFlags::CASTING != 0
}

fn new_entity(tick: u32, unit: &UnitView) -> EntityTrack {
    EntityTrack {
        id: unit.id,
        unit: unit.clone(),
        previous_seen_tick: tick,
        last_seen_tick: tick,
        velocity: None,
        hp_delta: 0,
        mana_delta: 0,
        visible: true,
        last_damage_dealt: None,
        last_damage_taken: None,
        last_heal_dealt: None,
        last_heal_received: None,
        last_mana_restoration_dealt: None,
        last_mana_restoration_received: None,
        last_death: None,
        last_ability_cast: None,
        last_possible_attack_landed: None,
        fight: None,
        attackers: [0; 2],
        talents: Talents::default(),
        casting_since: casting(unit).then_some(tick),
    }
}

fn update_entity(entity: &mut EntityTrack, tick: u32, unit: &UnitView) {
    let previous_pos = entity.unit.pos;
    let previous_tick = entity.last_seen_tick;
    entity.previous_seen_tick = previous_tick;
    entity.last_seen_tick = tick;
    entity.velocity = Some(VelocityEstimate {
        delta: Vec2 {
            x: unit
                .pos
                .x
                .checked_sub(previous_pos.x)
                .expect("position delta was validated"),
            y: unit
                .pos
                .y
                .checked_sub(previous_pos.y)
                .expect("position delta was validated"),
        },
        elapsed_ticks: tick - previous_tick,
    });
    entity.hp_delta = i64::from(unit.hp) - i64::from(entity.unit.hp);
    entity.mana_delta = i64::from(unit.mana) - i64::from(entity.unit.mana);
    // Unseen units lose it at the end of every snapshot, so what is held here
    // was held through the snapshot before.
    entity.casting_since = match casting(unit) {
        true => entity.casting_since.or(Some(tick)),
        false => None,
    };
    copy_unit_view(&mut entity.unit, unit);
    entity.visible = true;
}

/// Overwrites `target` with `source`, reusing every nested allocation.
///
/// The struct update copies every other field, so a new field is either
/// copied or, if it owns memory, a compile error here.
pub(crate) fn copy_unit_view(target: &mut UnitView, source: &UnitView) {
    let mut abilities = std::mem::take(&mut target.abilities);
    let mut items = std::mem::take(&mut target.items);
    let mut effects = std::mem::take(&mut target.effects);
    let mut queue = std::mem::take(&mut target.queue);
    abilities.clone_from(&source.abilities);
    items.clone_from(&source.items);
    effects.clone_from(&source.effects);
    queue.clone_from(&source.queue);
    *target = UnitView {
        abilities,
        items,
        effects,
        queue,
        ..*source
    };
}
