//! The recorder: watches the records of a replay and writes the metadata file of its match.
//!
//! It reads only what the replay's records hold, the match's start, the whole-map views, the
//! batches of events and the end, and never the clock, a connection or chat, so what it writes
//! is a function of the records alone: the server runs it while the match goes on, and a tool
//! running it over the replay afterwards writes the same bytes. It is read-only on the match.

use std::collections::HashMap;

use bota_proto::{
    EntityId, EventKind, ItemId, MatchInfo, MatchStats, PlayerView, ReplayRecord, ServerMsg,
    SlotId, Team, UnitKind, WorldView,
};

use super::record::{MetaHeader, MetaRecord, Who, stat};
use super::write::Writer;
use crate::limits::{MAX_MINUTES, MAX_TRACKED};

const MINUTE_TICKS: u32 = 60 * bota_core::rules::TICKS_PER_SECOND;

/// Everything about one seat that the file counts up from events.
#[derive(Clone, Copy, Default)]
struct Tally {
    hero_damage: i64,
    structure_damage: i64,
    healing: i64,
}

/// What a seat held at the latest view.
#[derive(Clone, Default)]
struct Held {
    slot: u8,
    team: u8,
    stats: Vec<(u16, i64)>,
    items: Vec<Option<u16>>,
}

pub struct Recorder {
    out: Writer,
    pregame: u32,
    /// The seats, in the order the first view lists them.
    seats: Vec<Held>,
    tallies: Vec<Tally>,
    /// Every hero body seen, with the seat that drove it (an index into `seats`).
    bodies: HashMap<EntityId, usize>,
    /// Every hero seen on the map, a created one and an illusion among them, with its side: what
    /// damage to a hero is counted against.
    heroes: HashMap<EntityId, Team>,
    /// The buildings of the first view, with their kind and side.
    structures: HashMap<EntityId, (UnitKind, Team)>,
    tick: u32,
    next_minute: u32,
    started: bool,
    ended: bool,
}

impl Default for Recorder {
    fn default() -> Self {
        Recorder {
            out: Writer::default(),
            pregame: bota_core::rules::PREGAME_TICKS,
            seats: Vec::new(),
            tallies: Vec::new(),
            bodies: HashMap::new(),
            heroes: HashMap::new(),
            structures: HashMap::new(),
            tick: 0,
            next_minute: 0,
            started: false,
            ended: false,
        }
    }
}

fn side(team: Team) -> Option<u8> {
    match team {
        Team::Radiant => Some(0),
        Team::Dire => Some(1),
        Team::Neutral => None,
    }
}

impl Recorder {
    pub fn new() -> Recorder {
        Recorder::default()
    }

    /// Takes the next record of a replay; those it has no use for pass unseen.
    pub fn push(&mut self, record: &ReplayRecord) {
        match record {
            ReplayRecord::Msg(ServerMsg::MatchStart { info }) => self.start(info),
            ReplayRecord::Msg(ServerMsg::Snapshot { view }) => self.view(view),
            ReplayRecord::Msg(ServerMsg::Events { tick, events }) => self.events(*tick, events),
            ReplayRecord::Msg(ServerMsg::MatchOver { winner, stats }) => self.over(*winner, stats),
            _ => {}
        }
    }

    /// The match begins.
    pub fn start(&mut self, info: &MatchInfo) {
        if self.started {
            return;
        }
        self.started = true;
        self.pregame = info.pregame_ticks;
        let header = MetaHeader {
            magic: MetaHeader::MAGIC,
            schema: MetaHeader::SCHEMA,
            wire_fingerprint: bota_core::WIRE_FINGERPRINT,
            rules_version: bota_core::RULES_VERSION,
            match_id: info.match_id,
            map: info.map.0,
            tick_rate: info.tick_rate,
            pregame_ticks: info.pregame_ticks,
            catalogue_items: info.catalogue.items,
            catalogue_digest: info.catalogue.digest,
        };
        self.out.push(&MetaRecord::Header(header), false);
    }

