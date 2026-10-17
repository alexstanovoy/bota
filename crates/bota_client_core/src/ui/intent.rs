//! What a player means by a key or a click, turned into orders and
//! selection. A front end maps its own keys and cursor to an [`Intent`] and
//! hands it to [`App::act`]; world positions are in world units.

use bota_proto::{
    AbilityId, AbilitySlot, Aim, ClientMsg, EntityId, Fixed, HeroId, ItemId, ItemSlot, Order,
    SlotId, Target, Team, UnitKind, Vec2, WorldView,
};

use crate::ui::app::{App, Pin, Source, Tap};
use crate::ui::slots::{BAG_SLOTS, Press, Slot, order_for, press};

/// How far past a body's edge an attack click still sticks to it, in world
/// units.
const ATTACK_SNAP: f32 = 25.0;

/// One thing a player means.
#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    /// The smart click at a world point: attack an enemy, follow a friend,
    /// pick up an item, or walk there.
    Smart((f32, f32)),
    /// An armed attack-move click at a world point.
    AttackAt((f32, f32)),
    /// Walk to a world point, whatever is there.
    MoveTo((f32, f32)),
    /// Attack-move to a world point, whatever is there.
    AttackMoveTo((f32, f32)),
    /// Stop.
    Stop,
    /// Hold position.
    Hold,
    /// Press a slot, by key or by click; with `ctrl` an ability slot spends a
    /// skill point.
    Press { slot: Slot, ctrl: bool },
    /// Spend the slot taken up for aiming at a world point.
    Aimed((f32, f32)),
    /// Let go of the slot taken up for aiming.
    CancelAim,
    /// Move an item from one slot to another.
    Swap { from: u8, to: u8 },
    /// Sell an item, or mark it for sale away from the shop.
    Sell(u8),
    /// Put an item out of the bag at a world point: into an allied bag under
    /// it, or onto the ground.
    PutAt { slot: u8, at: (f32, f32) },
    /// Hand an item to a seat's hero.
    HandTo { slot: u8, seat: SlotId },
    /// Buy an item.
    Buy(u16),
    /// Buy towards the quick-buy item.
    QuickBuy,
    /// Send the courier for what waits in the stash.
    CourierDeliver,
    /// Pick one's own hero; twice in a row pins the camera to it.
    SelectHero,
    /// Pick one's own courier; twice in a row pins the camera to it.
    SelectCourier,
    /// Pick whatever is at a world point, or nothing; twice pins.
    SelectAt((f32, f32)),
    /// Pick a seat's body; twice pins.
    SelectSeat(SlotId),
    /// Watch through the next eyes: a live spectator asks the server, a
    /// replay watches as the next seat.
    NextEyes,
    /// Pick a hero in the lobby.
    PickHero(u16),
    /// Flip readiness in the lobby.
    ToggleReady,
    /// Run a console line.
    Console(String),
    /// Pause or resume a replay.
    TogglePause,
    /// Play a replay twice as fast, up to 16 times.
    Faster,
    /// Play a replay half as fast, down to a quarter.
    Slower,
    /// Step a paused replay one tick.
    Step,
}

impl App {
    /// Acts on what the player means.
    pub fn act(&mut self, intent: Intent) {
        match intent {
            Intent::Smart(at) => self.smart(at),
            Intent::AttackAt(at) => self.attack_at(at),
            Intent::MoveTo(at) => self.send_order(Order::Move {
                target: Target::Pos(world_vec(at)),
            }),
            Intent::AttackMoveTo(at) => self.send_order(Order::Attack {
                target: Target::Pos(world_vec(at)),
            }),
            Intent::Stop => self.send_order(Order::Move {
                target: Target::None,
            }),
            Intent::Hold => self.send_order(Order::Attack {
                target: Target::None,
            }),
            Intent::Press { slot, ctrl } => self.press_slot(slot, ctrl),
            Intent::Aimed(at) => self.spend_aim(at),
            Intent::CancelAim => self.aiming = None,
            Intent::Swap { from, to } => self.send_order(Order::Swap {
                from: ItemSlot(from),
                to: ItemSlot(to),
            }),
            Intent::Sell(slot) => self.send_order(Order::Sell {
                slot: ItemSlot(slot),
            }),
            Intent::PutAt { slot, at } => self.put_at(slot, at),
            Intent::HandTo { slot, seat } => self.hand_to(slot, seat),
            Intent::Buy(item) => self.send_order(Order::Buy { item: ItemId(item) }),
            Intent::QuickBuy => {
                if let Some(item) = self.quick_buy_next() {
                    self.send_order(Order::Buy { item: ItemId(item) });
                }
            }
            Intent::CourierDeliver => self.courier_deliver(),
            other => self.act_on_view(other),
        }
    }

