//! The attack procs items carry (Dota 7.41f), met as a player meets them: a two-seat match with
//! cheats on, each hero raised and handed its items by Dota's cheats (`-teleport`, `-lvlup`,
//! `-item`, `-createhero NAME enemy`, `-levelbots`), attacks by validated orders while the struck
//! player keeps its hero standing with `-refresh`, and what lands is read from the events the
//! match tells.
//!
//! Every chance rolls with Dota's pseudo-random distribution on a sequence of its source's own:
//! where a test says which swings proc, it draws the same hidden stream as an independent
//! sequence of Dota's published constants C(P) (the n-th attempt since the last proc procs with
//! n·C; a draw is made only where the attempt may fail), never a tolerance over a sample.

use bota_core::{mitigate, rules};
use bota_proto::{
    Cheat, DamageKind, EventKind, HeroId, Inflictor, ItemId, Order, Pick, SlotId, Team, UnitKind,
    Vec2,
};

use crate::game::{Command, Entity, Purpose, Stream, World, wire_id};

use super::support::*;

/// Dota's published pseudo-random constants C(P), in parts of 2^32.
const C10: u64 = 63_332_921; // 0.014745844781072
const C25: u64 = 363_973_103; // 0.084744091852316
const C30: u64 = 510_882_893; // 0.118949192725403
const C60: u64 = 1_815_266_771; // 0.422649730810374

/// Where the first strike of an item in inventory slot `slot` keeps its rolls: past the ability
/// slots, four places to an inventory slot.
fn item_place(slot: usize) -> u8 {
    (64 + slot * 4) as u8
}

/// Dota's pseudo-random distribution of constant `c`, drawing from the hidden stream of one
/// source of a unit's item procs.
struct Prd {
    stream: Stream,
    c: u64,
    misses: u64,
}

impl Prd {
    fn new(world: &World, unit: Entity, place: u8, c: u64) -> Prd {
        Prd {
            stream: world.rng.for_unit(Purpose::ItemProc, wire_id(unit), place),
            c,
            misses: 0,
        }
    }

    /// Whether the next attempt procs.
    fn next(&mut self) -> bool {
        let threshold = self.c * (self.misses + 1);
        let hits = threshold >= 1 << 32 || u64::from(self.stream.next_u32()) < threshold;
        self.misses = if hits { 0 } else { self.misses + 1 };
        hits
    }
}

/// Seat `slot` gives a cheat, validated as any order, and the world advances one tick.
fn seat_cheat(world: &mut World, slot: u8, cheat: Cheat) {
    let command = seat_command(world, slot, Order::Cheat { cheat });
    world.advance_trusted(&[command]);
}

/// A two-seat match with cheats on: seat 0's `hero` moved to open ground (`-teleport`), raised to
/// level 25 (`-lvlup`) and handed `items` (`-item`) in the slots the cheat puts them, and seat 1's
/// `foe` (an enemy player) moved `offset` from it. The foe's player keeps it standing with
/// `-refresh` while it is struck ([`attack_until`]).
fn duel(hero: HeroId, items: &[ItemId], foe: HeroId, offset: Vec2) -> (World, Entity, Entity) {
    let mut cfg = config();
    cfg.cheats = true;
    cfg.picks = vec![
        Pick {
            slot: SlotId(0),
            team: Team::Radiant,
            hero,
        },
        Pick {
            slot: SlotId(1),
            team: Team::Dire,
            hero: foe,
        },
    ];
    let mut world = World::for_match(&cfg, cfg.rng());
    world.settle();
    let unit = world.seats[0].unit.expect("stood up");
    let enemy = world.seats[1].unit.expect("stood up");
    let at = an_empty_spot(&world);
    seat_cheat(&mut world, 0, Cheat::Teleport { at });
    seat_cheat(&mut world, 0, Cheat::Levels { count: 24 });
    for &item in items {
        seat_cheat(&mut world, 0, Cheat::Item { item });
    }
    seat_cheat(&mut world, 0, Cheat::Refresh);
    seat_cheat(&mut world, 1, Cheat::Teleport { at: at + offset });
    seat_cheat(&mut world, 1, Cheat::Levels { count: 24 });
    // Both stand still until a test orders an attack: no swing rolls before it does.
    let stand = Order::Move {
        target: bota_proto::Target::None,
    };
    let both = [0, 1].map(|seat| seat_command(&world, seat, stand));
    world.advance_trusted(&both);
    (world, unit, enemy)
}

/// The inventory slot `item` sits in.
fn slot_holding(world: &World, unit: Entity, item: ItemId) -> usize {
    let bag = world.inventory.get(unit).expect("a bag");
    bag.slots
        .iter()
        .position(|held| held.is_some_and(|stack| stack.id == item))
        .expect("the item is carried")
}

/// An enemy `hero` created `offset` from `of` (`-createhero NAME enemy`), raised to level 25
/// (`-levelbots`).
fn foe(world: &mut World, of: Entity, hero: HeroId, offset: Vec2) -> Entity {
    let before: Vec<Entity> = enemy_heroes(world);
    let at = world.pos(of).expect("standing") + offset;
    seat_cheat(
        world,
        0,
        Cheat::CreateHero {
            hero,
            enemy: true,
            at,
        },
    );
    let created = enemy_heroes(world)
        .into_iter()
        .find(|unit| !before.contains(unit))
        .expect("a created enemy");
    seat_cheat(world, 0, Cheat::LevelBots { count: 24 });
    // Its seat has it stand still, where it was put.
    let seat = seat_of(world, created).expect("a created hero has a seat");
    let stand = seat_command(
        world,
        seat,
        Order::Move {
            target: bota_proto::Target::None,
        },
    );
    world.advance_trusted(&[stand]);
    created
}

fn enemy_heroes(world: &World) -> Vec<Entity> {
    world
        .entities
        .iter()
        .filter(|&unit| {
            world.kind.get(unit) == Some(&UnitKind::Hero)
                && world.team.get(unit) == Some(&Team::Dire)
        })
        .collect()
}

/// One blow `by` dealt, as the match told it.
#[derive(Clone, Copy, Debug)]
struct Blow {
    tick: u32,
    target: Entity,
    amount: i32,
    kind: DamageKind,
    inflictor: Option<Inflictor>,
}

/// What `by` did while it attacked: the ticks its swings went off, and every blow it dealt.
struct Attacks {
    launched: Vec<u32>,
    blows: Vec<Blow>,
}

/// The seat whose hero `unit` is.
fn seat_of(world: &World, unit: Entity) -> Option<u8> {
    (0..world.seats.len())
        .find(|&at| world.seats[at].unit == Some(unit))
        .and_then(|at| u8::try_from(at).ok())
}

/// `by`, a seat's hero, attacks `on` (a validated attack order) until `swings` of its own attack
/// blows have landed on `on`; then a second more, for what leaps on. The player of `on`, where it
/// is a seat's hero, keeps it standing with `-refresh` every two seconds (Dota's `-refresh`
/// readies every hero's items too: [`attack_unrefreshed`] for what waits).
fn attack_until(world: &mut World, by: Entity, on: Entity, swings: usize) -> Attacks {
    attack_for(world, by, on, swings, true)
}

/// As [`attack_until`], with nobody's `-refresh`: the struck has to outlast the swings itself.
fn attack_unrefreshed(world: &mut World, by: Entity, on: Entity, swings: usize) -> Attacks {
    attack_for(world, by, on, swings, false)
}

fn attack_for(world: &mut World, by: Entity, on: Entity, swings: usize, refresh: bool) -> Attacks {
    let attacker = world.owner.get(by).expect("a seat's unit attacks").0;
    let unit = (seat_of(world, by).is_none()).then(|| wire_id(by));
    let rolled = world
        .crit
        .iter()
        .flatten()
        .any(|rolls| rolls.sources().iter().any(Option::is_some));
    assert!(!rolled, "nothing rolled before the attack");
    let order = Order::Attack {
        target: bota_proto::Target::Unit(wire_id(on)),
    };
    let mut attacks = Attacks {
        launched: Vec::new(),
        blows: Vec::new(),
    };
    let mut landed = 0;
    let mut after = None;
    let mut commands = vec![unit_command(world, attacker, unit, order)];
    for step in 0..swings * 90 + 120 {
        let defender = seat_of(world, on).filter(|_| refresh && step % 60 == 59);
        if let Some(seat) = defender {
            commands.push(seat_command(
                world,
                seat,
                Order::Cheat {
                    cheat: Cheat::Refresh,
                },
            ));
        }
        let tick = world.tick;
        for event in world.advance_trusted(&std::mem::take(&mut commands)) {
            match event.kind() {
                EventKind::AttackLaunched { attacker, .. } if attacker == wire_id(by) => {
                    attacks.launched.push(tick);
                }
                EventKind::Damaged {
                    source: Some(source),
                    target,
                    amount,
                    kind,
                    inflictor,
                    ..
                } if source == wire_id(by) => {
                    let Some(target) = world.entities.iter().find(|&unit| wire_id(unit) == target)
                    else {
                        continue;
                    };
                    if target == on && inflictor.is_none() && after.is_none() {
                        landed += 1;
                    }
                    attacks.blows.push(Blow {
                        tick,
                        target,
                        amount,
                        kind,
                        inflictor,
                    });
                }
                _ => {}
            }
        }
        if landed >= swings && after.is_none() {
            // What leaps on from the last swing lands within a second; no more swings count.
            after = Some(world.tick + rules::TICKS_PER_SECOND);
            let stand = Order::Move {
                target: bota_proto::Target::None,
            };
            commands.push(unit_command(world, attacker, unit, stand));
        }
        if after.is_some_and(|until| world.tick >= until) {
            return attacks;
        }
    }
    panic!("{landed} of {swings} swings landed");
}

/// A seat's order for its hero, validated as the server validates any.
fn seat_command(world: &World, slot: u8, order: Order) -> Command {
    unit_command(world, slot, None, order)
}

/// A seat's order for one of its units (its hero when `unit` is none), validated as the server
/// validates any.
fn unit_command(
    world: &World,
    slot: u8,
    unit: Option<bota_proto::EntityId>,
    order: Order,
) -> Command {
    assert_eq!(world.validate_order(SlotId(slot), unit, &order), Ok(()));
    Command {
        slot: SlotId(slot),
        unit,
        order,
        queue: false,
        seq: 0,
    }
}

/// The attack's own blows on `on`, in order.
fn attack_blows(blows: &[Blow], on: Entity) -> Vec<Blow> {
    blows
        .iter()
        .filter(|blow| blow.target == on && blow.inflictor.is_none())
        .copied()
        .collect()
}

/// The least and the most `by`'s attack damage rolls, before anything takes its share.
fn damage_range(world: &World, by: Entity) -> (i32, i32) {
    let stats = world.stats.get(by).expect("derived");
    (stats.damage, stats.damage + stats.damage_spread)
}

