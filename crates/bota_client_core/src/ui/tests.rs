//! The client core, tested without a window or a socket.

use crate::ui::ChannelStops;
use crate::ui::app::{App, CueKind, FeedKind, Killer, Tap, Victim, tap_again};
use crate::ui::slots::{Look, Press, Slot, ability_look, decide, item_look, order_for};
use bota_proto::{
    AbilityId, AbilitySlot, AbilityView, Aim, Angle, Attributes, EntityId, EventKind, Fixed,
    HeroId, ItemId, ItemView, Order, PlayerView, SlotId, StatusFlags, Target, Team, UnitKind,
    UnitView, Vec2, WorldView,
};

#[test]
fn the_console_turns_a_typed_line_into_a_cheat_order() {
    use bota_proto::Cheat;
    let parse = crate::commands::parse_console;
    for (line, cheat) in [
        ("-gold 500", Cheat::Gold { amount: 500 }),
        ("gold", Cheat::Gold { amount: 1000 }),
        ("-lvlup 3", Cheat::Levels { count: 3 }),
        ("-refresh", Cheat::Refresh),
        (
            "-item Butterfly",
            Cheat::Item {
                item: ItemId::BUTTERFLY,
            },
        ),
        (
            "-item 42",
            Cheat::Item {
                item: ItemId::MANGO,
            },
        ),
        (
            "-item item_blink",
            Cheat::Item {
                item: ItemId::BLINK_DAGGER,
            },
        ),
        (
            "dota_create_item item_flask",
            Cheat::Item {
                item: ItemId::HEALING_SALVE,
            },
        ),
    ] {
        assert_eq!(parse(line), Ok(Order::Cheat { cheat }), "{line:?}");
    }
    use crate::commands::{CommandError as Why, CursorUse};
    for (line, why) in [
        ("-fly", Why::UnknownCommand("fly".to_string())),
        ("-gold lots", Why::NotANumber("lots".to_string())),
        ("-item wings", Why::UnknownItem("wings".to_string())),
        ("-item", Why::NoItemNamed),
        ("-createhero", Why::NoHeroNamed),
        ("-createhero nobody", Why::UnknownHero("nobody".to_string())),
        ("-createhero 0 foe", Why::NotAllyOrEnemy("foe".to_string())),
        ("-createhero 0", Why::NeedsCursor(CursorUse::CreateHero)),
        ("-teleport", Why::NeedsCursor(CursorUse::Teleport)),
        ("-killcreeps rad", Why::NotCreepSide("rad".to_string())),
        ("dota_ability_debug 2", Why::NotOnOff("2".to_string())),
        ("", Why::Empty),
    ] {
        assert_eq!(parse(line), Err(why), "{line:?}");
    }
}

#[test]
fn a_part_in_hand_comes_off_what_the_shop_asks() {
    use crate::shop::{price_for, whole_price};
    let parts = [ItemId::BOOTS, ItemId::GLOVES, ItemId::BELT];
    let price = |item| whole_price(item).expect("sold");
    let built: i32 = parts.iter().map(|part| price(*part)).sum();
    assert_eq!(
        price_for(ItemId::POWER_TREADS, &[], None),
        built,
        "with nothing in hand the parts are asked for"
    );
    assert_eq!(
        price_for(ItemId::POWER_TREADS, &[ItemId::BOOTS], None),
        built - price(ItemId::BOOTS),
        "the boots already worn are not asked for twice"
    );
    assert_eq!(
        price_for(ItemId::BOOTS, &[ItemId::BOOTS], None),
        price(ItemId::BOOTS),
        "but a second of what is bought whole costs the whole of it"
    );
    assert_eq!(
        price_for(ItemId::POWER_TREADS, &[ItemId::POWER_TREADS], None),
        built,
        "and a second built one is built out of fresh parts"
    );
}

/// A hero body with every field named.
fn a_unit() -> UnitView {
    UnitView {
        id: EntityId {
            idx: 3,
            generation: 1,
        },
        kind: UnitKind::Hero,
        team: Team::Radiant,
        pos: Vec2 {
            x: Fixed::from_int(5000),
            y: Fixed::from_int(5000),
        },
        facing: Angle::default(),
        hp: 600,
        max_hp: 620,
        mana: 300,
        max_mana: 300,
        move_speed: Fixed::from_int(300),
        attack_damage: 55,
        attack_damage_min: 55,
        attack_damage_max: 55,
        hp_regen: Fixed::ZERO,
        mana_regen: Fixed::ZERO,
        town_portal: None,
        neutral: None,
        cast: None,
        queue: Vec::new(),
        status_resist: Fixed::ZERO,
        evasion: Fixed::ZERO,
        hit_health: None,
        vision_grant: None,
        cast_rates: None,
        seen_by: None,
        attack_range: Fixed::from_int(600),
        attack_time: 1700,
        attack_point: 500,
        attack_speed: 100,
        armor: Fixed::from_int(3),
        magic_resist: Fixed::from_ratio(25, 100),
        collision: Fixed::from_int(27),
        bound: Fixed::from_int(24),
        vision_radius: Fixed::from_int(1800),
        true_sight_radius: Fixed::ZERO,
        statuses: StatusFlags::default(),
        attributes: Attributes::ZERO,
        primary: None,
        hero: Some(HeroId::PUDGE),
        owner: Some(SlotId(0)),
        level: 4,
        abilities: Vec::new(),
        items: vec![None; 9],
        effects: Vec::new(),
    }
}

#[test]
fn a_press_means_what_its_slot_and_its_state_settle() {
    // Nothing here looks at cooldown, mana or points: the server is what
    // turns a press down, and it has to be reached to do it.
    let me = EntityId {
        idx: 3,
        generation: 1,
    };
    let own = Some(Some(Aim::Own));
    let point = Some(Some(Aim::Point));
    let unit = Some(Some(Aim::Unit));
    let (q, e, salve) = (Slot::Ability(0), Slot::Ability(2), Slot::Item(0));
    let cast = Press::Send(order_for(q, Target::None));
    let drink = |target| Press::Send(order_for(salve, target));
    let learn = Press::Send(Order::Learn {
        slot: AbilitySlot(2),
    });
    let cases = [
        // What needs no aim goes at once; a passive is nothing to press.
        (q, false, own, None, cast),
        (q, false, Some(None), None, Press::Nothing),
        (salve, false, Some(None), None, Press::Nothing),
        // What is aimed waits for a click.
        (q, false, point, None, Press::Aim(q)),
        // Control levels an ability, and is nothing to an item.
        (e, true, point, None, learn),
        (salve, true, own, None, drink(Target::None)),
        // An empty slot, the backpack and the stash answer nothing.
        (q, false, None, None, Press::Nothing),
        (Slot::Item(7), false, own, None, Press::Nothing),
        (Slot::Item(11), false, own, None, Press::Nothing),
        // A salve is taken up, and a second press drinks it; what is aimed
        // at the ground has no such shortcut.
        (salve, false, unit, None, Press::Aim(salve)),
        (salve, false, unit, Some(salve), drink(Target::Unit(me))),
        (salve, false, point, Some(salve), Press::Aim(salve)),
    ];
    for (case, (slot, ctrl, held, aiming, expected)) in cases.into_iter().enumerate() {
        let pressed = decide(slot, ctrl, held, aiming, Some(me));
        assert_eq!(pressed, expected, "case {case}");
    }
}

/// An ability slot with the numbers given.
fn an_ability(level: u8, cooldown: u32, mana_cost: i32, passive: bool, on: bool) -> AbilityView {
    AbilityView {
        id: AbilityId::MEAT_HOOK,
        level,
        max_level: 4,
        cooldown_left: cooldown,
        mana_cost,
        range: 600,
        aim: Aim::Unit,
        passive,
        on,
        can_level: false,
        learnable: Some(false),
    }
}

#[test]
fn what_keeps_an_ability_from_working_is_told_apart_from_what_it_is() {
    let ready = ability_look(Some(&an_ability(1, 0, 50, false, false)), 100);
    assert!(ready.filled && ready.learned && ready.usable, "ready");
    assert!(!ready.passive && !ready.toggled);

    let unlearned = ability_look(Some(&an_ability(0, 0, 50, false, false)), 100);
    assert!(unlearned.filled, "it is still in the slot");
    assert!(!unlearned.learned, "with no points in it");
    assert!(!unlearned.usable, "which is also why it cannot be used");

    let waiting = ability_look(Some(&an_ability(1, 45, 50, false, false)), 100);
    assert!(waiting.learned, "learned all the same");
    assert!(!waiting.usable);
    assert_eq!(waiting.cooldown_left, 45, "and the wait is worth showing");

    let poor = ability_look(Some(&an_ability(1, 0, 50, false, false)), 20);
    assert!(poor.learned && !poor.usable, "mana keeps it from working");
    assert_eq!(poor.cooldown_left, 0, "with nothing on the clock");

    let idle = ability_look(Some(&an_ability(1, 0, 0, true, false)), 100);
    assert!(idle.passive && !idle.usable, "a passive is never used");

    let running = ability_look(Some(&an_ability(1, 0, 0, false, true)), 100);
    assert!(running.toggled, "a toggle that is on says so");

    assert_eq!(
        ability_look(None, 100),
        Look::default(),
        "a slot this one does not carry is empty"
    );
}

/// An item view with the numbers given.
fn an_item(charges: Option<u8>, cooldown: u32, mana_cost: i32, aim: Option<Aim>) -> ItemView {
    ItemView {
        id: ItemId::BOOTS,
        charges,
        cooldown_left: cooldown,
        mute_left: 0,
        mode: None,
        mana_cost,
        range: 0,
        aim,
        for_sale: false,
        owner: SlotId(0),
        selling: Some(false),
        dispenser: None,
        locked: Some(false),
        disassembly: None,
        rune: None,
    }
}

