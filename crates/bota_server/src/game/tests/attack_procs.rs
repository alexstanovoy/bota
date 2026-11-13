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
    let attacker = seat_of(world, by).expect("a seat's hero attacks");
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
    let mut commands = vec![seat_command(world, attacker, order)];
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
            commands.push(seat_command(world, attacker, stand));
        }
        if after.is_some_and(|until| world.tick >= until) {
            return attacks;
        }
    }
    panic!("{landed} of {swings} swings landed");
}

/// A seat's order, validated as the server validates any.
fn seat_command(world: &World, slot: u8, order: Order) -> Command {
    assert_eq!(world.validate_order(SlotId(slot), None, &order), Ok(()));
    Command {
        slot: SlotId(slot),
        unit: None,
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
        along < 650 && aside * 2 < 150 + 210 * along / 650,
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

/// Battle Fury's Cleave deals 40% (`cleave_damage_percent_creep`) to a creep in the cone. The
/// creep is stood up directly: no Dota cheat stands a lane creep at a spot.
#[test]
fn a_battle_fury_cleaves_forty_percent_onto_a_creep_in_its_cone() {
    let (mut world, hero, enemy) = duel(
        HeroId::PUDGE,
        &[ItemId::BATTLE_FURY],
        HeroId::PUDGE,
        Vec2::from_ints(150, 0),
    );
    let at = world.pos(hero).expect("standing") + Vec2::from_ints(350, 0);
    let creep = thinking_creep_of(&mut world, Team::Dire, at);
    world.step();
    let share = taken_at(&world, hero, creep, 40);
    let attacks = attack_until(&mut world, hero, enemy, 1);
    let swing = attack_blows(&attacks.blows, enemy)[0];
    let bfury = Some(Inflictor::Item(ItemId::BATTLE_FURY));
    let cleaved: Vec<&Blow> = attacks
        .blows
        .iter()
        .filter(|blow| blow.tick == swing.tick && blow.inflictor == bfury)
        .collect();
    assert_eq!(cleaved.len(), 1, "{cleaved:?}");
    assert_eq!(cleaved[0].target, creep);
    assert!(
        share.contains(&cleaved[0].amount),
        "{:?}: {share:?}",
        cleaved[0]
    );
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

/// An AoE bonus widens a cleave (Dota 7.36: "AoE Bonuses now affect all auras and Cleave
/// sources"; Battle Fury's widths are `affected_by_aoe_increase`): a hero beside the cone's edge
/// is cleaved with a Chasm Stone's 40 (`aoe_bonus`) carried, and not without.
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
        let beside = foe(&mut world, hero, HeroId::PUDGE, Vec2::from_ints(400, 150));
        let from = world.pos(hero).expect("standing");
        let towards = world.pos(enemy).expect("standing");
        let (along, aside) = along_and_aside(from, towards, world.pos(beside).expect("standing"));
        // Outside the plain cone's half width there, inside the widened one's.
        assert!(aside * 2 > 150 + 210 * along / 650, "{along} {aside}");
        assert!(aside * 2 <= 190 + 210 * along / 650, "{along} {aside}");
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