/// What `on` takes of `amount` physical damage, through its armor.
fn physical_on(world: &World, on: Entity, amount: i32) -> i32 {
    let armor = world.stats.get(on).expect("derived").armor;
    mitigate(amount, DamageKind::Physical, armor, 0)
}

/// What `on` takes of `amount` magical damage, through its magic resistance.
fn magical_on(world: &World, on: Entity, amount: i32) -> i32 {
    let stats = world.stats.get(on).expect("derived");
    mitigate(
        amount,
        DamageKind::Magical,
        stats.armor,
        stats.magic_resist_bp,
    )
}

/// The span a swing of `by` at `on` takes off it when it deals `pct` percent of its damage roll.
fn taken_at(world: &World, by: Entity, on: Entity, pct: i32) -> std::ops::RangeInclusive<i32> {
    let (low, high) = damage_range(world, by);
    physical_on(world, on, low * pct / 100)..=physical_on(world, on, high * pct / 100)
}

/// A Crystalys carried makes its carrier's swings critical strikes of 160% (`crit_chance` 30,
/// `crit_multiplier` 160) on Dota's 30% pseudo-random run: the swings that strike critically are
/// those the run says, and each takes 160% of a damage roll.
#[test]
fn a_crystalys_strikes_critically_on_dotas_thirty_percent_run_for_160_percent() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::CRYSTALYS],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    let place = item_place(slot_holding(&world, hero, ItemId::CRYSTALYS));
    let mut run = Prd::new(&world, hero, place, C30);
    let (plain, crit) = (
        taken_at(&world, hero, enemy, 100),
        taken_at(&world, hero, enemy, 160),
    );
    assert!(plain.end() < crit.start(), "{plain:?} and {crit:?} apart");
    let blows = attack_blows(&attack_until(&mut world, hero, enemy, 40).blows, enemy);
    let expected: Vec<bool> = blows.iter().map(|_| run.next()).collect();
    assert!(expected.contains(&true) && expected.contains(&false));
    for (blow, critical) in blows.iter().zip(expected) {
        let span = if critical { &crit } else { &plain };
        assert!(
            span.contains(&blow.amount),
            "{blow:?}: {span:?}, critical {critical}"
        );
    }
}

/// Of several critical strikes the strongest rolls first and the first that lands counts alone
/// (Dota 7.30: "Multiple Critical Strike procs are now rolled from strongest to weakest and will
/// stop rolling once a proc occurs"): with a Daedalus (225%) and a Crystalys (160%), each swing
/// rolls the Daedalus's run, and the Crystalys's only when the Daedalus did not proc.
#[test]
fn of_a_daedalus_and_a_crystalys_the_daedalus_rolls_first_and_a_proc_stops_the_rolls() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::CRYSTALYS, ItemId::DAEDALUS],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    let place = |item| item_place(slot_holding(&world, hero, item));
    let mut daedalus = Prd::new(&world, hero, place(ItemId::DAEDALUS), C30);
    let mut crystalys = Prd::new(&world, hero, place(ItemId::CRYSTALYS), C30);
    let spans = [100, 160, 225].map(|pct| taken_at(&world, hero, enemy, pct));
    assert!(spans[0].end() < spans[1].start() && spans[1].end() < spans[2].start());
    let blows = attack_blows(&attack_until(&mut world, hero, enemy, 40).blows, enemy);
    let expected: Vec<usize> = blows
        .iter()
        .map(|_| {
            if daedalus.next() {
                2
            } else if crystalys.next() {
                1
            } else {
                0
            }
        })
        .collect();
    assert!(
        (0..3).all(|class| expected.contains(&class)),
        "{expected:?}"
    );
    for (blow, class) in blows.iter().zip(expected) {
        assert!(
            spans[class].contains(&blow.amount),
            "{blow:?}: {:?}",
            spans[class]
        );
    }
}

/// Revenant's Brooch's Phantom Critical leaves the swing as it is and deals 80% of its damage roll
/// on top as magical damage (`crit_chance` 30, `crit_multiplier` 80), on Dota's 30% run: the
/// magic lands the tick its swing does, laid to the Brooch, and only on the swings the run says.
#[test]
fn a_revenants_brooch_lands_its_critical_strike_as_magic_on_top_of_a_plain_swing() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::REVENANTS_BROOCH],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    let place = item_place(slot_holding(&world, hero, ItemId::REVENANTS_BROOCH));
    let mut run = Prd::new(&world, hero, place, C30);
    let plain = taken_at(&world, hero, enemy, 100);
    let (low, high) = damage_range(&world, hero);
    let magic =
        magical_on(&world, enemy, low * 80 / 100)..=magical_on(&world, enemy, high * 80 / 100);
    let all = attack_until(&mut world, hero, enemy, 40).blows;
    let brooch = Some(Inflictor::Item(ItemId::REVENANTS_BROOCH));
    for blow in attack_blows(&all, enemy) {
        assert!(
            plain.contains(&blow.amount),
            "{blow:?}: a plain swing, {plain:?}"
        );
        let riding: Vec<&Blow> = all
            .iter()
            .filter(|other| other.tick == blow.tick && other.inflictor == brooch)
            .collect();
        if run.next() {
            assert_eq!(riding.len(), 1, "the swing at {} procs", blow.tick);
            assert_eq!(riding[0].kind, DamageKind::Magical);
            assert_eq!(riding[0].target, enemy);
            assert!(
                magic.contains(&riding[0].amount),
                "{:?}: {magic:?}",
                riding[0]
            );
        } else {
            assert!(
                riding.is_empty(),
                "the swing at {} does not proc",
                blow.tick
            );
        }
    }
}

/// Phantom Critical "Counts as a Critical Strike for purposes of stacking": bota ranks it by the
/// share of the swing the struck takes in all (180%), so with a Crystalys (160%) the Brooch rolls
/// first and a Phantom Critical that lands leaves the Crystalys unrolled. A pin of bota's model:
/// no primary text says how Dota ranks it.
#[test]
fn a_phantom_critical_ranks_by_the_whole_it_deals_before_a_crystalys() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::CRYSTALYS, ItemId::REVENANTS_BROOCH],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    let place = |item| item_place(slot_holding(&world, hero, item));
    let mut brooch = Prd::new(&world, hero, place(ItemId::REVENANTS_BROOCH), C30);
    let mut crystalys = Prd::new(&world, hero, place(ItemId::CRYSTALYS), C30);
    let (plain, crit) = (
        taken_at(&world, hero, enemy, 100),
        taken_at(&world, hero, enemy, 160),
    );
    let all = attack_until(&mut world, hero, enemy, 40).blows;
    let magic = Some(Inflictor::Item(ItemId::REVENANTS_BROOCH));
    for blow in attack_blows(&all, enemy) {
        let phantom = all
            .iter()
            .any(|other| other.tick == blow.tick && other.inflictor == magic);
        let (expect_phantom, expect_crit) = if brooch.next() {
            (true, false)
        } else {
            (false, crystalys.next())
        };
        assert_eq!(phantom, expect_phantom, "{blow:?}");
        let span = if expect_crit { &crit } else { &plain };
        assert!(span.contains(&blow.amount), "{blow:?}: {span:?}");
    }
}

/// The span `on` takes of a swing of `by` that deals `pct` percent of its damage roll and `more`
/// on top.
fn taken_with(
    world: &World,
    by: Entity,
    on: Entity,
    pct: i32,
    more: i32,
) -> std::ops::RangeInclusive<i32> {
    let (low, high) = damage_range(world, by);
    physical_on(world, on, low * pct / 100 + more)..=physical_on(world, on, high * pct / 100 + more)
}

/// Which swings of a source that procs at constant `c` and then waits `wait` ticks proc, swing by
/// swing at the ticks `swings` went off: a swing while the source waits rolls nothing.
fn procs_with_wait(mut run: Prd, swings: &[u32], wait: u32) -> Vec<bool> {
    let mut last: Option<u32> = None;
    swings
        .iter()
        .map(|&tick| {
            let ready = last.is_none_or(|last| tick - last >= wait);
            let procs = ready && run.next();
            if procs {
                last = Some(tick);
            }
            procs
        })
        .collect()
}

/// A melee Skull Basher carrier bashes on Dota's 25% run (`bash_chance_melee`), rolled only while
/// the item is off its 2.3 s cooldown (`bash_cooldown`): each bash deals 100 more physical damage
/// (`bonus_chance_damage`) with the swing and stuns for 1.2 s (`bash_duration`), Dota's
/// `modifier_bashed`. A Yasha's attack speed keeps two swings apart shorter than the cooldown
/// and three longer.
#[test]
fn a_melee_skull_basher_bashes_on_a_quarter_run_while_off_its_cooldown() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::SKULL_BASHER, ItemId::YASHA],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    seat_cheat(
        &mut world,
        1,
        Cheat::Item {
            item: ItemId::HEART,
        },
    );
    let place = item_place(slot_holding(&world, hero, ItemId::SKULL_BASHER));
    let run = Prd::new(&world, hero, place, C25);
    let (plain, bashed) = (
        taken_with(&world, hero, enemy, 100, 0),
        taken_with(&world, hero, enemy, 100, 100),
    );
    assert!(plain.end() < bashed.start());
    let attacks = attack_unrefreshed(&mut world, hero, enemy, 24);
    let blows = attack_blows(&attacks.blows, enemy);
    assert_eq!(attacks.launched.len(), blows.len());
    let ticks: Vec<u32> = blows.iter().map(|blow| blow.tick).collect();
    let gaps: Vec<u32> = ticks.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let wait = ticks_of(2300);
    assert!(
        gaps.iter().all(|&gap| gap * 2 < wait && gap * 3 > wait),
        "{gaps:?}"
    );
    let expected = procs_with_wait(run, &ticks, wait);
    assert!(expected.contains(&true));
    for (blow, bash) in blows.iter().zip(expected) {
        let span = if bash { &bashed } else { &plain };
        assert!(
            span.contains(&blow.amount),
            "{blow:?}: {span:?}, bash {bash}"
        );
    }
}

/// A bash stuns for 1.2 s (`bash_duration`), Dota's `modifier_bashed`, laid to the Skull Basher.
#[test]
fn a_skull_bashers_bash_stuns_for_its_duration() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::SKULL_BASHER],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    seat_cheat(
        &mut world,
        0,
        Cheat::Item {
            item: ItemId::YASHA,
        },
    );
    let attacks = attack_until(&mut world, hero, enemy, 1);
    assert_eq!(attack_blows(&attacks.blows, enemy).len(), 1);
    attack_click(&mut world, enemy);
    for _ in 0..rules::TICKS_PER_SECOND * 20 {
        world.step();
        let stun = world.modifiers.get(enemy).and_then(|on_it| {
            on_it.active().find(|held| {
                held.inflictor == Some(Inflictor::Item(ItemId::SKULL_BASHER))
                    && matches!(held.kind, bota_core::ModifierKind::Control { .. })
            })
        });
        if let Some(stun) = stun {
            assert_eq!(
                stun.kind,
                bota_core::ModifierKind::Control {
                    control: bota_core::Control::Stun,
                    effect: bota_proto::EffectId::BASHED,
                }
            );
            let left = stun.ticks_left.expect("it wears off");
            assert!(
                left + 1 >= ticks_of(1200) && left <= ticks_of(1200),
                "{left}"
            );
            return;
        }
    }
    panic!("no bash in twenty seconds");
}