#[test]
fn an_item_with_no_charges_at_all_is_not_an_empty_one() {
    let boots = an_item(None, 0, 0, None);
    let worn = item_look(Some(&boots), 0, 0);
    assert!(worn.filled && worn.passive, "boots are worn, not used");
    assert!(!worn.usable, "and pressing them does nothing");

    let stick = an_item(Some(0), 0, 0, Some(Aim::Own));
    assert!(
        !item_look(Some(&stick), 0, 0).usable,
        "an empty stack cannot be spent"
    );
    let charged = an_item(Some(3), 0, 0, Some(Aim::Own));
    assert!(
        item_look(Some(&charged), 0, 0).usable,
        "one with charges can"
    );
    assert!(
        !item_look(Some(&charged), 7, 0).usable,
        "the same stack in the backpack cannot"
    );

    let dear = an_item(None, 0, 75, Some(Aim::Own));
    assert!(!item_look(Some(&dear), 0, 50).usable, "mana is short");
    assert!(item_look(Some(&dear), 0, 75).usable, "and now it is not");

    let waiting = an_item(None, 30, 0, Some(Aim::Own));
    let look = item_look(Some(&waiting), 0, 0);
    assert!(
        !look.usable && look.cooldown_left == 30,
        "still on the clock"
    );

    assert_eq!(item_look(None, 0, 0), Look::default(), "an empty slot");
}

#[test]
fn a_second_reach_for_the_same_thing_makes_a_pair() {
    let hero = Tap::Hero;
    let (again, kept) = tap_again(None, hero);
    assert!(!again, "the first press is only a press");
    assert!(kept.is_some(), "and it is remembered for a moment");

    let (again, kept) = tap_again(kept, hero);
    assert!(again, "the second makes the pair");
    assert!(
        kept.is_none(),
        "which is spent: a third press starts over rather than pairing again"
    );

    let (again, kept) = tap_again(kept, hero);
    assert!(!again, "so the third is a single press");
    let (again, _) = tap_again(kept, Tap::Courier);
    assert!(!again, "and reaching for something else never pairs");

    let unit = |idx| Tap::Unit(EntityId { idx, generation: 1 });
    let (_, kept) = tap_again(None, unit(4));
    let (again, _) = tap_again(kept, unit(5));
    assert!(!again, "two units clicked in turn are two picks");
}

/// A seat with every field named.
fn a_player() -> PlayerView {
    PlayerView {
        slot: SlotId(0),
        team: Team::Radiant,
        hero: HeroId::PUDGE,
        unit: None,
        level: 1,
        xp: 0,
        gold: Some(600),
        stash: Some(vec![None; 6]),
        kit: None,
        kills: 0,
        deaths: 0,
        assists: 0,
        last_hits: 0,
        denies: 0,
        respawn_left: 0,
        creep_score: Some(bota_proto::CreepScore {
            last_hits: 0,
            denies: 0,
        }),
        seen_at: Some(0),
    }
}

#[test]
fn a_click_takes_the_ground_item_nearest_the_cursor_and_only_within_reach() {
    let handle = |idx: u32| EntityId { idx, generation: 1 };
    let lying_at = |idx: u32, x: i32, y: i32| bota_proto::LootView {
        id: handle(idx),
        pos: Vec2::from_ints(x, y),
        item: ItemId::BOOTS,
        charges: None,
        seen_by: None,
    };
    let app = an_app_seeing_loot(vec![lying_at(21, 1000, 1000), lying_at(22, 1040, 1000)]);
    let taken = |at| app.loot_at(at).map(|l| l.id);
    assert_eq!(
        taken((1010.0, 1000.0)),
        Some(handle(21)),
        "the nearer of the two"
    );
    assert_eq!(
        taken((1044.0, 1000.0)),
        Some(handle(22)),
        "and the nearer from the other side"
    );
    assert_eq!(
        taken((5000.0, 5000.0)),
        None,
        "far from both, the click is for the ground"
    );
}

/// A match of seat 0 whose first view holds this loot.
fn an_app_seeing_loot(loot: Vec<bota_proto::LootView>) -> App {
    let mut app = a_silent_app();
    app.my_slot = Some(SlotId(0));
    app.handle(a_match_start());
    let ours = owned(1, UnitKind::Hero, Some(SlotId(0)));
    let us = ours.id;
    let bota_proto::ServerMsg::Snapshot { mut view } =
        snapshot_of(10, vec![ours], vec![a_seat(SlotId(0), Some(us))])
    else {
        panic!("a snapshot");
    };
    view.loot = loot;
    app.handle(bota_proto::ServerMsg::Snapshot { view });
    app
}

/// A view of the whole map lists what no side may see; a click takes, and a front end rings,
/// only what the viewing side sees of it.
#[test]
fn a_smart_click_takes_only_the_ground_items_the_viewing_side_sees() {
    let handle = |idx: u32| EntityId { idx, generation: 1 };
    let lying = |idx: u32, seen: bota_proto::SeenBy| bota_proto::LootView {
        id: handle(idx),
        pos: Vec2::from_ints(1000, 1000),
        item: ItemId::BOOTS,
        charges: None,
        seen_by: Some(seen),
    };
    let app = an_app_seeing_loot(vec![
        lying(
            21,
            bota_proto::SeenBy {
                radiant: false,
                dire: true,
            },
        ),
        lying(
            22,
            bota_proto::SeenBy {
                radiant: true,
                dire: false,
            },
        ),
    ]);
    let taken = app.lying_at((1000.0, 1000.0)).map(|l| l.id);
    assert_eq!(
        taken,
        Some(handle(22)),
        "the one Radiant sees, not the one only Dire does"
    );
    assert_eq!(
        app.loot_at((1000.0, 1000.0)).map(|l| l.id),
        Some(handle(22)),
        "and an item's name is no more told than it is taken"
    );
}

/// A tick holding these units and these seats.
fn a_tick(tick: u32, units: Vec<UnitView>, players: Vec<PlayerView>) -> WorldView {
    WorldView {
        tick,
        viewer: None,
        units,
        projectiles: Vec::new(),
        players,
        felled_trees: Vec::new(),
        planted_trees: Vec::new(),
        loot: Vec::new(),
        aggro: Vec::new(),
        order_delays: Vec::new(),
        skipped_ticks: Vec::new(),
        stock: Vec::new(),
        talents: Vec::new(),
        teleports: Vec::new(),
        glyphs: Vec::new(),
        clock: None,
        runes: Vec::new(),
        pools: Vec::new(),
    }
}

/// A seat with the body it stands in, or none while it is down.
fn a_seat(slot: SlotId, unit: Option<EntityId>) -> PlayerView {
    let mut seat = a_player();
    seat.slot = slot;
    seat.unit = unit;
    seat
}

/// A unit of one kind belonging to one seat.
fn owned(idx: u32, kind: UnitKind, slot: Option<SlotId>) -> UnitView {
    let mut body = a_unit();
    body.id = EntityId { idx, generation: 1 };
    body.kind = kind;
    body.owner = slot;
    body
}

#[test]
fn a_seat_is_picked_by_the_body_it_last_stood_in() {
    let mut app = a_started_app();
    let first = owned(1, UnitKind::Hero, Some(SlotId(0)));
    let id = first.id;
    app.handle(snapshot_of(
        10,
        vec![first],
        vec![a_seat(SlotId(0), Some(id))],
    ));
    app.handle(snapshot_of(60, Vec::new(), vec![a_seat(SlotId(0), None)]));
    assert_eq!(
        app.body_of(SlotId(0)),
        Some(id),
        "the fallen body is the handle"
    );
    assert_eq!(
        app.body_of(SlotId(1)),
        None,
        "nothing is known of another seat"
    );
    let (held, stale) = app.known(id).expect("it is remembered");
    assert_eq!((held.id, stale), (id, 50), "as it was, fifty ticks ago");

    let again = owned(2, UnitKind::Hero, Some(SlotId(0)));
    let fresh = again.id;
    app.handle(snapshot_of(
        600,
        vec![again],
        vec![a_seat(SlotId(0), Some(fresh))],
    ));
    app.handle(snapshot_of(601, Vec::new(), vec![a_seat(SlotId(0), None)]));
    assert_eq!(
        app.body_of(SlotId(0)),
        Some(fresh),
        "the seat answers with the body it stood in last"
    );
}

/// A hostile stream shows one seat a thousand heroes, then none of them:
/// the client remembers one body for the seat, not a thousand (it kept every
/// one, and a stream rotating seats and kinds grew it without bound).
#[test]
fn a_seat_shown_many_bodies_then_none_is_remembered_by_one() {
    let mut app = a_started_app();
    let bodies: Vec<UnitView> = (1..=1000)
        .map(|idx| owned(idx, UnitKind::Hero, Some(SlotId(0))))
        .collect();
    let ids: Vec<EntityId> = bodies.iter().map(|body| body.id).collect();
    app.handle(snapshot_of(10, bodies, vec![a_seat(SlotId(0), None)]));
    app.handle(snapshot_of(2000, Vec::new(), vec![a_seat(SlotId(0), None)]));
    assert!(!app.lost, "the stream holds");
    let remembered = ids.iter().filter(|id| app.known(**id).is_some()).count();
    assert_eq!(remembered, 1, "one body for the seat");
}

/// A unit owned by a seat the match does not have breaks the stream.
#[test]
fn a_unit_owned_by_a_seat_outside_the_match_breaks_the_stream() {
    let mut app = a_started_app();
    let stray = owned(1, UnitKind::Hero, Some(SlotId(7)));
    app.handle(snapshot_of(10, vec![stray], vec![a_seat(SlotId(0), None)]));
    assert!(app.lost, "the client stops listening");
}

/// Teleports are a seat's one channel each: a stream telling one seat's many
/// breaks, as a front end draws and sounds every one each frame against
/// every other.
#[test]
fn a_seat_told_teleporting_twice_breaks_the_stream() {
    let mut app = a_started_app();
    let teleport = bota_proto::TeleportView {
        slot: SlotId(0),
        team: Team::Radiant,
        building: EntityId {
            idx: 9,
            generation: 1,
        },
        destination: Vec2::ZERO,
        ticks_left: 30,
        ticks_total: 90,
    };
    let mut view = a_tick(10, Vec::new(), vec![a_seat(SlotId(0), None)]);
    view.teleports = vec![teleport; 2];
    app.handle(bota_proto::ServerMsg::Snapshot { view });
    assert!(app.lost, "the client stops listening");
}

