//! The ladder of wants, driven through the seam a match would drive it.

use bota_proto::{HeroId, ItemSlot, ItemView, Order, SlotId, Target, Team, UnitView, Vec2};

use crate::tests::fixtures;
use crate::{
    Ask, Bot, MAGIC_WAND, Playbook, Role, SALVE, SCROLL, TANGO, TREE_STANDOFF, point_along, span,
};

/// Out in the lane, well away from the shop.
const LANE: Vec2 = Vec2::from_ints(8700, 8800);

/// A bot that has been given seat nought and the terms of a match.
fn seated(trees: Vec<Vec2>) -> Playbook {
    stocked(Vec::new(), trees)
}

/// The same, with a shop that names what its rows cost.
fn stocked(shop: Vec<bota_proto::ShopEntry>, trees: Vec<Vec2>) -> Playbook {
    let mut bot = Playbook::new(Role::Mid, HeroId(2));
    bot.seated(Some(SlotId(0)));
    bot.match_started(&fixtures::started(shop, trees));
    bot
}

/// One tick with the hero standing in the lane, however the caller wants it.
fn tick(shape: impl Fn(&mut UnitView)) -> bota_proto::WorldView {
    let mut me = fixtures::hero(1, Team::Radiant, LANE);
    shape(&mut me);
    let mut units = fixtures::fountains();
    units.push(me);
    let seat = fixtures::seat(SlotId(0), Team::Radiant, HeroId(2), Some(fixtures::id(1)));
    fixtures::tick(1000, units, vec![seat])
}

/// A blow landed on the seat's own hero by somebody.
fn struck(source: bota_proto::EntityId) -> bota_proto::EventKind {
    bota_proto::EventKind::Damaged {
        source: Some(source),
        target: fixtures::id(1),
        amount: 21,
        kind: bota_proto::DamageKind::Physical,
        crit: false,
    }
}

/// An item slot holding a stack with the charges given.
fn stack(id: bota_proto::ItemId, charges: u8) -> ItemView {
    ItemView {
        charges: Some(charges),
        ..fixtures::item(id)
    }
}

/// One tick with the hero standing in its own shop, the mid towers up.
fn at_the_shop(shape: impl Fn(&mut UnitView)) -> bota_proto::WorldView {
    let mut me = fixtures::hero(1, Team::Radiant, Vec2::from_ints(2040, 2558));
    shape(&mut me);
    let mut units = fixtures::fountains();
    units.push(fixtures::works(
        10,
        bota_proto::UnitKind::Tower,
        Team::Radiant,
        Vec2::from_ints(7672, 7808),
    ));
    units.push(fixtures::works(
        11,
        bota_proto::UnitKind::Tower,
        Team::Dire,
        Vec2::from_ints(9740, 9868),
    ));
    units.push(me);
    let seat = fixtures::seat(SlotId(0), Team::Radiant, HeroId(2), Some(fixtures::id(1)));
    fixtures::tick(1000, units, vec![seat])
}

#[test]
fn a_scroll_carries_a_healthy_hero_back_to_its_lane() {
    let mut bot = stocked(vec![fixtures::sold(TANGO, 90, vec![])], Vec::new());
    let mut view = at_the_shop(|me| {
        me.items[0] = Some(stack(SCROLL, 1));
    });
    // Nothing in hand to spend, so the shop is not what answers first.
    view.players[0].gold = Some(0);
    let ask = bot.on_tick(&view);
    assert!(
        matches!(
            ask,
            Some(Ask {
                order: Order::Use {
                    slot: ItemSlot(0),
                    target: Target::Pos(_)
                },
                ..
            })
        ),
        "the lane is a walk away and a scroll is in hand, got {ask:?}"
    );
}

#[test]
fn a_scroll_is_not_spent_before_the_creeps_set_out() {
    // Walking there takes the whole of the pregame anyway, and the wait a
    // scroll leaves behind is longer than the walk it saved.
    let mut bot = stocked(vec![fixtures::sold(TANGO, 90, vec![])], Vec::new());
    let mut view = at_the_shop(|me| {
        me.items[0] = Some(stack(SCROLL, 1));
    });
    view.tick = 300;
    view.players[0].gold = Some(0);
    assert!(!matches!(
        bot.on_tick(&view),
        Some(Ask {
            order: Order::Use { .. },
            ..
        })
    ));
}