    /// The intents about what is watched rather than what is ordered.
    fn act_on_view(&mut self, intent: Intent) {
        match intent {
            Intent::SelectHero => {
                let hero = self.my_hero();
                let again = self.tapped_twice(Tap::Hero);
                self.choose(hero, again);
            }
            Intent::SelectCourier => {
                if let Some(courier) = self.my_courier() {
                    let again = self.tapped_twice(Tap::Courier);
                    self.choose(Some(courier), again);
                }
            }
            Intent::SelectAt(at) => {
                let hit = self
                    .view()
                    .and_then(|view| unit_under_cursor(view, at, None, false));
                let again = hit.is_some_and(|id| self.tapped_twice(Tap::Unit(id)));
                self.choose(hit, again);
            }
            Intent::SelectSeat(seat) => {
                let body = self.body_of(seat);
                let again = self.tapped_twice(Tap::Seat(seat));
                self.choose(body, again);
            }
            Intent::NextEyes => self.next_eyes(),
            Intent::PickHero(hero) => self
                .source
                .send(&ClientMsg::PickHero { hero: HeroId(hero) }),
            Intent::ToggleReady => {
                self.ready = !self.ready;
                let ready = self.ready;
                self.source.send(&ClientMsg::SetReady(ready));
            }
            Intent::Console(line) => match crate::ui::console::parse_console(&line) {
                Ok(order) => self.send_order(order),
                Err(why) => self.reject = Some((why, 2.5)),
            },
            other => self.act_on_replay(other),
        }
    }

    /// The intents that drive a replay's clock.
    fn act_on_replay(&mut self, intent: Intent) {
        let Source::Replay(player) = &mut self.source else {
            return;
        };
        match intent {
            Intent::TogglePause => player.paused = !player.paused,
            Intent::Faster => player.speed = (player.speed * 2.0).min(16.0),
            Intent::Slower => player.speed = (player.speed / 2.0).max(0.25),
            Intent::Step if player.paused => player.advance_ticks(1.0),
            _ => {}
        }
    }

    /// Answers a press of one slot. What a press means is [`press`].
    fn press_slot(&mut self, slot: Slot, ctrl: bool) {
        match press(self, slot, ctrl) {
            Press::Send(order) => {
                self.aiming = None;
                self.aimed_from = Some((self.seq() + 1, slot));
                self.send_order(order);
            }
            Press::Aim(slot) => self.aiming = Some(slot),
            Press::Nothing => {}
        }
    }

    /// Sends the order of the slot taken up at what a click at a world point
    /// landed on, and lets go of the slot.
    fn spend_aim(&mut self, at: (f32, f32)) {
        let Some(slot) = self.aiming.take() else {
            return;
        };
        let target = match self.aim_in(slot) {
            // A tree and a landing spot are both named by the ground under
            // the cursor; which one was meant is the server's to settle.
            Some(Aim::Point | Aim::Tree | Aim::Building) => Some(Target::Pos(world_vec(at))),
            // One's own hero is a target like any other here: a salve is
            // drunk by clicking the one drinking it.
            Some(Aim::Unit) => self
                .view()
                .and_then(|view| unit_under_cursor(view, at, None, true))
                .map(Target::Unit),
            Some(Aim::Own) | None => Some(Target::None),
        };
        if let Some(target) = target {
            self.aimed_from = Some((self.seq() + 1, slot));
            self.send_order(order_for(slot, target));
        }
    }

    /// An armed attack-move click. A unit under the cursor takes the attack
    /// order itself: on an ally this is the aggro-drop click. Near an enemy
    /// the order sticks to it.
    fn attack_at(&mut self, at: (f32, f32)) {
        let me = self.my_hero();
        let target = self.view().and_then(|view| {
            unit_under_cursor(view, at, me, true)
                .or_else(|| enemy_near_cursor(view, at, self.my_team()))
        });
        let target = match target {
            Some(target) => Target::Unit(target),
            None => Target::Pos(world_vec(at)),
        };
        self.send_order(Order::Attack { target });
    }

    /// A smart click: an enemy under the cursor is attacked, one of this
    /// side's own is followed, an item lying there is picked up, open
    /// ground is walked to. Near an enemy the click still attacks it.
    fn smart(&mut self, at: (f32, f32)) {
        let Some(view) = self.view() else {
            self.send_order(Order::Move {
                target: Target::Pos(world_vec(at)),
            });
            return;
        };
        let order = if let Some(id) = unit_under_cursor(view, at, self.my_hero(), true) {
            let own = view
                .unit(id)
                .is_some_and(|unit| Some(unit.team) == self.my_team());
            let target = Target::Unit(id);
            if own {
                Order::Move { target }
            } else {
                Order::Attack { target }
            }
        } else if let Some(item) = loot_under_cursor(view, at) {
            Order::Take {
                target: Target::Unit(item),
            }
        } else if let Some(mark) = enemy_near_cursor(view, at, self.my_team()) {
            Order::Attack {
                target: Target::Unit(mark),
            }
        } else {
            Order::Move {
                target: Target::Pos(world_vec(at)),
            }
        };
        self.send_order(order);
    }

    /// Puts a bag item out at a world point. Whoever the panel is about puts
    /// it: the courier lays its own load out the way the hero does. Its own
    /// body under the cursor reads as the ground at its feet. What the stash
    /// holds stays put.
    fn put_at(&mut self, slot: u8, at: (f32, f32)) {
        if slot >= BAG_SLOTS {
            return;
        }
        let carrier = self.commanded();
        let target = self
            .view()
            .and_then(|view| unit_under_cursor(view, at, carrier, true))
            .filter(|id| self.takes_a_handover(*id))
            .map(Target::Unit)
            .unwrap_or(Target::Pos(world_vec(at)));
        self.send_order(Order::Put {
            slot: ItemSlot(slot),
            target,
        });
    }

