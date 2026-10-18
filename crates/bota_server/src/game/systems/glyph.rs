//! The Glyph of Fortification: a side's structures and lane creeps take no
//! damage for a while, its towers shoot more targets, and one cooldown
//! serves the whole side.

use bota_core::{attack_class_pct, kind_def, rules};
use bota_proto::{EffectId, EffectView, GlyphView, RejectReason, SlotId, Team, UnitKind};

use crate::engine::Entity;
use crate::game::{GLYPH_MELEE_RAX, GLYPH_TIER_ONE, GLYPH_TIER_TWO, Stats, World};

/// The index of a side's Glyph in the world's two.
pub(crate) fn glyph_index(team: Team) -> Option<usize> {
    match team {
        Team::Radiant => Some(0),
        Team::Dire => Some(1),
        Team::Neutral => None,
    }
}

impl World {
    /// Whether a seat's side may activate its Glyph now.
    pub fn check_glyph(&self, slot: SlotId) -> Result<(), RejectReason> {
        let seat = self.seat(slot).ok_or(RejectReason::HeroDead)?;
        let index = glyph_index(seat.team).ok_or(RejectReason::NotYourUnit)?;
        if self.tick < self.glyphs[index].ready_at {
            return Err(RejectReason::OnCooldown);
        }
        Ok(())
    }

    /// Activates the Glyph of a seat's side, if it is ready.
    pub(crate) fn use_glyph(&mut self, slot: SlotId) {
        if self.check_glyph(slot).is_err() {
            return;
        }
        let Some(index) = self.seat(slot).and_then(|seat| glyph_index(seat.team)) else {
            return;
        };
        let now = self.tick;
        let glyph = &mut self.glyphs[index];
        glyph.ready_at = now + rules::GLYPH_COOLDOWN_TICKS;
        glyph.structures_until = now + rules::GLYPH_STRUCTURE_TICKS;
        glyph.creeps_until = now + rules::GLYPH_CREEP_TICKS;
    }

    /// Whether a side's structures are under its Glyph now.
    pub(crate) fn structures_glyphed(&self, team: Team) -> bool {
        glyph_index(team).is_some_and(|index| self.tick < self.glyphs[index].structures_until)
    }

    /// Refreshes a side's Glyph when the first tier one tower, the first
    /// tier two tower or the first melee barracks of the side falls.
    pub(crate) fn refresh_glyph_for(&mut self, fallen: Entity) {
        let (Some(kind), Some(team)) = (
            self.kind.get(fallen).copied(),
            self.team.get(fallen).copied(),
        ) else {
            return;
        };
        let Some(index) = glyph_index(team) else {
            return;
        };
        let bit = match kind {
            UnitKind::Tower => match self.tier.get(fallen).map(|tier| tier.0) {
                Some(1) => GLYPH_TIER_ONE,
                Some(2) => GLYPH_TIER_TWO,
                _ => return,
            },
            UnitKind::Barracks if self.rax.get(fallen).is_some_and(|rax| !rax.ranged) => {
                GLYPH_MELEE_RAX
            }
            _ => return,
        };
        let ready = self.tick + rules::GLYPH_REFRESH_TICKS;
        let glyph = &mut self.glyphs[index];
        if glyph.refreshed & bit == 0 {
            glyph.refreshed |= bit;
            glyph.ready_at = glyph.ready_at.min(ready);
        }
    }

    /// Wears the Glyph as [`Stats::glyph`] on the structures and lane creeps
    /// it covers.
    pub(crate) fn mark_glyph(&mut self) {
        let tick = self.tick;
        let glyphs = self.glyphs;
        for entity in self.entities.iter() {
            let (Some(kind), Some(team)) = (
                self.kind.get(entity).copied(),
                self.team.get(entity).copied(),
            ) else {
                continue;
            };
            let Some(index) = glyph_index(team) else {
                continue;
            };
            let def = kind_def(kind);
            let covered = (def.structure && tick < glyphs[index].structures_until)
                || (def.lane_creep && tick < glyphs[index].creeps_until);
            if covered && let Some(stats) = self.stats.get_mut(entity) {
                stats.glyph = true;
            }
        }
    }

    /// The Glyphs a viewer is told of: both sides', the other side's without
    /// its cooldown (Dota's announcer tells both teams when a Glyph is used).
    pub(crate) fn glyph_views(&self, viewer: Option<Team>) -> Vec<GlyphView> {
        [Team::Radiant, Team::Dire]
            .into_iter()
            .filter_map(|team| {
                let glyph = self.glyphs[glyph_index(team)?];
                let own = viewer.is_none_or(|seen| seen == team);
                Some(GlyphView {
                    team,
                    ready_in: if own {
                        glyph.ready_at.saturating_sub(self.tick)
                    } else {
                        0
                    },
                    active_for: glyph.structures_until.saturating_sub(self.tick),
                })
            })
            .collect()
    }

    /// The buff a unit under its side's Glyph wears, with the ticks its
    /// cover has left.
    pub(crate) fn fortified_effect(&self, entity: Entity) -> Option<EffectView> {
        if !self.stats.get(entity)?.glyph {
            return None;
        }
        let glyph = self.glyphs[glyph_index(*self.team.get(entity)?)?];
        let until = if kind_def(*self.kind.get(entity)?).structure {
            glyph.structures_until
        } else {
            glyph.creeps_until
        };
        Some(EffectView {
            id: EffectId::FORTIFIED,
            ticks_left: Some(until.saturating_sub(self.tick)),
            stacks: None,
        })
    }

    /// The enemies a tower under its side's Glyph shoots besides its target:
    /// the closest five that are creeps or heroes it may attack, in reach.
    pub(crate) fn multishot_targets(
        &self,
        tower: Entity,
        main: Entity,
        stats: &Stats,
    ) -> Vec<Entity> {
        let tier = self.tier.get(tower).map_or(0, |tier| tier.0);
        let side = self.team.get(tower).copied().unwrap_or(Team::Neutral);
        if self.kind.get(tower) != Some(&UnitKind::Tower)
            || tier < 2
            || !self.structures_glyphed(side)
        {
            return Vec::new();
        }
        let here = self.pos(tower);
        let mut found: Vec<(i64, Entity)> = self
            .entities
            .iter()
            .filter(|&other| other != main && self.reachable(tower, stats.attack_range, other))
            .filter(|&other| {
                self.kind
                    .get(other)
                    .is_some_and(|&kind| !kind_def(kind).structure && kind != UnitKind::Ward)
            })
            .filter_map(|other| {
                let apart = self.pos(other)?.distance_squared(here?);
                Some((apart, other))
            })
            .collect();
        found.sort();
        found
            .into_iter()
            .take(rules::GLYPH_MULTISHOT_TARGETS)
            .map(|(_, other)| other)
            .collect()
    }

    /// The share of a tower's blow an extra target takes, by its class.
    pub(crate) fn class_pct_on(&self, attacker: Entity, on: Entity) -> i32 {
        match (self.kind.get(attacker), self.kind.get(on)) {
            (Some(&from), Some(&to)) => attack_class_pct(from, to),
            _ => 100,
        }
    }
}
