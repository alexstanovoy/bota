//! Whether a deed can be done, and what order it turns into.
//!
//! The two halves of the same question, kept together so they cannot drift
//! apart: a deed that [`allowed`] calls legal must be one [`Deed::into_ask`]
//! can build an order for, and the test beside them walks every deed on a
//! made-up tick to check that it is so. A legal deed that decodes to nothing
//! would be a tick thrown away, and there is one order a tick.

use bota_proto::{AbilitySlot, ItemSlot, Order, Target};

use crate::{ABILITIES, Aim, Ask, DEEDS, Deed, Errand, Field, ITEMS, Place, STEP, STEPS};

/// How near the fountain buying is allowed, with room to spare.
const SHOP_REACH: f32 = 800.0;
/// How far ahead a cast aimed at the ground is put.
const AHEAD: f32 = 600.0;

/// Which deeds could be done this tick.
///
/// One flag per deed, in the numbering of the list. Everything a body cannot
/// do because it is dead, held, or has nothing to do it to comes out false;
/// what is left is what the model may choose between.
pub fn allowed(field: &Field) -> Vec<bool> {
    let mut out = vec![false; DEEDS];
    let Some(me) = field.me else {
        // Nothing standing decides nothing. Every flag stays false, and
        // whatever is choosing has to cope with a tick where there is nothing
        // to choose.
        return out;
    };
    let mut allow = |deed: Deed| out[deed.index()] = true;

    allow(Deed::Stand);
    for at in 0..field.creeps.len() {
        allow(Deed::Swing(at));
    }
    for (at, creep) in field.own_creeps.iter().enumerate() {
        // One of its own may only be swung at once it is worn far enough down
        // to be put out; below that the order is a walk towards it.
        if crate::part(creep.hp, creep.max_hp) < DENY_PART {
            allow(Deed::PutOut(at));
        }
    }
    for at in 0..field.heroes.len() {
        allow(Deed::Fight(at));
    }
    for at in 0..STEPS {
        allow(Deed::Step(at));
    }
    if field.home.is_some() {
        allow(Deed::GoTo(Place::Home));
    }
    if field.towers.0.is_some() {
        allow(Deed::GoTo(Place::OwnTower));
    }
    if field.towers.1.is_some() {
        allow(Deed::GoTo(Place::TheirTower));
    }
    for slot in 0..ABILITIES.min(me.abilities.len()) {
        let ability = me.abilities[slot];
        let ready = ability.level > 0 && ability.cooldown_left == 0 && me.mana >= ability.mana_cost;
        if !ready {
            continue;
        }
        // Only the way this ability is actually aimed, and only with its
        // target in reach: a cast in reach goes off on the tick it is asked
        // for, where one aimed past it has the server walking the body in —
        // a walk the next tick's deed calls off.
        for aim in [Aim::Own, Aim::Ahead, Aim::Hero, Aim::Creep] {
            if !crate::suits(ability.id, aim) {
                continue;
            }
            let reach = ability.range as f32;
            let has_a_target = match aim {
                Aim::Hero => field
                    .heroes
                    .first()
                    .is_some_and(|hero| crate::span(me.pos, hero.pos) <= reach),
                Aim::Creep => field
                    .creeps
                    .first()
                    .is_some_and(|creep| crate::span(me.pos, creep.pos) <= reach),
                Aim::Own | Aim::Ahead => true,
            };
            if has_a_target {
                allow(Deed::Cast(slot, aim));
            }
        }
    }
    for slot in 0..ITEMS {
        // Held, of a kind that has a use, and not still waiting. A snapshot
        // says the first and the third; which items have a use at all is ours
        // to know, the same as which abilities may be cast.
        let ready = me
            .items
            .get(slot)
            .and_then(|held| held.as_ref())
            .is_some_and(|held| crate::can_be_used(held.id.0) && held.cooldown_left == 0);
        if ready {
            allow(Deed::Use(slot));
        }
    }
    if wants_to_buy(field) {
        allow(Deed::Buy);
    }
    // Whether a point could go in is the server's sum to do — the razes
    // share a level and cost one point together — and the snapshot carries
    // its answer per slot.
    for slot in 0..ABILITIES.min(me.abilities.len()) {
        if me.abilities[slot].can_level {
            allow(Deed::Learn(slot));
        }
    }
    if let Some(courier) = field.courier {
        // An errand is a cast on the courier, and a cast still waiting is
        // refused like any other. Offered whenever there is a courier at all,
        // its turn of speed was asked for on every tick of its own cooldown.
        for errand in [
            Errand::TakeStash,
            Errand::Deliver,
            Errand::Burst,
            Errand::GoHome,
        ] {
            let wanted = errand.ability();
            let ready = courier
                .abilities
                .iter()
                .find(|ability| ability.id.0 == wanted)
                .is_some_and(|ability| ability.cooldown_left == 0);
            if ready {
                allow(Deed::Errand(errand));
            }
        }
    }
    for slot in 0..ITEMS {
        // Anything held may be sold — or marked to be sold and unmarked
        // again; which one an ask is, is settled by where the body stands.
        if me.items.get(slot).is_some_and(|held| held.is_some()) {
            allow(Deed::Sell(slot));
        }
    }
    out
}

