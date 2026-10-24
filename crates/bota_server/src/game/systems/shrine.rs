//! The Shrines of Wisdom, and the standing-zone countdown they run: Dota
//! 7.41's `ability_xp_fountain` and its contested countdown.

use bota_core::{ModifierKind, SHRINE_FIRST_TICK, shrine_activations, shrine_xp, side_index};
use bota_proto::{EventKind, ObjectiveKind, Team, UnitKind};

use crate::game::{Entity, Event, EventVisibility, Modifier, Purpose, World, wire_id};

impl World {
    /// Charges every Shrine on each activation of the game clock, from
    /// [`SHRINE_FIRST_TICK`] on, and runs each charged Shrine's countdown;
    /// one a side runs out is gathered by that side. A charge waits for its
    /// side however long it takes: one charge at most, never two.
    pub(crate) fn tick_shrines(&mut self) {
        if !self.objectives.has(ObjectiveKind::Shrines) {
            return;
        }
        let clock = self.clock();
        let activated = clock >= SHRINE_FIRST_TICK
            && shrine_activations(clock) != shrine_activations(clock - 1);
        let shrines: Vec<Entity> = self
            .entities
            .iter()
            .filter(|&entity| {
                self.kind.get(entity) == Some(&UnitKind::Shrine) && self.zone.get(entity).is_some()
            })
            .collect();
        for shrine in shrines {
            if activated && !self.wears(shrine, ModifierKind::ShrineReady) {
                self.put_modifier(
                    shrine,
                    Modifier::new(ModifierKind::ShrineReady, None, None, None),
                );
            }
            if !self.wears(shrine, ModifierKind::ShrineReady) {
                continue;
            }
            if let Some(side) = self.run_zone(shrine) {
                self.gather_shrine(shrine, side);
            }
        }
    }

    /// Runs the standing-zone countdown an entity keeps one tick: on while
    /// the heroes of its side alone stand within its radius, back down
    /// otherwise (contested, empty, or the other side's alone), and taken up
    /// by a side standing there alone once it is back at zero. Answers the
    /// side it ran out for this tick, starting it afresh.
    pub(crate) fn run_zone(&mut self, keeper: Entity) -> Option<Team> {
        let mut zone = *self.zone.get(keeper)?;
        let at = self.pos(keeper)?;
        let mut present = [false; 2];
        for unit in self.entities.iter() {
            let standing = self.kind.get(unit) == Some(&UnitKind::Hero)
                && self.illusion.get(unit).is_none()
                && self.alive(unit)
                && self
                    .pos(unit)
                    .is_some_and(|pos| pos.within(at, zone.radius));
            if let (true, Some(side)) = (standing, self.team.get(unit).copied())
                && let Some(index) = side_index(side)
            {
                present[index] = true;
            }
        }
        let alone = match present {
            [true, false] => Some(Team::Radiant),
            [false, true] => Some(Team::Dire),
            _ => None,
        };
        match zone.side {
            None => {
                if alone.is_some() {
                    zone.side = alone;
                    zone.progress = 1;
                }
            }
            Some(side) if alone == Some(side) => zone.progress += 1,
            Some(_) => {
                zone.progress -= 1;
                if zone.progress == 0 {
                    zone.side = None;
                }
            }
        }
        let done = zone.side.filter(|_| zone.progress >= zone.ticks);
        if done.is_some() {
            zone.side = None;
            zone.progress = 0;
        }
        self.zone.insert(keeper, zone);
        done
    }

    /// A side gathered a Shrine: the experience of this activation goes to
    /// one of its heroes standing in it, drawn at random, and to its hero
    /// with the least experience but that one [PN 7.38, 7.33c], and both
    /// sides are told.
    fn gather_shrine(&mut self, shrine: Entity, side: Team) {
        self.take_modifier(shrine, ModifierKind::ShrineReady, None);
        let activation = shrine_activations(self.clock());
        let xp = shrine_xp(activation);
        let Some(zone) = self.zone.get(shrine).copied() else {
            return;
        };
        let Some(at) = self.pos(shrine) else {
            return;
        };
        let gatherers: Vec<usize> = (0..self.seats.len())
            .filter(|&index| {
                let seat = &self.seats[index];
                seat.team == side
                    && seat.unit.is_some_and(|hero| {
                        self.alive(hero)
                            && self
                                .pos(hero)
                                .is_some_and(|pos| pos.within(at, zone.radius))
                    })
            })
            .collect();
        let gatherer = (!gatherers.is_empty()).then(|| {
            let mut draw = self
                .rng
                .for_unit(Purpose::Shrine, wire_id(shrine), activation as u8);
            gatherers[draw.below(gatherers.len() as u32) as usize]
        });
        let least = (0..self.seats.len())
            .filter(|&index| self.seats[index].team == side && Some(index) != gatherer)
            .min_by_key(|&index| (self.seats[index].xp, self.seats[index].slot));
        let mut events = Vec::new();
        for seat in gatherer.into_iter().chain(least) {
            self.grant_xp(seat, xp, &mut events);
        }
        self.events.append(&mut events);
        self.events.push(Event {
            kind: EventKind::ShrineActivated {
                unit: wire_id(shrine),
                team: side,
                xp,
                by: gatherer.and_then(|seat| self.seats[seat].unit).map(wire_id),
            },
            visible_to: EventVisibility::Everyone,
            gold_hidden_from: None,
            source_told_to: EventVisibility::OneTeam(side),
        });
    }
}