/// A ranged carrier bashes on Dota's 10% run (`bash_chance_ranged`).
#[test]
fn a_ranged_skull_basher_bashes_on_a_tenth_run() {
    let (mut world, hero, enemy) = duel(
        HeroId::SHADOW_FIEND,
        &[ItemId::SKULL_BASHER],
        HeroId::PUDGE,
        Vec2::from_ints(400, 0),
    );
    seat_cheat(
        &mut world,
        1,
        Cheat::Item {
            item: ItemId::HEART,
        },
    );
    let place = item_place(slot_holding(&world, hero, ItemId::SKULL_BASHER));
    let run = Prd::new(&world, hero, place, C10);
    let (plain, bashed) = (
        taken_with(&world, hero, enemy, 100, 0),
        taken_with(&world, hero, enemy, 100, 100),
    );
    assert!(plain.end() < bashed.start());
    let attacks = attack_unrefreshed(&mut world, hero, enemy, 30);
    let blows = attack_blows(&attacks.blows, enemy);
    // A missile lands as long after its swing went off as the last did: the waits of the
    // swings are the waits of their blows.
    let ticks: Vec<u32> = blows.iter().map(|blow| blow.tick).collect();
    let expected = procs_with_wait(run, &ticks, ticks_of(2300));
    assert!(expected.contains(&true));
    for (blow, bash) in blows.iter().zip(expected) {
        let span = if bash { &bashed } else { &plain };
        assert!(
            span.contains(&blow.amount),
            "{blow:?}: {span:?}, bash {bash}"
        );
    }
}

/// Bashes do not stack ("Does not stack with other sources of Bash"): with two Skull Bashers one
/// rolls, the first, on its own run and wait, and the second rolls nothing.
#[test]
fn two_skull_bashers_bash_as_one() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::SKULL_BASHER, ItemId::SKULL_BASHER, ItemId::YASHA],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    seat_cheat(
        &mut world,
        1,
        Cheat::Item {
            item: ItemId::HEART,
        },
    );
    let run = Prd::new(&world, hero, item_place(0), C25);
    let (plain, bashed) = (
        taken_with(&world, hero, enemy, 100, 0),
        taken_with(&world, hero, enemy, 100, 100),
    );
    let blows = attack_blows(
        &attack_unrefreshed(&mut world, hero, enemy, 24).blows,
        enemy,
    );
    let ticks: Vec<u32> = blows.iter().map(|blow| blow.tick).collect();
    let expected = procs_with_wait(run, &ticks, ticks_of(2300));
    for (blow, bash) in blows.iter().zip(expected) {
        let span = if bash { &bashed } else { &plain };
        assert!(
            span.contains(&blow.amount),
            "{blow:?}: {span:?}, bash {bash}"
        );
    }
    let rolls = world
        .crit
        .get(hero.index().0 as usize)
        .and_then(Option::as_ref)
        .expect("rolled");
    let second = rolls.sources().get(usize::from(item_place(1)));
    assert!(second.is_none_or(Option::is_none), "the second rolled");
}

/// An item's critical strike "does not work against buildings", and a Skull Basher bashes none:
/// a Daedalus and Skull Basher carrier's swings on a tower roll neither, and none is critical.
#[test]
fn no_item_critical_strike_or_bash_lands_on_a_building() {
    let (mut world, hero, _) = duel(
        HeroId::PUDGE,
        &[ItemId::DAEDALUS, ItemId::SKULL_BASHER],
        HeroId::PUDGE,
        Vec2::from_ints(1200, 0),
    );
    let tower = world
        .entities
        .iter()
        .filter(|&unit| {
            world.kind.get(unit) == Some(&UnitKind::Tower)
                && world.team.get(unit) == Some(&Team::Dire)
        })
        .min_by_key(|&unit| world.tier.get(unit).map_or(u8::MAX, |tier| tier.0))
        .expect("a Dire tower");
    let near = world.pos(tower).expect("standing");
    let at = [
        (-300, 0),
        (0, -300),
        (300, 0),
        (0, 300),
        (-300, -300),
        (300, 300),
    ]
    .map(|(x, y)| near + Vec2::from_ints(x, y))
    .into_iter()
    .find(|&at| {
        let order = Order::Cheat {
            cheat: Cheat::Teleport { at },
        };
        world.validate_order(SlotId(0), None, &order).is_ok()
    })
    .expect("open ground beside the tower");
    seat_cheat(&mut world, 0, Cheat::Teleport { at });
    let blows = attack_blows(&attack_until(&mut world, hero, tower, 6).blows, tower);
    let least = blows.iter().map(|blow| blow.amount).min().expect("blows");
    assert!(
        blows.iter().all(|blow| blow.amount * 100 < least * 160),
        "{blows:?}"
    );
    let opened = world
        .crit
        .get(hero.index().0 as usize)
        .and_then(Option::as_ref)
        .is_some_and(|rolls| rolls.sources().iter().any(Option::is_some));
    assert!(!opened, "an item proc rolled against a building");
}

/// Where `at` lies from a cone opening at `from` towards `towards`: how far along its axis and how
/// far aside, in whole units.
fn along_and_aside(from: Vec2, towards: Vec2, at: Vec2) -> (i64, i64) {
    let axis = towards - from;
    let off = at - from;
    let length = bota_core::isqrt64(axis.length_squared());
    let (ax, ay) = (i128::from(axis.x.raw), i128::from(axis.y.raw));
    let (ox, oy) = (i128::from(off.x.raw), i128::from(off.y.raw));
    let unit = i128::from(length) * 65_536;
    (
        ((ox * ax + oy * ay) / unit) as i64,
        ((ox * ay - oy * ax).abs() / unit) as i64,
    )
}

/// Battle Fury's Cleave: each swing of a melee carrier deals 60% of the swing as dealt
/// (`cleave_damage_percent`) to an enemy hero in the cone behind its target, laid to the Battle
/// Fury, and nothing to one behind the attacker or past the cone's 650 (`cleave_distance`).
#[test]
fn a_battle_fury_cleaves_sixty_percent_of_each_swing_onto_a_hero_in_its_cone() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::BATTLE_FURY],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    let behind = foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(400, 0));
    let back = foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(-300, 0));
    let past = foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(1000, 0));
    let from = world.pos(hero).expect("standing");
    let towards = world.pos(enemy).expect("standing");
    let (along, aside) = along_and_aside(from, towards, world.pos(behind).expect("standing"));
    assert!(
        along < 650 && aside < 150 + 210 * along / 650,
        "{along} {aside}"
    );
    let share = taken_at(&world, hero, behind, 60);
    let attacks = attack_until(&mut world, hero, enemy, 8);
    let swings = attack_blows(&attacks.blows, enemy);
    let bfury = Some(Inflictor::Item(ItemId::BATTLE_FURY));
    for swing in &swings {
        let cleaved: Vec<&Blow> = attacks
            .blows
            .iter()
            .filter(|blow| blow.tick == swing.tick && blow.inflictor == bfury)
            .collect();
        assert_eq!(cleaved.len(), 1, "{swing:?}: {cleaved:?}");
        assert_eq!(cleaved[0].target, behind);
        assert_eq!(cleaved[0].kind, DamageKind::Physical);
        assert!(
            share.contains(&cleaved[0].amount),
            "{:?}: {share:?}",
            cleaved[0]
        );
    }
    assert!(
        attacks
            .blows
            .iter()
            .all(|blow| blow.target != back && blow.target != past),
        "only the cone is cleaved"
    );
}

/// Battle Fury's Cleave deals 40% (`cleave_damage_percent_creep`) to each creep in its trapezoid:
/// a camp of enemy lane creeps placed behind the struck by `-createcreeps`.
#[test]
fn a_battle_fury_cleaves_forty_percent_onto_creeps_in_its_cone() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::BATTLE_FURY],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    let at = world.pos(hero).expect("standing") + Vec2::from_ints(350, 0);
    seat_cheat(&mut world, 0, Cheat::CreateCreeps { enemy: true, at });
    let attacks = attack_until(&mut world, hero, enemy, 1);
    let bfury = Some(Inflictor::Item(ItemId::BATTLE_FURY));
    let cleaved: Vec<&Blow> = attacks
        .blows
        .iter()
        .filter(|blow| blow.inflictor == bfury)
        .collect();
    assert!(
        !cleaved.is_empty(),
        "the creeps behind the struck are cleaved"
    );
    for blow in cleaved {
        assert_ne!(world.kind.get(blow.target), Some(&UnitKind::Hero));
        let share = taken_at(&world, hero, blow.target, 40);
        assert!(share.contains(&blow.amount), "{blow:?}: {share:?}");
    }
}

/// Cleave is a melee carrier's ("(Melee Only)"): a ranged Battle Fury carrier cleaves nothing.
#[test]
fn a_ranged_battle_fury_carrier_cleaves_nothing() {
    let (mut world, hero, enemy) = duel(
        HeroId::SHADOW_FIEND,
        &[ItemId::BATTLE_FURY],
        HeroId::PUDGE,
        Vec2::from_ints(400, 0),
    );
    foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(600, 0));
    let attacks = attack_until(&mut world, hero, enemy, 6);
    let bfury = Some(Inflictor::Item(ItemId::BATTLE_FURY));
    assert!(attacks.blows.iter().all(|blow| blow.inflictor != bfury));
}

/// Cleave's starting and ending values are radii to either side of its axis (Dota 7.00, Cleave
/// Mechanics: "starting radius", "ending radius"; 7.20 widened the end to 360 and the reach to
/// 650): a hero 200 aside of the axis 400 along it (the radius there is 279) is cleaved, one 100
/// along and 200 aside (radius 182) is not.
#[test]
fn a_battle_furys_cleave_reaches_its_radius_to_either_side() {
    for (offset, inside) in [((400, 200), true), ((100, 200), false), ((100, 160), true)] {
        let (mut world, hero, enemy) = duel(
            HeroId::PUDGE,
            &[ItemId::BATTLE_FURY],
            HeroId::PUDGE,
            Vec2::from_ints(150, 0),
        );
        let beside = foe(
            &mut world,
            hero,
            HeroId::PUDGE,
            Vec2::from_ints(offset.0, offset.1),
        );
        let from = world.pos(hero).expect("standing");
        let towards = world.pos(enemy).expect("standing");
        let (along, aside) = along_and_aside(from, towards, world.pos(beside).expect("standing"));
        let radius = 150 + 210 * along / 650;
        assert_eq!(
            aside < radius,
            inside,
            "{offset:?}: {along} along, {aside} aside, radius {radius}"
        );
        let attacks = attack_until(&mut world, hero, enemy, 2);
        let cleaved = attacks.blows.iter().any(|blow| blow.target == beside);
        assert_eq!(cleaved, inside, "{offset:?}");
    }
}