#[test]
fn a_scroll_is_not_channelled_with_a_foe_at_hand() {
    let mut bot = stocked(vec![fixtures::sold(TANGO, 90, vec![])], Vec::new());
    let mut view = at_the_shop(|me| {
        me.items[0] = Some(stack(SCROLL, 1));
    });
    view.players[0].gold = Some(0);
    view.units
        .push(fixtures::hero(5, Team::Dire, Vec2::from_ints(2400, 2900)));
    assert!(!matches!(
        bot.on_tick(&view),
        Some(Ask {
            order: Order::Use { .. },
            ..
        })
    ));
}

#[test]
fn a_tango_is_eaten_at_a_tree_within_its_reach() {
    let tree = Vec2::from_ints(8800, 8800);
    let mut bot = seated(vec![tree]);
    let view = tick(|me| {
        me.hp = me.max_hp / 3;
        me.items[0] = Some(stack(TANGO, 3));
    });
    assert_eq!(
        bot.on_tick(&view),
        Some(Ask::use_item(ItemSlot(0), Target::Pos(tree)))
    );
}

#[test]
fn a_hero_walks_to_the_tree_its_tango_needs() {
    // The lane is cleared of trees for four hundred and fifty units either
    // side of its centre, so the tango a hero carries up it never has one in
    // reach where it stands.
    let tree = Vec2::from_ints(8700, 8000);
    let mut bot = seated(vec![tree]);
    let view = tick(|me| {
        me.hp = me.max_hp / 3;
        me.items[0] = Some(stack(TANGO, 3));
    });
    let Some(Ask {
        order: Order::Move {
            target: Target::Pos(going),
        },
        ..
    }) = bot.on_tick(&view)
    else {
        panic!("the hero should walk to the tree");
    };
    assert_eq!(going, point_along(tree, LANE, TREE_STANDOFF));
    assert!(span(going, tree) < 165.0, "it stops within a tango's reach");
}

#[test]
fn a_tree_further_up_the_lane_is_not_walked_to() {
    // Towards the other side's fountain, which is not where a hero on a third
    // of its health should be going.
    let tree = Vec2::from_ints(9400, 9500);
    let mut bot = seated(vec![tree]);
    let view = tick(|me| {
        me.hp = me.max_hp / 3;
        me.items[0] = Some(stack(TANGO, 3));
    });
    let ask = bot.on_tick(&view);
    let going = match ask {
        Some(Ask {
            order: Order::Move {
                target: Target::Pos(going),
            },
            ..
        }) => going,
        other => panic!("expected a walk, got {other:?}"),
    };
    assert_ne!(going, point_along(tree, LANE, TREE_STANDOFF));
}

#[test]
fn a_tango_is_not_eaten_on_the_way_to_the_fountain() {
    // Down to a fifth, so the hero is walking home; a tango's hundred and
    // fifteen would run out well short of fighting health, and the fountain
    // gives that back in a moment for nothing.
    let tree = Vec2::from_ints(8800, 8800);
    let mut bot = seated(vec![tree]);
    let view = tick(|me| {
        me.hp = me.max_hp / 5;
        me.items[0] = Some(stack(TANGO, 3));
    });
    let ask = bot.on_tick(&view);
    assert!(
        !matches!(
            ask,
            Some(Ask {
                order: Order::Use { .. },
                ..
            })
        ),
        "expected the walk home, got {ask:?}"
    );
}

#[test]
fn a_salve_is_still_drunk_on_the_way_to_the_fountain() {
    // Four hundred carries the hero back over the bar it turns round at, so
    // the walk is what the salve saves rather than what it wastes.
    let mut bot = seated(Vec::new());
    let view = tick(|me| {
        me.hp = me.max_hp / 5;
        me.items[0] = Some(stack(SALVE, 1));
    });
    assert_eq!(
        bot.on_tick(&view),
        Some(Ask::use_item(ItemSlot(0), Target::Unit(fixtures::id(1))))
    );
}

#[test]
fn a_tango_is_still_eaten_by_a_hero_staying_in_its_lane() {
    let tree = Vec2::from_ints(8800, 8800);
    let mut bot = seated(vec![tree]);
    let view = tick(|me| {
        // Hurt enough to want the tango, not enough to leave.
        me.hp = me.max_hp / 2;
        me.items[0] = Some(stack(TANGO, 3));
    });
    assert_eq!(
        bot.on_tick(&view),
        Some(Ask::use_item(ItemSlot(0), Target::Pos(tree)))
    );
}

