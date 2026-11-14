//! Every health gain uses the same restoration factors and post-cap redirection.

use bota_core::ModifierKind;
use bota_proto::{Fixed, Inflictor, ItemId};

use crate::game::{Entity, Health, Modifier, Modifiers, Stats, Table, World};

/// A requested restoration after the two restoration channels, before the recipient's HP cap.
pub(crate) struct Gain {
    pub amount: Fixed,
    pub redirected: Fixed,
}

pub(crate) enum Effect {
    Cut(i32),
    Redirect {
        source: Entity,
        item: ItemId,
        collector: u8,
        pct: i32,
    },
}

/// Derived receiver stats depend on both their own template and their source's active modifier.
#[derive(Clone, Copy, Default)]
pub(crate) struct Changes {
    pub receiver: bool,
    pub source: bool,
}

impl Changes {
    pub(crate) fn of(on_it: Option<&Modifiers>) -> Self {
        Self {
            receiver: on_it.is_some_and(|on_it| {
                on_it.active().any(|held| {
                    bota_core::item_modifier(held.kind)
                        .is_some_and(|template| template.restoration.is_some())
                })
            }),
            source: false,
        }
    }

    pub(crate) fn note(&mut self, kind: ModifierKind) {
        let ModifierKind::ItemBuff { item, index, .. } = kind else {
            return;
        };
        let def = bota_core::item_def(item).expect("an item modifier has a catalogue row");
        self.receiver |= def.modifiers[usize::from(index)].restoration.is_some();
        self.source |= def.modifiers.iter().any(|template| {
            template
                .restoration
                .is_some_and(|aura| aura.active == index)
        });
    }
}

/// The receiver callback reads its source's current active state, including during aura linger.
pub(crate) fn effect(held: &Modifier, modifiers: &Table<Modifiers>) -> Option<Effect> {
    let ModifierKind::ItemBuff { item, .. } = held.kind else {
        return None;
    };
    let restoration = bota_core::item_modifier(held.kind)?.restoration?;
    let source = held.source?;
    let active = modifiers.get(source).is_some_and(|on_it| {
        on_it.active().any(|active| {
            matches!(active.kind, ModifierKind::ItemBuff { item: active_item, index, .. }
            if active_item == item && index == restoration.active)
        })
    });
    Some(if active {
        Effect::Redirect {
            source,
            item,
            collector: restoration.active,
            pct: restoration.active_redirect_pct,
        }
    } else {
        Effect::Cut(restoration.passive_cut_pct)
    })
}

/// The same factor pair drives immediate healing and displayed/applied regeneration.
pub(crate) fn gain(requested: Fixed, stats: &Stats) -> Gain {
    Gain {
        amount: if requested > Fixed::ZERO {
            super::bubble::restored(requested, stats.restoration_bp)
        } else {
            requested
        },
        redirected: if requested > Fixed::ZERO {
            super::bubble::restored(requested, stats.restoration_redirect_bp)
        } else {
            Fixed::ZERO
        },
    }
}

pub(crate) fn heal(
    target: Entity,
    requested: Fixed,
    origin: Option<ItemId>,
    stats: &Table<Stats>,
    health: &mut Table<Health>,
    modifiers: &mut Table<Modifiers>,
) -> Fixed {
    let Some(stat) = stats.get(target) else {
        return Fixed::ZERO;
    };
    heal_scaled(
        target,
        gain(requested, stat),
        origin,
        stats,
        health,
        modifiers,
    )
}

/// Dota emits the diverted amount after applying the HP cap, proportioned by actual gain.
pub(crate) fn heal_scaled(
    target: Entity,
    gain: Gain,
    origin: Option<ItemId>,
    stats: &Table<Stats>,
    health: &mut Table<Health>,
    modifiers: &mut Table<Modifiers>,
) -> Fixed {
    let (Some(stat), Some(pool)) = (stats.get(target), health.get_mut(target)) else {
        return Fixed::ZERO;
    };
    if pool.hp <= Fixed::ZERO {
        return Fixed::ZERO;
    }
    let before = pool.hp;
    pool.hp = Fixed {
        raw: (i64::from(before.raw) + i64::from(gain.amount.raw))
            .min(i64::from(stat.max_hp.raw))
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
    };
    let gained = pool.hp - before;
    if gained > Fixed::ZERO && gain.amount > Fixed::ZERO && gain.redirected > Fixed::ZERO {
        let diverted =
            i64::from(gain.redirected.raw) * i64::from(gained.raw) / i64::from(gain.amount.raw);
        credit(
            target,
            diverted,
            origin,
            stat.spell_immune,
            health,
            modifiers,
        );
    }
    gained
}

fn credit(
    target: Entity,
    amount: i64,
    origin: Option<ItemId>,
    spell_immune: bool,
    health: &Table<Health>,
    modifiers: &mut Table<Modifiers>,
) {
    let destination = modifiers.get(target).and_then(|on_it| {
        on_it.active().find_map(|held| {
            if !super::support::item_works(held.kind, spell_immune) {
                return None;
            }
            match effect(held, modifiers)? {
                Effect::Redirect {
                    source,
                    item,
                    collector,
                    ..
                } => Some((source, item, collector)),
                Effect::Cut(_) => None,
            }
        })
    });
    let Some((source, item, index)) = destination else {
        return;
    };
    // The native receiver excludes healing originating from this same item, preventing feedback.
    if origin == Some(item) || health.get(source).is_none_or(|pool| pool.hp <= Fixed::ZERO) {
        return;
    }
    if let Some(on_it) = modifiers.get_mut(source)
        && let Some(collector) = on_it.0.iter_mut().find(|held| held.ticks_left != Some(0)
            && matches!(held.kind, ModifierKind::ItemBuff { item: held_item, index: held_index, .. }
                if held_item == item && held_index == index))
        && let ModifierKind::ItemBuff { amount: pending, .. } = &mut collector.kind {
        *pending = (i64::from(*pending) + amount).clamp(0, i64::from(i32::MAX)) as i32;
    }
}

impl World {
    pub(crate) fn refresh_restoration(&mut self, changed: Entity, changes: Changes) {
        if changes.receiver && self.alive(changed) {
            self.refresh_stats_of(changed);
        }
        if !changes.source {
            return;
        }
        let receivers: Vec<_> = self
            .entities
            .iter()
            .filter(|&target| {
                self.alive(target)
                    && self.modifiers.get(target).is_some_and(|on_it| {
                        on_it.active().any(|held| {
                            held.source == Some(changed)
                                && bota_core::item_modifier(held.kind)
                                    .is_some_and(|template| template.restoration.is_some())
                        })
                    })
            })
            .collect();
        for target in receivers {
            self.refresh_stats_of(target);
        }
    }

    pub(crate) fn collect_restoration(&mut self, entity: Entity, item: ItemId, amount: i32) {
        let healed = heal(
            entity,
            Fixed { raw: amount },
            Some(item),
            &self.stats,
            &mut self.health,
            &mut self.modifiers,
        );
        self.tell_healed((entity, Inflictor::Item(item)), entity, healed.to_int(), 0);
    }
}
