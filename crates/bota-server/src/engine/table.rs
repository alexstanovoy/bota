//! Component storage: one table per kind of component.

use crate::engine::{Entity, Generation};

/// One slot's worth of a component.
struct Slot<T> {
    /// The generation of the entity the value was written for.
    generation: Generation,
    /// The component.
    value: T,
}

/// A component held by some of the entities, looked up by [`Entity`].
///
/// A slot keeps the generation it was written with, so a value a dead entity
/// left behind never reads back as the next tenant's. The table does not know
/// which entities are live: it answers only about the handle it is given, and
/// walking every entity is [`EntityAllocator::iter`] plus a lookup here.
///
/// [`EntityAllocator::iter`]: crate::engine::EntityAllocator::iter
pub struct Table<T> {
    /// One entry per slot the table has ever been written for, indexed by
    /// [`Entity::index`]. `None` where nothing was ever written.
    slots: Vec<Option<Slot<T>>>,
}

impl<T> Default for Table<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Table<T> {
    /// A table holding nothing.
    pub fn new() -> Self {
        Self { slots: Vec::new() }
    }

    /// Gives the entity this component, returning what it held before.
    ///
    /// Anything left in the slot by an earlier tenant is dropped.
    pub fn insert(&mut self, entity: Entity, value: T) -> Option<T> {
        let index = entity.index().0 as usize;
        if index >= self.slots.len() {
            self.slots.resize_with(index + 1, || None);
        }
        let slot = &mut self.slots[index];
        if let Some(held) = slot
            && held.generation == entity.generation()
        {
            return Some(std::mem::replace(&mut held.value, value));
        }
        debug_assert!(
            slot.as_ref()
                .is_none_or(|held| held.generation <= entity.generation()),
            "writing for an entity that has already been replaced"
        );
        *slot = Some(Slot {
            generation: entity.generation(),
            value,
        });
        None
    }

    /// Takes the component away from the entity, returning it.
    pub fn remove(&mut self, entity: Entity) -> Option<T> {
        let slot = self.slots.get_mut(entity.index().0 as usize)?;
        if slot
            .as_ref()
            .is_none_or(|held| held.generation != entity.generation())
        {
            return None;
        }
        slot.take().map(|held| held.value)
    }

    /// The entity's component, if it has one.
    pub fn get(&self, entity: Entity) -> Option<&T> {
        let held = self.slots.get(entity.index().0 as usize)?.as_ref()?;
        (held.generation == entity.generation()).then_some(&held.value)
    }

    /// The entity's component, if it has one.
    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        let held = self.slots.get_mut(entity.index().0 as usize)?.as_mut()?;
        (held.generation == entity.generation()).then_some(&mut held.value)
    }

    /// Whether the entity has this component.
    pub fn contains(&self, entity: Entity) -> bool {
        self.get(entity).is_some()
    }

    /// Whether nothing has ever been written, or everything written has been
    /// taken away. A slot left by a dead entity still counts as written.
    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }
}