#[test]
fn a_hero_in_the_middle_of_a_channel_is_told_nothing() {
    // An order is how a channel is thrown away, and a scroll takes three
    // seconds of standing still.
    let mut bot = seated(Vec::new());
    let view = tick(|me| {
        me.statuses = bota_proto::StatusFlags {
            bits: bota_proto::StatusFlags::CHANNELLING,
        };
    });
    assert_eq!(bot.on_tick(&view), None);
}

#[test]
fn a_held_hero_is_told_nothing() {
    let mut bot = seated(Vec::new());
    let view = tick(|me| {
        me.statuses = bota_proto::StatusFlags {
            bits: bota_proto::StatusFlags::STUNNED,
        };
    });
    assert_eq!(bot.on_tick(&view), None);
}

#[test]
fn a_wand_is_pressed_for_the_pool_it_would_fill() {
    let mut bot = seated(Vec::new());
    let view = tick(|me| {
        me.mana = 50;
        me.items[0] = Some(stack(MAGIC_WAND, 10));
    });
    assert_eq!(
        bot.on_tick(&view),
        Some(Ask::use_item(ItemSlot(0), Target::None))
    );
}

#[test]
fn a_wand_holding_little_is_left_alone() {
    let mut bot = seated(Vec::new());
    let view = tick(|me| {
        me.mana = 50;
        me.items[0] = Some(stack(MAGIC_WAND, 2));
    });
    assert!(
        !matches!(
            bot.on_tick(&view),
            Some(Ask {
                order: Order::Use { .. },
                ..
            })
        ),
        "two charges are not worth the press"
    );
}

#[test]
fn a_wand_is_left_alone_when_nothing_is_missing() {
    let mut bot = seated(Vec::new());
    let view = tick(|me| {
        me.items[0] = Some(stack(MAGIC_WAND, 10));
    });
    assert!(
        !matches!(
            bot.on_tick(&view),
            Some(Ask {
                order: Order::Use { .. },
                ..
            })
        ),
        "a full hero wastes what it presses"
    );
}

/// The lane laid out with its two tier-one towers, the hero where it is put.
fn in_the_lane(at: Vec2, shape: impl Fn(&mut Vec<UnitView>)) -> bota_proto::WorldView {
    let mut units = fixtures::fountains();
    units.push(fixtures::works(
        10,
        bota_proto::UnitKind::Tower,
        Team::Radiant,
        Vec2::from_ints(7672, 7808),
    ));
    units.push(fixtures::works(
        11,
        bota_proto::UnitKind::Tower,
        Team::Dire,
        Vec2::from_ints(9740, 9868),
    ));
    units.push(fixtures::hero(1, Team::Radiant, at));
    shape(&mut units);
    let seat = fixtures::seat(SlotId(0), Team::Radiant, HeroId(2), Some(fixtures::id(1)));
    fixtures::tick(1000, units, vec![seat])
}

/// Where the waves should meet on the lane the fixtures lay out: halfway
/// between the two tier-one towers.
const MEETING: Vec2 = Vec2::from_ints(8706, 8838);

/// A tick where two of their creeps are chewing on the hero.
fn being_chewed() -> (Playbook, bota_proto::WorldView) {
    let mut bot = seated(Vec::new());
    let view = in_the_lane(MEETING, |units| {
        units.push(fixtures::creep(
            20,
            Team::Dire,
            Vec2::from_ints(8760, 8890),
            550,
        ));
        units.push(fixtures::creep(
            21,
            Team::Dire,
            Vec2::from_ints(8780, 8910),
            550,
        ));
        // One of its own to point at, which is the only thing that lets
        // creeps go.
        units.push(fixtures::creep(
            30,
            Team::Radiant,
            Vec2::from_ints(8680, 8790),
            550,
        ));
    });
    // One tick first, so the bot knows which body is its own; only then do
    // the blows landed on it mean anything.
    bot.on_tick(&view);
    (bot, view)
}

#[test]
fn creeps_chewing_on_the_hero_are_shaken_off() {
    let (mut bot, view) = being_chewed();
    bot.on_events(999, &[struck(fixtures::id(20)), struck(fixtures::id(21))]);
    assert_eq!(
        bot.on_tick(&view),
        Some(Ask::swing_at(fixtures::id(30))),
        "it points at one of its own creeps, never at itself"
    );
}

