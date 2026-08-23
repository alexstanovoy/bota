//! What an entity has gathered and keeps.

/// A kind of thing that is gathered and counted rather than timed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackKind {
    /// What has died near a hero and been kept.
    FleshHeap,
    /// The souls of what a hero has brought down.
    Souls,
}

/// How many kinds are counted.
pub const STACK_KINDS: usize = 2;

impl StackKind {
    /// Every kind there is, in the order they are counted.
    pub const ALL: [StackKind; STACK_KINDS] = [StackKind::FleshHeap, StackKind::Souls];

    /// Where its count sits.
    pub const fn at(self) -> usize {
        match self {
            StackKind::FleshHeap => 0,
            StackKind::Souls => 1,
        }
    }
}

/// What one entity has gathered, counted by kind.
///
/// Nothing here runs out on its own. It outlives the body: the counts wait on
/// the seat while the hero that gathered them is down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stacks {
    /// One count per kind, indexed by [`StackKind::at`].
    counts: [u32; STACK_KINDS],
}

impl Stacks {
    /// How many of one kind are held.
    pub const fn of(&self, kind: StackKind) -> u32 {
        self.counts[kind.at()]
    }

    /// Puts more of one kind on.
    pub const fn gather(&mut self, kind: StackKind, many: u32) {
        self.gather_up_to(kind, many, u32::MAX);
    }

    /// Puts more of one kind on, holding no more than `cap` of it in all.
    pub const fn gather_up_to(&mut self, kind: StackKind, many: u32, cap: u32) {
        let at = kind.at();
        let held = self.counts[at].saturating_add(many);
        self.counts[at] = if held > cap { cap } else { held };
    }

    /// Sets how many of one kind are held.
    pub const fn set(&mut self, kind: StackKind, many: u32) {
        self.counts[kind.at()] = many;
    }

    /// Every kind held, with its count. A kind none of which is held is left
    /// out.
    pub fn held(&self) -> impl Iterator<Item = (StackKind, u32)> + '_ {
        StackKind::ALL
            .into_iter()
            .map(|kind| (kind, self.of(kind)))
            .filter(|(_, many)| *many > 0)
    }
}
