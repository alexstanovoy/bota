//! Everything the client knows, and how server messages change it.

use bota_proto::{
    ClientMsg, EntityId, EventKind, HeroId, LobbySlot, MatchStats, Order, PlayerId, ServerMsg,
    SlotId, Team, TickMode, UnitKind, WorldView,
};

use crate::camera::Camera;
use crate::net::Net;
use crate::replay_play::ReplayPlayer;

/// Where messages come from: a socket or a file.
pub enum Source {
    /// A live match.
    Live(Net),
    /// A recorded one.
    Replay(Box<ReplayPlayer>),
}

impl Source {
    /// Everything due since the last frame.
    pub fn poll(&mut self, dt: f32) -> Vec<ServerMsg> {
        match self {
            Source::Live(net) => net.poll(),
            Source::Replay(player) => player.poll(dt),
        }
    }

    /// Sends upstream. A replay swallows everything.
    pub fn send(&mut self, msg: &ClientMsg) {
        if let Source::Live(net) = self {
            net.send(msg);
        }
    }
}

/// One seat's latest order, kept to be drawn over the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShownOrder {
    /// Which seat gave it.
    pub slot: SlotId,
    /// Which unit it was for. Absent means the seat's own hero.
    pub unit: Option<EntityId>,
    /// The tick it was given on, which decides how faded it is drawn.
    pub tick: u32,
    /// The order itself.
    pub order: Order,
}

/// Which screen the client is on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for seats, picks and readiness.
    Lobby,
    /// The match itself, including its end banner.
    Playing,
}

/// What the camera is carried by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pin {
    /// A seat's hero, whichever body it is standing in.
    ///
    /// A hero that dies comes back as a new entity, and a camera pinned to
    /// the body rather than the seat would be left behind by that.
    Hero(SlotId),
    /// One unit, for as long as it stands.
    Unit(EntityId),
}

/// Keeps the last state of every unit a seat owns.
///
/// Only what a seat owns is kept, which bounds this to a handful of entries:
/// an enemy hero out of sight is worth remembering, the creep wave it walked
/// past is not. A seat stands in one body at a time, so a body it has left
/// for a new one is dropped rather than piling up.
pub fn remember(seen: &mut Vec<(u32, bota_proto::UnitView)>, view: &WorldView) {
    for unit in view.units.iter().filter(|unit| unit.owner.is_some()) {
        match seen.iter_mut().find(|(_, held)| held.id == unit.id) {
            Some(held) => *held = (view.tick, unit.clone()),
            None => seen.push((view.tick, unit.clone())),
        }
    }
    seen.retain(|(_, held)| {
        view.units.iter().any(|unit| unit.id == held.id)
            || !view.units.iter().any(|unit| {
                unit.owner == held.owner && unit.kind == held.kind && unit.id != held.id
            })
    });
}

/// What is known of a unit, and how many ticks ago that was true.
pub fn known_in<'a>(
    seen: &'a [(u32, bota_proto::UnitView)],
    view: &'a WorldView,
    id: EntityId,
) -> Option<(&'a bota_proto::UnitView, u32)> {
    if let Some(live) = view.units.iter().find(|unit| unit.id == id) {
        return Some((live, 0));
    }
    seen.iter()
        .find(|(_, held)| held.id == id)
        .map(|(tick, held)| (held, view.tick.saturating_sub(*tick)))
}

/// The body a seat last stood in, whether or not it still stands.
///
/// A seat with its hero down has no unit on the wire at all, so the body it
/// left behind is the only handle there is to pick it by.
pub fn body_in(
    seen: &[(u32, bota_proto::UnitView)],
    view: &WorldView,
    slot: SlotId,
) -> Option<EntityId> {
    if let Some(unit) = view
        .players
        .iter()
        .find(|p| p.slot == slot)
        .and_then(|p| p.unit)
    {
        return Some(unit);
    }
    seen.iter()
        .find(|(_, held)| held.owner == Some(slot) && held.kind == UnitKind::Hero)
        .map(|(_, held)| held.id)
}