#[test]
fn a_shake_is_followed_by_standing_still() {
    let (mut bot, view) = being_chewed();
    bot.on_events(999, &[struck(fixtures::id(20)), struck(fixtures::id(21))]);
    bot.on_tick(&view);
    let mut next = view.clone();
    next.tick += 1;
    assert_eq!(
        bot.on_tick(&next),
        Some(Ask::mine(Order::Move {
            target: Target::None
        })),
        "an order at a unit is a follow, and the follow is called off"
    );
}

#[test]
fn standing_still_after_a_shake_is_not_itself_a_shake() {
    // The tick after a shake is answered by standing still. If that tick
    // counted as a shake, every tick after it would be the tick after a
    // shake, and the hero would stand still for the rest of the match.
    let (mut bot, view) = being_chewed();
    bot.on_events(999, &[struck(fixtures::id(20)), struck(fixtures::id(21))]);
    bot.on_tick(&view);
    let mut after = view.clone();
    for step in 1..4 {
        after.tick = view.tick + step;
        let ask = bot.on_tick(&after);
        let standing = ask
            == Some(Ask::mine(Order::Move {
                target: Target::None,
            }));
        assert_eq!(
            standing,
            step == 1,
            "tick {step} after the shake answered {ask:?}"
        );
    }
}

#[test]
fn creeps_that_have_not_touched_the_hero_are_left_alone() {
    let (mut bot, view) = being_chewed();
    bot.on_events(999, &[struck(fixtures::id(20))]);
    assert_ne!(
        bot.on_tick(&view),
        Some(Ask::swing_at(fixtures::id(1))),
        "one creep is not a wave worth spending a tick on"
    );
}

/// A tick where the wave has been pushed a long way past the meeting spot,
/// with the hero following it up the lane.
fn wave_pushed(health: i32) -> (Playbook, bota_proto::WorldView) {
    let bot = seated(Vec::new());
    let view = in_the_lane(Vec2::from_ints(9200, 9300), |units| {
        units.push(fixtures::creep(
            20,
            Team::Dire,
            Vec2::from_ints(9400, 9500),
            550,
        ));
        // Its own wave is up there with it, which is how a lane comes to be
        // pushed in the first place, and what keeps the tower off the hero.
        units.push(fixtures::creep(
            30,
            Team::Radiant,
            Vec2::from_ints(9350, 9450),
            550,
        ));
        units.push(fixtures::creep(
            31,
            Team::Radiant,
            Vec2::from_ints(9300, 9400),
            550,
        ));
        units.push(fixtures::hero(5, Team::Dire, Vec2::from_ints(9900, 10000)));
        if let Some(me) = units.iter_mut().find(|unit| unit.id == fixtures::id(1)) {
            me.hp = me.max_hp * health / 100;
        }
    });
    (bot, view)
}

#[test]
fn a_wave_pushed_past_the_meeting_spot_is_called_back() {
    let (mut bot, view) = wave_pushed(100);
    assert_eq!(
        bot.on_tick(&view),
        Some(Ask::swing_at(fixtures::id(5))),
        "the order names their hero, which is what calls their creeps on"
    );
}

#[test]
fn a_hurt_hero_does_not_take_a_wave_onto_itself() {
    let (mut bot, view) = wave_pushed(50);
    assert_ne!(bot.on_tick(&view), Some(Ask::swing_at(fixtures::id(5))));
}

#[test]
fn goods_stranded_in_the_backpack_are_moved_forward() {
    let mut bot = seated(Vec::new());
    let view = tick(|me| {
        me.items[0] = Some(stack(TANGO, 3));
        me.items[7] = Some(stack(SCROLL, 1));
    });
    assert_eq!(
        bot.on_tick(&view),
        Some(Ask::mine(Order::Swap {
            from: ItemSlot(7),
            to: ItemSlot(1),
        }))
    );
}

#[test]
fn a_full_bag_leaves_the_backpack_where_it_is() {
    let mut bot = seated(Vec::new());
    let view = tick(|me| {
        for at in 0..6 {
            me.items[at] = Some(stack(bota_proto::ItemId(20), 1));
        }
        me.items[7] = Some(stack(SCROLL, 1));
    });
    assert!(!matches!(
        bot.on_tick(&view),
        Some(Ask {
            order: Order::Swap { .. },
            ..
        })
    ));
}
