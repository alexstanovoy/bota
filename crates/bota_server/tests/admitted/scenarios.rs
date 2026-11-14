//! The scripted matches of an admitted wire's corpus (`bota_core::ADMITTED_WIRES`).
//!
//! A build of the admitted wire plays them and writes what it recorded (`examples/admitted_corpus.rs`,
//! run by `tools/admitted.sh corpus`); this build plays them again and must record the same
//! (`tests/admitted.rs`). The orders are a fixed script: they depend on nothing but the catalogue
//! the corpus was written for and on what the match itself shows, so two builds that play the
//! catalogue's rules alike give each other's records, snapshots and world fingerprints bit for bit.
//!
//! The script only takes what every build since the first admitted wire has: a cheat match on the
//! demo map, the stat-change, item, gold, level, respawn and teleport cheats, and seat orders. A
//! change here changes the corpora's records: write them again for every admitted wire.

use std::collections::{BTreeMap, BTreeSet};

use bota_core::Catalogue;
use bota_proto::{
    AbilitySlot, Aim, Cheat, CreepSide, EventKind, ItemId, ItemSlot, MapId, ModifierSpec, Order,
    OutgoingOrder, Pick, RejectReason, ReplayRecord, ServerMsg, SlotId, Target, Team, TickMode,
    UnitKind, Vec2,
};
use bota_server::game::{MatchConfig, OrderDelay};
use bota_server::local::LocalMatch;

/// A scripted match, played on the items of a catalogue.
pub type Play = fn(Catalogue) -> Played;

/// The scripted matches, by the name their replay is filed under.
pub const SCENARIOS: [(&str, Play); 2] = [
    ("every_item", every_item),
    ("free_components", free_components),
];

/// A scripted match as one build played it.
pub struct Played {
    /// What a server's replay of the match holds after its header: the match start, then the
    /// records of every tick.
    pub records: Vec<ReplayRecord>,
    /// The world's fingerprint after every tick, folded.
    pub world: u64,
    /// Every message each seat was sent, encoded and folded in seat order.
    pub views: u64,
    /// The items a hero of the match held as a fight began.
    pub held: BTreeSet<ItemId>,
    /// The items a hero carried while its attack struck a lane creep: what answers to creeps
    /// (Quelling Blade's bonus) came up.
    pub struck_creeps: BTreeSet<ItemId>,
    /// The items whose use took effect: the match told of it (`EventKind::ItemUsed`).
    pub used: BTreeSet<ItemId>,
    /// The channelled items whose channel ran to its end: the hero arrived
    /// (`EventKind::Teleported`, landed).
    pub channelled: BTreeSet<ItemId>,
    /// The uses refused, and why.
    pub refused: Vec<(ItemId, RejectReason)>,
}