/// A view's numbers at their extremes are checked and shown without
/// overflowing: an attack point of u32::MAX milliseconds under a swing, and
/// a last hit by an attacker of i32::MAX damage.
#[test]
fn a_view_of_extreme_numbers_is_taken_in_without_overflow() {
    let mut app = a_started_app();
    let mut striker = owned(1, UnitKind::CreepMelee, None);
    striker.attack_point = u32::MAX;
    striker.attack_damage_min = i32::MAX;
    striker.attack_damage = i32::MAX;
    striker.attack_damage_max = i32::MAX;
    let target = owned(2, UnitKind::CreepMelee, None);
    let mut view = a_tick(
        10,
        vec![striker.clone(), target.clone()],
        vec![a_seat(SlotId(0), None)],
    );
    view.aggro = vec![bota_proto::AggroView {
        unit: striker.id,
        target: Some(target.id),
        swing: Some(bota_proto::SwingView {
            target: target.id,
            strike_in: u32::MAX,
        }),
        hold_left: 0,
        chase_left: 0,
        draw_cooldown: 0,
        returning: false,
        held_for: Some(0),
        chased_for: None,
        draw_wait: Some(0),
    }];
    app.handle(bota_proto::ServerMsg::Snapshot { view });
    assert!(app.lost, "a swing that far out is refused, not overflowed");
    let hit = crate::next_hit_raised(&striker, &target, Fixed::ZERO, i32::MAX);
    assert!(hit.least <= hit.most);
}

/// A lobby a server tells is checked before the lobby screen keeps it: more
/// rows than a match has seats, or a name longer than a server shows, breaks
/// the stream and is not kept.
#[test]
fn a_lobby_past_its_bounds_breaks_the_stream_and_is_not_kept() {
    let row = |slot: u8, name: String| bota_proto::LobbySlot {
        slot: SlotId(slot),
        team: Team::Radiant,
        name,
        role: None,
        hero: None,
        ready: false,
    };
    let state = |slots| bota_proto::ServerMsg::LobbyState {
        slots,
        same_heroes: true,
        pick: bota_proto::PickPhase::Waiting,
        heroes: Vec::new(),
        catalogue: bota_core::Catalogue::FULL.prefix(),
    };
    let crowded = (0..=crate::MAX_SEATS as u8)
        .map(|slot| row(slot, "seat".to_owned()))
        .collect();
    let long = vec![row(
        0,
        "n".repeat(bota_core::rules::MAX_SHOWN_NAME_BYTES + 1),
    )];
    for slots in [crowded, long] {
        let mut app = a_silent_app();
        app.handle(state(slots));
        assert!(app.lost, "the client stops listening");
        assert!(app.lobby.slots.is_empty() && app.names.is_empty());
    }
}

/// The heroes a lobby offers are a pool of the heroes there are: a server that names more is
/// refused before the lobby screen keeps (and walks, every frame) a list of its making.
#[test]
fn a_lobby_offering_more_heroes_than_there_are_breaks_the_stream() {
    let state = |heroes| bota_proto::ServerMsg::LobbyState {
        slots: Vec::new(),
        same_heroes: true,
        pick: bota_proto::PickPhase::Waiting,
        heroes,
        catalogue: bota_core::Catalogue::FULL.prefix(),
    };
    let every: Vec<HeroId> = (0..bota_core::HEROES.len() as u16).map(HeroId).collect();
    let mut app = a_silent_app();
    app.handle(state(every.clone()));
    assert!(!app.lost, "every hero there is is a pool");
    assert_eq!(app.lobby.heroes, every);
    let mut app = a_silent_app();
    app.handle(state(vec![HeroId::PUDGE; bota_core::HEROES.len() + 1]));
    assert!(app.lost, "the client stops listening");
    assert!(app.lobby.heroes.is_empty());
}

/// A unit told to see farther than any unit of the game can lights no more than the farthest
/// sight there is: a view of thousands of such units cannot make the fog take for ever.
#[test]
fn a_unit_told_to_see_past_the_farthest_sight_there_is_lights_no_farther() {
    use bota_core::{CellGrid, rules};
    let cells = rules::GRID_CELLS * rules::GRID_CELLS;
    let bota_proto::ServerMsg::MatchStart { mut info } = a_match_start() else {
        unreachable!("a match start");
    };
    info.terrain_cells = rules::GRID_CELLS as u32;
    info.terrain_rle = vec![
        (u16::MAX, 0x80),
        ((cells - usize::from(u16::MAX)) as u16, 0x80),
    ];
    let statics = crate::MatchStatic::new(&info).expect("an open map");
    let ground = crate::SightGround::lay(&statics, None).expect("the sight rule's grid");
    let centre = (rules::GRID_CELLS / 2, rules::GRID_CELLS / 2);
    let mut unit = a_unit();
    unit.pos = CellGrid::cell_center(centre);
    unit.vision_radius = Fixed::MAX;
    let mut lit = vec![false; cells];
    ground.lit(&a_tick(1, vec![unit], Vec::new()), Team::Radiant, &mut lit);
    let reach = (bota_core::MAX_VISION / rules::GRID_CELL_SIZE + 1) as usize;
    let farthest = lit
        .iter()
        .enumerate()
        .filter(|(_, lit)| **lit)
        .map(|(at, _)| {
            let (x, y) = (at % rules::GRID_CELLS, at / rules::GRID_CELLS);
            x.abs_diff(centre.0).max(y.abs_diff(centre.1))
        })
        .max();
    assert!(farthest.is_some(), "the unit sees");
    assert!(
        farthest <= Some(reach),
        "a cell {farthest:?} cells away is lit, past the farthest sight's {reach}"
    );
}

fn snapshot_of(tick: u32, units: Vec<UnitView>, players: Vec<PlayerView>) -> bota_proto::ServerMsg {
    bota_proto::ServerMsg::Snapshot {
        view: a_tick(tick, units, players),
    }
}

/// An app watching a replay of a match on an open 128-cell map with these
/// fountains, a seat for one.
fn a_started_app() -> App {
    let mut app = a_silent_app();
    app.my_slot = Some(SlotId(0));
    app.handle(a_match_start());
    app
}

/// The start of that match.
fn a_match_start() -> bota_proto::ServerMsg {
    bota_proto::ServerMsg::MatchStart {
        info: bota_proto::MatchInfo {
            match_id: 1,
            map: bota_proto::MapId(0),
            tick_rate: 30,
            pregame_ticks: 0,
            trees: Vec::new(),
            terrain_cells: 128,
            terrain_rle: vec![(16384, 0x80)],
            opaque_cells: Vec::new(),
            mode: bota_proto::TickMode::Lockstep,
            picks: vec![bota_proto::Pick {
                slot: SlotId(0),
                team: Team::Radiant,
                hero: HeroId::PUDGE,
            }],
            shop: Vec::new(),
            fountains: [Vec2::ZERO; 2],
            shop_range: 0,
            secret_shops: Vec::new(),
            universal_shop: false,
            cheats: false,
            objectives: bota_proto::ObjectiveSet::NONE,
            runes: bota_proto::RuneSet::NONE,
            catalogue: bota_core::Catalogue::FULL.prefix(),
        },
    }
}

#[test]
fn items_chosen_in_the_tactical_phase_are_bought_one_a_tick_once_the_match_begins() {
    use crate::ui::intent::Intent;
    let mut app = a_silent_app();
    app.my_slot = Some(SlotId(0));
    app.handle(bota_proto::ServerMsg::LobbyState {
        slots: vec![bota_proto::LobbySlot {
            slot: SlotId(0),
            team: Team::Radiant,
            name: "me".to_string(),
            role: Some(bota_proto::Role::Player),
            hero: Some(HeroId::PUDGE),
            ready: true,
        }],
        same_heroes: true,
        pick: bota_proto::PickPhase::Tactical { left_ms: 20_000 },
        heroes: Vec::new(),
        catalogue: bota_core::Catalogue::FULL.prefix(),
    });
    assert!(app.in_tactical_phase());
    app.act(Intent::Buy(ItemId::MANGO.0));
    app.act(Intent::Buy(ItemId::CLARITY.0));
    app.act(Intent::Buy(ItemId::BUTTERFLY.0));
    assert_eq!(
        app.queued_buys,
        vec![ItemId::MANGO.0, ItemId::CLARITY.0],
        "what the starting gold cannot pay is refused, not queued"
    );
    assert!(app.reject.is_some());
    assert!(app.gold_after_queue() < bota_core::rules::STARTING_GOLD);
    app.handle(a_match_start());
    let sent = |app: &App| app.shown_orders.last().map(|shown| shown.order);
    app.handle(snapshot_of(1, Vec::new(), vec![a_seat(SlotId(0), None)]));
    assert_eq!(
        sent(&app),
        Some(Order::Buy {
            item: ItemId::MANGO
        })
    );
    app.handle(snapshot_of(2, Vec::new(), vec![a_seat(SlotId(0), None)]));
    assert_eq!(
        sent(&app),
        Some(Order::Buy {
            item: ItemId::CLARITY
        })
    );
    assert!(app.queued_buys.is_empty());
}

