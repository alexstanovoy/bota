//! The Twin Gates: a warp channelled at one gate carries its caster to the
//! other, both gates glowing while it runs; broken off, it gives its mana
//! back.

use bota_core::{ModifierKind, TWIN_GATE, TwinGateDef, WARP_RANGE, facing_towards, rules};
use bota_proto::{AbilityId, EventKind, Fixed, Target, Team};

use crate::game::{Entity, Event, EventVisibility, Modifier, TwinGate, Warp, World, wire_id};

impl World {
    /// Stands up a map's pair of Twin Gates, each facing as the map has it
    /// and knowing the other.
    pub(crate) fn stand_up_twin_gates(&mut self, gates: &'static [TwinGateDef]) {
        let [first, second] = gates else {
            assert!(gates.is_empty(), "Twin Gates come in a pair");
            return;
        };
        let stand = |world: &mut World, def: &TwinGateDef| {
            let entity = world.spawn_objective(&TWIN_GATE, Team::Neutral, def.at);
            if let Some(at) = world.transform.get_mut(entity) {
                at.facing = def.facing;
            }
            entity
        };
        let one = stand(self, first);
        let other = stand(self, second);
        self.gate.insert(
            one,
            TwinGate {
                partner: other,
                exit: first.exit,
            },
        );
        self.gate.insert(
            other,
            TwinGate {
                partner: one,
                exit: second.exit,
            },
        );
    }

    /// The Twin Gate a warp is aimed at and the gate it carries to, while
    /// both stand.
    fn warp_gates(&self, target: Target) -> Option<(Entity, Entity)> {
        let Target::Unit(id) = target else {
            return None;
        };
        let gate = self.of_wire(id).filter(|gate| self.alive(*gate))?;
        let partner = self.gate.get(gate)?.partner;
        self.alive(partner).then_some((gate, partner))
    }

    /// `twin_gate_portal_warp` goes off: the channel begins at a gate whose
    /// pair stands, both gates glow, and what it cost is kept to be given
    /// back. A rooted caster never gets here (`ROOT_DISABLES`).
    pub(crate) fn warp_begins(&mut self, caster: Entity, target: Target) -> bool {
        let Some((gate, _)) = self.warp_gates(target) else {
            return false;
        };
        let within = match (self.pos(caster), self.pos(gate)) {
            (Some(from), Some(at)) => from.within(at, rules::units(WARP_RANGE)),
            _ => false,
        };
        if !self.alive(caster) || !within {
            return false;
        }
        let paid = self.ability_mana_cost(caster, AbilityId::TWIN_GATE_WARP, 1);
        self.warp.insert(caster, Warp { gate, paid });
        self.light_gates(gate);
        true
    }

    /// One tick of a warp's channel: it goes on while both gates stand and
    /// the caster is not rooted (Dota 7.41e: "Channeling the Twin Gate can
    /// now be interrupted by root").
    pub(crate) fn warp_goes_on(&mut self, caster: Entity, target: Target) -> bool {
        self.warp.get(caster).is_some() && self.warp_gates(target).is_some() && !self.rooted(caster)
    }

    /// A warp's channel was broken off: it does nothing but give its mana
    /// back (Dota 7.40: "Twin Gates now refund the mana cost if the
    /// teleporting channel was interrupted"), and the gates go dark unless
    /// another unit still warps through them.
    pub(crate) fn warp_breaks_off(&mut self, caster: Entity, _: Target) {
        let Some(warp) = self.warp.remove(caster) else {
            return;
        };
        if let (Some(mana), Some(stats)) = (self.mana.get_mut(caster), self.stats.get(caster)) {
            mana.mana = (mana.mana + Fixed::from_int(warp.paid)).min(stats.max_mana);
        }
        self.light_gates(warp.gate);
    }

    /// A warp's channel ran its whole length: its caster stands at the
    /// other gate's exit, or the open ground nearest it, facing away from
    /// that gate, with its walk laid afresh and its queued orders kept
    /// (Liquipedia, *Twin Gate*: "Does not interrupt shift-queue orders").
    /// Missiles on their way to it fly on to it ("Does not disjoint incoming
    /// projectiles"). Where no open ground is left round the exit (walls a
    /// match stood there), nothing carries it: the warp ends as one broken
    /// off, its mana given back, and nobody is told of a landing.
    pub(crate) fn warp_completes(&mut self, caster: Entity, target: Target) {
        let landed = self.warp_gates(target).and_then(|(gate, partner)| {
            let exit = self.gate.get(partner)?.exit;
            Some((gate, partner, self.pos(partner)?, self.open_landing(exit)?))
        });
        let Some((gate, partner, beyond, landing)) = landed else {
            self.warp_breaks_off(caster, target);
            return;
        };
        let Some(warp) = self.warp.remove(caster) else {
            return;
        };
        self.light_gates(warp.gate);
        if let Some(at) = self.transform.get_mut(caster) {
            at.pos = landing;
            at.facing = facing_towards(beyond, landing);
        }
        self.forget_walk(caster);
        self.tell_warp(caster, gate, partner);
    }

    /// Tells the sides a warp landed: a side that sees either gate hears it,
    /// and learns who came through only if it sees that unit arrive.
    fn tell_warp(&mut self, caster: Entity, from: Entity, to: Entity) {
        let sees = |world: &World, side| {
            [from, to]
                .into_iter()
                .filter_map(|gate| world.pos(gate))
                .any(|at| world.can_see_point(side, at))
                || world.sees_now(side, caster)
        };
        let told = |world: &World| {
            EventVisibility::of(sees(world, Team::Radiant), sees(world, Team::Dire))
        };
        let visible_to = told(self);
        let source_told_to = self.who_may_know(caster, None);
        self.events.push(Event::Fact {
            kind: EventKind::GateWarped {
                from: wire_id(from),
                to: wire_id(to),
                unit: Some(wire_id(caster)),
            },
            visible_to,
            gold_hidden_from: None,
            source_told_to,
        });
    }

    /// Lights a gate and its pair while any unit channels a warp at either,
    /// and puts them out once none does: Dota plays the channel at both ends
    /// (Liquipedia, *Twin Gate*: "Both gates plays the ability channeling
    /// animation and sound effects"; the Dota 2 Wiki, *Twin Gate*: "While
    /// channeling, the other Twin Gate plays its channeling animation and
    /// channeling sounds as well"; the server's warp emits TwinGate.Channel on
    /// both gates). Each glow shows to a side that sees that gate's spot
    /// ([`bota_core::EffectDef::sighted_only`]), so a side watching the exit
    /// sees it light up for the whole channel, as Dota shows it, without
    /// seeing who channels at the other end.
    fn light_gates(&mut self, gate: Entity) {
        let Some(partner) = self.gate.get(gate).map(|held| held.partner) else {
            return;
        };
        let lit = self.entities.iter().any(|unit| {
            self.warp
                .get(unit)
                .is_some_and(|warp| warp.gate == gate || warp.gate == partner)
        });
        for end in [gate, partner] {
            let glows = self.wears(end, ModifierKind::TwinGateActive);
            if lit && !glows {
                self.put_modifier(
                    end,
                    Modifier::new(ModifierKind::TwinGateActive, None, None, None),
                );
            } else if !lit && glows {
                self.take_modifier(end, ModifierKind::TwinGateActive, None);
            }
        }
    }
}