    /// The whole-map view of a tick.
    pub fn view(&mut self, view: &WorldView) {
        if !self.started || self.ended {
            return;
        }
        self.tick = view.tick;
        if self.seats.is_empty() {
            self.seat_up(view);
        }
        for player in &view.players {
            if let (Some(unit), Some(at)) = (player.unit, self.seat_of(player.slot))
                && self.bodies.len() < MAX_TRACKED
            {
                self.bodies.insert(unit, at);
            }
        }
        for unit in view.units.iter().filter(|u| u.kind == UnitKind::Hero) {
            if self.heroes.len() < MAX_TRACKED {
                self.heroes.entry(unit.id).or_insert(unit.team);
            }
        }
        for at in 0..self.seats.len() {
            if let Some(player) = view
                .players
                .iter()
                .find(|p| p.slot.0 == self.seats[at].slot)
            {
                self.hold(view, player, at);
            }
        }
        let minute = view.game_tick().saturating_sub(self.pregame) / MINUTE_TICKS;
        while self.next_minute <= minute.min(MAX_MINUTES) {
            self.sample(false);
            self.next_minute += 1;
        }
    }

    fn seat_of(&self, slot: SlotId) -> Option<usize> {
        self.seats.iter().position(|s| s.slot == slot.0)
    }

    /// The seats and the buildings of the first view.
    fn seat_up(&mut self, view: &WorldView) {
        for player in &view.players {
            let Some(team) = side(player.team) else {
                continue;
            };
            let slot = player.slot.0;
            self.seats.push(Held {
                slot,
                team,
                ..Held::default()
            });
            self.tallies.push(Tally::default());
            self.out.push(
                &MetaRecord::Seat {
                    slot,
                    team,
                    hero: player.hero.0,
                },
                false,
            );
        }
        for unit in &view.units {
            if bota_core::kind_def(unit.kind).structure && self.structures.len() < MAX_TRACKED {
                self.structures.insert(unit.id, (unit.kind, unit.team));
            }
        }
    }

    /// What a seat holds in this view, kept for the next sample.
    fn hold(&mut self, view: &WorldView, player: &PlayerView, at: usize) {
        let tally = self.tallies[at];
        let (last_hits, denies) = player
            .creep_score
            .map_or((0, 0), |c| (c.last_hits, c.denies));
        let carried = carried(view, player);
        let held = &mut self.seats[at];
        held.stats.clear();
        held.stats.extend([
            (stat::GOLD, i64::from(player.gold.unwrap_or(0))),
            (stat::NET_WORTH, i64::from(net_worth(view, player))),
            (stat::XP, i64::from(player.xp)),
            (stat::LEVEL, i64::from(player.level)),
            (stat::KILLS, i64::from(player.kills)),
            (stat::DEATHS, i64::from(player.deaths)),
            (stat::ASSISTS, i64::from(player.assists)),
            (stat::LAST_HITS, i64::from(last_hits)),
            (stat::DENIES, i64::from(denies)),
            (stat::HERO_DAMAGE, tally.hero_damage),
            (stat::STRUCTURE_DAMAGE, tally.structure_damage),
            (stat::HEALING, tally.healing),
        ]);
        held.items = carried.iter().map(|i| i.as_ref().map(|i| i.id.0)).collect();
    }

    /// A sample of every seat as of the latest view.
    fn sample(&mut self, closing: bool) {
        for at in 0..self.seats.len() {
            let tally = self.tallies[at];
            let held = &mut self.seats[at];
            // The damage and the healing the events told since the view.
            for (id, value) in [
                (stat::HERO_DAMAGE, tally.hero_damage),
                (stat::STRUCTURE_DAMAGE, tally.structure_damage),
                (stat::HEALING, tally.healing),
            ] {
                if let Some(entry) = held.stats.iter_mut().find(|(s, _)| *s == id) {
                    entry.1 = value;
                }
            }
            let record = MetaRecord::Sample {
                tick: self.tick,
                slot: held.slot,
                stats: held.stats.clone(),
            };
            self.out.push(&record, closing);
        }
    }

    /// A batch of events.
    pub fn events(&mut self, tick: u32, events: &[EventKind]) {
        if !self.started || self.ended {
            return;
        }
        for event in events {
            self.event(tick, event);
        }
    }

    fn event(&mut self, tick: u32, event: &EventKind) {
        match event {
            EventKind::Damaged {
                source: Some(source),
                target,
                amount,
                ..
            } => self.damage(*source, *target, *amount),
            EventKind::Healed {
                source: Some(source),
                target,
                amount,
                ..
            } => self.heal(*source, *target, *amount),
            EventKind::Died {
                unit,
                killer,
                bounty,
                ..
            } => self.death(tick, *unit, *killer, bounty.unwrap_or(0)),
            EventKind::ItemBought { slot, item } => {
                self.out.push(
                    &MetaRecord::Purchase {
                        tick,
                        slot: slot.0,
                        item: item.0,
                    },
                    false,
                );
            }
            EventKind::LevelUp { unit, level } => {
                if let Some(at) = self.bodies.get(unit) {
                    let slot = self.seats[*at].slot;
                    self.out.push(
                        &MetaRecord::LevelUp {
                            tick,
                            slot,
                            level: *level,
                        },
                        false,
                    );
                }
            }
            EventKind::StructureDestroyed { unit, team } => {
                if let (Some((kind, _)), Some(team)) = (self.structures.get(unit), side(*team)) {
                    self.out.push(
                        &MetaRecord::Structure {
                            tick,
                            kind: *kind as u8,
                            team,
                        },
                        false,
                    );
                }
            }
            _ => {}
        }
    }