#[test]
fn a_channel_that_wants_hold_or_stop_keeps_the_last_order_back_until_it_ends() {
    use crate::ui::intent::Intent;
    let channelling = |app: &mut App, tick: u32, on: bool| {
        let mut hero = owned(1, UnitKind::Hero, Some(SlotId(0)));
        if on {
            hero.statuses.bits |= StatusFlags::CHANNELLING;
        }
        let id = hero.id;
        app.handle(snapshot_of(
            tick,
            vec![hero],
            vec![a_seat(SlotId(0), Some(id))],
        ));
    };
    let sent = |app: &App| app.shown_orders.last().map(|shown| shown.order);
    let walk = |x: f32| Order::Move {
        target: Target::Pos(Vec2::from_ints(x as i32, 0)),
    };
    for (stops, held) in [
        (ChannelStops::default(), false),
        (
            ChannelStops {
                teleport: false,
                channels: true,
            },
            true,
        ),
    ] {
        let mut app = a_started_app();
        app.channel_stops = stops;
        channelling(&mut app, 10, true);
        app.act(Intent::MoveTo((100.0, 0.0)));
        app.act(Intent::MoveTo((200.0, 0.0)));
        assert_eq!(sent(&app), (!held).then(|| walk(200.0)), "{stops:?}");
        if !held {
            continue;
        }
        channelling(&mut app, 11, false);
        assert_eq!(
            sent(&app),
            Some(walk(200.0)),
            "only the last one is carried out"
        );
        // A stop is never kept back, and takes the kept order with it.
        channelling(&mut app, 12, true);
        app.act(Intent::MoveTo((300.0, 0.0)));
        app.act(Intent::Stop);
        assert_eq!(
            sent(&app),
            Some(Order::Move {
                target: Target::None
            })
        );
        channelling(&mut app, 13, false);
        assert_eq!(
            sent(&app),
            Some(Order::Move {
                target: Target::None
            }),
            "the move given before the stop is gone"
        );
    }
}

/// Dota keeps the two "Require Hold/Stop" settings apart: a Town Portal's
/// channel holds orders back by the teleport one alone, any other channel by
/// the other.
#[test]
fn a_teleport_holds_orders_back_by_its_own_option_alone() {
    use crate::ui::intent::Intent;
    let teleporting = |app: &mut App| {
        let mut hero = owned(1, UnitKind::Hero, Some(SlotId(0)));
        hero.statuses.bits |= StatusFlags::CHANNELLING;
        let id = hero.id;
        let mut view = a_tick(10, vec![hero], vec![a_seat(SlotId(0), Some(id))]);
        view.teleports = vec![bota_proto::TeleportView {
            slot: SlotId(0),
            team: Team::Radiant,
            building: EntityId {
                idx: 9,
                generation: 1,
            },
            destination: Vec2::ZERO,
            ticks_left: 30,
            ticks_total: 90,
        }];
        app.handle(bota_proto::ServerMsg::Snapshot { view });
        assert!(!app.lost, "the stream holds");
    };
    let walk = Order::Move {
        target: Target::Pos(Vec2::from_ints(100, 0)),
    };
    for (teleport, channels, held) in [(true, false, true), (false, true, false)] {
        let mut app = a_started_app();
        app.channel_stops = ChannelStops { teleport, channels };
        teleporting(&mut app);
        app.act(Intent::MoveTo((100.0, 0.0)));
        assert_eq!(
            app.shown_orders.last().map(|shown| shown.order),
            (!held).then_some(walk),
            "teleport option {teleport}, channels option {channels}"
        );
    }
}

/// An immediate use breaks a channel it does not ignore, as the server has it: "Require Stop" keeps
/// a Bottle back while a channel runs, and lets the Phase Boots, which ignore it, go through.
#[test]
fn a_channel_that_wants_stop_keeps_back_an_immediate_use_it_does_not_ignore() {
    use crate::ui::intent::Intent;
    use crate::ui::slots::Slot;
    let mut app = a_started_app();
    app.channel_stops = ChannelStops {
        teleport: false,
        channels: true,
    };
    let mut hero = owned(1, UnitKind::Hero, Some(SlotId(0)));
    hero.statuses.bits |= StatusFlags::CHANNELLING;
    let held = |id| {
        Some(ItemView {
            id,
            ..an_item(Some(3), 0, 0, Some(bota_proto::Aim::Own))
        })
    };
    hero.items[0] = held(ItemId::BOTTLE);
    hero.items[1] = held(ItemId::PHASE_BOOTS);
    let id = hero.id;
    app.handle(snapshot_of(
        10,
        vec![hero],
        vec![a_seat(SlotId(0), Some(id))],
    ));
    let sent = |app: &App| app.shown_orders.last().map(|shown| shown.order);
    app.act(Intent::Press {
        slot: Slot::Item(0),
        ctrl: false,
    });
    assert_eq!(sent(&app), None, "the Bottle would break the channel");
    app.act(Intent::Press {
        slot: Slot::Item(1),
        ctrl: false,
    });
    assert_eq!(
        sent(&app),
        Some(Order::Use {
            slot: bota_proto::ItemSlot(1),
            target: Target::None
        })
    );
}

#[test]
fn a_cast_point_shortens_while_a_feast_runs_on_a_hero_with_the_talent() {
    let mut app = a_started_app();
    let mut fiend = owned(1, UnitKind::Hero, Some(SlotId(0)));
    fiend.hero = Some(HeroId::SHADOW_FIEND);
    let id = fiend.id;
    let seat = vec![a_seat(SlotId(0), Some(id))];
    let told = bota_proto::TalentView {
        slot: SlotId(0),
        learned: bota_proto::Talents::default().with(bota_proto::TalentSlot(7)),
        learnable: bota_proto::Talents::default(),
        attribute_bonus: 0,
        attribute_bonus_learnable: false,
        may_learn: Some(bota_proto::Talents::default()),
        may_learn_attribute_bonus: Some(false),
    };
    let mut view = a_tick(10, vec![fiend.clone()], seat);
    view.talents = vec![told];
    app.handle(bota_proto::ServerMsg::Snapshot { view });
    let raze = AbilityId::RAZE_NEAR;
    assert_eq!(app.cast_point_secs(&fiend, raze), Some(0.55));
    fiend.effects.push(bota_proto::EffectView {
        id: bota_proto::EffectId::FEASTING,
        ticks_left: None,
        stacks: None,
        caster: None,
        inflictor: None,
        ticks_total: None,
    });
    assert_eq!(
        app.cast_point_secs(&fiend, raze),
        Some(0.385),
        "the level-25 talent cuts the cast point to 70% while a feast runs"
    );
    assert_eq!(
        app.cast_point_secs(&fiend, AbilityId::ROT),
        Some(0.0),
        "the rot has no cast point of its own"
    );
}

/// An app that has heard nothing, over an empty recording.
fn a_silent_app() -> crate::ui::app::App {
    let player = crate::ui::replay::ReplayPlayer::from_reader(std::io::Cursor::new(Vec::new()));
    crate::ui::app::App::new(crate::ui::app::Source::Replay(Box::new(player)))
}

#[test]
fn only_what_the_seat_bought_and_keeps_counts_toward_the_price() {
    let mut body = a_unit();
    body.id = EntityId {
        idx: 1,
        generation: 1,
    };
    let kept = ItemView {
        id: ItemId::BOOTS,
        ..an_item(None, 0, 0, None)
    };
    let sold = ItemView {
        id: ItemId::GLOVES,
        selling: Some(true),
        ..kept
    };
    let found = ItemView {
        id: ItemId::BELT,
        owner: SlotId(1),
        ..kept
    };
    body.items = vec![Some(kept), Some(sold), Some(found)];
    let mut seat = a_player();
    seat.unit = Some(body.id);
    seat.stash = Some(vec![Some(ItemView {
        id: ItemId::BLADES_OF_ATTACK,
        ..kept
    })]);
    let view = a_tick(1, vec![body], vec![seat.clone()]);
    assert_eq!(
        crate::shop::held_items(&view, &seat),
        vec![ItemId::BOOTS, ItemId::BLADES_OF_ATTACK],
        "a part marked for sale or bought by another seat is not spent on a build"
    );
}

#[test]
fn the_next_hit_is_what_the_server_would_land() {
    use crate::{HitRange, next_hit_raised};
    let next_hit =
        |attacker: &UnitView, target: &UnitView| next_hit_raised(attacker, target, target.armor, 0);
    let blade = |mute_left| ItemView {
        id: ItemId::QUELLING_BLADE,
        mute_left,
        ..an_item(None, 0, 0, None)
    };
    let mut hero = a_unit();
    hero.attack_damage = 45;
    hero.attack_damage_min = 40;
    hero.attack_damage_max = 50;
    // Two working blades (each adds against creeps), one muted, one in the backpack.
    hero.items = vec![
        Some(blade(0)),
        Some(blade(30)),
        Some(blade(0)),
        None,
        None,
        None,
        Some(blade(0)),
    ];
    let mut creep = a_unit();
    creep.kind = UnitKind::CreepMelee;
    creep.team = Team::Dire;
    creep.armor = Fixed::from_int(2);
    // (40 + 16), (45 + 16) and (50 + 16), each times 100 / 112, rounded down.
    assert_eq!(
        next_hit(&hero, &creep),
        HitRange {
            least: 50,
            mid: 54,
            most: 58
        }
    );
    // A ranged hero's blade adds 4, not 8: (40 + 8), (45 + 8), (50 + 8).
    let mut fiend = hero.clone();
    fiend.hero = Some(bota_proto::HeroId::SHADOW_FIEND);
    assert_eq!(
        next_hit(&fiend, &creep),
        HitRange {
            least: 42,
            mid: 47,
            most: 51
        }
    );

    // The creep bonus is worth nothing against a tower, which also takes half.
    let mut tower = creep.clone();
    tower.kind = UnitKind::Tower;
    tower.armor = Fixed::ZERO;
    assert_eq!(
        next_hit(&hero, &tower),
        HitRange {
            least: 20,
            mid: 22,
            most: 25
        }
    );
}

#[test]
fn a_unit_is_drawn_between_two_ticks_and_jumps_are_not_drawn_out() {
    let mut walker = a_unit();
    walker.id = EntityId {
        idx: 1,
        generation: 1,
    };
    let mut blinker = walker.clone();
    blinker.id = EntityId {
        idx: 2,
        generation: 1,
    };
    walker.pos = Vec2::from_ints(1000, 1000);
    blinker.pos = Vec2::from_ints(1000, 1000);
    let before = a_tick(1, vec![walker.clone(), blinker.clone()], vec![]);
    walker.pos = Vec2::from_ints(1030, 1000);
    blinker.pos = Vec2::from_ints(2000, 1000);
    let after = a_tick(2, vec![walker, blinker], vec![]);
    let bodies = crate::ui::smooth::bodies(&after, Some(&before), 0.5);
    assert!((bodies[0].x - 1015.0).abs() < 0.01, "halfway along a step");
    assert!(
        (bodies[1].x - 2000.0).abs() < 0.01,
        "a blink is shown at once"
    );
}