/// What was last reached for, so a second reach for the same thing counts as
/// a pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tap {
    /// The key for one's own hero.
    Hero,
    /// The key for one's own courier.
    Courier,
    /// A unit, clicked in the world.
    Unit(EntityId),
    /// A seat, clicked in the top bar.
    Seat(SlotId),
}

/// How long after one press a second counts as a double.
pub const DOUBLE_TAP: f32 = 0.35;

/// Whether reaching for something makes a pair with what was reached for
/// last, and what is worth remembering afterwards.
///
/// A pair is spent when it is made: three presses are one pair and one
/// single, not two pairs.
pub fn tap_again(last: Option<(Tap, f32)>, what: Tap) -> (bool, Option<(Tap, f32)>) {
    if last.map(|(seen, _)| seen) == Some(what) {
        (true, None)
    } else {
        (false, Some((what, DOUBLE_TAP)))
    }
}

/// What a refusal is called in plain words.
pub fn refusal(reason: bota_proto::RejectReason) -> &'static str {
    use bota_proto::RejectReason as Why;
    match reason {
        Why::NotYourSlot => "that seat is not yours",
        Why::HeroDead => "your hero is not standing",
        Why::UnknownTarget => "no such target",
        Why::WrongTargetKind => "not aimed at what it takes",
        Why::OnCooldown => "still on cooldown",
        Why::OutOfRange => "out of range",
        Why::NotEnoughMana => "not enough mana",
        Why::NotEnoughGold => "not enough gold",
        Why::EmptySlot => "that slot is empty",
        Why::NotCastable => "it works on its own",
        Why::NotLearned => "no points in it yet",
        Why::NoCharges => "no charges left",
        Why::NotReady => "it is not working yet",
        Why::NotYourUnit => "you do not drive that unit",
        Why::UnknownItem => "the shop does not sell that",
        Why::CannotLevelUp => "no skill point for it",
        Why::NotAtShop => "only at the shop",
        Why::InventoryFull => "no room for it",
        Why::Disabled => "you cannot act right now",
        Why::NotPlaying => "the match is not running",
        Why::NotYourItem => "not yours to sell",
        Why::ClosedGround => "nothing can lie there",
        Why::NotInBag => "not carried in the bag",
    }
}

/// A damage number floating off a unit.
pub struct Floater {
    /// The text shown.
    pub text: String,
    /// World position it rises from.
    pub world: (f32, f32),
    /// Seconds since it appeared.
    pub age: f32,
}

/// One line of the kill feed.
pub struct FeedLine {
    /// The text shown.
    pub text: String,
    /// Seconds since it appeared.
    pub age: f32,
}