/// An AoE bonus widens a cleave (Dota 7.36: "AoE Bonuses now affect all auras and Cleave
/// sources"; Battle Fury's radii are `affected_by_aoe_increase`): a hero 100 along and 200 aside
/// of the axis, outside the plain radius there (182), is cleaved with a Chasm Stone's 40
/// (`aoe_bonus`: 222) carried, and not without.
#[test]
fn a_chasm_stone_widens_a_battle_furys_cleave() {
    for chasm in [false, true] {
        let items: &[ItemId] = if chasm {
            &[ItemId::BATTLE_FURY, ItemId::CHASM_STONE]
        } else {
            &[ItemId::BATTLE_FURY]
        };
        let (mut world, hero, enemy) =
            duel(HeroId::PUDGE, items, HeroId::PUDGE, Vec2::from_ints(150, 0));
        let beside = foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(100, 200));
        let from = world.pos(hero).expect("standing");
        let towards = world.pos(enemy).expect("standing");
        let (along, aside) = along_and_aside(from, towards, world.pos(beside).expect("standing"));
        assert!(aside > 150 + 210 * along / 650, "{along} {aside}");
        assert!(aside <= 190 + 210 * along / 650, "{along} {aside}");
        let attacks = attack_until(&mut world, hero, enemy, 3);
        let cleaved = attacks.blows.iter().any(|blow| blow.target == beside);
        assert_eq!(cleaved, chasm, "with a Chasm Stone: {chasm}");
    }
}

/// Maelstrom's Chain Lightning: on Dota's 25% run (`chain_chance`) a swing lets loose a bolt that
/// deals 110 magical damage (`chain_damage`) to its target, then leaps to the nearest enemy within
/// 650 (`chain_radius`) of the last struck, 0.25 s apart (`chain_delay`), until it has struck 4
/// (`chain_strikes`): a fifth enemy in reach is spared.
#[test]
fn maelstroms_chain_lightning_strikes_four_on_a_quarter_run() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::MAELSTROM],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    // Each a leap from the last; the fifth, behind the attacker, is never the nearest.
    let line: Vec<Entity> = [(350, 0), (550, 0), (750, 0), (-200, 0)]
        .map(|(x, y)| foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(x, y)))
        .into();
    let place = item_place(slot_holding(&world, hero, ItemId::MAELSTROM));
    let mut run = Prd::new(&world, hero, place, C25);
    let struck = [enemy, line[0], line[1], line[2]];
    let bolt = struck.map(|unit| magical_on(&world, unit, 110));
    let attacks = attack_until(&mut world, hero, enemy, 24);
    let maelstrom = Some(Inflictor::Item(ItemId::MAELSTROM));
    let chained: Vec<&Blow> = attacks
        .blows
        .iter()
        .filter(|blow| blow.inflictor == maelstrom)
        .collect();
    let leap = ticks_of(250);
    let mut procs = 0;
    for swing in attack_blows(&attacks.blows, enemy) {
        let bolts: Vec<&&Blow> = chained
            .iter()
            .filter(|blow| blow.tick >= swing.tick && blow.tick <= swing.tick + 3 * leap + 1)
            .collect();
        if run.next() {
            procs += 1;
            let got: Vec<(Entity, i32)> = bolts
                .iter()
                .map(|blow| (blow.target, blow.amount))
                .collect();
            let want: Vec<(Entity, i32)> = struck.iter().copied().zip(bolt).collect();
            assert_eq!(got, want, "the swing at {}", swing.tick);
            assert!(
                bolts[0].tick - swing.tick <= 1,
                "{} {}",
                bolts[0].tick,
                swing.tick
            );
            for pair in bolts.windows(2) {
                assert_eq!(pair[1].tick - pair[0].tick, leap);
            }
        } else {
            assert!(
                bolts.is_empty(),
                "the swing at {} lets loose none",
                swing.tick
            );
        }
    }
    assert!(procs > 0);
    assert!(
        chained.iter().all(|blow| blow.target != line[3]),
        "the fifth is spared"
    );
}

/// "Lightning proc pierces evasion": a swing that lets loose Chain Lightning lands through the
/// struck's evasion (a Talisman of Evasion its player carries); swings that do not may miss.
#[test]
fn a_swing_that_lets_loose_chain_lightning_goes_through_evasion() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::MAELSTROM],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    seat_cheat(
        &mut world,
        1,
        Cheat::Item {
            item: ItemId::TALISMAN_OF_EVASION,
        },
    );
    let place = item_place(slot_holding(&world, hero, ItemId::MAELSTROM));
    let mut run = Prd::new(&world, hero, place, C25);
    let attacks = attack_until(&mut world, hero, enemy, 30);
    let landed: Vec<u32> = attack_blows(&attacks.blows, enemy)
        .iter()
        .map(|blow| blow.tick)
        .collect();
    let mut missed = 0;
    for &launch in &attacks.launched {
        let hit = landed.contains(&launch);
        if run.next() {
            assert!(hit, "the chain swing at {launch} landed");
        } else if !hit {
            missed += 1;
        }
    }
    assert!(missed > 0, "the talisman evaded some plain swings");
}

/// Specialist's Array's Splitshot: on Dota's 30% run (`proc_chance`) a ranged swing also throws at
/// the 2 nearest enemies (`count`) within its attack range and 150 more
/// (`secondary_target_range_bonus`), 120 degrees in front (`secondary_target_angle`): each takes
/// 20 + 75% of a plain swing (`base_proc_dmg`, `proc_dmg_pct`), and the swing itself 20 + 100%.
/// One behind the thrower, a third farther off and one out of reach take nothing.
#[test]
fn specialists_array_splits_a_ranged_swing_onto_the_two_nearest_in_front() {
    let (mut world, hero, enemy) = duel(
        HeroId::SHADOW_FIEND,
        &[ItemId::SPECIALISTS_ARRAY],
        HeroId::PUDGE,
        Vec2::from_ints(350, 0),
    );
    let near = [
        foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(400, 150)),
        foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(400, -150)),
    ];
    let spared = [
        foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(600, 0)),
        foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(-300, 0)),
        foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(1100, 0)),
    ];
    let place = item_place(slot_holding(&world, hero, ItemId::SPECIALISTS_ARRAY));
    let mut run = Prd::new(&world, hero, place, C30);
    let (plain, split) = (
        taken_with(&world, hero, enemy, 100, 0),
        taken_with(&world, hero, enemy, 100, 20),
    );
    let extra = near.map(|unit| taken_with(&world, hero, unit, 75, 20));
    let attacks = attack_until(&mut world, hero, enemy, 30);
    let mut procs = 0;
    for blow in attack_blows(&attacks.blows, enemy) {
        let splits = run.next();
        procs += usize::from(splits);
        let span = if splits { &split } else { &plain };
        assert!(
            span.contains(&blow.amount),
            "{blow:?}: {span:?}, split {splits}"
        );
    }
    assert!(procs > 0);
    for (unit, span) in near.iter().zip(&extra) {
        let took: Vec<&Blow> = attacks
            .blows
            .iter()
            .filter(|blow| blow.target == *unit)
            .collect();
        assert_eq!(took.len(), procs, "one missile a split");
        for blow in took {
            assert_eq!(blow.inflictor, None, "an attack's blow");
            assert!(span.contains(&blow.amount), "{blow:?}: {span:?}");
        }
    }
    assert!(
        attacks
            .blows
            .iter()
            .all(|blow| !spared.contains(&blow.target))
    );
}

/// Splitshot is a ranged carrier's ("Ranged Only"): a melee carrier splits nothing.
#[test]
fn a_melee_specialists_array_carrier_splits_nothing() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::SPECIALISTS_ARRAY],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    let beside = foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(300, 150));
    let attacks = attack_until(&mut world, hero, enemy, 10);
    assert!(attacks.blows.iter().all(|blow| blow.target != beside));
}

/// Vanguard's Damage Block: on Dota's 60% run (`block_chance`), each attack on a melee carrier
/// loses 50 (`block_damage_melee`) before its armor; a ranged carrier blocks 25
/// (`block_damage_ranged`). The rolls are the carrier's, one a blow.
#[test]
fn vanguard_blocks_attacks_on_a_sixty_percent_run_fifty_melee_twenty_five_ranged() {
    for (carrier, amount) in [(HeroId::PUDGE, 50), (HeroId::SHADOW_FIEND, 25)] {
        let (mut world, hero, enemy) = duel(
            carrier,
            &[ItemId::VANGUARD],
            HeroId::PUDGE,
            Vec2::from_ints(150, 0),
        );
        let mut run = Prd::new(&world, hero, 128, C60);
        let (plain, blocked) = (
            taken_with(&world, enemy, hero, 100, 0),
            taken_with(&world, enemy, hero, 100, -amount),
        );
        assert!(blocked.end() < plain.start(), "{blocked:?} {plain:?}");
        let blows = attack_blows(&attack_until(&mut world, enemy, hero, 30).blows, hero);
        for blow in blows {
            let span = if run.next() { &blocked } else { &plain };
            assert!(
                span.contains(&blow.amount),
                "{carrier:?} {blow:?}: {span:?}"
            );
        }
    }
}

/// Battle Fury's Chop Tree fells a tree as a Quelling Blade's does and waits its 3 s cooldown (Dota
/// 7.41f: "Chop Tree cooldown decreased from 4s to 3s").
#[test]
fn a_battle_fury_chops_a_tree_and_waits_three_seconds() {
    let (mut world, hero, tree) = a_hero_by_the_trees(ItemId::BATTLE_FURY, 0);
    seat_order(
        &mut world,
        Order::Use {
            slot: bota_proto::ItemSlot(0),
            target: bota_proto::Target::Pos(tree),
        },
    );
    for _ in 0..rules::TICKS_PER_SECOND {
        if world.trees.felled().count() > 0 {
            break;
        }
        world.step();
    }
    assert_eq!(world.trees.felled().count(), 1, "the tree is down");
    let stack = slot_of(&world, hero, 0).expect("the Battle Fury is kept");
    assert_eq!(stack.id, ItemId::BATTLE_FURY);
    let waited = ticks_of(3000) - stack.cooldown;
    assert!(
        waited <= rules::TICKS_PER_SECOND,
        "{} of {}",
        stack.cooldown,
        ticks_of(3000)
    );
}

/// Whether a unit's proc source at `place` has rolled at all.
fn rolled(world: &World, unit: Entity, place: u8) -> bool {
    world
        .crit
        .get(unit.index().0 as usize)
        .and_then(Option::as_ref)
        .and_then(|rolls| rolls.sources().get(usize::from(place)))
        .is_some_and(Option::is_some)
}

