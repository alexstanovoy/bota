//! The shared policy for a lair's journey, its interruptions and contact damage.
use crate::game::{ActionState, Entity, Journey, Push, PushPhase, World};
use bota_core::{NeutralAbility, migration_of, rules};
use bota_proto::{AbilityId, DamageKind, Inflictor};

impl World {
    pub(crate) fn begin_journey(&mut self, unit: Entity, ability: Option<NeutralAbility>) {
        let lair = self
            .lairs
            .iter_mut()
            .find(|l| l.unit == Some(unit))
            .expect("journey belongs to a lair");
        lair.journey = ability.map(|ability| {
            assert!(migration_of(ability).is_some());
            Journey {
                ability,
                stops: 0,
                throws: 0,
            }
        });
    }
    pub(crate) fn end_journey(&mut self, unit: Entity) {
        if let Some(lair) = self.lairs.iter_mut().find(|l| l.unit == Some(unit)) {
            lair.journey = None;
        }
        if let Some(ai) = self.neutral_ai.get_mut(unit) {
            ai.roused_by = None;
        }
    }
    pub(crate) fn journey(&self, unit: Entity) -> Option<(NeutralAbility, u8, u8)> {
        let j = self.lairs.iter().find(|l| l.unit == Some(unit))?.journey?;
        Some((j.ability, j.stops, j.throws))
    }
    pub(crate) fn journey_can_cast(&self, unit: Entity, spell: NeutralAbility) -> bool {
        let Some((ability, stops, _)) = self.journey(unit) else {
            return false;
        };
        let Some(policy) = migration_of(ability) else {
            return false;
        };
        self.migrating(unit)
            && stops < policy.max_stops
            && policy.interrupts.contains(&spell)
            && self.alive(unit)
            && !self.held(unit)
            && !self.feared(unit)
            && self.pending_cast(unit).is_none()
            && !matches!(
                self.action.get(unit).map(|a| a.state),
                None | Some(ActionState::CastAbility { .. } | ActionState::UseItem { .. })
            )
    }
    pub(crate) fn note_journey_stop(&mut self, unit: Entity, id: AbilityId) {
        let Some(spell) = NeutralAbility::of(id) else {
            return;
        };
        let Some((ability, stops, throws)) = self.journey(unit) else {
            return;
        };
        let Some(policy) = migration_of(ability) else {
            return;
        };
        if !policy.interrupts.contains(&spell) || stops >= policy.max_stops {
            return;
        }
        self.lairs
            .iter_mut()
            .find(|l| l.unit == Some(unit))
            .expect("lair")
            .journey = Some(Journey {
            ability,
            stops: stops + 1,
            throws,
        });
    }
    pub(crate) fn note_journey_throw(&mut self, unit: Entity) {
        let Some((ability, stops, throws)) = self.journey(unit) else {
            return;
        };
        let cap = migration_of(ability).expect("journey policy").max_stops;
        self.lairs
            .iter_mut()
            .find(|l| l.unit == Some(unit))
            .expect("lair")
            .journey = Some(Journey {
            ability,
            stops,
            throws: throws.saturating_add(1).min(cap),
        });
    }
    pub(crate) fn tick_journey_contacts(&mut self) {
        for index in 0..self.lairs.len() {
            let Some(unit) = self.lairs[index]
                .unit
                .filter(|u| self.lairs[index].travelling && self.alive(*u))
            else {
                continue;
            };
            let Some((ability, _, _)) = self.journey(unit) else {
                continue;
            };
            let Some(at) = self.pos(unit) else {
                continue;
            };
            if self.stats.get(unit).is_some_and(|s| s.flies)
                || self.push.get(unit).is_some_and(|p| p.carries())
            {
                continue;
            }
            let policy = migration_of(ability).expect("movement policy");
            for foe in self.foes_within(unit, at, rules::units(policy.contact.radius)) {
                if self.push.contains(foe) {
                    continue;
                }
                self.journey_contact(unit, foe, ability, true);
            }
        }
    }
    fn start_contact_clock(
        &mut self,
        source: Entity,
        on: Entity,
        ability: AbilityId,
        effect: bota_proto::EffectId,
        wait: u32,
    ) -> bool {
        if !self.contacts.contains(on) {
            self.contacts.insert(on, crate::game::Contacts::default());
        }
        let clocks = self.contacts.get_mut(on).expect("contact clocks");
        clocks.0.retain(|c| c.until > self.tick);
        assert!(clocks.0.len() <= crate::game::MAX_CONTACT_SOURCES);
        if clocks
            .0
            .iter()
            .any(|c| c.source == source && c.ability == ability)
            || clocks.0.len() == crate::game::MAX_CONTACT_SOURCES
        {
            return false;
        }
        clocks.0.push(crate::game::ContactClock {
            source,
            ability,
            effect,
            until: self.tick.saturating_add(wait),
        });
        assert!(clocks.0.len() <= crate::game::MAX_CONTACT_SOURCES);
        true
    }

    pub(crate) fn journey_contact(
        &mut self,
        source: Entity,
        on: Entity,
        ability: NeutralAbility,
        moves: bool,
    ) {
        let Some(policy) = migration_of(ability) else {
            return;
        };
        let contact = policy.contact;
        if !self.start_contact_clock(
            source,
            on,
            ability.id(),
            contact.timeout_effect,
            contact.cooldown,
        ) {
            return;
        }
        if moves
            && !self.held_by_hook(on)
            && !self.stats.get(on).is_some_and(|s| s.forced_move_immune)
        {
            let (Some(from), Some(at)) = (self.pos(source), self.pos(on)) else {
                return;
            };
            let aim = bota_core::clamp_to_map(bota_core::point_along(
                at,
                if from == at {
                    at + bota_proto::Vec2::from_ints(1, 0)
                } else {
                    at + (at - from)
                },
                rules::units(contact.distance),
            ));
            self.push.insert(
                on,
                Push {
                    source,
                    inflictor: Inflictor::Ability(ability.id()),
                    team: self
                        .team
                        .get(source)
                        .copied()
                        .unwrap_or(bota_proto::Team::Neutral),
                    aim,
                    speed: rules::units(250),
                    clear: at,
                    effect: Some(contact.effect),
                    phase: PushPhase::Arc {
                        from: at,
                        started: self.tick,
                        duration: contact.duration,
                        height: contact.height,
                        tree_radius: contact.tree_radius,
                        landing: None,
                    },
                },
            );
        }
        self.push_hit(
            Some(source),
            on,
            contact.damage,
            DamageKind::Magical,
            Some(Inflictor::Ability(ability.id())),
        );
    }
}