/// The one talent tree is Shadow Fiend's, whatever id the hero list gives him.
#[test]
fn the_talent_tree_is_shadow_fiends() {
    for (id, hero) in bota_core::HEROES.iter().enumerate() {
        assert_eq!(
            crate::catalog::has_talents(id as u16),
            hero.name == "Shadow Fiend",
            "{}",
            hero.name
        );
    }
}

/// A hero's attributes of its level are floored as the server floors them;
/// a creep has no base to split its damage by (the hero's split is the
/// server's E2E test `a_branch_is_white_damage_and_green_attributes_...`).
#[test]
fn level_attributes_floor_and_a_creep_does_not_split() {
    let mut fiend = a_unit();
    fiend.hero = Some(HeroId::SHADOW_FIEND);
    fiend.primary = Some(bota_proto::Attribute::Agility);
    fiend.level = 11;
    // Growth is Fixed arithmetic, as the server's own: 19 + 2.7 * 10 stands a
    // hair under 46, and the server floors it too.
    assert_eq!(crate::catalog::base_attributes(&fiend), Some([45, 60, 37]));
    let mut creep = a_unit();
    creep.hero = None;
    creep.attack_damage_min = 19;
    creep.attack_damage_max = 23;
    let d = crate::catalog::damage_range(&creep);
    assert_eq!((d.min, d.max, d.bonus), (19, 23, 0));
}

#[test]
fn a_unit_ability_aimed_at_no_unit_stays_taken_up() {
    use crate::ui::intent::Intent;
    let mut app = a_started_app();
    let mut hero = owned(1, UnitKind::Hero, Some(SlotId(0)));
    hero.abilities = vec![an_ability(1, 0, 0, false, false)];
    let target = EntityId {
        idx: 2,
        generation: 1,
    };
    app.handle(snapshot_of(
        1,
        vec![hero.clone()],
        vec![a_seat(SlotId(0), Some(hero.id))],
    ));
    app.aiming = Some(Slot::Ability(0));
    app.act(Intent::Aimed {
        at: (10.0, 10.0),
        unit: None,
    });
    assert_eq!(app.aiming, Some(Slot::Ability(0)), "the mode is kept");
    assert!(
        app.reject.is_none(),
        "nothing to tell: the cursor still aims"
    );
    assert!(app.shown_orders.is_empty(), "no order goes out");
    app.act(Intent::Aimed {
        at: (10.0, 10.0),
        unit: Some(target),
    });
    assert_eq!(app.aiming, None);
    assert_eq!(
        app.shown_orders.last().map(|shown| shown.order),
        Some(Order::Cast {
            slot: AbilitySlot(0),
            target: Target::Unit(target)
        })
    );
}

#[test]
fn the_front_end_is_handed_what_the_server_told_until_the_next_pump() {
    let mut app = a_started_app();
    app.handle(snapshot_of(1, Vec::new(), vec![a_seat(SlotId(0), None)]));
    let told = vec![
        EventKind::Teleported {
            slot: SlotId(0),
            landed: true,
        },
        EventKind::Landed {
            inflictor: bota_proto::Inflictor::Item(bota_proto::ItemId::BLOOD_GRENADE),
            caster: None,
            at: bota_proto::Vec2::from_ints(100, 200),
            missile: None,
        },
    ];
    app.handle(bota_proto::ServerMsg::Events {
        tick: 1,
        events: told.clone(),
    });
    assert_eq!(app.told, told, "a teleport's end and a landing, in order");
    app.pump(0.0);
    assert!(app.told.is_empty(), "each pump starts afresh");
}

/// A replay of the whole map holds both sides' team lines; watched through
/// a seat's eyes it shows everyone's lines and that side's own only, as the
/// seat heard them.
#[test]
fn a_replay_through_a_seats_eyes_hides_the_other_sides_team_lines() {
    use bota_proto::{Audience, Body, Heard, Line, LobbySlot, PickPhase, ServerMsg, Speaker};
    let mut app = a_silent_app();
    let seat = |slot, team| LobbySlot {
        slot: SlotId(slot),
        team,
        name: format!("seat {slot}"),
        role: Some(bota_proto::Role::Bot),
        hero: None,
        ready: false,
    };
    app.handle(ServerMsg::LobbyState {
        slots: vec![seat(0, Team::Radiant), seat(1, Team::Dire)],
        same_heroes: true,
        pick: PickPhase::Waiting,
        heroes: Vec::new(),
        catalogue: bota_core::Catalogue::FULL.prefix(),
    });
    let line = |slot, to, text: &str| {
        ServerMsg::Heard(Heard {
            tick: None,
            from: Speaker::Seat(SlotId(slot)),
            line: Line {
                to,
                body: Body::Text(text.to_owned()),
            },
        })
    };
    app.handle(line(0, Audience::Team, "radiant plans"));
    app.handle(line(1, Audience::Team, "dire plans"));
    app.handle(line(1, Audience::All, "gg"));
    let shown = |app: &crate::ui::app::App| -> Vec<String> {
        app.chat_lines()
            .map(|line| match &line.heard.line.body {
                Body::Text(text) => text.clone(),
                Body::Ping(_) => String::new(),
            })
            .collect()
    };
    assert_eq!(shown(&app), ["radiant plans", "dire plans", "gg"]);
    app.pov = Some(SlotId(0));
    assert_eq!(shown(&app), ["radiant plans", "gg"]);
    app.pov = Some(SlotId(1));
    assert_eq!(shown(&app), ["dire plans", "gg"]);
}

/// An app seated in slot 0 (Radiant Pudge) of a match against slot 1 (Dire
/// Shadow Fiend), shown our hero `idx` 1 standing and the enemy hero `idx`
/// 2 out of sight but on the scoreboard, as a seat's view tells it.
fn a_duel_app() -> (App, EntityId, EntityId) {
    let mut app = a_silent_app();
    app.my_slot = Some(SlotId(0));
    let mut start = a_match_start();
    if let bota_proto::ServerMsg::MatchStart { info } = &mut start {
        info.picks.push(bota_proto::Pick {
            slot: SlotId(1),
            team: Team::Dire,
            hero: HeroId::SHADOW_FIEND,
        });
    }
    app.handle(start);
    let ours = owned(1, UnitKind::Hero, Some(SlotId(0)));
    let theirs = EntityId {
        idx: 2,
        generation: 1,
    };
    let mut enemy = a_seat(SlotId(1), Some(theirs));
    enemy.team = Team::Dire;
    enemy.hero = HeroId::SHADOW_FIEND;
    let us = ours.id;
    app.handle(snapshot_of(
        10,
        vec![ours],
        vec![a_seat(SlotId(0), Some(us)), enemy],
    ));
    (app, us, theirs)
}

#[test]
fn a_core_cue_keeps_the_event_that_determines_its_visibility() {
    let (mut app, us, _) = a_duel_app();
    let event = EventKind::AbilityCast {
        caster: us,
        ability: bota_proto::AbilityId::RAZE_NEAR,
    };
    app.handle(bota_proto::ServerMsg::Events {
        tick: 10,
        events: vec![event.clone()],
    });
    let cue = app
        .cues
        .iter()
        .find(|cue| cue.kind == crate::ui::app::CueKind::Raze)
        .expect("the cast's real cue");
    assert_eq!(cue.told.as_ref(), Some(&event));
}

#[test]
fn an_events_previous_unit_stays_at_its_origin_after_later_snapshots_arrive() {
    let (mut app, us, _) = a_duel_app();
    let mut before = app.view().unwrap().clone();
    before.tick = 11;
    let from = before.unit(us).unwrap().pos;
    app.handle(bota_proto::ServerMsg::Snapshot {
        view: before.clone(),
    });
    let mut after = before;
    after.tick = 12;
    let unit = after.units.iter_mut().find(|unit| unit.id == us).unwrap();
    unit.pos = from + bota_proto::Vec2::from_ints(1, 0);
    let to = unit.pos;
    app.handle(bota_proto::ServerMsg::Snapshot {
        view: after.clone(),
    });
    let event = EventKind::ItemUsed {
        caster: Some(us),
        item: bota_proto::ItemId::BLINK_DAGGER,
        at: Some(to),
        target: None,
    };
    app.handle(bota_proto::ServerMsg::Events {
        tick: 12,
        events: vec![event.clone()],
    });
    for tick in [13, 14] {
        after.tick = tick;
        app.handle(bota_proto::ServerMsg::Snapshot {
            view: after.clone(),
        });
    }
    let index = app.told.iter().position(|told| *told == event).unwrap();
    assert_eq!(app.prev_view().unwrap().unit(us).unwrap().pos, to);
    assert_eq!(app.before_event(index, us).unwrap().pos, from);
}

/// A coexistence guard for the HUD feed. The server and headless flagbearer_gold tests
/// reproduce the split awards; this test keeps the one-award hero feed path intact.
#[test]
fn death_awards_keep_one_hero_feed_line_and_do_not_repeat_its_gold_caption_guard() {
    use bota_proto::{GoldAward, GoldAwardKind};
    let (mut app, us, theirs) = a_duel_app();
    let recipient = app.my_slot.expect("own seat");
    let team = app.my_team().expect("own team");
    // The duel helper puts the enemy in the fog; a world caption needs its visible body.
    let mut view = app.view().expect("a view").clone();
    let mut victim = owned(theirs.idx, UnitKind::Hero, Some(SlotId(1)));
    victim.id = theirs;
    victim.team = Team::Dire;
    view.units.push(victim);
    view.tick = 11;
    app.handle(bota_proto::ServerMsg::Snapshot { view });
    app.handle(bota_proto::ServerMsg::Events {
        tick: 11,
        events: vec![EventKind::Died {
            credited_hero: None,
            unit: theirs,
            killer: Some(us),
            denied: false,
            gold: 0,
            bounty: Some(200),
            assists: 0b100,
            awards: Some(Box::new(vec![GoldAward {
                recipient,
                team,
                kind: GoldAwardKind::KillBounty,
                amount: 200,
            }])),
        }],
    });
    assert_eq!(app.feed.len(), 1, "one death makes one feed line");
    assert!(matches!(
        app.feed[0].kind,
        crate::ui::app::FeedKind::HeroDied {
            gold: 200,
            assists: 0b100,
            ..
        }
    ));
    let captions: Vec<_> = app
        .floaters
        .iter()
        .filter(|floater| floater.kind == crate::ui::app::FloaterKind::Gold)
        .map(|floater| floater.amount)
        .collect();
    assert_eq!(captions, [200], "the feed does not duplicate its award");
}

