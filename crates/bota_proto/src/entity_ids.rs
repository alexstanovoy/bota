//! Entity references mapped to the names visible to each side.

use crate::{EntityId, EventKind, SlotOrder, WorldView};

impl EventKind {
    /// Calls `visit` on every entity handle the event carries.
    pub fn visit_ids(&mut self, visit: &mut dyn FnMut(&mut EntityId)) {
        match self {
            EventKind::AttackLaunched { attacker, target } => {
                visit(attacker);
                target.iter_mut().for_each(visit);
            }
            EventKind::Damaged { source, target, .. }
            | EventKind::Missed { source, target }
            | EventKind::Healed { source, target, .. } => {
                source.iter_mut().for_each(&mut *visit);
                visit(target);
            }
            EventKind::Died {
                unit,
                killer,
                credited_hero,
                ..
            } => {
                visit(unit);
                killer.iter_mut().for_each(&mut *visit);
                credited_hero.as_deref_mut().into_iter().for_each(visit);
            }
            EventKind::Landed {
                caster, missile, ..
            } => {
                caster.iter_mut().for_each(&mut *visit);
                missile.iter_mut().for_each(visit);
            }
            EventKind::TreeFelled { by, .. } => by.iter_mut().for_each(visit),
            EventKind::ItemUsed { caster, target, .. } => {
                caster.iter_mut().for_each(&mut *visit);
                target.iter_mut().for_each(visit);
            }
            EventKind::Captured { unit, by, .. } | EventKind::ShrineActivated { unit, by, .. } => {
                visit(unit);
                by.iter_mut().for_each(visit);
            }
            EventKind::AbilityCast { caster: unit, .. }
            | EventKind::LevelUp { unit, .. }
            | EventKind::StructureDestroyed { unit, .. }
            | EventKind::RuneActivated { unit, .. }
            | EventKind::RuneBottled { unit, .. }
            | EventKind::RuneDenied { unit, .. } => visit(unit),
            EventKind::LotusGrown { pool, .. } => visit(pool),
            EventKind::LotusPlucked { pool, unit, .. } => {
                visit(pool);
                unit.iter_mut().for_each(visit);
            }
            EventKind::GateWarped { from, to, unit } => {
                visit(from);
                visit(to);
                unit.iter_mut().for_each(visit);
            }
            EventKind::BossRoared { .. }
            | EventKind::ItemPlaced { .. }
            | EventKind::BossFell { .. }
            | EventKind::ItemDestroyed { .. }
            | EventKind::ItemTaken { .. }
            | EventKind::ItemBought { .. }
            | EventKind::Teleported { .. }
            | EventKind::TreeGrew { .. }
            | EventKind::RuneSpawned { .. } => {}
        }
    }
}

impl SlotOrder {
    /// Calls `visit` on every entity handle the order carries: the unit it
    /// is for and the one it names.
    pub fn visit_ids(&mut self, visit: &mut dyn FnMut(&mut EntityId)) {
        self.unit.iter_mut().for_each(&mut *visit);
        self.order.visit_ids(visit);
    }
}

impl WorldView {
    /// Calls `visit` on every entity handle the view carries, those of its
    /// skipped ticks' events included: the one walk that puts a side's own
    /// names into a view, so a field that adds a handle is walked here.
    pub fn visit_ids(&mut self, visit: &mut dyn FnMut(&mut EntityId)) {
        for unit in &mut self.units {
            visit(&mut unit.id);
            for order in &mut unit.queue {
                order.visit_ids(visit);
            }
            for effect in &mut unit.effects {
                effect.caster.iter_mut().for_each(&mut *visit);
            }
        }
        for shot in &mut self.projectiles {
            visit(&mut shot.id);
            shot.source.iter_mut().for_each(&mut *visit);
            shot.target.iter_mut().for_each(&mut *visit);
        }
        for player in &mut self.players {
            player.unit.iter_mut().for_each(&mut *visit);
        }
        for loot in &mut self.loot {
            visit(&mut loot.id);
        }
        for rune in &mut self.runes {
            visit(&mut rune.id);
        }
        for pool in &mut self.pools {
            visit(&mut pool.id);
        }
        for entry in &mut self.aggro {
            visit(&mut entry.unit);
            entry.target.iter_mut().for_each(&mut *visit);
            if let Some(swing) = &mut entry.swing {
                visit(&mut swing.target);
            }
        }
        for teleport in &mut self.teleports {
            visit(&mut teleport.building);
        }
        for skipped in &mut self.skipped_ticks {
            for event in &mut skipped.events {
                event.visit_ids(visit);
            }
        }
    }
}