/// A two-seat match on the full map with every rune and cheats on. Seat 0's `hero`, raised to
/// level 25 and handed `items` (`-lvlup`, `-item`), takes an Illusion Rune spawned at the top river
/// spot (`-spawnrune`, a validated take order); then the hero is moved away (`-teleport`) and
/// seat 1's `foe` (`-lvlup`) is moved `offset` from the first illusion, which seat 0 commands.
/// Answers the world, the hero, that illusion and the foe.
fn illusion_duel(
    hero: HeroId,
    items: &[ItemId],
    foe: HeroId,
    offset: Vec2,
) -> (World, Entity, Entity, Entity) {
    let mut cfg = config();
    cfg.cheats = true;
    cfg.map = bota_proto::MapId(0);
    cfg.runes = bota_proto::RuneSet::ALL;
    cfg.objectives = bota_proto::ObjectiveSet::NONE;
    cfg.picks = vec![
        Pick {
            slot: SlotId(0),
            team: Team::Radiant,
            hero,
        },
        Pick {
            slot: SlotId(1),
            team: Team::Dire,
            hero: foe,
        },
    ];
    let mut world = World::for_match(&cfg, cfg.rng());
    world.settle();
    let unit = world.seats[0].unit.expect("stood up");
    let enemy = world.seats[1].unit.expect("stood up");
    let spot = bota_core::map_of(bota_proto::MapId(0)).rune_spots[0].at;
    seat_cheat(
        &mut world,
        0,
        Cheat::Teleport {
            at: spot - Vec2::from_ints(300, 0),
        },
    );
    seat_cheat(&mut world, 0, Cheat::Levels { count: 24 });
    for &item in items {
        seat_cheat(&mut world, 0, Cheat::Item { item });
    }
    seat_cheat(&mut world, 0, Cheat::Refresh);
    seat_cheat(
        &mut world,
        0,
        Cheat::SpawnRune {
            kind: bota_proto::RuneKind::Illusion,
            at: spot,
        },
    );
    let rune = world
        .view_full()
        .runes
        .iter()
        .find(|rune| rune.kind == bota_proto::RuneKind::Illusion)
        .expect("the rune lies")
        .id;
    let take = seat_command(
        &world,
        0,
        Order::Take {
            target: bota_proto::Target::Unit(rune),
        },
    );
    world.advance_trusted(&[take]);
    let illusion = (0..300)
        .find_map(|_| {
            world.step();
            world
                .entities
                .iter()
                .find(|&other| world.illusion.get(other).is_some_and(|it| it.of == unit))
        })
        .expect("the rune makes illusions");
    seat_cheat(
        &mut world,
        0,
        Cheat::Teleport {
            at: spot - Vec2::from_ints(1500, 0),
        },
    );
    let at = world.pos(illusion).expect("standing") + offset;
    seat_cheat(&mut world, 1, Cheat::Teleport { at });
    seat_cheat(&mut world, 1, Cheat::Levels { count: 24 });
    let stand = Order::Move {
        target: bota_proto::Target::None,
    };
    let still: Vec<Command> = world
        .entities
        .iter()
        .filter(|&other| world.illusion.get(other).is_some_and(|it| it.of == unit))
        .map(|other| unit_command(&world, 0, Some(wire_id(other)), stand))
        .chain([seat_command(&world, 1, stand)])
        .collect();
    world.advance_trusted(&still);
    (world, unit, illusion, enemy)
}

/// What an illusion's swing of `pct` percent of its damage roll, amplified as the illusion's
/// attacks are, takes off `on`.
fn illusion_taken(
    world: &World,
    by: Entity,
    on: Entity,
    pct: i32,
) -> std::ops::RangeInclusive<i32> {
    let stats = world.stats.get(by).expect("derived");
    let amp = stats.damage_amp_bp(DamageKind::Physical);
    let through = |raw: i32| {
        let dealt = bota_core::amplify(raw * pct / 100, amp);
        let taken = physical_on(world, on, dealt);
        let share = world
            .stats
            .get(on)
            .expect("derived")
            .illusion_attack_taken_bp;
        (i64::from(taken) * i64::from(share) / i64::from(rules::NOMINAL_BP)) as i32
    };
    let (low, high) = (stats.damage, stats.damage + stats.damage_spread);
    through(low)..=through(high)
}

/// An illusion strikes critically with what its hero carries, on a run of its own: a Crystalys's
/// 160% of its weakened swing on Dota's 30% run.
#[test]
fn an_illusion_strikes_critically_with_a_crystalys_on_its_own_run() {
    let (mut world, _, illusion, enemy) = illusion_duel(
        HeroId::PUDGE,
        &[ItemId::CRYSTALYS],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    let place = item_place(slot_holding(&world, illusion, ItemId::CRYSTALYS));
    let mut run = Prd::new(&world, illusion, place, C30);
    let (plain, crit) = (
        illusion_taken(&world, illusion, enemy, 100),
        illusion_taken(&world, illusion, enemy, 160),
    );
    assert!(plain.end() < crit.start(), "{plain:?} {crit:?}");
    let blows = attack_blows(&attack_until(&mut world, illusion, enemy, 20).blows, enemy);
    let mut crits = 0;
    for blow in blows {
        let critical = run.next();
        crits += usize::from(critical);
        let span = if critical { &crit } else { &plain };
        assert!(
            span.contains(&blow.amount),
            "{blow:?}: {span:?}, critical {critical}"
        );
    }
    assert!(crits > 0);
}

/// An illusion's Phantom Critical lands its magic weakened as the illusion's damage is (Dota 7.38b
/// fixed Revenant's Brooch dealing too much off illusions): 80% of its damage roll, amplified by
/// the illusion's magical share, on Dota's 30% run.
#[test]
fn an_illusions_phantom_critical_lands_its_weakened_magic() {
    let (mut world, _, illusion, enemy) = illusion_duel(
        HeroId::PUDGE,
        &[ItemId::REVENANTS_BROOCH],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    let place = item_place(slot_holding(&world, illusion, ItemId::REVENANTS_BROOCH));
    let mut run = Prd::new(&world, illusion, place, C30);
    let stats = *world.stats.get(illusion).expect("derived");
    let amp = stats.spell_damage_amp_bp(DamageKind::Magical);
    assert!(amp < rules::NOMINAL_BP, "an illusion's magic is weakened");
    let magic = |raw: i32| magical_on(&world, enemy, bota_core::amplify(raw * 80 / 100, amp));
    let span = magic(stats.damage)..=magic(stats.damage + stats.damage_spread);
    let all = attack_until(&mut world, illusion, enemy, 20).blows;
    let brooch = Some(Inflictor::Item(ItemId::REVENANTS_BROOCH));
    let mut procs = 0;
    for blow in attack_blows(&all, enemy) {
        let riding: Vec<&Blow> = all
            .iter()
            .filter(|other| other.tick == blow.tick && other.inflictor == brooch)
            .collect();
        if run.next() {
            procs += 1;
            assert_eq!(riding.len(), 1, "{blow:?}");
            assert!(
                span.contains(&riding[0].amount),
                "{:?}: {span:?}",
                riding[0]
            );
        } else {
            assert!(riding.is_empty(), "{blow:?}");
        }
    }
    assert!(procs > 0);
}

/// An illusion bashes, cleaves and lets loose chain lightning never, carrying a Skull Basher, a
/// Battle Fury and a Maelstrom as its hero does: none of their numbers is rolled for it, it deals
/// nothing laid to them, and the struck wears no bash, a hero in its cleave takes nothing.
#[test]
fn an_illusion_bashes_cleaves_and_lets_loose_lightning_never() {
    let items = [ItemId::SKULL_BASHER, ItemId::BATTLE_FURY, ItemId::MAELSTROM];
    let (mut world, _, illusion, enemy) = illusion_duel(
        HeroId::PUDGE,
        &items,
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    let behind = foe(&mut world, illusion, HeroId::PUDGE, Vec2::from_ints(350, 0));
    let mut bashed = false;
    let attacks = attack_until(&mut world, illusion, enemy, 20);
    for _ in 0..rules::TICKS_PER_SECOND {
        world.step();
        bashed |= world.modifiers.get(enemy).is_some_and(|on_it| {
            on_it
                .active()
                .any(|held| held.inflictor == Some(Inflictor::Item(ItemId::SKULL_BASHER)))
        });
    }
    assert!(!bashed, "no bash of the illusion's");
    assert!(
        attacks
            .blows
            .iter()
            .all(|blow| blow.inflictor.is_none() && blow.target != behind),
        "{:?}",
        attacks
            .blows
            .iter()
            .filter(|blow| blow.inflictor.is_some())
            .collect::<Vec<_>>()
    );
    for item in items {
        let place = item_place(slot_holding(&world, illusion, item));
        assert!(
            !rolled(&world, illusion, place),
            "{item:?} rolled for the illusion"
        );
    }
}

/// An illusion splits no shot: a Shadow Fiend's illusion carrying a Specialist's Array throws at
/// its target alone, enemies beside it in reach untouched, and nothing rolled.
#[test]
fn an_illusion_splits_no_shot() {
    let (mut world, _, illusion, enemy) = illusion_duel(
        HeroId::SHADOW_FIEND,
        &[ItemId::SPECIALISTS_ARRAY],
        HeroId::PUDGE,
        Vec2::from_ints(350, 0),
    );
    let beside = foe(
        &mut world,
        illusion,
        HeroId::PUDGE,
        Vec2::from_ints(400, 150),
    );
    let attacks = attack_until(&mut world, illusion, enemy, 20);
    assert!(attacks.blows.iter().all(|blow| blow.target != beside));
    let place = item_place(slot_holding(&world, illusion, ItemId::SPECIALISTS_ARRAY));
    assert!(!rolled(&world, illusion, place));
}

/// An illusion's Vanguard blocks the attacks on it as its hero's would, on its own 60% run: a
/// damage block is no attack modifier of the illusion's.
#[test]
fn an_illusions_vanguard_blocks_attacks_on_its_own_run() {
    let (mut world, _, illusion, enemy) = illusion_duel(
        HeroId::PUDGE,
        &[ItemId::VANGUARD],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    let mut run = Prd::new(&world, illusion, 128, C60);
    let stats = world.stats.get(enemy).expect("derived");
    let (low, high) = (stats.damage, stats.damage + stats.damage_spread);
    let taken = |raw: i32| {
        let share = world.stats.get(illusion).expect("derived").damage_taken_bp;
        let through = physical_on(&world, illusion, raw);
        (i64::from(through) * i64::from(share) / i64::from(rules::NOMINAL_BP)) as i32
    };
    let (plain, blocked) = (taken(low)..=taken(high), taken(low - 50)..=taken(high - 50));
    assert!(blocked.end() < plain.start(), "{blocked:?} {plain:?}");
    let mut blocks = 0;
    for blow in attack_blows(
        &attack_unrefreshed(&mut world, enemy, illusion, 8).blows,
        illusion,
    ) {
        let block = run.next();
        blocks += usize::from(block);
        let span = if block { &blocked } else { &plain };
        assert!(span.contains(&blow.amount), "{blow:?}: {span:?}");
    }
    assert!(blocks > 0);
}

/// Damage Block answers to a cleave as to an attack (Damage Block: cleave is blocked): a Vanguard
/// carrier in a Battle Fury's cleave takes its 60% share with 50 blocked before its armor on its
/// own 60% run.
#[test]
fn a_vanguard_blocks_a_cleave_before_armor() {
    let (mut world, hero, carrier) = duel(
        HeroId::PUDGE,
        &[ItemId::BATTLE_FURY],
        HeroId::PUDGE,
        Vec2::from_ints(400, 0),
    );
    let struck = foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(150, 0));
    seat_cheat(
        &mut world,
        1,
        Cheat::Item {
            item: ItemId::VANGUARD,
        },
    );
    let mut run = Prd::new(&world, carrier, 128, C60);
    let (low, high) = damage_range(&world, hero);
    let span = |less: i32| {
        physical_on(&world, carrier, low * 60 / 100 - less)
            ..=physical_on(&world, carrier, high * 60 / 100 - less)
    };
    let (plain, blocked) = (span(0), span(50));
    let attacks = attack_unrefreshed(&mut world, hero, struck, 8);
    let bfury = Some(Inflictor::Item(ItemId::BATTLE_FURY));
    let mut blocks = 0;
    for blow in attacks
        .blows
        .iter()
        .filter(|blow| blow.target == carrier && blow.inflictor == bfury)
    {
        let block = run.next();
        blocks += usize::from(block);
        let span = if block { &blocked } else { &plain };
        assert!(
            span.contains(&blow.amount),
            "{blow:?}: {span:?}, block {block}"
        );
    }
    assert!(blocks > 0);
}

/// Chain Lightning's 0.2 s wait (`chain_cooldown`) is a timer of its own: "neither displayed on
/// the item icon when procced nor is affected by cooldown manipulation" (Liquipedia, Maelstrom).
/// With an Octarine Core's 25% cooldown cut carried, a proc leaves the Maelstrom showing no
/// cooldown and its wait 6 ticks long, while a Skull Basher's bash waits its item cooldown cut by
/// the Octarine (2.3 s: 69 ticks, cut to 51 or 52).
#[test]
fn maelstroms_wait_is_its_own_and_a_skull_bashers_is_its_items() {
    let items = [
        ItemId::MAELSTROM,
        ItemId::SKULL_BASHER,
        ItemId::OCTARINE_CORE,
    ];
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &items,
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    seat_cheat(
        &mut world,
        1,
        Cheat::Item {
            item: ItemId::HEART,
        },
    );
    let maelstrom = slot_holding(&world, hero, ItemId::MAELSTROM);
    let basher = slot_holding(&world, hero, ItemId::SKULL_BASHER);
    let attacks = attack_unrefreshed(&mut world, hero, enemy, 1);
    assert_eq!(attacks.launched.len(), 1);
    let (mut chained, mut bashed) = (false, false);
    attack_click(&mut world, enemy);
    for _ in 0..rules::TICKS_PER_SECOND * 30 {
        let tick = world.tick;
        let events = world.step();
        let launched = events.iter().any(|event| {
            matches!(event.kind(), EventKind::AttackLaunched { attacker, .. } if attacker == wire_id(hero))
        });
        if !launched {
            continue;
        }
        let bag = world.inventory.get(hero).expect("a bag");
        let shows = |slot: usize| bag.slots[slot].expect("carried").cooldown;
        assert_eq!(shows(maelstrom), 0, "the Maelstrom shows no cooldown");
        let roll = world
            .crit
            .get(hero.index().0 as usize)
            .and_then(Option::as_ref)
            .and_then(|rolls| rolls.sources().get(usize::from(item_place(maelstrom))))
            .and_then(Option::as_ref);
        if let Some(roll) = roll
            && !roll.ready(tick + 1)
        {
            chained = true;
            // The step ran the swing on this tick or the next: the wait ends 6 ticks on.
            let until = (tick..tick + 30)
                .find(|&at| roll.ready(at))
                .expect("it ends");
            let wait = ticks_of(200);
            assert!(
                until == tick + wait || until == tick + wait + 1,
                "{} ticks",
                until - tick
            );
        }
        if shows(basher) > 0 && !bashed {
            bashed = true;
            assert!((51..=52).contains(&shows(basher)), "{}", shows(basher));
        }
        if chained && bashed {
            return;
        }
    }
    panic!("chained {chained}, bashed {bashed} in thirty seconds");
}

/// "If the wielder dies, all their currently ongoing Chain Lightning instances stop immediately"
/// (Liquipedia, Maelstrom): once the first bolt struck, its wielder falls (ours: a lethal blow
/// laid on the queue, no Dota cheat kills a hero at a tick) and the bolt leaps no more.
#[test]
fn maelstroms_lightning_stops_when_its_wielder_dies() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::MAELSTROM],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(350, 0));
    // Both sides see everything (`-allvision`): the wielder's death takes no sight off the leap.
    seat_cheat(&mut world, 0, Cheat::AllVision { on: true });
    attack_click(&mut world, enemy);
    let bolt = |events: &[crate::game::Event]| {
        events.iter().any(|event| {
            matches!(
                event.kind(),
                EventKind::Damaged {
                    inflictor: Some(Inflictor::Item(ItemId::MAELSTROM)),
                    ..
                }
            )
        })
    };
    for _ in 0..rules::TICKS_PER_SECOND * 60 {
        if !bolt(&world.step()) {
            continue;
        }
        world.push_hit(None, hero, 1_000_000, DamageKind::Pure, None);
        for _ in 0..rules::TICKS_PER_SECOND {
            assert!(!bolt(&world.step()), "a bolt leapt after its wielder fell");
        }
        assert!(!world.alive(hero));
        return;
    }
    panic!("no bolt in a minute");
}

