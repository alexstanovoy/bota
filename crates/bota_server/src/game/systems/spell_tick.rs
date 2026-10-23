//! What a neutral's spells leave running: chains leaping on, a gust carrying
//! its target, a slow wearing off.

use bota_core::{Act, ModifierKind, NeutralAbility, Spell, move_towards, per_tick, rules};
use bota_proto::DamageKind;

use crate::game::{Chain, World};

use crate::engine::Entity;

impl World {
    /// Runs every chain one tick on: when its wait is over it leaps to the
    /// nearest foe within its radius of the last it struck, seen and not yet
    /// struck, for a share of the last blow. A chain with nowhere to leap
    /// ends.
    pub(super) fn tick_chains(&mut self) {
        let entities = self.take_entity_snapshot();
        for marker in entities.iter().copied() {
            let Some(mut chain) = self.chain.get(marker).cloned() else {
                continue;
            };
            chain.wait = chain.wait.saturating_sub(1);
            if chain.wait == 0 {
                chain.wait = chain.delay;
                if !self.leap(marker, &mut chain) {
                    chain.leaps_left = 0;
                }
            }
            if chain.leaps_left == 0 {
                self.chain.remove(marker);
                self.discard(marker);
            } else {
                self.chain.insert(marker, chain);
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    /// One leap of a chain. False when there was nobody to leap to.
    fn leap(&mut self, marker: Entity, chain: &mut Chain) -> bool {
        let from = chain
            .struck
            .last()
            .filter(|last| self.alive(**last))
            .and_then(|last| self.pos(*last))
            .unwrap_or(chain.at);
        let next = self
            .entities
            .iter()
            .filter(|other| {
                !chain.struck.contains(other)
                    && self.hostile(marker, *other)
                    && self.is_foe(marker, *other)
                    && self
                        .pos(*other)
                        .is_some_and(|pos| pos.within(from, chain.radius))
            })
            .min_by_key(|other| {
                self.pos(*other)
                    .map_or(i64::MAX, |pos| pos.distance_squared(from))
            });
        let Some(next) = next else {
            return false;
        };
        self.push_hit(
            Some(chain.source),
            next,
            chain.damage,
            DamageKind::Magical,
            Some(chain.inflictor),
        );
        chain.struck.push(next);
        chain.at = self.pos(next).unwrap_or(chain.at);
        chain.leaps_left -= 1;
        chain.damage = chain.damage * chain.keep_pct / 100;
        true
    }

    /// Carries every pushed unit a tick on its way. It is held while it
    /// goes, and a body that has come to rest on ground it cannot stand on
    /// is set back on the last that it can.
    pub(super) fn tick_pushes(&mut self) {
        let entities = self.take_entity_snapshot();
        for on in entities.iter().copied() {
            let Some(mut push) = self.push.get(on).copied() else {
                continue;
            };
            let Some(at) = self.pos(on).filter(|_| self.alive(on)) else {
                self.push.remove(on);
                continue;
            };
            let next = move_towards(at, push.aim, per_tick(push.speed));
            let open = self.clearance.walkable(next);
            let to = if open { next } else { at };
            if self.clearance.stands_clear(to) {
                push.clear = to;
            }
            let done = !open || to == push.aim;
            if let Some(transform) = self.transform.get_mut(on) {
                transform.pos = if done { push.clear } else { to };
            }
            self.forget_walk(on);
            self.put_modifier(
                on,
                crate::game::Modifier::new(
                    ModifierKind::Stunned,
                    Some(push.source),
                    Some(push.inflictor),
                    Some(2),
                ),
            );
            if done {
                self.push.remove(on);
            } else {
                self.push.insert(on, push);
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    /// Lets every purge's slow fall: from its start by a fifth of it for
    /// every second that has gone, so that the last second holds a fifth.
    pub(super) fn tick_purges(&mut self) {
        let entities = self.take_entity_snapshot();
        for on in entities.iter().copied() {
            let Some(on_it) = self.modifiers.get_mut(on) else {
                continue;
            };
            for held in on_it.0.iter_mut() {
                let (
                    ModifierKind::Spell {
                        ability, amount, ..
                    },
                    Some(left),
                ) = (&mut held.kind, held.ticks_left)
                else {
                    continue;
                };
                let Spell::Cast {
                    act: Act::Purge { start_pct, .. },
                    ..
                } = *ability.spell()
                else {
                    continue;
                };
                if left == 0 || *ability != NeutralAbility::Purge {
                    continue;
                }
                let seconds_left = (left - 1) / rules::TICKS_PER_SECOND + 1;
                *amount = (start_pct as u32 * seconds_left / 5).min(start_pct as u32) as u16;
            }
        }
        self.recycle_entity_snapshot(entities);
    }
}
