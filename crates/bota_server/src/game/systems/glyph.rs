//! The Glyph of Fortification: a side's structures and lane creeps take no
//! damage for a while, its towers shoot more targets, and one cooldown
//! serves the whole side.

use bota_core::{ModifierKind, attack_class_pct, kind_def, rules, side_index};
use bota_proto::{GlyphView, RejectReason, SlotId, Team, UnitKind};

use crate::engine::Entity;
use crate::game::{GLYPH_MELEE_RAX, GLYPH_TIER_ONE, GLYPH_TIER_TWO, Modifier, Stats, World};

impl World {
    /// Whether a seat's side may activate its Glyph now.
    pub fn check_glyph(&self, slot: SlotId) -> Result<(), RejectReason> {
        let seat = self.seat(slot).ok_or(RejectReason::HeroDead)?;
        let index = side_index(seat.team).ok_or(RejectReason::NotYourUnit)?;
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
        let Some(team) = self.seat(slot).map(|seat| seat.team) else {
            return;
        };
        let Some(index) = side_index(team) else {
            return;
        };
        let now = self.tick;
        let glyph = &mut self.glyphs[index];
        glyph.ready_at = now + rules::GLYPH_COOLDOWN_TICKS;
        glyph.structures_until = now + rules::GLYPH_STRUCTURE_TICKS;
        self.fortify(team);
    }

    /// Puts the Fortified buff on every structure and lane creep of a side
    /// that stands now, as Dota's Glyph does: a creep that spawns later
    /// does not get it.
    fn fortify(&mut self, team: Team) {
        let covered: Vec<(Entity, u32)> = self
            .entities
            .iter()
            .filter(|&entity| self.team.get(entity) == Some(&team))
            .filter_map(|entity| {
                let def = kind_def(*self.kind.get(entity)?);
                let ticks = if def.structure {
                    rules::GLYPH_STRUCTURE_TICKS
                } else if def.lane_creep {
                    rules::GLYPH_CREEP_TICKS
                } else {
                    return None;
                };
                Some((entity, ticks))
            })
            .collect();
        for (entity, ticks) in covered {
            self.put_modifier(
                entity,
                Modifier::new(ModifierKind::Fortified, None, None, Some(ticks)),
            );
        }
    }

    /// Whether a side's structures are under its Glyph now.
    pub(crate) fn structures_glyphed(&self, team: Team) -> bool {
        side_index(team).is_some_and(|index| self.tick < self.glyphs[index].structures_until)
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
        let Some(index) = side_index(team) else {
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

    /// The Glyphs a viewer is told of: both sides', the other side's without
    /// its cooldown (Dota's announcer tells both teams when a Glyph is used).
    pub(crate) fn glyph_views(&self, viewer: Option<Team>) -> Vec<GlyphView> {
        [Team::Radiant, Team::Dire]
            .into_iter()
            .filter_map(|team| {
                let glyph = self.glyphs[side_index(team)?];
                let own = viewer.is_none_or(|seen| seen == team);
                Some(GlyphView {
                    team,
                    ready_in: 0,
                    active_for: glyph.structures_until.saturating_sub(self.tick),
                    ready_after: own.then(|| glyph.ready_at.saturating_sub(self.tick)),
                })
            })
            .collect()
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