/// Folds bytes into a running FNV-1a.
pub fn fold(hash: &mut u64, bytes: &[u8]) {
    for &byte in bytes {
        *hash = (*hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
}

/// Where a running FNV-1a starts.
pub const FOLD_START: u64 = 0xcbf2_9ce4_8422_2325;

/// Where the two heroes fight: the middle of the demo map's lane, trees at hand, a raze's reach
/// apart.
const FIGHT: [Vec2; 2] = [Vec2::from_ints(9000, 9216), Vec2::from_ints(9200, 9216)];

/// The most ticks a use is waited for: walking to a tree and a scroll's channel included.
const USE_TICKS: u32 = 150;

/// Ticks the heroes fight lane creeps a wave.
const CREEP_TICKS: u32 = 45;

/// Ticks a wave's fight lasts.
const FIGHT_TICKS: u32 = 260;

/// Items a hero carries into one fight: its inventory.
const CARRIED: usize = 6;

/// Shadow Fiend (Radiant) against Pudge (Dire): every item of `catalogue` but the retired, six a
/// hero a wave, bought at the fountain where the shop sells it and handed out by the item cheat
/// where it does not; then both heroes stand a raze apart with status resistance on (16%, which
/// shortens the slows), each strikes a lane creep of a wave stood up beside it, uses every item
/// it carries (each use waited for until the match tells of it), attack, raze, rot, hook and
/// dismember, use everything again, and once more refreshed. Spell damage, cooldowns, cast ranges and walking into them, regeneration,
/// slows and damage over time all come up.
pub fn every_item(catalogue: Catalogue) -> Played {
    let mut script = Script::start(catalogue);
    // No lane waves: the two heroes alone, a small replay.
    script.order(
        0,
        Order::Cheat {
            cheat: Cheat::CreepSpawning { on: false },
        },
    );
    for seat in 0..2 {
        script.order(
            seat,
            Order::Cheat {
                cheat: Cheat::Levels { count: 24 },
            },
        );
    }
    script.step();
    for _ in 0..4 {
        for slot in 0..6 {
            for seat in 0..2 {
                script.order(
                    seat,
                    Order::Learn {
                        slot: AbilitySlot(slot),
                    },
                );
            }
            script.step();
        }
    }
    let (sold, given): (Vec<ItemId>, Vec<ItemId>) = (0..catalogue.items())
        .map(ItemId)
        .filter(|&item| catalogue.item(item).is_some())
        .partition(|&item| catalogue.sold(item).is_some());
    // The items no shop sells come last: some of them (the Aegis) cannot be sold to make room.
    let items: Vec<ItemId> = sold.into_iter().chain(given).collect();
    let bags = bags(catalogue, &items);
    for wave in bags.chunks(2) {
        let none = Vec::new();
        script.wave([&wave[0], wave.get(1).unwrap_or(&none)]);
    }
    script.played
}

/// Unlocked components of later recipes, bought together through the shop. The records also
/// guard assembly: recipes outside a match's catalogue must not take the components away.
fn free_components(catalogue: Catalogue) -> Played {
    let mut script = Script::start(catalogue);
    script.order(
        0,
        Order::Cheat {
            cheat: Cheat::CreepSpawning { on: false },
        },
    );
    for seat in 0..2 {
        script.order(
            seat,
            Order::Cheat {
                cheat: Cheat::Gold { amount: 60_000 },
            },
        );
    }
    script.step();
    let bundles: [&[&str]; 2] = [
        &["item_boots", "item_wind_lace", "item_ring_of_regen"],
        &["item_relic", "item_talisman_of_evasion"],
    ];
    for (seat, keys) in bundles.into_iter().enumerate() {
        for key in keys {
            let item = (0..catalogue.items())
                .map(ItemId)
                .find(|&item| catalogue.item(item).is_some_and(|def| def.key == *key))
                .expect("the components belong to every admitted catalogue");
            script.order(seat, Order::Buy { item });
            script.step();
        }
    }
    script.fight();
    script.played
}

/// The items in bags of six, Shadow Fiend's and Pudge's in turn, each item in the first bag it
/// fits: one where it completes no build's parts and holds none of its own, parts of parts too (a
/// bag holding them all would put them together, and buying a build takes the parts held: either
/// way they would not be carried into the fight), and Pudge's for one that blocks magical blows, as Shadow Fiend's razes
/// are the large ones of the match.
fn bags(catalogue: Catalogue, items: &[ItemId]) -> Vec<Vec<ItemId>> {
    let parts_of = |item: ItemId| -> Vec<&'static [ItemId]> {
        catalogue.item(item).map_or_else(Vec::new, |def| {
            std::iter::once(def.components)
                .chain(def.alternatives.iter().map(|recipe| recipe.parts))
                .filter(|parts| !parts.is_empty())
                .collect()
        })
    };
    // Everything a purchase of the item buys or takes on the way, parts of parts too.
    let tree_of = |item: ItemId| -> BTreeSet<ItemId> {
        let mut tree = BTreeSet::new();
        let mut open = vec![item];
        while let Some(next) = open.pop() {
            for &part in parts_of(next).iter().copied().flatten() {
                if tree.insert(part) {
                    open.push(part);
                }
            }
        }
        tree
    };
    let builds: Vec<&[ItemId]> = (0..catalogue.items())
        .flat_map(|item| parts_of(ItemId(item)))
        .collect();
    let mut bags: Vec<Vec<ItemId>> = Vec::new();
    for &item in items {
        let blocks = catalogue
            .item(item)
            .is_some_and(|def| def.magic_block.is_some());
        let fits = |at: usize, bag: &Vec<ItemId>| {
            let builds_one = builds.iter().any(|parts| {
                parts.iter().any(|&part| part != item)
                    && parts
                        .iter()
                        .all(|&part| part == item || bag.contains(&part))
            });
            let takes_one = tree_of(item).iter().any(|part| bag.contains(part));
            bag.len() < CARRIED && !builds_one && !takes_one && (!blocks || at % 2 == 1)
        };
        let at = match bags.iter().enumerate().position(|(at, bag)| fits(at, bag)) {
            Some(at) => at,
            None => {
                bags.push(Vec::new());
                if blocks && bags.len() % 2 == 1 {
                    bags.push(Vec::new());
                }
                bags.len() - 1
            }
        };
        bags[at].push(item);
    }
    bags
}

/// A scripted match being played.
struct Script {
    game: LocalMatch,
    catalogue: Catalogue,
    /// Each seat's orders for the next tick.
    orders: [Vec<OutgoingOrder>; 2],
    /// The last sequence number given.
    seq: u32,
    /// The sequence numbers refused, and why.
    refused: BTreeMap<u32, RejectReason>,
    /// The last item each seat's hero was told to have used, since it was last cleared.
    last_used: [Option<ItemId>; 2],
    /// Whether each seat's last channel since it was last cleared ended, and landed.
    channel_end: [Option<bool>; 2],
    /// The lane creeps a creep fight stood up, as the world names them.
    creeps: BTreeSet<bota_proto::EntityId>,
    /// Every tree of the map.
    trees: Vec<Vec2>,
    /// The trees a use was aimed at, by their index in `trees`: none is aimed at twice.
    felled: BTreeSet<usize>,
    played: Played,
}

impl Script {
    fn start(catalogue: Catalogue) -> Script {
        let map = MapId(1);
        let pick = |slot, team, hero| Pick {
            slot: SlotId(slot),
            team,
            hero,
        };
        let cfg = MatchConfig {
            starting_gold_lost: Vec::new(),
            economy_events: Vec::new(),
            match_id: 774,
            stream: 774,
            master_key: [3; 32],
            picks: vec![
                pick(0, Team::Radiant, bota_proto::HeroId::SHADOW_FIEND),
                pick(1, Team::Dire, bota_proto::HeroId::PUDGE),
            ],
            map,
            mode: TickMode::Lockstep,
            ack_timeout_ticks: 0,
            cheats: true,
            spawn_modifiers: Vec::new(),
            immortal_structures: false,
            immortal_heroes: false,
            objectives: bota_core::map_of(map).default_objectives,
            runes: bota_core::map_of(map).default_runes,
            catalogue,
        };
        let (game, opening) = LocalMatch::new_recorded(&cfg, &[OrderDelay::NONE, OrderDelay::NONE])
            .expect("the scripted match starts");
        let mut played = Played {
            records: Vec::new(),
            world: FOLD_START,
            views: FOLD_START,
            held: BTreeSet::new(),
            struck_creeps: BTreeSet::new(),
            used: BTreeSet::new(),
            channelled: BTreeSet::new(),
            refused: Vec::new(),
        };
        let mut trees = Vec::new();
        for message in opening.iter().flatten() {
            fold(&mut played.views, &encode(message));
            if let ServerMsg::MatchStart { info } = message {
                trees.clone_from(&info.trees);
            }
        }
        played
            .records
            .push(ReplayRecord::Msg(opening[0][0].clone()));
        let mut script = Script {
            game,
            catalogue,
            orders: [Vec::new(), Vec::new()],
            seq: 0,
            refused: BTreeMap::new(),
            last_used: [None; 2],
            channel_end: [None; 2],
            creeps: BTreeSet::new(),
            trees,
            felled: BTreeSet::new(),
            played,
        };
        script.take_records();
        script
    }

    /// Gives `seat` an order for the next tick.
    fn order(&mut self, seat: usize, order: Order) -> u32 {
        self.seq += 1;
        self.orders[seat].push(OutgoingOrder {
            seq: self.seq,
            unit: None,
            order,
            seen: self.game.tick(),
            queue: false,
        });
        self.seq
    }

    /// Advances one tick with the orders given, folding everything it told.
    fn step(&mut self) {
        let orders = std::mem::take(&mut self.orders);
        let streams = self
            .game
            .step(&[&orders[0], &orders[1]])
            .expect("the scripted match steps");
        fold(
            &mut self.played.world,
            &self.game.world().hash().to_le_bytes(),
        );
        for message in streams.iter().flatten() {
            fold(&mut self.played.views, &encode(message));
            if let ServerMsg::OrderRejected { seq, reason } = message {
                self.refused.insert(*seq, *reason);
            }
        }
        self.take_records();
    }

    /// Keeps the tick's records, and notes what their events tell of the heroes: the uses that
    /// took effect, channels ending, blows on the creeps.
    fn take_records(&mut self) {
        let records = self.game.take_records();
        let heroes = [0, 1].map(|seat| self.game.hero_of(seat).map(bota_server::game::wire_id));
        for record in &records {
            let ReplayRecord::Msg(ServerMsg::Events { events, .. }) = record else {
                continue;
            };
            for event in events {
                self.note(event, heroes);
            }
        }
        self.played.records.extend(records);
    }

    fn note(&mut self, event: &EventKind, heroes: [Option<bota_proto::EntityId>; 2]) {
        let seat_of = |unit: Option<bota_proto::EntityId>| {
            unit.and_then(|unit| heroes.iter().position(|&hero| hero == Some(unit)))
        };
        match *event {
            EventKind::ItemUsed { caster, item, .. } => {
                if let Some(seat) = seat_of(caster) {
                    self.played.used.insert(item);
                    self.last_used[seat] = Some(item);
                }
            }
            EventKind::Teleported { slot, landed } => {
                // The two seats are slots 0 and 1.
                if let Some(end) = self.channel_end.get_mut(usize::from(slot.0)) {
                    *end = Some(landed);
                }
            }
            EventKind::Damaged {
                source,
                target,
                inflictor: None,
                ..
            } if self.creeps.contains(&target) => {
                if let Some(seat) = seat_of(source) {
                    let carried = self.carried(seat);
                    self.played
                        .struck_creeps
                        .extend(carried.into_iter().map(|(_, item)| item));
                }
            }
            _ => {}
        }
    }

    fn steps(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.step();
        }
    }

    /// One wave: both heroes back at their fountains, their bags emptied and filled with the
    /// wave's items, then a fight.
    fn wave(&mut self, bags: [&Vec<ItemId>; 2]) {
        for seat in 0..2 {
            self.order(
                seat,
                Order::Cheat {
                    cheat: Cheat::Respawn,
                },
            );
            self.order(
                seat,
                Order::Cheat {
                    cheat: Cheat::Gold { amount: 60_000 },
                },
            );
        }
        self.step();
        for slot in (0..bota_proto::HERO_ITEM_SLOTS).chain([bota_proto::TOWN_PORTAL_SLOT]) {
            for seat in 0..2 {
                self.order(
                    seat,
                    Order::Sell {
                        slot: ItemSlot(slot as u8),
                    },
                );
            }
            self.step();
        }
        for at in 0..CARRIED {
            for (seat, items) in bags.into_iter().enumerate() {
                if let Some(&item) = items.get(at) {
                    let order = if self.on_sale(item) {
                        Order::Buy { item }
                    } else {
                        Order::Cheat {
                            cheat: Cheat::Item { item },
                        }
                    };
                    self.order(seat, order);
                }
            }
            self.step();
        }
        self.fight();
    }

    /// Whether the shop sells `item` now: one sold from a stock that opens later (Infused
    /// Raindrops, at 4:30) is handed out by the item cheat instead.
    fn on_sale(&self, item: ItemId) -> bool {
        self.catalogue.sold(item).is_some_and(|def| {
            def.stock
                .is_none_or(|stock| stock.initial > 0 || stock.opens_ticks <= self.game.tick())
        })
    }

    /// The heroes a raze apart, each with 16% status resistance put on by the other, use what they
    /// carry, fight with attacks and spells, and use it all again.
    fn fight(&mut self) {
        for (seat, &at) in FIGHT.iter().enumerate() {
            self.order(
                seat,
                Order::Cheat {
                    cheat: Cheat::Teleport { at },
                },
            );
        }
        self.step();
        for seat in 0..2 {
            let spec = ModifierSpec {
                status_resist: 1_600,
                ..ModifierSpec::NOMINAL
            };
            let target = self.enemy(seat);
            self.order(
                seat,
                Order::Cheat {
                    cheat: Cheat::ApplyModifier {
                        target,
                        spec,
                        ticks: 10 * FIGHT_TICKS,
                    },
                },
            );
            let carried = self.carried(seat);
            self.played
                .held
                .extend(carried.into_iter().map(|(_, item)| item));
        }
        self.step();
        self.creep_fight();
        self.use_all();
        let started = self.game.tick();
        for seat in 0..2 {
            let enemy = self.enemy(seat);
            self.order(seat, Order::Attack { target: enemy });
        }
        self.steps(8);
        let pudge = self.enemy(0);
        let fiend = self.enemy(1);
        // Shadow Fiend's razes first, then Pudge's spells, which hold him.
        for raze in 0..3 {
            self.order(
                0,
                Order::Cast {
                    slot: AbilitySlot(raze),
                    target: Target::None,
                },
            );
            self.steps(20);
        }
        let hook_at = self.position(0).map_or(Target::None, Target::Pos);
        for (slot, target) in [(1, Target::None), (0, hook_at), (3, fiend)] {
            self.order(
                1,
                Order::Cast {
                    slot: AbilitySlot(slot),
                    target,
                },
            );
            self.steps(20);
        }
        self.order(0, Order::Attack { target: pudge });
        self.order(1, Order::Attack { target: fiend });
        self.steps(40);
        // Mana spent and the wands charged by the spells: what the first uses left are taken now.
        self.use_all();
        // Waits of a use shared between items (the mangoes') cleared, so each is used once.
        for seat in 0..2 {
            self.order(
                seat,
                Order::Cheat {
                    cheat: Cheat::Refresh,
                },
            );
        }
        self.step();
        self.use_all();
        let spent = self.game.tick() - started;
        self.steps(FIGHT_TICKS.saturating_sub(spent));
    }

    /// Each hero stands up a wave of enemy lane creeps beside it and attacks the nearest creep
    /// of it for a while; then every creep is taken away.
    fn creep_fight(&mut self) {
        for seat in 0..2 {
            if let Some(at) = self.position(seat) {
                let at = at + Vec2::from_ints(0, if seat == 0 { -300 } else { 300 });
                self.order(
                    seat,
                    Order::Cheat {
                        cheat: Cheat::CreateCreeps { enemy: true, at },
                    },
                );
            }
        }
        self.step();
        let world = self.game.world();
        let creeps: Vec<(bota_server::game::Entity, Team)> = world
            .entities
            .iter()
            .filter(|&unit| {
                matches!(
                    world.kind.get(unit),
                    Some(
                        UnitKind::CreepMelee
                            | UnitKind::CreepRanged
                            | UnitKind::CreepFlagbearer
                            | UnitKind::CreepSiege
                    )
                )
            })
            .filter_map(|unit| Some((unit, *world.team.get(unit)?)))
            .collect();
        self.creeps = creeps
            .iter()
            .map(|&(unit, _)| bota_server::game::wire_id(unit))
            .collect();
        for seat in 0..2 {
            let (Some(hero), Some(at)) = (self.game.hero_of(seat), self.position(seat)) else {
                continue;
            };
            let team = self.game.world().team.get(hero).copied();
            let nearest = creeps
                .iter()
                .filter(|&&(_, side)| Some(side) != team)
                .filter_map(|&(unit, _)| {
                    let stands = self.game.world().transform.get(unit)?.pos;
                    Some((stands.distance_squared(at), unit))
                })
                .min_by_key(|&(apart, _)| apart)
                .and_then(|(_, unit)| self.game.handle_of(seat, unit));
            if let Some(creep) = nearest {
                self.order(
                    seat,
                    Order::Attack {
                        target: Target::Unit(creep),
                    },
                );
            }
        }
        self.steps(CREEP_TICKS);
        self.order(
            0,
            Order::Cheat {
                cheat: Cheat::KillCreeps {
                    which: CreepSide::All,
                },
            },
        );
        self.step();
        self.creeps.clear();
    }

    /// Each hero in turn uses every item it carries, one at a time: each use is waited for until
    /// the match tells of it (a channel: until it ends), it is refused, or [`USE_TICKS`] pass.
    fn use_all(&mut self) {
        for seat in 0..2 {
            for (slot, item) in self.carried(seat) {
                let Some(def) = self.catalogue.item(item) else {
                    continue;
                };
                let Some(aim) = def.aim else {
                    continue;
                };
                let channel = def.behavior.has(bota_core::Behavior::CHANNELLED);
                let target = self.aim(seat, aim);
                if aim == Aim::Tree {
                    self.fell(target);
                }
                self.last_used[seat] = None;
                self.channel_end[seat] = None;
                let seq = self.order(
                    seat,
                    Order::Use {
                        slot: ItemSlot(slot as u8),
                        target,
                    },
                );
                for _ in 0..USE_TICKS {
                    self.step();
                    if let Some(&reason) = self.refused.get(&seq) {
                        self.played.refused.push((item, reason));
                        break;
                    }
                    let used = self.last_used[seat] == Some(item);
                    if channel {
                        if used && let Some(landed) = self.channel_end[seat] {
                            if landed {
                                self.played.channelled.insert(item);
                            }
                            break;
                        }
                    } else if used {
                        break;
                    }
                }
            }
        }
    }

    /// What a seat aims a use of an item of `aim` at: itself, the enemy hero's spot, the nearest
    /// tree no use was aimed at before, its fountain.
    fn aim(&self, seat: usize, aim: Aim) -> Target {
        match aim {
            Aim::Own | Aim::Building => Target::None,
            Aim::Point => self.position(1 - seat).map_or(Target::None, Target::Pos),
            Aim::Unit => self.own(seat),
            Aim::Tree => self
                .position(seat)
                .and_then(|at| {
                    self.trees
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| !self.felled.contains(index))
                        .min_by_key(|(_, tree)| tree.distance_squared(at))
                })
                .map_or(Target::None, |(_, &tree)| Target::Pos(tree)),
        }
    }

    /// Notes the tree a use is aimed at as one no later use aims at: the use may fell it.
    fn fell(&mut self, target: Target) {
        if let Target::Pos(at) = target
            && let Some(index) = self.trees.iter().position(|&tree| tree == at)
        {
            self.felled.insert(index);
        }
    }

    /// The slots of a seat's hero holding something, the town portal slot last, and what.
    fn carried(&self, seat: usize) -> Vec<(usize, ItemId)> {
        let world = self.game.world();
        let bag = self
            .game
            .hero_of(seat)
            .and_then(|hero| world.inventory.get(hero).cloned());
        let mut held = Vec::new();
        if let Some(bag) = bag {
            for (slot, stack) in bag
                .slots
                .iter()
                .enumerate()
                .take(bota_proto::INVENTORY_SLOTS)
            {
                if let Some(stack) = stack {
                    held.push((slot, stack.id));
                }
            }
            if let Some(Some(stack)) = bag.town_portal {
                held.push((bota_proto::TOWN_PORTAL_SLOT, stack.id));
            }
        }
        held
    }

    /// Where a seat's hero stands.
    fn position(&self, seat: usize) -> Option<Vec2> {
        let hero = self.game.hero_of(seat)?;
        Some(self.game.world().transform.get(hero)?.pos)
    }

    /// A seat's hero as that seat names it.
    fn own(&self, seat: usize) -> Target {
        self.named(seat, seat)
    }

    /// The other seat's hero as `seat` names it.
    fn enemy(&self, seat: usize) -> Target {
        self.named(seat, 1 - seat)
    }

    fn named(&self, seat: usize, of: usize) -> Target {
        self.game
            .hero_of(of)
            .and_then(|hero| self.game.handle_of(seat, hero))
            .map_or(Target::None, Target::Unit)
    }
}

fn encode(message: &ServerMsg) -> Vec<u8> {
    bota_proto::encode_frame_to_vec(message).expect("a message the server sent encodes")
}