/// A ranged swing's bolt goes off where its missile lands, not as it is thrown: a Shadow Fiend
/// with a Maelstrom 500 off its target; every bolt strikes the target the tick its missile does.
#[test]
fn a_ranged_maelstrom_lets_loose_its_bolt_where_the_missile_lands() {
    let (mut world, hero, enemy) = duel(
        HeroId::SHADOW_FIEND,
        &[ItemId::MAELSTROM],
        HeroId::PUDGE,
        Vec2::from_ints(500, 0),
    );
    let attacks = attack_until(&mut world, hero, enemy, 24);
    let maelstrom = Some(Inflictor::Item(ItemId::MAELSTROM));
    let swings = attack_blows(&attacks.blows, enemy);
    let first_bolts: Vec<&Blow> = attacks
        .blows
        .iter()
        .filter(|blow| blow.target == enemy && blow.inflictor == maelstrom)
        .collect();
    assert!(!first_bolts.is_empty());
    // The k-th missile that landed is the k-th thrown.
    for bolt in first_bolts {
        let swing = swings
            .iter()
            .position(|swing| bolt.tick == swing.tick || bolt.tick == swing.tick + 1)
            .unwrap_or_else(|| panic!("the bolt at {} with no missile landing then", bolt.tick));
        let thrown = attacks.launched[swing];
        assert!(
            bolt.tick >= thrown + 3,
            "the bolt at {} of the missile thrown at {thrown}",
            bolt.tick
        );
    }
}

/// Chain Lightning "can proc ... On already dead enemies (i.e. target died before projectile
/// impact)" (Liquipedia, Maelstrom): a Shadow Fiend's missile that went off with a bolt is in
/// flight when its target falls (ours: a lethal blow laid on the queue), and the bolt still leaps
/// from where the target fell onto the enemy beside it.
#[test]
fn a_bolt_goes_off_where_the_target_fell_before_the_missile_landed() {
    let (mut world, hero, enemy) = duel(
        HeroId::SHADOW_FIEND,
        &[ItemId::MAELSTROM],
        HeroId::PUDGE,
        Vec2::from_ints(500, 0),
    );
    let beside = foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(650, 0));
    let place = item_place(slot_holding(&world, hero, ItemId::MAELSTROM));
    let mut run = Prd::new(&world, hero, place, C25);
    attack_click(&mut world, enemy);
    let maelstrom = Some(Inflictor::Item(ItemId::MAELSTROM));
    for _ in 0..rules::TICKS_PER_SECOND * 60 {
        let events = world.step();
        let launched = events.iter().any(|event| {
            matches!(event.kind(), EventKind::AttackLaunched { attacker, .. } if attacker == wire_id(hero))
        });
        if !launched || !run.next() {
            continue;
        }
        world.push_hit(None, enemy, 1_000_000, DamageKind::Pure, None);
        // The thrower stands: no swing of its own strikes anybody else.
        let stand = seat_command(
            &world,
            0,
            Order::Move {
                target: bota_proto::Target::None,
            },
        );
        let mut events = world.advance_trusted(&[stand]);
        let mut struck = Vec::new();
        for _ in 0..rules::TICKS_PER_SECOND * 2 {
            for event in std::mem::take(&mut events) {
                if let EventKind::Damaged {
                    target,
                    inflictor,
                    source,
                    ..
                } = event.kind()
                {
                    let swing = source == Some(wire_id(hero)) && inflictor.is_none();
                    assert!(!swing, "a swing landed: {:?}", event.kind());
                    if inflictor == maelstrom {
                        struck.push(target);
                    }
                }
            }
            events = world.step();
        }
        assert!(!world.alive(enemy));
        assert!(struck.contains(&wire_id(beside)), "{struck:?}");
        return;
    }
    panic!("no proc in a minute");
}

/// A target turned invulnerable while a missile flies takes neither the swing nor what rides with
/// it: a Shadow Fiend with a Maelstrom and a Revenant's Brooch; its player turns the target
/// invulnerable (Dota's Demo Hero's Invulnerable) after a throw, and no blow of the swing lands.
#[test]
fn an_invulnerable_target_takes_no_bolt_and_no_phantom_magic() {
    let (mut world, hero, enemy) = duel(
        HeroId::SHADOW_FIEND,
        &[ItemId::MAELSTROM, ItemId::REVENANTS_BROOCH],
        HeroId::PUDGE,
        Vec2::from_ints(500, 0),
    );
    let beside = foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(650, 0));
    attack_click(&mut world, enemy);
    for _ in 0..rules::TICKS_PER_SECOND * 10 {
        let events = world.step();
        let launched = events.iter().any(|event| {
            matches!(event.kind(), EventKind::AttackLaunched { attacker, .. } if attacker == wire_id(hero))
        });
        if launched {
            break;
        }
    }
    seat_cheat(&mut world, 1, Cheat::Invulnerable { on: true });
    let stand = seat_command(
        &world,
        0,
        Order::Move {
            target: bota_proto::Target::None,
        },
    );
    world.advance_trusted(&[stand]);
    for _ in 0..rules::TICKS_PER_SECOND * 2 {
        for event in world.step() {
            if let EventKind::Damaged {
                source: Some(source),
                target,
                ..
            } = event.kind()
            {
                assert!(
                    source != wire_id(hero)
                        || (target != wire_id(enemy) && target != wire_id(beside)),
                    "{:?}",
                    event.kind()
                );
            }
        }
    }
}