#[test]
fn an_unseen_kill_award_floats_over_its_recipients_own_hero() {
    use bota_proto::{GoldAward, GoldAwardKind};
    let (mut app, us, theirs) = a_duel_app();
    assert!(
        app.freshest(theirs).is_none(),
        "the fallen hero was never seen"
    );
    let recipient = app.my_slot.expect("own seat");
    let team = app.my_team().expect("own side");
    app.handle(bota_proto::ServerMsg::Events {
        tick: 10,
        events: vec![EventKind::Died {
            credited_hero: None,
            unit: theirs,
            killer: Some(us),
            denied: false,
            gold: 0,
            bounty: Some(37),
            assists: 0,
            awards: Some(Box::new(vec![GoldAward {
                recipient,
                team,
                kind: GoldAwardKind::KillBounty,
                amount: 37,
            }])),
        }],
    });
    let captions: Vec<_> = app
        .floaters
        .iter()
        .filter(|floater| floater.kind == crate::ui::app::FloaterKind::Gold)
        .map(|floater| (floater.unit, floater.amount))
        .collect();
    assert_eq!(captions, [(us, 37)]);
    assert_eq!(
        app.feed.len(),
        1,
        "the unseen hero kill still has one feed line"
    );
}

#[test]
fn a_hero_slain_in_the_fog_is_fed_by_its_seat_with_its_killer() {
    let (mut app, us, theirs) = a_duel_app();
    app.handle(bota_proto::ServerMsg::Events {
        tick: 10,
        events: vec![EventKind::Died {
            credited_hero: None,
            unit: theirs,
            killer: Some(us),
            denied: false,
            gold: 0,
            bounty: Some(200),
            assists: 0b100,
            awards: None,
        }],
    });
    assert_eq!(
        app.feed.last().map(|line| line.kind.clone()),
        Some(FeedKind::HeroDied {
            victim: Victim::Seat(SlotId(1)),
            killer: Some(Killer::Seat(SlotId(0))),
            gold: 200,
            assists: 0b100,
        }),
        "the feed names the hero never in sight by its seat, and the seats that assisted"
    );
}

#[test]
fn a_fallen_tower_is_fed_with_what_it_was_and_where_it_stood() {
    let (mut app, _, _) = a_duel_app();
    app.handle(bota_proto::ServerMsg::Events {
        tick: 10,
        events: vec![EventKind::StructureDestroyed {
            unit: EntityId {
                idx: 9,
                generation: 1,
            },
            team: Team::Dire,
            kind: UnitKind::Tower,
            lane: Some(2),
            tier: Some(1),
            ranged: None,
        }],
    });
    assert_eq!(
        app.feed.last().map(|line| line.kind.clone()),
        Some(FeedKind::StructureFell {
            team: Team::Dire,
            kind: UnitKind::Tower,
            lane: Some(2),
            tier: Some(1),
            ranged: None,
            killer: None,
        })
    );
}

#[test]
fn a_fallen_tower_is_fed_with_the_hero_that_brought_it_down() {
    let (mut app, us, _) = a_duel_app();
    let tower = EntityId {
        idx: 9,
        generation: 1,
    };
    app.handle(bota_proto::ServerMsg::Events {
        tick: 10,
        events: vec![
            EventKind::Died {
                unit: tower,
                killer: Some(us),
                denied: false,
                gold: 0,
                bounty: None,
                assists: 0,
                awards: None,
                credited_hero: None,
            },
            EventKind::StructureDestroyed {
                unit: tower,
                team: Team::Dire,
                kind: UnitKind::Tower,
                lane: Some(0),
                tier: Some(1),
                ranged: None,
            },
        ],
    });
    assert_eq!(
        app.feed.last().map(|line| line.kind.clone()),
        Some(FeedKind::StructureFell {
            team: Team::Dire,
            kind: UnitKind::Tower,
            lane: Some(0),
            tier: Some(1),
            ranged: None,
            killer: Some(Killer::Seat(SlotId(0))),
        }),
        "the line names the hero that struck it"
    );
}

#[test]
fn a_fallen_building_is_cued_with_the_event_that_told_of_it() {
    let (mut app, _, _) = a_duel_app();
    let told = EventKind::StructureDestroyed {
        unit: EntityId {
            idx: 9,
            generation: 1,
        },
        team: Team::Dire,
        kind: UnitKind::Barracks,
        lane: Some(0),
        tier: None,
        ranged: Some(false),
    };
    app.handle(bota_proto::ServerMsg::Events {
        tick: 10,
        events: vec![told.clone()],
    });
    let cue = app
        .cues
        .iter()
        .find(|cue| cue.kind == CueKind::StructureFell)
        .expect("a cue for the fall");
    assert_eq!(cue.told, Some(told), "the cue says which building fell");
}

#[test]
fn our_hero_slain_from_the_fog_names_the_hero_the_feed_announced() {
    let (mut app, us, theirs) = a_duel_app();
    app.handle(bota_proto::ServerMsg::Events {
        tick: 10,
        events: vec![EventKind::Died {
            credited_hero: None,
            unit: us,
            killer: Some(theirs),
            denied: false,
            gold: 0,
            bounty: Some(200),
            assists: 0,
            awards: None,
        }],
    });
    let line = app.feed.last().map(|line| line.kind.clone()).expect("fed");
    assert!(
        matches!(
            line,
            FeedKind::HeroDied {
                killer: Some(Killer::Seat(SlotId(1))),
                ..
            }
        ),
        "{line:?}"
    );
}

#[test]
fn our_kill_of_a_creep_never_in_sight_sounds_its_last_hit() {
    let (mut app, us, _) = a_duel_app();
    app.cues.clear();
    app.handle(bota_proto::ServerMsg::Events {
        tick: 10,
        events: vec![EventKind::Died {
            credited_hero: None,
            unit: EntityId {
                idx: 3,
                generation: 1,
            },
            killer: Some(us),
            denied: false,
            gold: 0,
            bounty: Some(40),
            assists: 0,
            awards: None,
        }],
    });
    app.pump(0.0);
    assert!(
        app.cues
            .iter()
            .any(|cue| cue.kind == crate::ui::app::CueKind::LastHit),
        "the gold of a kill in the fog is ours, and sounds"
    );
    assert!(app.feed.is_empty(), "no hero fell");
}

#[test]
fn a_hero_falling_cues_its_death_for_that_hero() {
    let (mut app, us, theirs) = a_duel_app();
    app.cues.clear();
    app.handle(bota_proto::ServerMsg::Events {
        tick: 10,
        events: vec![EventKind::Died {
            credited_hero: None,
            unit: theirs,
            killer: Some(us),
            denied: false,
            gold: 0,
            bounty: Some(200),
            assists: 0,
            awards: None,
        }],
    });
    app.pump(0.0);
    let cue = app
        .cues
        .iter()
        .find(|cue| cue.kind == crate::ui::app::CueKind::HeroDeath)
        .expect("a hero fell");
    assert_eq!(
        cue.hero,
        Some(HeroId::SHADOW_FIEND),
        "the death sound is the fallen hero's own"
    );
}

#[test]
fn our_blow_cues_the_unit_it_lands_on() {
    let (mut app, us, theirs) = a_duel_app();
    app.cues.clear();
    app.handle(bota_proto::ServerMsg::Events {
        tick: 10,
        events: vec![EventKind::Damaged {
            source: Some(us),
            target: theirs,
            amount: 50,
            kind: bota_proto::DamageKind::Physical,
            inflictor: None,
            removal: false,
        }],
    });
    app.pump(0.0);
    let cue = app
        .cues
        .iter()
        .find(|cue| cue.kind == crate::ui::app::CueKind::Hit)
        .expect("a blow landed");
    assert_eq!(
        cue.target,
        Some(theirs),
        "the blow is on the unit it lands on"
    );
}

/// A Radiant view of the duel's two seats, the Dire seat's creep score as
/// given.
fn a_radiant_view_scoring_dire(score: Option<bota_proto::CreepScore>) -> bota_proto::ServerMsg {
    let mut ours = a_seat(SlotId(0), None);
    ours.team = Team::Radiant;
    let mut theirs = a_seat(SlotId(1), None);
    theirs.team = Team::Dire;
    theirs.hero = HeroId::SHADOW_FIEND;
    theirs.gold = None;
    theirs.stash = None;
    theirs.creep_score = score;
    theirs.seen_at = None;
    let mut view = a_tick(20, Vec::new(), vec![ours, theirs]);
    view.viewer = Some(Team::Radiant);
    bota_proto::ServerMsg::Snapshot { view }
}

#[test]
fn a_side_view_that_tells_the_other_sides_creep_score_is_refused() {
    let (mut app, _, _) = a_duel_app();
    app.handle(a_radiant_view_scoring_dire(None));
    assert!(!app.lost, "withheld, it is absent");
    let (mut app, _, _) = a_duel_app();
    app.handle(a_radiant_view_scoring_dire(Some(bota_proto::CreepScore {
        last_hits: 0,
        denies: 0,
    })));
    assert!(app.lost, "told, even as zero, it breaks the contract");
}

