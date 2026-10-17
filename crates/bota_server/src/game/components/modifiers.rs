//! What has been put on an entity: what it does, who put it, how long it
//! holds.

use bota_core::{ModifierKind, rules};

use crate::engine::Entity;

/// One modifier on an entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modifier {
    /// What it does, and how much of it there is.
    pub kind: ModifierKind,
    /// Who put it on. Absent for the world's own.
    pub source: Option<Entity>,
    /// Ticks before it lifts. Absent for one that does not lift on its own.
    pub ticks_left: Option<u32>,
}

impl Modifier {
    pub fn new(kind: ModifierKind, source: Option<Entity>, ticks_left: Option<u32>) -> Modifier {
        Modifier {
            kind,
            source,
            ticks_left,
        }
    }
}

/// Everything on an entity right now. Present on every unit; empty when
/// nothing is on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Modifiers(pub Vec<Modifier>);

impl Modifiers {
    /// Every modifier that has not run out.
    pub fn active(&self) -> impl Iterator<Item = &Modifier> {
        self.0.iter().filter(|held| held.ticks_left != Some(0))
    }

    /// Replaces the same kind; Shadowraze replaces only the same source's
    /// record.
    pub fn put(&mut self, modifier: Modifier) {
        let same = std::mem::discriminant(&modifier.kind);
        self.0.retain(|held| match (held.kind, modifier.kind) {
            (ModifierKind::Shadowraze { .. }, ModifierKind::Shadowraze { .. }) => {
                held.ticks_left != Some(0) && held.source != modifier.source
            }
            _ => std::mem::discriminant(&held.kind) != same,
        });
        if let ModifierKind::Shadowraze { stacks } = modifier.kind {
            assert!(stacks > 0);
            let left = modifier.ticks_left.expect("a Shadowraze record runs out");
            assert!(left > 0);
            assert!(left <= rules::RAZE_DEBUFF_TICKS);
            let count = self
                .0
                .iter()
                .filter(|held| matches!(held.kind, ModifierKind::Shadowraze { .. }))
                .count();
            assert!(count <= rules::RAZE_MAX_SOURCES);
            if count == rules::RAZE_MAX_SOURCES {
                let at = self
                    .0
                    .iter()
                    .enumerate()
                    .filter(|(_, held)| matches!(held.kind, ModifierKind::Shadowraze { .. }))
                    .min_by_key(|(_, held)| held.ticks_left.unwrap_or(u32::MAX))
                    .map(|(at, _)| at)
                    .expect("a full Shadowraze source set has an expiry");
                self.0.remove(at);
            }
        }
        self.0.push(modifier);
    }

    /// Puts a timed modifier on, adding its time to what the same kind from
    /// the same source already holds, up to `cap` ticks in all.
    pub fn extend(&mut self, modifier: Modifier, cap: u32) {
        let more = modifier.ticks_left.expect("an extended modifier runs out");
        let same = std::mem::discriminant(&modifier.kind);
        let held = self
            .active()
            .find(|held| {
                std::mem::discriminant(&held.kind) == same && held.source == modifier.source
            })
            .and_then(|held| held.ticks_left)
            .unwrap_or(0);
        self.put(Modifier {
            ticks_left: Some(held.saturating_add(more).min(cap)),
            ..modifier
        });
    }

    /// Takes off every modifier of one kind from one source, whatever there
    /// is of it.
    pub fn take(&mut self, like: ModifierKind, source: Option<Entity>) {
        let same = std::mem::discriminant(&like);
        self.0
            .retain(|held| std::mem::discriminant(&held.kind) != same || held.source != source);
    }

    /// Active Shadowraze hits held for this exact caster generation; zero
    /// when absent.
    pub fn raze_stacks(&self, caster: Entity) -> u8 {
        self.active()
            .find_map(|held| match held.kind {
                ModifierKind::Shadowraze { stacks } if held.source == Some(caster) => Some(stacks),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Adds one successful hit and refreshes its caster's complete stack to
    /// [`rules::RAZE_DEBUFF_TICKS`].
    pub fn stack_raze(&mut self, caster: Entity) {
        let stacks = self.raze_stacks(caster).saturating_add(1);
        self.put(Modifier::new(
            ModifierKind::Shadowraze { stacks },
            Some(caster),
            Some(rules::RAZE_DEBUFF_TICKS),
        ));
    }
}