/// The whole client.
pub struct App {
    /// Socket or file.
    pub source: Source,
    /// Whether the server hung up on us.
    pub lost: bool,
    /// Current screen.
    pub phase: Phase,
    /// Our wire identity, once welcomed.
    pub me: Option<PlayerId>,
    /// Our seat, if playing.
    pub my_slot: Option<SlotId>,
    /// Ticks per second, from Welcome or MatchStart.
    pub tick_rate: u16,
    /// Ticks before the game clock reaches zero, from MatchStart.
    pub pregame_ticks: u32,
    /// The slot taken up and waiting for a click to aim it, if one is.
    pub aiming: Option<crate::slots::Slot>,
    /// An item slot picked up and waiting for the destination click.
    pub held_item: Option<u8>,
    /// Whether the shop panel is open. Toggled by key or button; buying
    /// away from home lands in the stash.
    pub shop_open: bool,
    /// The catalog row the shop panel starts at.
    pub shop_scroll: usize,
    /// What the shop sells and asks for it, from MatchStart.
    pub shop: Vec<bota_proto::ShopEntry>,
    /// Every tree on the map, from MatchStart.
    pub trees: Vec<(f32, f32)>,
    /// Cells per terrain axis, from MatchStart.
    pub terrain_cells: usize,
    /// Decoded terrain cells, row-major from the south-west corner: bit 7
    /// walkable, bit 6 water, low bits the elevation tier.
    pub terrain: Vec<u8>,
    /// One bit per terrain cell: set where a tree or a fog blocker wall
    /// blocks sight lines.
    pub opaque: Vec<u64>,
    /// How the server advances ticks.
    pub mode: Option<TickMode>,
    /// The lobby as last broadcast.
    pub lobby: Vec<LobbySlot>,
    /// Seat names captured from the lobby for the scoreboard.
    pub names: Vec<(SlotId, String)>,
    /// The freshest state of the world.
    pub view: Option<WorldView>,
    /// The state one snapshot ago, for naming what died.
    pub prev_view: Option<WorldView>,
    /// The last state seen of every unit a seat owns, and the tick it was
    /// seen on.
    ///
    /// Only what a seat owns is kept, which bounds this to a handful of
    /// entries: an enemy hero out of sight is worth remembering, the creep
    /// wave it walked past is not.
    pub seen: Vec<(u32, bota_proto::UnitView)>,
    /// The eye.
    pub camera: Camera,
    /// Sequence number of the next order.
    pub seq: u32,
    /// Whether the next left click is an attack-move.
    pub attack_move_armed: bool,
    /// The unit picked, if one is. Nothing picked falls back to one's own
    /// hero, so orders always have somewhere to go.
    pub selected: Option<EntityId>,
    /// What the camera is pinned to. Nothing means a free camera.
    pub pinned: Option<Pin>,
    /// What was last reached for, and how long is left to make it a pair.
    pub tapped: Option<(Tap, f32)>,
    /// Whether the camera has been put on one's own hero once, at the start.
    pub found_my_hero: bool,
    /// Whether readiness has been declared.
    pub ready: bool,
    /// Whether a hero pick has been sent.
    pub picked: bool,
    /// The latest order of every seat it is known for: this seat's own in a
    /// live match, whatever the server tells a spectator, everybody's in a
    /// replay.
    pub shown_orders: Vec<ShownOrder>,
    /// The seat whose eyes a spectator has asked to watch through.
    pub eyes: Option<SlotId>,
    /// Damage numbers in flight.
    pub floaters: Vec<Floater>,
    /// Kill feed lines.
    pub feed: Vec<FeedLine>,
    /// Which slot the order of a sequence number came from, so a refusal
    /// can be shown on the slot that earned it.
    pub aimed_from: Option<(u32, crate::slots::Slot)>,
    /// The slot whose order was last refused, and the seconds left on it.
    pub refused: Option<(crate::slots::Slot, f32)>,
    /// The last rejected order, shown briefly: text and seconds left.
    pub reject: Option<(String, f32)>,
    /// The end of the match, once it came.
    pub over: Option<(Team, MatchStats)>,
    /// Set when the window should close.
    pub quit: bool,
}

impl App {
    /// A client over a source, before anything has been heard.
    pub fn new(source: Source) -> App {
        App {
            source,
            lost: false,
            phase: Phase::Lobby,
            me: None,
            my_slot: None,
            tick_rate: 30,
            pregame_ticks: 0,
            aiming: None,
            held_item: None,
            shop_open: false,
            shop_scroll: 0,
            shop: Vec::new(),
            trees: Vec::new(),
            terrain_cells: 0,
            terrain: Vec::new(),
            opaque: Vec::new(),
            mode: None,
            lobby: Vec::new(),
            names: Vec::new(),
            view: None,
            prev_view: None,
            seen: Vec::new(),
            camera: Camera::over(9216.0, 9216.0),
            seq: 0,
            attack_move_armed: false,
            selected: None,
            pinned: None,
            tapped: None,
            found_my_hero: false,
            ready: false,
            picked: false,
            shown_orders: Vec::new(),
            eyes: None,
            floaters: Vec::new(),
            feed: Vec::new(),
            aimed_from: None,
            refused: None,
            reject: None,
            over: None,
            quit: false,
        }
    }