#[test]
fn a_side_view_that_tells_the_other_sides_glyph_cooldown_is_refused() {
    let glyph = |ready_after| bota_proto::GlyphView {
        team: Team::Dire,
        ready_in: 0,
        active_for: 0,
        ready_after,
    };
    for (ready_after, lost) in [(None, false), (Some(0), true)] {
        let (mut app, _, _) = a_duel_app();
        let mut msg = a_radiant_view_scoring_dire(None);
        if let bota_proto::ServerMsg::Snapshot { view } = &mut msg {
            view.glyphs = vec![glyph(ready_after)];
        }
        app.handle(msg);
        assert_eq!(app.lost, lost, "{ready_after:?}");
    }
}

#[test]
fn a_missile_that_looks_as_something_other_than_what_let_it_go_is_refused() {
    use bota_proto::{AbilityId, Angle, Inflictor, ProjectileModel, ProjectileView, Vec2};
    let shot = |team, inflictor, model| ProjectileView {
        id: EntityId {
            idx: 40,
            generation: 0,
        },
        pos: Vec2::from_ints(5000, 5000),
        facing: Angle::default(),
        team,
        ability: None,
        source: None,
        target: None,
        inflictor,
        seen_by: None,
        model,
    };
    let creep = Some(ProjectileModel::RangedCreep);
    let spell = Some(Inflictor::Ability(AbilityId::REQUIEM));
    for (missile, lost) in [
        (shot(Team::Dire, None, creep), false),
        (shot(Team::Dire, spell, None), false),
        (shot(Team::Dire, spell, creep), true),
        (shot(Team::Neutral, None, creep), true),
    ] {
        let (mut app, _, _) = a_duel_app();
        let mut msg = a_radiant_view_scoring_dire(None);
        if let bota_proto::ServerMsg::Snapshot { view } = &mut msg {
            view.projectiles = vec![missile];
        }
        app.handle(msg);
        assert_eq!(app.lost, lost, "{missile:?}");
    }
}

#[test]
fn a_captured_outpost_is_fed_with_its_building_and_carries_its_event() {
    let (mut app, us, theirs) = a_duel_app();
    let mut outpost = owned(5, UnitKind::Outpost, None);
    outpost.pos = bota_proto::Vec2::from_ints(3000, 3000);
    outpost.team = Team::Dire;
    app.handle(snapshot_of(
        11,
        vec![owned(1, UnitKind::Hero, Some(SlotId(0))), outpost.clone()],
        vec![a_seat(SlotId(0), Some(us)), {
            let mut enemy = a_seat(SlotId(1), Some(theirs));
            enemy.team = Team::Dire;
            enemy.hero = HeroId::SHADOW_FIEND;
            enemy
        }],
    ));
    assert!(
        app.freshest(outpost.id).is_some(),
        "the outpost is in view: {:?}",
        app.feed.last().map(|line| line.kind.clone())
    );
    let told = |team| EventKind::Captured {
        unit: outpost.id,
        team,
        by: None,
    };
    app.handle(bota_proto::ServerMsg::Events {
        tick: 11,
        events: vec![told(Team::Radiant), told(Team::Dire)],
    });
    let lines: Vec<(FeedKind, Option<EventKind>)> = app
        .feed
        .iter()
        .map(|line| (line.kind.clone(), line.told.clone()))
        .collect();
    assert_eq!(
        lines,
        [
            (
                FeedKind::Captured {
                    building: Some((UnitKind::Outpost, Team::Dire, outpost.pos)),
                    by: Team::Radiant,
                },
                Some(told(Team::Radiant))
            ),
            (
                FeedKind::Captured {
                    building: Some((UnitKind::Outpost, Team::Dire, outpost.pos)),
                    by: Team::Dire,
                },
                Some(told(Team::Dire))
            ),
        ],
        "each line carries the event it tells"
    );
}

#[test]
fn a_gathered_shrine_is_fed_with_the_seat_that_gathered_it() {
    let (mut app, us, _) = a_duel_app();
    let shrine = EntityId {
        idx: 9,
        generation: 1,
    };
    let told = |team, by| EventKind::ShrineActivated {
        unit: shrine,
        team,
        xp: 200,
        by,
    };
    app.handle(bota_proto::ServerMsg::Events {
        tick: 11,
        events: vec![told(Team::Radiant, Some(us)), told(Team::Dire, None)],
    });
    let kinds: Vec<FeedKind> = app.feed.iter().map(|line| line.kind.clone()).collect();
    assert_eq!(
        kinds,
        [
            FeedKind::ShrineActivated {
                by: Some(SlotId(0)),
                team: Team::Radiant,
                xp: 200
            },
            FeedKind::ShrineActivated {
                by: None,
                team: Team::Dire,
                xp: 200
            },
        ]
    );
}

#[test]
fn a_front_end_lists_every_effect_but_the_bar_ones() {
    use bota_proto::EffectId;
    assert!(crate::catalog::effect_listed(EffectId::MENDING.0));
    assert!(!crate::catalog::effect_listed(EffectId::TIMED_LIFE.0));
    assert!(!crate::catalog::effect_listed(EffectId::FOUNTAIN_PASSIVE.0));
    assert!(!crate::catalog::effect_listed(0), "the retired effect");
    assert!(!crate::catalog::effect_listed(u16::MAX));
}

/// Owner report: quick buy counted the stash only. What the hero carries and
/// what its courier carries are owned just as much, and only what is still
/// missing is listed.
#[test]
fn quick_buy_lists_only_what_is_missing_after_the_hero_stash_and_courier() {
    let mut app = a_started_app();
    let mut hero = owned(1, UnitKind::Hero, Some(SlotId(0)));
    hero.items = vec![Some(ItemView {
        id: ItemId::CIRCLET,
        ..an_item(None, 0, 0, None)
    })];
    let mut courier = owned(2, UnitKind::Courier, Some(SlotId(0)));
    courier.items = vec![Some(ItemView {
        id: ItemId::GAUNTLETS,
        ..an_item(None, 0, 0, None)
    })];
    let mut seat = a_seat(SlotId(0), Some(hero.id));
    seat.stash = Some(vec![None; 6]);
    app.handle(snapshot_of(10, vec![hero, courier], vec![seat]));
    app.set_quick_buy(ItemId::BRACER.0, false);
    assert_eq!(
        app.missing_for(ItemId::BRACER.0),
        vec![ItemId::RECIPE_BRACER]
    );
    assert_eq!(
        app.price_of(ItemId::BRACER.0).0,
        bota_core::item_def(ItemId::RECIPE_BRACER)
            .expect("a recipe")
            .cost,
        "the price shown is the recipe alone"
    );
}

/// An app with our hero holding an item in its first slot, an allied hero and courier, and an
/// enemy hero.
fn an_app_with_an_item_and_allies(item: ItemId) -> (App, EntityId, EntityId, EntityId) {
    let mut app = a_started_app();
    let mut hero = owned(1, UnitKind::Hero, Some(SlotId(0)));
    hero.items = vec![Some(ItemView {
        id: item,
        aim: bota_core::item_def(item).and_then(|def| def.aim),
        range: bota_core::item_def(item).map_or(0, |def| def.cast_range),
        ..an_item(None, 0, 0, None)
    })];
    let courier = owned(2, UnitKind::Courier, Some(SlotId(0)));
    let ally = owned(4, UnitKind::Hero, None);
    let mut enemy = owned(5, UnitKind::Hero, None);
    enemy.team = Team::Dire;
    let (hero_id, courier_id, ally_id) = (hero.id, courier.id, ally.id);
    app.handle(snapshot_of(
        10,
        vec![hero, courier, ally, enemy],
        vec![a_seat(SlotId(0), Some(hero_id))],
    ));
    (
        app,
        courier_id,
        ally_id,
        EntityId {
            idx: 5,
            generation: 1,
        },
    )
}

/// Control on an item that does something for an ally takes it up for one: the ring is the ally
/// use's reach, the slot is aimed at a unit, and a click on an allied hero uses it on that hero.
#[test]
fn control_takes_up_a_mango_for_an_allied_hero_at_its_reach() {
    use crate::ui::intent::Intent;
    use crate::ui::slots::Slot;
    let (mut app, _courier, ally, enemy) = an_app_with_an_item_and_allies(ItemId::MANGO);
    let slot = Slot::Item(0);
    assert_eq!(app.aim_in(slot), Some(bota_proto::Aim::Own));
    app.act(Intent::Press { slot, ctrl: true });
    assert_eq!(
        app.aim_in(slot),
        Some(bota_proto::Aim::Unit),
        "aimed at a unit now"
    );
    assert_eq!(
        app.reach_of(slot),
        400,
        "and its ring is a Mango's ally reach"
    );
    let sent = |app: &App| app.shown_orders.last().map(|shown| shown.order);
    app.act(Intent::Aimed {
        at: (0.0, 0.0),
        unit: Some(enemy),
    });
    app.act(Intent::Aimed {
        at: (0.0, 0.0),
        unit: None,
    });
    assert_eq!(
        sent(&app),
        None,
        "an enemy or the ground is no one to use it on"
    );
    app.act(Intent::Aimed {
        at: (0.0, 0.0),
        unit: Some(ally),
    });
    assert_eq!(
        sent(&app),
        Some(Order::Use {
            slot: bota_proto::ItemSlot(0),
            target: Target::Unit(ally)
        })
    );
    assert_eq!(app.aiming, None);
    // Pressed plain it is the Mango for its user again.
    app.act(Intent::Press { slot, ctrl: false });
    assert_eq!(app.aim_in(slot), Some(bota_proto::Aim::Own));
}

/// A ward is given to a hero or a courier, a Mango's effect lands on a hero only; neither goes to
/// an enemy.
#[test]
fn using_an_item_on_a_unit_names_only_one_it_does_something_for() {
    use crate::ui::intent::Intent;
    for (item, courier_ok) in [(ItemId::OBSERVER_WARD, true), (ItemId::MANGO, false)] {
        let (mut app, courier, ally, enemy) = an_app_with_an_item_and_allies(item);
        let sent = |app: &App| app.shown_orders.last().map(|shown| shown.order);
        app.act(Intent::UseOn {
            slot: 0,
            unit: enemy,
        });
        assert_eq!(sent(&app), None, "{item:?}: not an enemy");
        app.act(Intent::UseOn {
            slot: 0,
            unit: courier,
        });
        assert_eq!(sent(&app).is_some(), courier_ok, "{item:?}: the courier");
        app.act(Intent::UseOn {
            slot: 0,
            unit: ally,
        });
        assert_eq!(
            sent(&app),
            Some(Order::Use {
                slot: bota_proto::ItemSlot(0),
                target: Target::Unit(ally)
            }),
            "{item:?}: the allied hero"
        );
    }
}