    /// Hands a bag item to an allied seat's body wherever it stands.
    fn hand_to(&mut self, slot: u8, seat: SlotId) {
        let ally = self
            .player(seat)
            .is_some_and(|p| Some(p.team) == self.my_team());
        if slot < BAG_SLOTS
            && ally
            && let Some(body) = self.body_of(seat)
        {
            self.send_order(Order::Put {
                slot: ItemSlot(slot),
                target: Target::Unit(body),
            });
        }
    }

    /// Whether a unit is one an item can be handed to: an allied hero or
    /// courier.
    fn takes_a_handover(&self, id: EntityId) -> bool {
        let Some(view) = self.view() else {
            return false;
        };
        view.unit(id).is_some_and(|u| {
            Some(u.team) == self.my_team() && matches!(u.kind, UnitKind::Hero | UnitKind::Courier)
        })
    }

    /// Sends the courier for what waits in the stash.
    fn courier_deliver(&mut self) {
        if let Some(courier) = self.my_courier()
            && let Some(slot) = self.courier_slot(AbilityId::TAKE_STASH.0)
        {
            self.send_order_to(
                Some(courier),
                Order::Cast {
                    slot: AbilitySlot(slot),
                    target: Target::None,
                },
            );
        }
    }

    /// Moves on to the next eyes to watch through: everything, then each
    /// seat in turn.
    fn next_eyes(&mut self) {
        let seats: Vec<SlotId> = self
            .view()
            .map(|view| view.players.iter().map(|player| player.slot).collect())
            .unwrap_or_default();
        let next = |current: Option<SlotId>| match current {
            None => seats.first().copied(),
            Some(current) => seats
                .iter()
                .position(|slot| *slot == current)
                .and_then(|at| seats.get(at + 1))
                .copied(),
        };
        match &self.source {
            Source::Live(_) if self.my_slot.is_none() => {
                self.eyes = next(self.eyes);
                let seat = self.eyes;
                self.source.send(&ClientMsg::ViewAs { seat });
            }
            Source::Replay(_) => {
                self.pov = next(self.pov);
                self.my_slot = self.pov;
                self.selected = None;
                self.pinned = self.pov.map(Pin::Hero);
            }
            Source::Live(_) => {}
        }
    }
}

/// The ground item nearest a world point, if it is close enough.
pub fn loot_under_cursor(view: &WorldView, (wx, wy): (f32, f32)) -> Option<EntityId> {
    nearest(
        wx,
        wy,
        view.loot.iter().map(|lying| (lying.id, lying.pos, 35.0)),
    )
}

/// The clickable unit nearest a world point, if it is close enough.
///
/// `exclude` names a unit that never matches. `for_orders` skips fountains,
/// which selection still picks.
pub fn unit_under_cursor(
    view: &WorldView,
    (wx, wy): (f32, f32),
    exclude: Option<EntityId>,
    for_orders: bool,
) -> Option<EntityId> {
    let clickable = view
        .units
        .iter()
        .filter(|u| Some(u.id) != exclude && !(for_orders && u.kind == UnitKind::Fountain))
        .map(|u| (u.id, u.pos, u.collision.to_f32().max(20.0) + 15.0));
    nearest(wx, wy, clickable)
}

/// The enemy nearest a world point within the attack snap, if one is that
/// near.
///
/// Fountains never stick. An order at one of this side's own (a follow, an
/// aggro drop or a deny) takes a click on the body itself.
pub fn enemy_near_cursor(
    view: &WorldView,
    (wx, wy): (f32, f32),
    mine: Option<Team>,
) -> Option<EntityId> {
    let enemies = view
        .units
        .iter()
        .filter(|u| u.kind != UnitKind::Fountain && Some(u.team) != mine)
        .map(|u| (u.id, u.pos, u.collision.to_f32() + ATTACK_SNAP));
    nearest(wx, wy, enemies)
}

/// The candidate nearest a world point among those within their own reach of
/// it, the first one listed on a tie.
fn nearest(
    wx: f32,
    wy: f32,
    candidates: impl Iterator<Item = (EntityId, Vec2, f32)>,
) -> Option<EntityId> {
    let mut best: Option<(f32, EntityId)> = None;
    for (id, pos, reach) in candidates {
        let dx = pos.x.to_f32() - wx;
        let dy = pos.y.to_f32() - wy;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist <= reach && best.is_none_or(|(b, _)| dist < b) {
            best = Some((dist, id));
        }
    }
    best.map(|(_, id)| id)
}

/// A world position as the wire carries it.
pub fn world_vec((x, y): (f32, f32)) -> Vec2 {
    Vec2 {
        x: Fixed {
            raw: (x * 65536.0) as i32,
        },
        y: Fixed {
            raw: (y * 65536.0) as i32,
        },
    }
}
