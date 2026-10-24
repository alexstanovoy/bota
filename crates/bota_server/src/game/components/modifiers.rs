//! What has been put on an entity: what it does, who put it on and with
//! what, how long it holds.

use bota_core::{ModifierKind, NeutralAbility, rules, spell_effect};
use bota_proto::{AbilityId, Inflictor};

use crate::engine::Entity;

/// One modifier on an entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modifier {
    /// What it does, and how much of it there is.
    pub kind: ModifierKind,
    /// Who put it on: the caster, or the bearer of an innate. Absent for the
    /// world's own.
    pub source: Option<Entity>,
    /// The ability or item it came from. Absent for one that came from
    /// neither, as the Glyph's.
    pub inflictor: Option<Inflictor>,
    /// Ticks before it lifts. Absent for one that does not lift on its own.
    pub ticks_left: Option<u32>,
    /// Ticks it was put on for, or last renewed for: set as it is put on.
    pub ticks_total: Option<u32>,
}

impl Modifier {
    pub fn new(
        kind: ModifierKind,
        source: Option<Entity>,
        inflictor: Option<Inflictor>,
        ticks_left: Option<u32>,
    ) -> Modifier {
        Modifier {
            kind,
            source,
            inflictor,
            ticks_left,
            ticks_total: ticks_left,
        }
    }
}

/// Whether putting `fresh` on takes `held` off: one of the same kind, save
/// that a Shadowraze record leaves the other casters' standing ones, and that
/// a neutral's spell replaces the one showing the same effect (the three
/// Water Bubbles are one modifier), but only from its own source for a
/// stacking aura or a strike.
fn replaces(fresh: &Modifier, held: &Modifier) -> bool {
    match (held.kind, fresh.kind) {
        (ModifierKind::Shadowraze { .. }, ModifierKind::Shadowraze { .. }) => {
            held.ticks_left == Some(0) || held.source == fresh.source
        }
        (ModifierKind::Spell { ability: had, .. }, ModifierKind::Spell { ability, .. }) => {
            spell_effect(had) == spell_effect(ability)
                && (!ability.per_source() || held.source == fresh.source)
        }
        _ => std::mem::discriminant(&held.kind) == std::mem::discriminant(&fresh.kind),
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
    /// record, and a neutral's spell the same spell (the same source's, for
    /// those [`per_source`](bota_core::NeutralAbility::per_source) keeps
    /// apart). Its time left is its full length from here.
    pub fn put(&mut self, modifier: Modifier) {
        let modifier = Modifier {
            ticks_total: modifier.ticks_left,
            ..modifier
        };
        self.0.retain(|held| !replaces(&modifier, held));
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

    /// The count a counted kind holds, `of` making that kind from a count:
    /// zero while none is on.
    pub fn count_of(&self, of: fn(u32) -> ModifierKind) -> u32 {
        let like = std::mem::discriminant(&of(0));
        self.active()
            .find(|held| std::mem::discriminant(&held.kind) == like)
            .and_then(|held| held.kind.stacks())
            .unwrap_or(0)
    }

    /// Sets the count of a counted kind where it stands in the list: put on
    /// last, untimed, by `bearer` with its innate `ability`, when none was on;
    /// taken off at zero.
    pub fn set_count(
        &mut self,
        of: fn(u32) -> ModifierKind,
        count: u32,
        (bearer, ability): (Entity, AbilityId),
    ) {
        let like = std::mem::discriminant(&of(0));
        let at = self
            .0
            .iter()
            .position(|held| std::mem::discriminant(&held.kind) == like);
        match (at, count) {
            (Some(at), 0) => {
                self.0.remove(at);
            }
            (Some(at), _) => self.0[at].kind = of(count),
            (None, 0) => {}
            (None, _) => self.0.push(Modifier::new(
                of(count),
                Some(bearer),
                Some(Inflictor::Ability(ability)),
                None,
            )),
        }
    }

    /// Takes off every modifier of one kind from one source, whatever there
    /// is of it.
    pub fn take(&mut self, like: ModifierKind, source: Option<Entity>) {
        let same = std::mem::discriminant(&like);
        self.0
            .retain(|held| std::mem::discriminant(&held.kind) != same || held.source != source);
    }

    /// The active modifier a neutral's spell left, from `source` when the
    /// spell keeps its sources apart.
    pub fn spell(&self, ability: NeutralAbility, source: Option<Entity>) -> Option<&Modifier> {
        self.active().find(|held| {
            matches!(held.kind, ModifierKind::Spell { ability: it, .. } if it == ability)
                && (!ability.per_source() || held.source == source)
        })
    }

    /// Whether what is on already outranks a fresh modifier an aura hands
    /// out, which then stays off: the one the fresh one would take off is
    /// the stronger instance of the same aura ([`ModifierKind::outranks`]),
    /// and it holds while it lasts, the weaker going on once it has run
    /// out. Of two equal ones the fresh one replaces the other, so the
    /// carrier later in the slots is named on it and its time is the
    /// freshest. Instances of an aura that stacks (a source's own Dragonhide
    /// or Rally) never meet here: each source keeps its own.
    pub fn outranks(&self, fresh: &Modifier) -> bool {
        self.active()
            .any(|held| replaces(fresh, held) && held.kind.outranks(fresh.kind))
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

    /// Adds one successful hit of the raze `inflictor` names and refreshes
    /// its caster's complete stack to [`rules::RAZE_DEBUFF_TICKS`].
    pub fn stack_raze(&mut self, caster: Entity, inflictor: Option<Inflictor>) {
        let stacks = self.raze_stacks(caster).saturating_add(1);
        self.put(Modifier::new(
            ModifierKind::Shadowraze { stacks },
            Some(caster),
            inflictor,
            Some(rules::RAZE_DEBUFF_TICKS),
        ));
    }
}