/// A dead hero may swap what it carries, as in Dota: the panel's bag slots
/// read its kit while it lies dead and is the one commanded.
#[test]
fn a_dead_heros_bag_slots_read_its_kit() {
    let mut app = a_started_app();
    let hero = EntityId {
        idx: 7,
        generation: 1,
    };
    let boots = an_item(None, 0, 0, None);
    let mut dead = a_player();
    dead.unit = Some(hero);
    dead.respawn_left = 90;
    dead.kit = Some(bota_proto::Kit {
        abilities: Vec::new(),
        items: vec![Some(boots), None, None, None, None, None, None, None, None],
        town_portal: None,
    });
    app.handle(snapshot_of(1, Vec::new(), vec![dead]));
    assert_eq!(app.item_in(0), Some(boots), "its kit, while it lies dead");
    assert_eq!(app.item_in(1), None);
}

/// A link that is closed from its first poll on, and hands over the end of the match only at its
/// second: the end of the match sent just before the server hung up, still on its way when the
/// client polled.
#[derive(Default)]
struct HangingUp {
    polls: u32,
}

impl crate::ui::app::LiveLink for HangingUp {
    fn poll(&mut self) -> Vec<bota_proto::ServerMsg> {
        self.polls += 1;
        match self.polls {
            1 => Vec::new(),
            _ => vec![bota_proto::ServerMsg::MatchOver {
                winner: Team::Radiant,
                stats: bota_proto::MatchStats {
                    duration: 10,
                    slots: Vec::new(),
                },
            }],
        }
    }

    fn send(&mut self, _: &bota_proto::ClientMsg) -> std::io::Result<()> {
        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.polls >= 1
    }
}

#[test]
fn a_match_whose_end_is_still_on_the_way_when_the_link_closes_is_not_lost() {
    let mut app = App::new(crate::ui::app::Source::Live(Box::new(HangingUp::default())));
    app.my_slot = Some(SlotId(0));
    app.handle(a_match_start());

    app.pump(0.016);
    assert!(!app.lost, "the end of the match was not read yet");

    app.pump(0.016);
    assert!(app.over.is_some(), "the end of the match came");
    assert!(!app.lost, "a match that ended is not a lost connection");
}

#[test]
fn dead_carriers_grant_neither_secret_shop_access_nor_purchase_space() {
    let mut hero = a_unit();
    hero.kind = UnitKind::Hero;
    hero.owner = Some(SlotId(0));
    hero.pos = Vec2::from_ints(200, 200);
    hero.hp = 0;
    let mut player = a_player();
    player.unit = Some(hero.id);
    player.stash = Some(vec![None; bota_core::rules::STASH_SLOTS]);
    for courier in [false, true] {
        let mut body = hero.clone();
        if courier {
            body.kind = UnitKind::Courier;
            body.hero = None;
            player.unit = None;
        }
        let mut start = a_match_start();
        let bota_proto::ServerMsg::MatchStart { info } = &mut start else {
            unreachable!()
        };
        info.secret_shops = vec![bota_proto::ShopZoneView {
            pos: Vec2::from_ints(200, 200),
            corners: [(100, 100), (300, 100), (300, 300), (100, 300)]
                .map(|(x, y)| Vec2::from_ints(x, y))
                .to_vec(),
        }];
        let mut app = a_silent_app();
        app.my_slot = Some(SlotId(0));
        app.handle(start);
        app.handle(snapshot_of(1, vec![body.clone()], vec![player.clone()]));
        assert!(!app.at_secret_shop());
        assert!(!app.unit_at_secret_shop(body.id));
        // The pure prediction must likewise ignore a dead physical carrier on Dota's map.
        let mut view = a_tick(1, vec![body], vec![player.clone()]);
        view.units[0].pos = bota_core::DOTA_SECRET_SHOPS[0].pos;
        assert!(!crate::shop::purchase_fits(
            &view,
            &player,
            bota_core::map_of(bota_proto::MapId(2)),
            ItemId::DEMON_EDGE
        ));
    }
}

#[test]
fn quick_buy_refuses_an_excess_copy_with_a_message_and_denial_cue() {
    let mut app = a_started_app();
    for _ in 0..crate::MAX_QUICK_BUY_TARGETS {
        app.set_quick_buy(ItemId::BOOTS.0, true);
    }
    app.set_quick_buy(ItemId::BOOTS.0, true);
    assert_eq!(app.quick_buy.targets().len(), crate::MAX_QUICK_BUY_TARGETS);
    assert_eq!(
        app.reject.as_ref().map(|(why, _)| why),
        Some(&crate::Refused::QuickBuyFull)
    );
    assert!(
        app.cues
            .iter()
            .any(|cue| cue.kind == crate::CueKind::QuickBuyFull)
    );
    app.set_quick_buy(ItemId::CLARITY.0, false);
    assert_eq!(
        app.quick_buy.targets(),
        &[ItemId::CLARITY.0],
        "replacement still works at the bound"
    );
}

#[test]
fn assist_gold_floats_over_its_recipient_when_the_victim_is_unseen() {
    use bota_proto::{GoldAward, GoldAwardKind};
    let (mut app, us, victim) = a_duel_app();
    assert!(app.view().unwrap().unit(victim).is_none());
    app.handle(bota_proto::ServerMsg::Events {
        tick: 10,
        events: vec![EventKind::Died {
            credited_hero: None,
            unit: victim,
            killer: None,
            denied: false,
            gold: 0,
            bounty: Some(345),
            assists: 1,
            awards: Some(Box::new(vec![
                GoldAward {
                    recipient: SlotId(0),
                    team: Team::Radiant,
                    kind: GoldAwardKind::AssistGold,
                    amount: 77,
                },
                GoldAward {
                    recipient: SlotId(1),
                    team: Team::Dire,
                    kind: GoldAwardKind::AssistGold,
                    amount: 99,
                },
            ])),
        }],
    });
    let gold: Vec<_> = app
        .floaters
        .iter()
        .filter(|floater| floater.kind == crate::ui::app::FloaterKind::Gold)
        .map(|floater| (floater.unit, floater.amount))
        .collect();
    assert_eq!(
        gold,
        [(us, 77)],
        "only the watched recipient's actual credit is presented"
    );
}

#[test]
fn shared_kill_gold_floats_over_its_recipient_when_no_hero_lands_the_last_blow() {
    use bota_proto::{GoldAward, GoldAwardKind};
    let (mut app, us, victim) = a_duel_app();
    app.handle(bota_proto::ServerMsg::Events {
        tick: 10,
        events: vec![EventKind::Died {
            credited_hero: None,
            unit: victim,
            killer: None,
            denied: false,
            gold: 0,
            bounty: Some(133),
            assists: 0,
            awards: Some(Box::new(vec![GoldAward {
                recipient: SlotId(0),
                team: Team::Radiant,
                kind: GoldAwardKind::KillBounty,
                amount: 133,
            }])),
        }],
    });
    let gold: Vec<_> = app
        .floaters
        .iter()
        .filter(|floater| floater.kind == crate::ui::app::FloaterKind::Gold)
        .map(|floater| (floater.unit, floater.amount))
        .collect();
    assert_eq!(gold, [(us, 133)]);
}

#[test]
fn a_credited_tower_kill_names_the_hero_but_tracks_the_physical_attacker() {
    let (mut app, us, victim) = a_duel_app();
    let tower = EntityId {
        idx: 9,
        generation: 1,
    };
    let mut view = app.view().unwrap().clone();
    view.tick += 1;
    let mut enemy = owned(2, UnitKind::Hero, Some(SlotId(1)));
    enemy.team = Team::Dire;
    view.units.push(enemy);
    view.units.push(owned(9, UnitKind::Tower, None));
    app.handle(bota_proto::ServerMsg::Snapshot { view });
    app.handle(bota_proto::ServerMsg::Events {
        tick: 11,
        events: vec![EventKind::Died {
            unit: victim,
            killer: Some(tower),
            denied: false,
            gold: 0,
            bounty: Some(370),
            assists: 0,
            awards: None,
            credited_hero: Some(Box::new(us)),
        }],
    });
    assert!(
        matches!(
            app.feed.last().unwrap().kind,
            FeedKind::HeroDied {
                killer: Some(Killer::Seat(SlotId(0))),
                ..
            }
        ),
        "the public announcement names the credited hero"
    );
    assert_eq!(
        app.session()
            .unwrap()
            .entity(victim)
            .unwrap()
            .last_death
            .unwrap()
            .killer,
        Some(tower),
        "the physical death observation retains its meaning"
    );
    assert!(
        app.cues
            .iter()
            .any(|cue| cue.kind == crate::ui::app::CueKind::HeroKill)
    );
}

#[test]
fn scenery_reconnect_keeps_the_original_scenery_for_players_and_spectators() {
    for slot in [Some(SlotId(0)), None] {
        let mut app = App::new(crate::ui::app::Source::Live(Box::<HangingUp>::default()));
        for trees in [vec![Vec2::from_ints(100, 100)], Vec::new()] {
            app.handle(bota_proto::ServerMsg::Welcome {
                player_id: bota_proto::PlayerId(1),
                slot,
                tick_rate: 30,
                mode: bota_proto::TickMode::Realtime,
            });
            app.handle(bota_proto::ServerMsg::Scenery {
                map: bota_proto::MapId(0),
                trees,
            });
        }
        assert!(
            app.lost,
            "a reconnect cannot replace previously used scenery"
        );
        assert!(
            matches!(app.feed.last().map(|line| &line.kind), Some(FeedKind::ProtocolBroken { why }) if why == "server told another scenery")
        );
    }
}