    fn damage(&mut self, source: EntityId, target: EntityId, amount: i32) {
        let Some(&from) = self.bodies.get(&source) else {
            return;
        };
        let amount = i64::from(amount);
        if let Some(team) = self.heroes.get(&target) {
            if side(*team).is_some_and(|team| team != self.seats[from].team) {
                self.tallies[from].hero_damage += amount;
            }
        } else if let Some((_, team)) = self.structures.get(&target)
            && side(*team).is_some_and(|team| team != self.seats[from].team)
        {
            self.tallies[from].structure_damage += amount;
        }
    }

    fn heal(&mut self, source: EntityId, target: EntityId, amount: i32) {
        if let (Some(&from), Some(&to)) = (self.bodies.get(&source), self.bodies.get(&target))
            && self.seats[from].team == self.seats[to].team
            && amount > 0
        {
            self.tallies[from].healing += i64::from(amount);
        }
    }

    fn death(&mut self, tick: u32, unit: EntityId, killer: Option<EntityId>, gold: i32) {
        let who = |recorder: &Recorder, id: EntityId| {
            recorder
                .bodies
                .get(&id)
                .map(|&at| Who::Hero(recorder.seats[at].slot))
        };
        let victim = match who(self, unit) {
            Some(victim) => victim,
            None if self.heroes.contains_key(&unit) => Who::Other,
            None => return,
        };
        let killer = killer.map(|k| who(self, k).unwrap_or(Who::Other));
        self.out.push(
            &MetaRecord::Kill {
                tick,
                victim,
                killer,
                gold,
            },
            false,
        );
    }

    /// The match was decided: the final samples, what each hero carried, and the end.
    pub fn over(&mut self, winner: Team, stats: &MatchStats) {
        if !self.started || self.ended {
            return;
        }
        self.conclude(Some((winner, stats)));
    }

    fn conclude(&mut self, over: Option<(Team, &MatchStats)>) {
        self.ended = true;
        if let Some((_, stats)) = over {
            for earned in &stats.slots {
                if let Some(at) = self.seat_of(earned.slot) {
                    let value = i64::from(earned.net_worth);
                    self.seats[at].stats.push((stat::GOLD_EARNED, value));
                }
            }
        }
        self.sample(true);
        for held in &self.seats {
            let record = MetaRecord::Inventory {
                tick: self.tick,
                slot: held.slot,
                items: held.items.clone(),
            };
            self.out.push(&record, true);
        }
        if let Some((winner, _)) = over {
            self.out.push(
                &MetaRecord::End {
                    tick: self.tick,
                    winner: side(winner),
                },
                true,
            );
        }
    }

    /// The file. A match that was not decided is ended without an `End` record.
    pub fn finish(mut self) -> Vec<u8> {
        if self.started && !self.ended {
            self.conclude(None);
        }
        self.out.into_bytes()
    }
}

/// The items a seat's hero carries, inventory then backpack: those of its body, or of what it
/// fell with.
fn carried(view: &WorldView, player: &PlayerView) -> Vec<Option<bota_proto::ItemView>> {
    if let Some(unit) = player.unit.and_then(|id| view.unit(id)) {
        return unit.items.clone();
    }
    player
        .kit
        .as_ref()
        .map_or_else(Vec::new, |kit| kit.items.clone())
}

/// The gold a seat holds and the value of everything it carries and stashes, at what the rules
/// of this build price the items. The town portal slot is not counted: the scroll every hero starts with was never bought,
/// is not counted, so a hero that only bought and never sold or died is worth what it earned.
fn net_worth(view: &WorldView, player: &PlayerView) -> i32 {
    let held: i32 = carried(view, player)
        .iter()
        .flatten()
        .chain(player.stash.iter().flatten().flatten())
        .map(|item| cost(item.id))
        .sum();
    player.gold.unwrap_or(0).saturating_add(held)
}

fn cost(id: ItemId) -> i32 {
    bota_core::item_def(id).map_or(0, |def| def.cost)
}