/// A swing broken off before its attack point rolls nothing: Dota decides an attack's procs at the
/// attack point. A Pudge with a Maelstrom and a Crystalys ordered to stand the tick after its
/// attack order leaves both runs unrolled.
#[test]
fn a_swing_broken_off_before_its_attack_point_rolls_nothing() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::MAELSTROM, ItemId::CRYSTALYS],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    attack_click(&mut world, enemy);
    let stand = seat_command(
        &world,
        0,
        Order::Move {
            target: bota_proto::Target::None,
        },
    );
    let events = world.advance_trusted(&[stand]);
    assert!(
        events
            .iter()
            .all(|event| !matches!(event.kind(), EventKind::AttackLaunched { .. }))
    );
    for _ in 0..rules::TICKS_PER_SECOND * 2 {
        world.step();
    }
    for item in [ItemId::MAELSTROM, ItemId::CRYSTALYS] {
        let place = item_place(slot_holding(&world, hero, item));
        assert!(!rolled(&world, hero, place), "{item:?} rolled");
    }
}

/// Chain Lightning "Cannot bounce on ... Enemies inside the Fog of War" (Liquipedia, Maelstrom): a
/// fourth enemy 600 past the third, 1150 off the wielder, is struck by day (in its 1800 sight) and
/// not at night (800, Dota's Demo Hero's night toggle).
#[test]
fn chain_lightning_leaps_to_no_enemy_in_the_fog() {
    for night in [false, true] {
        let (mut world, hero, enemy) = duel(
            HeroId::PUDGE,
            &[ItemId::MAELSTROM],
            HeroId::PUDGE,
            Vec2::from_ints(150, 0),
        );
        let near = foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(350, 0));
        let far = foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(950, 0));
        seat_cheat(&mut world, 0, Cheat::TimeOfDay { night });
        let attacks = attack_until(&mut world, hero, enemy, 12);
        let maelstrom = Some(Inflictor::Item(ItemId::MAELSTROM));
        let struck = |unit| {
            attacks
                .blows
                .iter()
                .any(|blow| blow.target == unit && blow.inflictor == maelstrom)
        };
        assert!(struck(near), "night {night}: a bolt went off");
        assert_eq!(struck(far), !night, "night {night}");
    }
}

/// Of two Maelstroms one rolls (one chain a swing): the first's run decides every swing's bolt,
/// and the second's is never rolled.
#[test]
fn two_maelstroms_roll_as_one() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::MAELSTROM, ItemId::MAELSTROM],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    let mut run = Prd::new(&world, hero, item_place(0), C25);
    let attacks = attack_until(&mut world, hero, enemy, 24);
    let maelstrom = Some(Inflictor::Item(ItemId::MAELSTROM));
    for swing in attack_blows(&attacks.blows, enemy) {
        let bolts = attacks
            .blows
            .iter()
            .filter(|blow| {
                blow.target == enemy
                    && blow.inflictor == maelstrom
                    && (blow.tick == swing.tick || blow.tick == swing.tick + 1)
            })
            .count();
        assert_eq!(
            bolts,
            usize::from(run.next()),
            "the swing at {}",
            swing.tick
        );
    }
    assert!(!rolled(&world, hero, item_place(1)), "the second rolled");
}

/// A Phantom Critical's magic rides with its attack and lands only if the attack does: against a
/// Talisman of Evasion carrier, every swing rolls the Brooch's run, and a proc swing that is evaded
/// lands no magic.
#[test]
fn a_revenants_brooch_lands_no_magic_with_an_evaded_swing() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::REVENANTS_BROOCH],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    seat_cheat(
        &mut world,
        1,
        Cheat::Item {
            item: ItemId::TALISMAN_OF_EVASION,
        },
    );
    let place = item_place(slot_holding(&world, hero, ItemId::REVENANTS_BROOCH));
    let mut run = Prd::new(&world, hero, place, C30);
    let attacks = attack_until(&mut world, hero, enemy, 40);
    let brooch = Some(Inflictor::Item(ItemId::REVENANTS_BROOCH));
    let mut evaded_procs = 0;
    for &launch in &attacks.launched {
        let landed = attacks
            .blows
            .iter()
            .any(|blow| blow.tick == launch && blow.target == enemy && blow.inflictor.is_none());
        let magic = attacks
            .blows
            .iter()
            .any(|blow| blow.tick == launch && blow.inflictor == brooch);
        let procs = run.next();
        assert_eq!(magic, procs && landed, "the swing at {launch}");
        evaded_procs += usize::from(procs && !landed);
    }
    assert!(evaded_procs > 0, "some proc swing was evaded");
}

/// Splitshot never goes off against a building: a Shadow Fiend with a Specialist's Array, kept
/// standing by our Invulnerable cheat (Dota's Demo Hero's), shoots a tower and rolls nothing.
#[test]
fn specialists_array_never_rolls_against_a_building() {
    let (mut world, hero, _) = duel(
        HeroId::SHADOW_FIEND,
        &[ItemId::SPECIALISTS_ARRAY],
        HeroId::PUDGE,
        Vec2::from_ints(1200, 0),
    );
    let tower = world
        .entities
        .iter()
        .filter(|&unit| {
            world.kind.get(unit) == Some(&UnitKind::Tower)
                && world.team.get(unit) == Some(&Team::Dire)
        })
        .min_by_key(|&unit| world.tier.get(unit).map_or(u8::MAX, |tier| tier.0))
        .expect("a Dire tower");
    let near = world.pos(tower).expect("standing");
    let at = [(-400, 0), (0, -400), (400, 0), (0, 400)]
        .map(|(x, y)| near + Vec2::from_ints(x, y))
        .into_iter()
        .find(|&at| {
            let order = Order::Cheat {
                cheat: Cheat::Teleport { at },
            };
            world.validate_order(SlotId(0), None, &order).is_ok()
        })
        .expect("open ground beside the tower");
    seat_cheat(&mut world, 0, Cheat::Teleport { at });
    seat_cheat(&mut world, 0, Cheat::Invulnerable { on: true });
    attack_until(&mut world, hero, tower, 10);
    let place = item_place(slot_holding(&world, hero, ItemId::SPECIALISTS_ARRAY));
    assert!(
        !rolled(&world, hero, place),
        "Splitshot rolled against a tower"
    );
}

/// Splitshot picks heroes before other units: with a camp of enemy lane creeps nearer
/// (`-createcreeps`) and an enemy hero farther but in reach, every split throws at the hero.
#[test]
fn specialists_array_splits_onto_a_hero_before_nearer_creeps() {
    let (mut world, hero, enemy) = duel(
        HeroId::SHADOW_FIEND,
        &[ItemId::SPECIALISTS_ARRAY],
        HeroId::PUDGE,
        Vec2::from_ints(400, 0),
    );
    let other = foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(600, 0));
    let at = world.pos(hero).expect("standing") + Vec2::from_ints(250, 0);
    seat_cheat(&mut world, 0, Cheat::CreateCreeps { enemy: true, at });
    let place = item_place(slot_holding(&world, hero, ItemId::SPECIALISTS_ARRAY));
    let mut run = Prd::new(&world, hero, place, C30);
    let attacks = attack_until(&mut world, hero, enemy, 10);
    let procs = attack_blows(&attacks.blows, enemy)
        .iter()
        .filter(|_| run.next())
        .count();
    let on_hero = attacks
        .blows
        .iter()
        .filter(|blow| blow.target == other)
        .count();
    assert!(procs > 0);
    assert_eq!(on_hero, procs, "one missile at the hero every split");
}

/// Battle Fury's Quell stacks with a Quelling Blade's but not another Battle Fury's: each kind of
/// quell counts once (the install's Quelling Blade: "Effects of multiple quelling blades do not
/// stack"). Two Battle Furies quell 10 (`quelling_bonus`), a Battle Fury and a Quelling Blade 18.
#[test]
fn two_battle_furies_quell_as_one_and_a_quelling_blade_adds() {
    for (items, quell) in [
        (&[ItemId::BATTLE_FURY, ItemId::BATTLE_FURY][..], 10),
        (&[ItemId::BATTLE_FURY, ItemId::QUELLING_BLADE][..], 18),
    ] {
        let (world, hero, _) = duel(HeroId::PUDGE, items, HeroId::PUDGE, Vec2::from_ints(150, 0));
        let got = world.stats.get(hero).expect("derived").damage_to_creeps;
        assert_eq!(got, quell, "{items:?}");
    }
}

/// Seat `slot`'s courier, walked by a validated order of its seat to `offset` from `of` and left
/// standing there.
fn courier_beside(world: &mut World, slot: u8, of: Entity, offset: Vec2) -> Entity {
    let courier = world.seats[usize::from(slot)].courier.expect("a courier");
    let at = world.pos(of).expect("standing") + offset;
    let walk = unit_command(
        world,
        slot,
        Some(wire_id(courier)),
        Order::Move {
            target: bota_proto::Target::Pos(at),
        },
    );
    world.advance_trusted(&[walk]);
    for _ in 0..rules::TICKS_PER_SECOND * 90 {
        world.step();
        if world
            .pos(courier)
            .is_some_and(|pos| pos.within(at, rules::units(40)))
        {
            let stand = unit_command(
                world,
                slot,
                Some(wire_id(courier)),
                Order::Move {
                    target: bota_proto::Target::None,
                },
            );
            world.advance_trusted(&[stand]);
            return courier;
        }
    }
    panic!("the courier never arrived");
}

/// A courier is spell immune, and Chain Lightning's bolt does not go through spell immunity (its
/// script names no SpellImmunityType): a Shadow Fiend's swings at an enemy courier, an enemy hero
/// beside it, roll no Chain Lightning and strike nobody with a bolt. SECONDARY-SOURCED (Liquipedia,
/// Courier: "Couriers are spell immune", "Most abilities do not work on couriers"); on the release
/// checklist.
#[test]
fn maelstrom_goes_off_on_no_swing_at_a_spell_immune_courier() {
    let (mut world, hero, enemy) = duel(
        HeroId::SHADOW_FIEND,
        &[ItemId::MAELSTROM],
        HeroId::PUDGE,
        Vec2::from_ints(550, 0),
    );
    let courier = courier_beside(&mut world, 1, hero, Vec2::from_ints(350, 0));
    assert!(world.stats.get(courier).expect("derived").spell_immune);
    let attacks = attack_for_ticks(&mut world, hero, courier, rules::TICKS_PER_SECOND * 6);
    assert!(!attacks.launched.is_empty(), "the courier was attacked");
    let place = item_place(slot_holding(&world, hero, ItemId::MAELSTROM));
    assert!(
        !rolled(&world, hero, place),
        "Chain Lightning rolled at the courier"
    );
    let maelstrom = Some(Inflictor::Item(ItemId::MAELSTROM));
    assert!(attacks.blows.iter().all(|blow| blow.inflictor != maelstrom));
    assert!(world.alive(enemy));
}

