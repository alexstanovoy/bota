//! A trainer-like Map2 mid skirmish of two Shadow Fiends on a fixed
//! order stream, each tick advanced, then seen by both sides with their
//! events.

use bota_proto::{
    AbilitySlot, EventKind, HeroId, ItemId, Order, Pick, SlotId, Target, Team, TickMode, Vec2,
    WorldView,
};

use crate::game::{Command, EventVisibility, Fnv, ITEM_MANGO, MAP2_ID, MatchConfig, World, rules};

/// Ticks between two decisions of a seat.
const DECISION_TICKS: u32 = 3;
/// Identity and seed of the skirmish.
const MATCH_ID: u64 = 11;
/// The Shadow Fiend.
const SHADOW_FIEND: HeroId = HeroId(2);
/// Half the side of the square the heroes fight about, in world units.
const SPREAD: u32 = 500;

/// A Map2 match between two Shadow Fiends and the stream that drives them.
pub(crate) struct Skirmish {
    /// The world the match is played in.
    pub(crate) world: World,
    /// Where each side's hero fights, Radiant first: a sixth of the way
    /// from its own first mid tower to the other's.
    fronts: [Vec2; 2],
    /// The xorshift state the order stream draws from.
    state: u64,
    /// The fingerprint of every view and event list handed out so far.
    digest: Fnv,
}

impl Skirmish {
    /// The skirmish at tick zero.
    pub(crate) fn build() -> Skirmish {
        let cfg = match_config();
        let world = World::for_match(&cfg, cfg.rng());
        let tower = |towers: &[(u8, u8, Vec2)]| {
            towers
                .iter()
                .find(|&&(lane, tier, _)| lane == rules::LANE_MID && tier == 1)
                .map(|&(_, _, at)| at)
                .expect("a first mid tower")
        };
        let (west, east) = (
            tower(world.map.radiant_towers),
            tower(world.map.dire_towers),
        );
        let third = |from: Vec2, to: Vec2| {
            Vec2::from_ints(
                from.x.to_int() + (to.x.to_int() - from.x.to_int()) / 6,
                from.y.to_int() + (to.y.to_int() - from.y.to_int()) / 6,
            )
        };
        Skirmish {
            world,
            fronts: [third(west, east), third(east, west)],
            state: 0x5eed_0000_0000_0001,
            digest: Fnv::new(),
        }
    }

    /// One tick: the orders due, validated, the world advanced, and
    /// both sides' views and events taken.
    pub(crate) fn step(&mut self) {
        let mut commands = Vec::new();
        if self.world.tick.is_multiple_of(DECISION_TICKS) {
            for (slot, front) in [SlotId(0), SlotId(1)].into_iter().zip(self.fronts) {
                let order = self.order(front);
                if self.world.validate_order(slot, None, &order).is_ok() {
                    commands.push(Command {
                        slot,
                        unit: None,
                        order,
                    });
                }
            }
        }
        let events = self.world.advance(&commands);
        for team in [Team::Radiant, Team::Dire] {
            let view = self.world.view(team);
            let told: Vec<EventKind> = events
                .iter()
                .filter(|event| match event.visible_to {
                    EventVisibility::Everyone => true,
                    EventVisibility::OneTeam(side) => side == team,
                })
                .map(|event| event.kind.clone())
                .collect();
            fold(&mut self.digest, &view, told.len());
        }
    }

    /// The world fingerprint and the fingerprint of everything handed out.
    pub(crate) fn digest(&self) -> (u64, u64) {
        (self.world.hash(), self.digest.done())
    }

    /// The next order of the stream: mostly a fight about a front, now and
    /// then a raze, a point learned or a mango bought.
    fn order(&mut self, front: Vec2) -> Order {
        match self.draw() % 12 {
            0 => Order::Cast {
                slot: AbilitySlot((self.draw() % 3) as u8),
                target: Target::None,
            },
            1 => Order::Learn {
                slot: AbilitySlot((self.draw() % 4) as u8),
            },
            2 => Order::Buy {
                item: ItemId(ITEM_MANGO),
            },
            _ => {
                let span = u64::from(SPREAD) * 2 + 1;
                let dx = (self.draw() % span) as i32 - SPREAD as i32;
                let dy = (self.draw() % span) as i32 - SPREAD as i32;
                Order::Attack {
                    target: Target::Pos(Vec2::from_ints(
                        front.x.to_int() + dx,
                        front.y.to_int() + dy,
                    )),
                }
            }
        }
    }

    /// The next draw of the stream.
    fn draw(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }
}

/// The configuration of the skirmish.
fn match_config() -> MatchConfig {
    MatchConfig {
        match_id: MATCH_ID,
        master_key: [0x11; 32],
        picks: vec![
            Pick {
                slot: SlotId(0),
                team: Team::Radiant,
                hero: SHADOW_FIEND,
            },
            Pick {
                slot: SlotId(1),
                team: Team::Dire,
                hero: SHADOW_FIEND,
            },
        ],
        map: MAP2_ID,
        tick_rate: rules::TICKS_PER_SECOND as u16,
        mode: TickMode::Lockstep,
        ack_timeout_ticks: 0,
        cheats: false,
        spawn_modifiers: Vec::new(),
        immortal_structures: false,
        immortal_heroes: false,
    }
}

/// Folds one side's view and the count of its events into a fingerprint.
fn fold(hash: &mut Fnv, view: &WorldView, events: usize) {
    hash.u32(view.tick);
    hash.u32(events as u32);
    hash.u32(view.units.len() as u32);
    for unit in &view.units {
        hash.u32(unit.id.idx);
        hash.u32(unit.id.generation);
        hash.vec2(unit.pos);
        hash.i32(unit.hp);
        hash.i32(unit.mana);
        hash.u32(unit.effects.len() as u32);
    }
    hash.u32(view.projectiles.len() as u32);
    hash.u32(view.loot.len() as u32);
}