    /// The display name of a seat.
    pub fn seat_name(&self, slot: SlotId) -> String {
        self.names
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| format!("seat {}", slot.0))
    }

    /// What is known of a unit: its live state, or the last one seen of it
    /// and how many ticks ago that was.
    pub fn known(&self, id: EntityId) -> Option<(&bota_proto::UnitView, u32)> {
        known_in(&self.seen, self.view.as_ref()?, id)
    }

    /// The body a seat last stood in, whether or not it still stands.
    pub fn body_of(&self, slot: SlotId) -> Option<EntityId> {
        body_in(&self.seen, self.view.as_ref()?, slot)
    }

    /// Our hero's unit id, while it is alive and known.
    pub fn my_hero(&self) -> Option<bota_proto::EntityId> {
        let slot = self.my_slot?;
        let view = self.view.as_ref()?;
        view.players.iter().find(|p| p.slot == slot)?.unit
    }

    /// Which unit the slot panel and the keys are about.
    ///
    /// Whatever is picked, if anything is; one's own hero otherwise.
    pub fn commanded(&self) -> Option<bota_proto::EntityId> {
        match self.selected {
            Some(id) => Some(id),
            None => self.my_hero(),
        }
    }

    /// Picks a unit, and pins the camera to it when the same thing is
    /// reached for twice.
    ///
    /// Picking alone never moves the camera: what a player is looking at and
    /// what a player is commanding are two different questions.
    pub fn choose(&mut self, unit: Option<EntityId>, again: bool) {
        self.selected = unit;
        self.aiming = None;
        if again {
            self.pinned = unit.map(|id| match self.hero_seat(id) {
                Some(slot) => Pin::Hero(slot),
                None => Pin::Unit(id),
            });
        }
    }

    /// Which seat a unit is the standing hero of, if it is one.
    fn hero_seat(&self, id: EntityId) -> Option<SlotId> {
        let (unit, _) = self.known(id)?;
        (unit.kind == UnitKind::Hero)
            .then_some(unit.owner)
            .flatten()
    }

    /// Which unit the camera is carried by right now, if any still stands.
    pub fn pinned_unit(&self) -> Option<EntityId> {
        let view = self.view.as_ref()?;
        match self.pinned? {
            Pin::Hero(slot) => view.players.iter().find(|p| p.slot == slot)?.unit,
            Pin::Unit(id) => view.units.iter().find(|u| u.id == id).map(|u| u.id),
        }
    }

    /// Whether this reach for something is the second of a pair.
    pub fn tapped_twice(&mut self, what: Tap) -> bool {
        let (again, kept) = tap_again(self.tapped, what);
        self.tapped = kept;
        again
    }

    /// The seat whose gold, stash and score the panel is about.
    ///
    /// Whoever owns what is picked, and one's own seat when what is picked
    /// answers to nobody.
    pub fn panel_slot(&self) -> Option<SlotId> {
        let owner = self
            .selected
            .and_then(|id| self.known(id))
            .and_then(|(unit, _)| unit.owner);
        owner.or_else(|| self.default_slot())
    }

    /// Our own courier, while one stands.
    pub fn my_courier(&self) -> Option<bota_proto::EntityId> {
        let view = self.view.as_ref()?;
        let mine = self.my_slot?;
        view.units
            .iter()
            .find(|unit| unit.kind == bota_proto::UnitKind::Courier && unit.owner == Some(mine))
            .map(|unit| unit.id)
    }

    /// Whether a unit is one this seat drives.
    pub fn drives(&self, id: bota_proto::EntityId) -> bool {
        let Some(view) = self.view.as_ref() else {
            return false;
        };
        view.units
            .iter()
            .find(|unit| unit.id == id)
            .is_some_and(|unit| unit.owner.is_some() && unit.owner == self.my_slot)
    }

    /// Which of our units an order goes to.
    ///
    /// Whatever of ours is selected takes it; with our own hero selected, or
    /// nothing of ours at all, the order names nobody and the server sends it
    /// to the hero.
    pub fn ordering_unit(&self) -> Option<bota_proto::EntityId> {
        let id = self.selected?;
        let view = self.view.as_ref()?;
        let mine = self.my_slot?;
        let unit = view.units.iter().find(|unit| unit.id == id)?;
        (unit.owner == Some(mine) && Some(id) != self.my_hero()).then_some(id)
    }

    /// Which side this player is on, once the match has told us.
    pub fn my_team(&self) -> Option<bota_proto::Team> {
        let slot = self.my_slot?;
        let view = self.view.as_ref()?;
        view.players.iter().find(|p| p.slot == slot).map(|p| p.team)
    }

    /// Whether the current selection is a unit this player commands.
    ///
    /// Orders go to the selection; anything not ours ignores them, so an
    /// enemy or a creep can be inspected without stealing the keys. What is
    /// ours answers wherever it came from: a hero, a courier, or anything
    /// else this seat is given to drive.
    pub fn controls_selection(&self) -> bool {
        match self.selected {
            None => true,
            Some(id) => self.drives(id),
        }
    }

    /// The team whose fog this client lives under. Absent for spectators,
    /// who see everything.
    pub fn fog_team(&self) -> Option<bota_proto::Team> {
        let view = self.view.as_ref()?;
        if let Some(slot) = self.my_slot {
            return view.players.iter().find(|p| p.slot == slot).map(|p| p.team);
        }
        // A spectator's fog is whatever eyes the snapshot came through.
        view.viewer
    }

    /// Whether our hero stands in its home shop area.
    pub fn at_home_shop(&self) -> bool {
        let Some(slot) = self.my_slot else {
            return false;
        };
        let Some(view) = &self.view else {
            return false;
        };
        let Some(p) = view.players.iter().find(|p| p.slot == slot) else {
            return false;
        };
        let Some(unit) = p.unit.and_then(|id| view.units.iter().find(|u| u.id == id)) else {
            return false;
        };
        let (fx, fy) = match p.team {
            bota_proto::Team::Radiant => (1760.0, 2278.0),
            bota_proto::Team::Dire => (16624.0, 16064.0),
            bota_proto::Team::Neutral => return false,
        };
        let dx = unit.pos.x.to_f32() - fx;
        let dy = unit.pos.y.to_f32() - fy;
        dx * dx + dy * dy <= 1000.0 * 1000.0
    }

    /// Whether one of the panel's fifteen item slots holds an item right now.
    pub fn item_at(&self, slot: u8) -> bool {
        self.item_in(slot).is_some()
    }

    /// The seat the bottom panel falls back to: our own, else the first one.
    pub fn default_slot(&self) -> Option<SlotId> {
        self.my_slot
            .or_else(|| Some(self.view.as_ref()?.players.first()?.slot))
    }

    /// Sends an order with the next sequence number.
    pub fn send_order(&mut self, order: Order) {
        let unit = self.ordering_unit();
        self.send_order_to(unit, order);
    }

    /// Sends one order to a named unit, whatever is selected.
    ///
    /// What is selected is where orders go by default; this is for the few
    /// that name their unit themselves, so that sending one does not mean
    /// looking away from the fight.
    pub fn send_order_to(&mut self, unit: Option<bota_proto::EntityId>, order: Order) {
        self.seq += 1;
        let seq = self.seq;
        if let (Some(slot), Some(view)) = (self.my_slot, self.view.as_ref()) {
            self.note_order(slot, unit, view.tick, order);
        }
        self.source.send(&ClientMsg::Order { seq, unit, order });
    }

    /// Remembers one seat's order for the overlay, replacing its last one.
    pub fn note_order(&mut self, slot: SlotId, unit: Option<EntityId>, tick: u32, order: Order) {
        self.shown_orders.retain(|shown| shown.slot != slot);
        self.shown_orders.push(ShownOrder {
            slot,
            unit,
            tick,
            order,
        });
    }

    /// Which slot of our courier holds one ability.
    pub fn courier_slot(&self, ability: u16) -> Option<u8> {
        let view = self.view.as_ref()?;
        let courier = self.my_courier()?;
        let unit = view.units.iter().find(|unit| unit.id == courier)?;
        unit.abilities
            .iter()
            .position(|held| held.id.0 == ability)
            .map(|slot| slot as u8)
    }

    /// Applies one server message.
    pub fn handle(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Welcome {
                player_id,
                slot,
                tick_rate,
                mode,
            } => {
                self.me = Some(player_id);
                self.my_slot = slot;
                self.tick_rate = tick_rate;
                self.mode = Some(mode);
                if slot.is_some() && !self.picked {
                    self.picked = true;
                    self.source.send(&ClientMsg::PickHero { hero: HeroId(0) });
                }
            }
            ServerMsg::LobbyState { slots } => {
                self.names = slots
                    .iter()
                    .filter(|s| !s.name.is_empty())
                    .map(|s| (s.slot, s.name.clone()))
                    .collect();
                self.lobby = slots;
            }
            ServerMsg::Orders { tick, orders } => {
                for given in orders {
                    self.note_order(given.slot, given.unit, tick, given.order);
                }
            }
            ServerMsg::MatchStart { info } => {
                self.shown_orders.clear();
                self.tick_rate = info.tick_rate;
                self.pregame_ticks = info.pregame_ticks;
                self.shop = info.shop.clone();
                self.trees = info
                    .trees
                    .iter()
                    .map(|t| (t.x.to_f32(), t.y.to_f32()))
                    .collect();
                self.terrain_cells = info.terrain_cells as usize;
                self.terrain = info
                    .terrain_rle
                    .iter()
                    .flat_map(|&(n, v)| std::iter::repeat_n(v, usize::from(n)))
                    .collect();
                let n = self.terrain_cells;
                self.opaque = vec![0u64; (n * n).div_ceil(64)];
                for &(cx, cy) in &info.opaque_cells {
                    let idx = usize::from(cy) * n + usize::from(cx);
                    self.opaque[idx / 64] |= 1 << (idx % 64);
                }
                self.phase = Phase::Playing;
            }
            ServerMsg::Snapshot { view } => {
                if self.mode == Some(TickMode::Lockstep) && self.my_slot.is_some() {
                    self.source.send(&ClientMsg::Ack { tick: view.tick });
                }
                remember(&mut self.seen, &view);
                self.prev_view = self.view.replace(view);
            }
            ServerMsg::Events { events, .. } => {
                for event in events {
                    self.absorb_event(event);
                }
            }
            ServerMsg::OrderRejected { seq, reason } => {
                self.reject = Some((refusal(reason).to_string(), 2.5));
                // The slot that earned it lights up, so the answer is where
                // the press was rather than only in a line of text.
                self.refused = self
                    .aimed_from
                    .filter(|(sent, _)| *sent == seq)
                    .map(|(_, slot)| (slot, 1.2));
            }
            ServerMsg::MatchOver { winner, stats } => {
                self.over = Some((winner, stats));
            }
            ServerMsg::ParticipantLeft { slot, .. } => {
                if let Some(slot) = slot {
                    self.feed.push(FeedLine {
                        text: format!("{} disconnected", self.seat_name(slot)),
                        age: 0.0,
                    });
                }
            }
        }
    }

    fn absorb_event(&mut self, event: EventKind) {
        match event {
            EventKind::Damaged { target, amount, .. } => {
                if let Some(pos) = self.unit_pos(target) {
                    self.floaters.push(Floater {
                        text: format!("{amount}"),
                        world: pos,
                        age: 0.0,
                    });
                }
            }
            EventKind::Died {
                unit, killer, gold, ..
            } => {
                let Some((kind, team, owner)) = self.unit_identity(unit) else {
                    return;
                };
                if kind != UnitKind::Hero {
                    return;
                }
                let victim = match owner {
                    Some(slot) => self.seat_name(slot),
                    None => format!("a {team:?} hero"),
                };
                let by =
                    killer
                        .and_then(|k| self.unit_identity(k))
                        .map(|(kind, team, owner)| match (kind, owner) {
                            (UnitKind::Hero, Some(slot)) => self.seat_name(slot),
                            (UnitKind::Tower, _) => format!("the {team:?} tower"),
                            (UnitKind::Fountain, _) => format!("the {team:?} fountain"),
                            (kind, _) => format!("a {team:?} {kind:?}"),
                        });
                let text = match by {
                    Some(by) if gold > 0 => format!("{victim} was slain by {by} for {gold} gold"),
                    Some(by) => format!("{victim} was slain by {by}"),
                    None => format!("{victim} died"),
                };
                self.feed.push(FeedLine { text, age: 0.0 });
            }
            EventKind::StructureDestroyed { team, .. } => {
                self.feed.push(FeedLine {
                    text: format!("a {team:?} structure has fallen"),
                    age: 0.0,
                });
            }
            EventKind::LevelUp { unit, level } => {
                if let Some(pos) = self.unit_pos(unit) {
                    self.floaters.push(Floater {
                        text: format!("level {level}"),
                        world: pos,
                        age: 0.0,
                    });
                }
            }
            EventKind::Healed { .. }
            | EventKind::AbilityCast { .. }
            | EventKind::ItemBought { .. } => {}
        }
    }

    /// The position of a unit in the freshest view that still has it.
    fn unit_pos(&self, id: bota_proto::EntityId) -> Option<(f32, f32)> {
        for view in [self.view.as_ref(), self.prev_view.as_ref()]
            .into_iter()
            .flatten()
        {
            if let Some(u) = view.units.iter().find(|u| u.id == id) {
                return Some((u.pos.x.to_f32(), u.pos.y.to_f32()));
            }
        }
        None
    }

    /// Kind, team and owner of a unit, from the freshest view that has it.
    fn unit_identity(&self, id: bota_proto::EntityId) -> Option<(UnitKind, Team, Option<SlotId>)> {
        for view in [self.view.as_ref(), self.prev_view.as_ref()]
            .into_iter()
            .flatten()
        {
            if let Some(u) = view.units.iter().find(|u| u.id == id) {
                return Some((u.kind, u.team, u.owner));
            }
        }
        None
    }

    /// Notices a dead socket once and says so.
    pub fn check_connection(&mut self) {
        if self.lost || self.over.is_some() {
            return;
        }
        if let Source::Live(net) = &self.source
            && net.is_closed()
        {
            self.lost = true;
            self.feed.push(FeedLine {
                text: "connection to the server lost".to_string(),
                age: 0.0,
            });
        }
    }

    /// Ages and expires the transient effects.
    pub fn tick_effects(&mut self, dt: f32) {
        for f in &mut self.floaters {
            f.age += dt;
        }
        self.floaters.retain(|f| f.age < 1.2);
        for l in &mut self.feed {
            l.age += dt;
        }
        self.feed.retain(|l| l.age < 8.0);
        if let Some((_, left)) = &mut self.reject {
            *left -= dt;
            if *left <= 0.0 {
                self.reject = None;
            }
        }
        if let Some((_, left)) = &mut self.refused {
            *left -= dt;
            if *left <= 0.0 {
                self.refused = None;
            }
        }
        if let Some((_, left)) = &mut self.tapped {
            *left -= dt;
            if *left <= 0.0 {
                self.tapped = None;
            }
        }
    }
}