/// A bolt leaps onto no courier (a chain leaps onto no unit of fixed health, the neutrals' rule): an
/// enemy courier nearer the struck than the next enemy hero is passed over, and the hero is struck.
#[test]
fn chain_lightning_leaps_past_a_spell_immune_courier() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::MAELSTROM],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    let next = foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(550, 0));
    let courier = courier_beside(&mut world, 1, hero, Vec2::from_ints(300, 100));
    let attacks = attack_until(&mut world, hero, enemy, 12);
    let maelstrom = Some(Inflictor::Item(ItemId::MAELSTROM));
    let struck = |unit| {
        attacks
            .blows
            .iter()
            .any(|blow| blow.target == unit && blow.inflictor == maelstrom)
    };
    assert!(struck(next), "a bolt leapt on");
    assert!(!struck(courier), "a bolt leapt onto the courier");
    assert!(attacks.blows.iter().all(|blow| blow.target != courier));
}

/// `by` attacks `on` by a validated order for `ticks` ticks, whatever lands, and stands as soon as
/// `on` has fallen: it swings at nothing else.
fn attack_for_ticks(world: &mut World, by: Entity, on: Entity, ticks: u32) -> Attacks {
    let attacker = world.owner.get(by).expect("a seat's unit").0;
    let order = Order::Attack {
        target: bota_proto::Target::Unit(wire_id(on)),
    };
    let command = unit_command(world, attacker, None, order);
    let mut events = world.advance_trusted(&[command]);
    let mut attacks = Attacks {
        launched: Vec::new(),
        blows: Vec::new(),
    };
    for _ in 0..ticks {
        let tick = world.tick;
        for event in std::mem::take(&mut events) {
            match event.kind() {
                EventKind::AttackLaunched { attacker, .. } if attacker == wire_id(by) => {
                    attacks.launched.push(tick);
                }
                EventKind::Damaged {
                    source: Some(source),
                    target,
                    amount,
                    kind,
                    inflictor,
                    ..
                } if source == wire_id(by) => {
                    if let Some(target) =
                        world.entities.iter().find(|&unit| wire_id(unit) == target)
                    {
                        attacks.blows.push(Blow {
                            tick,
                            target,
                            amount,
                            kind,
                            inflictor,
                        });
                    }
                }
                _ => {}
            }
        }
        if !world.alive(on) {
            let stand = unit_command(
                world,
                attacker,
                None,
                Order::Move {
                    target: bota_proto::Target::None,
                },
            );
            world.advance_trusted(&[stand]);
            return attacks;
        }
        events = world.step();
    }
    attacks
}

/// Cleave and Splitshot pass couriers over (Liquipedia, Courier: Battle Fury's Cleave "can't
/// affect courier"; Specialist's Array: no couriers among its targets): an enemy courier in the
/// cleave's trapezoid and in Splitshot's reach takes nothing while its owner's hero is struck.
#[test]
fn cleave_and_splitshot_pass_couriers_over() {
    for (hero, item, gap, beside) in [
        (
            HeroId::PUDGE,
            ItemId::BATTLE_FURY,
            150,
            Vec2::from_ints(350, 0),
        ),
        (
            HeroId::SHADOW_FIEND,
            ItemId::SPECIALISTS_ARRAY,
            350,
            Vec2::from_ints(400, 100),
        ),
    ] {
        let (mut world, unit, enemy) = duel(hero, &[item], HeroId::PUDGE, Vec2::from_ints(gap, 0));
        let courier = courier_beside(&mut world, 1, unit, beside);
        let attacks = attack_until(&mut world, unit, enemy, 12);
        assert!(
            attacks.blows.iter().all(|blow| blow.target != courier),
            "{item:?}"
        );
        let place = item_place(slot_holding(&world, unit, item));
        if item == ItemId::SPECIALISTS_ARRAY {
            assert!(
                rolled(&world, unit, place),
                "Splitshot rolled: the pass-over is not vacuous"
            );
        }
    }
}

/// Chain Lightning's own wait outlasts a refresh: Dota's `-refresh` readies items and abilities,
/// and the bolt's timer is neither (a Maelstrom shows no cooldown).
#[test]
fn a_refresh_leaves_chain_lightnings_wait_as_it_was() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::MAELSTROM],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    let place = usize::from(item_place(slot_holding(&world, hero, ItemId::MAELSTROM)));
    let wait_of = |world: &World| {
        let roll = world
            .crit
            .get(hero.index().0 as usize)
            .and_then(Option::as_ref)
            .and_then(|rolls| rolls.sources().get(place))
            .and_then(Option::as_ref)?;
        (world.tick..world.tick + 30).find(|&at| roll.ready(at))
    };
    attack_click(&mut world, enemy);
    for _ in 0..rules::TICKS_PER_SECOND * 60 {
        world.step();
        let Some(until) = wait_of(&world).filter(|&until| until > world.tick + 2) else {
            continue;
        };
        seat_cheat(&mut world, 0, Cheat::Refresh);
        assert_eq!(wait_of(&world), Some(until), "the refresh moved the wait");
        return;
    }
    panic!("no bolt in a minute");
}

/// A damage block answers to a blow's category and kind, never to who or what dealt it: a Vanguard
/// carrier blocks a physical cleave on its run, and neither a plain physical blow nor a magical
/// cleave (ours: blows laid on the queue with no item named, so no item can stand for the
/// category).
#[test]
fn a_damage_block_answers_to_a_physical_cleave_and_nothing_else_that_is_not_an_attack() {
    for (category, kind, blocks) in [
        (crate::game::Category::Plain, DamageKind::Physical, false),
        (crate::game::Category::Cleave, DamageKind::Magical, false),
        (crate::game::Category::Cleave, DamageKind::Physical, true),
    ] {
        let (mut world, hero, enemy) =
            duel(HeroId::PUDGE, &[], HeroId::PUDGE, Vec2::from_ints(150, 0));
        seat_cheat(
            &mut world,
            1,
            Cheat::Item {
                item: ItemId::VANGUARD,
            },
        );
        let mut run = Prd::new(&world, enemy, 128, C60);
        let mut blocked = 0;
        for _ in 0..6 {
            world.push_hit(Some(hero), enemy, 100, kind, None);
            world.hits.back_mut().expect("a blow queued").category = category;
            let block = blocks && run.next();
            blocked += usize::from(block);
            let expected = match kind {
                DamageKind::Physical => physical_on(&world, enemy, if block { 50 } else { 100 }),
                _ => magical_on(&world, enemy, 100),
            };
            let took: Vec<i32> = world
                .step()
                .iter()
                .filter_map(|event| match event.kind() {
                    EventKind::Damaged {
                        source: Some(source),
                        target,
                        amount,
                        ..
                    } if source == wire_id(hero) && target == wire_id(enemy) => Some(amount),
                    _ => None,
                })
                .collect();
            assert_eq!(took, vec![expected], "{category:?} {kind:?}");
        }
        assert_eq!(blocked > 0, blocks, "{category:?} {kind:?}");
    }
}

/// Quell counts each kind once, for melee and ranged carriers alike: two Quelling Blades 8 / 4,
/// two Battle Furies 10 / 5, one of each 18 / 9 (`damage_bonus`, `damage_bonus_ranged`,
/// `quelling_bonus`, `quelling_bonus_ranged`).
#[test]
fn quell_counts_each_kind_once_for_melee_and_ranged_carriers() {
    for (hero, qb, bf) in [(HeroId::PUDGE, 8, 10), (HeroId::SHADOW_FIEND, 4, 5)] {
        for (items, quell) in [
            ([ItemId::QUELLING_BLADE, ItemId::QUELLING_BLADE], qb),
            ([ItemId::BATTLE_FURY, ItemId::BATTLE_FURY], bf),
            ([ItemId::QUELLING_BLADE, ItemId::BATTLE_FURY], qb + bf),
        ] {
            let (world, unit, _) = duel(hero, &items, HeroId::PUDGE, Vec2::from_ints(300, 0));
            let got = world.stats.get(unit).expect("derived").damage_to_creeps;
            assert_eq!(got, quell, "{hero:?} {items:?}");
        }
    }
}

/// Splitshot never goes off on a deny: an allied lane creep (`-createcreeps`), worn down by the
/// enemy player's attacks until it may be denied, is denied by a Shadow Fiend with a Specialist's
/// Array through a validated attack order, and the Array rolls nothing.
#[test]
fn specialists_array_rolls_nothing_on_a_deny() {
    let (mut world, hero, enemy) = duel(
        HeroId::SHADOW_FIEND,
        &[ItemId::SPECIALISTS_ARRAY],
        HeroId::SHADOW_FIEND,
        Vec2::from_ints(600, 0),
    );
    let at = world.pos(hero).expect("standing") + Vec2::from_ints(300, 0);
    seat_cheat(&mut world, 0, Cheat::CreateCreeps { enemy: false, at });
    let creep = world
        .entities
        .iter()
        .filter(|&unit| {
            world.team.get(unit) == Some(&Team::Radiant)
                && world
                    .kind
                    .get(unit)
                    .is_some_and(|kind| bota_core::kind_def(*kind).lane_creep)
        })
        .min_by_key(|&unit| {
            world
                .pos(unit)
                .map_or(i64::MAX, |pos| pos.distance_squared(at))
        })
        .expect("an allied lane creep");
    let hurt = seat_command(
        &world,
        1,
        Order::Attack {
            target: bota_proto::Target::Unit(wire_id(creep)),
        },
    );
    world.advance_trusted(&[hurt]);
    let deniable = (0..rules::TICKS_PER_SECOND * 10).any(|_| {
        world.step();
        world.deniable(creep)
    });
    assert!(
        deniable && world.alive(creep),
        "the enemy's swings leave a deniable creep"
    );
    let stop = seat_command(
        &world,
        1,
        Order::Move {
            target: bota_proto::Target::None,
        },
    );
    world.advance_trusted(&[stop]);
    let attacks = attack_for_ticks(&mut world, hero, creep, rules::TICKS_PER_SECOND * 6);
    assert!(
        attacks.blows.iter().any(|blow| blow.target == creep),
        "the deny landed"
    );
    let place = item_place(slot_holding(&world, hero, ItemId::SPECIALISTS_ARRAY));
    assert!(!rolled(&world, hero, place), "Splitshot rolled on a deny");
    assert!(world.alive(enemy));
}