/// Part of its whole health at which one of its own may be put out.
const DENY_PART: f32 = 0.5;

/// Whether there is anything worth buying and the room and gold for it.
///
/// Kept blunt on purpose: what to buy is the shop's business, and the model is
/// only being asked whether to spend a tick on it.
fn wants_to_buy(field: &Field) -> bool {
    let Some(me) = field.me else {
        return false;
    };
    let gold = field.seat.gold.unwrap_or(0);
    if gold < CHEAPEST {
        return false;
    }
    let at_shop = field
        .home
        .is_some_and(|home| crate::span(me.pos, home) <= SHOP_REACH);
    // Somewhere for it to land: a slot on the hero when it is bought in hand,
    // a slot in the stash and somebody to fetch it when it is bought away from
    // the shop.
    if at_shop {
        me.items.iter().take(crate::BAG_SLOTS).any(Option::is_none)
    } else {
        field
            .seat
            .stash
            .as_ref()
            .is_some_and(|slots| slots.iter().any(Option::is_none))
            && field.courier.is_some()
    }
}

/// The cheapest thing the shop sells.
const CHEAPEST: i32 = 50;

impl Deed {
    /// The order this deed asks for, if it can be turned into one.
    ///
    /// Answers nothing for a deed that could not be done; [`allowed`] is the
    /// same question asked ahead of time, and the two are checked against each
    /// other by a test.
    pub fn into_ask(self, field: &Field) -> Option<Ask> {
        let me = field.me?;
        let hero = |order: Order| Some(Ask { unit: None, order });
        match self {
            Deed::Stand => hero(Order::Move {
                target: Target::None,
            }),
            Deed::Swing(at) => hero(Order::Attack {
                target: Target::Unit(field.creeps.get(at)?.id),
            }),
            Deed::PutOut(at) => hero(Order::Attack {
                target: Target::Unit(field.own_creeps.get(at)?.id),
            }),
            Deed::Fight(at) => hero(Order::Attack {
                target: Target::Unit(field.heroes.get(at)?.id),
            }),
            Deed::Step(at) => {
                let turn = std::f32::consts::TAU * at as f32 / STEPS as f32;
                hero(Order::Move {
                    target: Target::Pos(field.spot_towards(STEP * turn.cos(), STEP * turn.sin())),
                })
            }
            Deed::GoTo(place) => {
                let spot = match place {
                    Place::Home => field.home?,
                    Place::OwnTower => field.towers.0?.pos,
                    Place::TheirTower => field.towers.1?.pos,
                };
                hero(Order::Move {
                    target: Target::Pos(spot),
                })
            }
            Deed::Cast(slot, aim) => {
                let ability = me.abilities.get(slot)?;
                if ability.level == 0 || ability.cooldown_left > 0 {
                    return None;
                }
                if !crate::suits(ability.id, aim) {
                    return None;
                }
                let target = match aim {
                    Aim::Own => Target::None,
                    Aim::Hero => Target::Unit(field.heroes.first()?.id),
                    Aim::Creep => Target::Unit(field.creeps.first()?.id),
                    // No further ahead than the cast reaches, so it goes off
                    // where the body stands.
                    Aim::Ahead => {
                        Target::Pos(field.spot_towards(AHEAD.min(ability.range as f32), 0.0))
                    }
                };
                hero(Order::Cast {
                    slot: AbilitySlot(slot as u8),
                    target,
                })
            }
            Deed::Use(slot) => {
                me.items.get(slot)?.as_ref()?;
                hero(Order::Use {
                    slot: ItemSlot(slot as u8),
                    target: Target::None,
                })
            }
            Deed::Buy => hero(Order::Buy {
                item: crate::next_to_buy(field)?,
            }),
            Deed::Learn(slot) => {
                me.abilities.get(slot)?;
                hero(Order::Learn {
                    slot: AbilitySlot(slot as u8),
                })
            }
            Deed::Errand(errand) => {
                let courier = field.courier?;
                let wanted = errand.ability();
                // Which slot holds which errand is the server's business, so
                // it is read off the courier rather than assumed.
                let at = courier
                    .abilities
                    .iter()
                    .position(|ability| ability.id.0 == wanted)?;
                Some(Ask {
                    unit: Some(courier.id),
                    order: Order::Cast {
                        slot: AbilitySlot(at as u8),
                        target: Target::None,
                    },
                })
            }
            Deed::Sell(slot) => {
                me.items.get(slot)?.as_ref()?;
                hero(Order::Sell {
                    slot: ItemSlot(slot as u8),
                })
            }
        }
    }
}
