//! Entity handles and the allocator that hands them out.

use std::num::NonZeroU32;

/// The slot an entity holds. Component tables are indexed by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Index(pub u32);

/// Which tenant of a slot an entity is. Never zero, so a zeroed handle names
/// nobody.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Generation(pub NonZeroU32);

impl Generation {
    /// What a slot carries the first time it is handed out.
    pub const MIN: Self = Self(NonZeroU32::MIN);

    /// What the next tenant of the slot carries. Wraps back to
    /// [`Generation::MIN`] rather than reaching zero.
    pub fn next(self) -> Self {
        Self(self.0.checked_add(1).unwrap_or(NonZeroU32::MIN))
    }
}

/// A handle to an entity: the slot it holds, and which tenant of that slot it
/// is.
///
/// Two entities that held one slot one after another never compare equal, so a
/// handle kept past a death resolves to nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Entity {
    index: Index,
    generation: Generation,
}

impl Entity {
    /// The slot it holds.
    pub fn index(&self) -> Index {
        self.index
    }

    /// Which tenant of that slot it is.
    pub fn generation(&self) -> Generation {
        self.generation
    }
}

/// What is known about one slot.
struct EntityMeta {
    /// The generation of its current tenant, or of its last one while free.
    generation: Generation,
    /// Whether the slot is waiting to be handed out again.
    free: bool,
}

/// Hands out [`Entity`] handles and knows which of them are still live.
///
/// A freed slot goes back to be handed out again ahead of any slot never yet
/// used, carrying a raised generation. Iteration runs in slot order.
pub struct EntityAllocator {
    /// One entry per slot ever handed out, indexed by [`Index`].
    entities: Vec<EntityMeta>,
    /// Slots waiting to be handed out again, the most recently freed last.
    free: Vec<Index>,
    /// How many entities are live.
    len: usize,
}

impl EntityAllocator {
    /// An allocator that has handed out nothing.
    pub fn new() -> Self {
        Self {
            entities: Default::default(),
            free: Default::default(),
            len: 0,
        }
    }

    /// How many entities are live.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether no entity is live.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Hands out a handle, taking a free slot when there is one.
    ///
    /// The generation is raised as the slot changes hands, so handles to the
    /// slot's previous tenant stop resolving.
    pub fn alloc(&mut self) -> Entity {
        if let Some(index) = self.free.pop() {
            let meta = &mut self.entities[index.0 as usize];
            debug_assert!(meta.free);
            let generation = meta.generation.next();
            meta.free = false;
            meta.generation = generation;
            self.len += 1;
            return Entity { index, generation };
        }
        debug_assert!(self.entities.len() <= u32::MAX as usize);
        let index = Index(self.entities.len() as u32);
        let generation = Generation::MIN;
        self.entities.push(EntityMeta {
            generation,
            free: false,
        });
        self.len += 1;
        Entity { index, generation }
    }

    /// Gives up the entity's slot. False when the handle named nobody live,
    /// which covers freeing twice.
    pub fn free(&mut self, entity: Entity) -> bool {
        let meta = match self.entities.get_mut(entity.index.0 as usize) {
            Some(v) => v,
            None => return false,
        };
        if meta.free || meta.generation != entity.generation {
            return false;
        }
        debug_assert!(self.len > 0);
        self.free.push(entity.index);
        meta.free = true;
        self.len -= 1;
        true
    }

    /// Whether the handle still names a live entity.
    pub fn contains(&self, entity: Entity) -> bool {
        let meta = match self.entities.get(entity.index.0 as usize) {
            Some(v) => v,
            None => return false,
        };
        !meta.free && meta.generation == entity.generation
    }

    /// The live entity holding a slot as a given tenant, if there is one.
    pub fn resolve(&self, index: Index, generation: u32) -> Option<Entity> {
        let generation = Generation(NonZeroU32::new(generation)?);
        let meta = self.entities.get(index.0 as usize)?;
        (!meta.free && meta.generation == generation).then_some(Entity { index, generation })
    }

    /// Every live entity, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = Entity> {
        self.entities
            .iter()
            .enumerate()
            .filter_map(|(idx, entity)| {
                if entity.free {
                    None
                } else {
                    Some(Entity {
                        index: Index(idx as u32),
                        generation: entity.generation,
                    })
                }
            })
    }
}

impl Default for EntityAllocator {
    fn default() -> Self {
        Self::new()
    }
}
