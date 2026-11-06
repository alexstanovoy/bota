//! Hero credit and assist gold follow participation, including distant contributors.
//! https://liquipedia.net/dota2/Gold#Hero_kills
//! https://liquipedia.net/dota2/Gold#Assists_(AoE_gold)

use bota_core::{MELEE_CREEP, ModifierKind, tower_def};
use bota_proto::{DamageKind, EventKind, Fixed, Team, Vec2};

use super::support::*;
use crate::game::{Entity, Modifier, UnitOrder, World, wire_id};

const FALL: Vec2 = Vec2::from_ints(5000, 5000);

fn scene(allies: usize) -> (World, Vec<Entity>, Entity) {
    scene_for_hero(allies, bota_proto::HeroId::SHADOW_FIEND)
}

fn scene_for_hero(allies: usize, hero: bota_proto::HeroId) -> (World, Vec<Entity>, Entity) {
    let mut world = World::new();
    world.creeps_halted = true;
    let first = world.spawn_hero(
        Team::Radiant,
        FALL + Vec2::from_ints(-100, 0),
        bota_proto::SlotId(0),
        hero,
    );
    let mut seat = crate::game::Seat::new(
        bota_proto::SlotId(0),
        Team::Radiant,
        hero,
        0,
        bota_core::rules::STASH_SLOTS,
    );
    seat.unit = Some(first);
    world.seats.push(seat);
    let mut ours = vec![first];
    let victim = seat_hero(&mut world, 1, Team::Dire, FALL);
    for index in 1..allies {
        ours.push(seat_hero(
            &mut world,
            (index + 1) as u8,
            Team::Radiant,
            FALL + Vec2::from_ints(300 * index as i32, 0),
        ));
    }
    for seat in &mut world.seats {
        seat.gold = if seat.team == Team::Radiant {
            1000
        } else {
            1000 * allies as i32
        };
        seat.net_worth = seat.gold;
    }
    world.settle();
    for hero in ours.iter().copied().chain([victim]) {
        world.set_order(hero, UnitOrder::Stand);
    }
    (world, ours, victim)
}

fn touch(world: &mut World, source: Entity, victim: Entity, damage: bool) {
    if damage {
        world.push_hit(Some(source), victim, 10, DamageKind::Pure, None);
        world.step();
    } else {
        world.put_modifier(
            victim,
            Modifier::new(
                ModifierKind::ArmorBroken {
                    armor: Fixed::from_int(1),
                },
                Some(source),
                None,
                Some(90),
            ),
        );
    }
    world
        .transform
        .get_mut(source)
        .expect("the contributor")
        .pos = FALL + Vec2::from_ints(-3000, 1000);
}

fn finish(world: &mut World, victim: Entity, source: Entity) -> Vec<crate::game::Event> {
    world.push_hit(Some(source), victim, 10_000, DamageKind::Pure, None);
    world.step()
}

fn finisher(world: &mut World, tower: bool) -> Entity {
    let unit = world.spawn_unit(
        if tower { tower_def(1) } else { &MELEE_CREEP },
        Team::Radiant,
        FALL + Vec2::from_ints(600, 0),
    );
    world.settle();
    world.set_order(unit, UnitOrder::Stand);
    unit
}

#[test]
fn kill_credit_assigns_a_creep_or_tower_finish_to_the_single_recent_enemy_damager() {
    for tower in [false, true] {
        let (mut world, ours, victim) = scene(1);
        touch(&mut world, ours[0], victim, true);
        let source = finisher(&mut world, tower);
        let before = world.seats[0].gold;
        let events = finish(&mut world, victim, source);
        assert_eq!(world.seats[0].kills, 1, "the sole damager gets the kill");
        assert_eq!(
            world.seats[0].assists, 0,
            "its own credited kill is not an assist"
        );
        // Level-one bounty 125+8, First Blood 135; assist 15 + 50 + 0.037 * 1000, equal team worth.
        assert_eq!(world.seats[0].gold - before, 370);
        assert!(events.iter().any(|event| matches!(event.kind(), EventKind::Died { unit, killer: Some(killer), ref credited_hero, .. } if unit == wire_id(victim) && killer == wire_id(source) && credited_hero.as_deref() == Some(&wire_id(ours[0])))));
    }
}

#[test]
fn kill_credit_distinguishes_the_single_damager_from_a_debuff_only_contributor() {
    let (mut world, ours, victim) = scene(2);
    touch(&mut world, ours[0], victim, true);
    touch(&mut world, ours[1], victim, false);
    assert_eq!(world.seats[2].hero_damage, 0);
    let source = finisher(&mut world, false);
    let before: Vec<_> = world.seats.iter().map(|seat| seat.gold).collect();
    finish(&mut world, victim, source);
    assert_eq!(world.seats[0].kills, 1);
    assert_eq!(world.seats[2].kills, 0);
    // Both contributors receive 15 + (50 + 0.037 * 2000) / 2 = 77; only the damager gets 268 kill gold.
    assert_eq!(world.seats[0].gold - before[0], 345);
    assert_eq!(world.seats[2].gold - before[2], 77);
}

#[test]
fn kill_credit_assist_gold_includes_distant_damage_or_debuff_and_nearby_nonparticipants() {
    for damage in [false, true] {
        let (mut world, ours, victim) = scene(4);
        // Four Radiant heroes with 500 each match the victim's 2000 team worth.
        for seat in &mut world.seats {
            seat.gold = if seat.team == Team::Radiant {
                500
            } else {
                2000
            };
            seat.net_worth = seat.gold;
        }
        touch(&mut world, ours[1], victim, damage);
        world.transform.get_mut(ours[2]).unwrap().pos = FALL + Vec2::from_ints(1400, 0);
        world.transform.get_mut(ours[3]).unwrap().pos = FALL + Vec2::from_ints(3000, 0);
        let before: Vec<_> = world.seats.iter().map(|seat| seat.gold).collect();
        finish(&mut world, victim, ours[0]);
        // Killer, distant participant, nearby observer: 15 + (50 + 0.037 * 2000) / 3 rounds down to 56.
        assert_eq!(world.seats[0].gold - before[0], 324);
        assert_eq!(
            world.seats[2].gold - before[2],
            56,
            "the distant participant shares gold"
        );
        assert_eq!(
            world.seats[3].gold - before[3],
            56,
            "proximity also qualifies"
        );
        assert_eq!(
            world.seats[4].gold - before[4],
            0,
            "distance alone outside1500 does not qualify"
        );
    }
}

#[test]
fn kill_credit_does_not_assign_an_arbitrary_hero_after_multiple_or_expired_damage() {
    for tower in [false, true] {
        for expired in [false, true] {
            let (mut world, ours, victim) = scene(2);
            touch(&mut world, ours[0], victim, true);
            if expired {
                world.tick += 601;
            } else {
                touch(&mut world, ours[1], victim, true);
            }
            let source = finisher(&mut world, tower);
            let before: Vec<_> = world.seats.iter().map(|seat| seat.gold).collect();
            let events = finish(&mut world, victim, source);
            assert_eq!(world.seats[0].kills, 0);
            assert_eq!(world.seats[2].kills, 0);
            assert_eq!(world.first_blood_claimed, !expired);
            // Fresh damagers share 268 / 2 kill gold and 77 assist gold each.
            // Expired damage leaves 133 / 2 for each enemy; only the nearby ally gets 139 assist gold.
            assert_eq!(
                world.seats[0].gold - before[0],
                if expired { 66 } else { 211 }
            );
            assert_eq!(
                world.seats[2].gold - before[2],
                if expired { 205 } else { 211 }
            );
            assert!(events.iter().any(|event| matches!(event.kind(), EventKind::Died { unit, killer: Some(killer), .. } if unit == wire_id(victim) && killer == wire_id(source))));
        }
    }
}

#[test]
fn kill_credit_a_distant_participant_still_receives_assist_gold_after_dying() {
    let (mut world, ours, victim) = scene(4);
    for seat in &mut world.seats {
        seat.gold = if seat.team == Team::Radiant {
            500
        } else {
            2000
        };
        seat.net_worth = seat.gold;
    }
    touch(&mut world, ours[1], victim, true);
    world.transform.get_mut(ours[2]).unwrap().pos = FALL + Vec2::from_ints(1400, 0);
    world.transform.get_mut(ours[3]).unwrap().pos = FALL + Vec2::from_ints(3000, 0);
    world.health.insert(ours[1], health(0));
    world.bury(vec![(ours[1], None)], &mut Vec::new());
    let before = world.seats[2].gold;
    finish(&mut world, victim, ours[0]);
    assert!(!world.alive(ours[1]));
    // Its death leaves its team at 1988 worth: floor(2000 / 1988 * (15 + 124 / 3)) = 56.
    assert_eq!(world.seats[2].gold - before, 56);
}

#[test]
fn kill_credit_a_fresh_debuff_does_not_refresh_expired_damage_credit() {
    let (mut world, ours, victim) = scene(1);
    touch(&mut world, ours[0], victim, true);
    world.tick += 601;
    touch(&mut world, ours[0], victim, false);
    let source = finisher(&mut world, false);
    finish(&mut world, victim, source);
    assert_eq!(
        world.seats[0].kills, 0,
        "only fresh damage can claim a creep finish"
    );
}

#[test]
fn kill_credit_damage_from_a_previous_life_cannot_claim_the_next_death() {
    let (mut world, ours, victim) = scene(1);
    touch(&mut world, ours[0], victim, true);
    let source = finisher(&mut world, false);
    finish(&mut world, victim, source);
    assert_eq!(world.seats[0].kills, 1);
    world.seats[1].respawn_left = 1;
    world.step();
    assert!(world.alive(victim));
    world.transform.get_mut(victim).unwrap().pos = FALL;
    world.advance_trusted(&[crate::game::Command {
        slot: bota_proto::SlotId(1),
        unit: None,
        order: bota_proto::Order::Move {
            target: bota_proto::Target::None,
        },
        queue: false,
        seq: 0,
    }]);
    finish(&mut world, victim, source);
    assert_eq!(
        world.seats[1].deaths, 2,
        "both lives ended in actual finishing blows"
    );
    assert_eq!(
        world.seats[0].kills, 1,
        "the old contribution was spent by the first death"
    );
}

#[test]
fn kill_credit_assist_participation_expires_after_twenty_seconds() {
    for damage in [false, true] {
        for ticks in [600, 601] {
            let (mut world, ours, victim) = scene(2);
            touch(&mut world, ours[1], victim, damage);
            // The finishing step advances one tick; both causes use the same age.
            world.tick += ticks - 1;
            let before = world.seats[2].gold;
            finish(&mut world, victim, ours[0]);
            assert_eq!(
                world.seats[2].gold - before,
                if ticks == 600 { 77 } else { 0 },
                "damage={damage}, age={ticks} ticks"
            );
        }
    }
}

#[test]
fn kill_credit_neutral_finishes_and_denies_keep_the_physical_killer() {
    for side in [Team::Neutral, Team::Dire] {
        let (mut world, ours, victim) = scene(1);
        touch(&mut world, ours[0], victim, true);
        let source = world.spawn_unit(&MELEE_CREEP, side, FALL + Vec2::from_ints(600, 0));
        world.settle();
        world.set_order(source, UnitOrder::Stand);
        let events = finish(&mut world, victim, source);
        assert_eq!(world.seats[0].kills, 0);
        assert!(events.iter().any(|event| matches!(event.kind(),
            EventKind::Died { unit, killer: Some(killer), denied, .. }
                if unit == wire_id(victim) && killer == wire_id(source) && denied == (side == Team::Dire))));
    }
}

fn death_awards(events: &[crate::game::Event], victim: Entity) -> &[bota_proto::GoldAward] {
    events
        .iter()
        .find_map(|event| match event {
            crate::game::Event::Fact {
                kind:
                    EventKind::Died {
                        unit,
                        awards: Some(awards),
                        ..
                    },
                ..
            } if *unit == wire_id(victim) => Some(awards.as_slice()),
            _ => None,
        })
        .expect("the hero's death reports actual gold credits")
}

#[test]
fn kill_credit_awards_report_scaled_assist_gold_and_filter_it_by_recipient_team() {
    use crate::game::{AppliedModifier, AppliedModifiers, AppliedOrigin};
    use bota_proto::{GoldAward, GoldAwardKind, ModifierSpec, SlotId};
    let (mut world, ours, victim) = scene(2);
    let mut applied = AppliedModifiers::default();
    applied.push(AppliedModifier {
        spec: ModifierSpec {
            gold_income: 20_000,
            ..ModifierSpec::NOMINAL
        },
        ticks_left: Some(900),
        origin: AppliedOrigin::Cheat,
    });
    world.applied.insert(ours[1], applied);
    world.settle();
    touch(&mut world, ours[1], victim, true);
    let before: Vec<_> = world.seats.iter().map(|seat| seat.gold).collect();
    let events = finish(&mut world, victim, ours[0]);
    let awards = death_awards(&events, victim);
    assert_eq!(
        awards,
        [
            GoldAward {
                recipient: SlotId(0),
                team: Team::Radiant,
                kind: GoldAwardKind::KillBounty,
                amount: 268
            },
            GoldAward {
                recipient: SlotId(0),
                team: Team::Radiant,
                kind: GoldAwardKind::AssistGold,
                amount: 77
            },
            GoldAward {
                recipient: SlotId(2),
                team: Team::Radiant,
                kind: GoldAwardKind::AssistGold,
                amount: 154
            },
        ]
    );
    for index in [0, 2] {
        let seat = &world.seats[index];
        assert_eq!(
            awards
                .iter()
                .filter(|award| award.recipient == seat.slot)
                .map(|award| award.amount)
                .sum::<i32>(),
            seat.gold - before[index]
        );
    }
    assert_eq!(
        world.seats[2].assists, 1,
        "gold must not spend the ledger before scoreboard credit"
    );
    for viewing in [None, Some(Team::Radiant), Some(Team::Dire)] {
        let projected = crate::game_loop::visible_events(&events, viewing);
        let event = projected
            .iter()
            .find(|event| matches!(event, EventKind::Died { unit, .. } if *unit == wire_id(victim)))
            .expect("a hero death is public");
        let EventKind::Died {
            awards: disclosed,
            bounty,
            assists,
            ..
        } = event
        else {
            unreachable!()
        };
        assert_eq!(
            *bounty,
            Some(345),
            "legacy bounty stays the killer's aggregate"
        );
        assert_eq!(*assists, 1 << 2);
        assert_eq!(
            disclosed.as_deref().map(Vec::as_slice),
            if viewing == Some(Team::Dire) {
                None
            } else {
                Some(awards)
            }
        );
    }
}

#[test]
fn kill_credit_awards_cover_every_participant_in_a_ten_seat_match_once() {
    use bota_proto::GoldAwardKind;
    let (mut world, ours, victim) = scene(9);
    for &helper in &ours[1..] {
        touch(&mut world, helper, victim, false);
    }
    let before: Vec<_> = world.seats.iter().map(|seat| seat.gold).collect();
    let events = finish(&mut world, victim, ours[0]);
    let awards = death_awards(&events, victim);
    assert!(awards.len() <= bota_proto::MAX_DEATH_GOLD_AWARDS);
    assert_eq!(awards.len(), 10, "one kill bounty and nine assist shares");
    assert_eq!(
        awards
            .iter()
            .filter(|award| award.kind == GoldAwardKind::KillBounty)
            .count(),
        1
    );
    for (index, seat) in world
        .seats
        .iter()
        .enumerate()
        .filter(|(_, seat)| seat.team == Team::Radiant)
    {
        assert_eq!(
            awards
                .iter()
                .filter(
                    |award| award.recipient == seat.slot && award.kind == GoldAwardKind::AssistGold
                )
                .count(),
            1
        );
        assert_eq!(
            awards
                .iter()
                .filter(|award| award.recipient == seat.slot)
                .map(|award| award.amount)
                .sum::<i32>(),
            seat.gold - before[index]
        );
    }
}

#[test]
fn kill_credit_an_undamaged_creep_or_tower_finish_shares_bounty_with_every_enemy() {
    for tower in [false, true] {
        let (mut world, ours, victim) = scene(2);
        for &hero in &ours {
            world.transform.get_mut(hero).unwrap().pos = FALL + Vec2::from_ints(-3000, 0);
        }
        let source = finisher(&mut world, tower);
        let before: Vec<_> = world.seats.iter().map(|seat| seat.gold).collect();
        let events = finish(&mut world, victim, source);
        for index in [0, 2] {
            assert_eq!(
                world.seats[index].gold - before[index],
                66,
                "133 base bounty split between two enemies, without First Blood"
            );
            assert_eq!(world.seats[index].kills, 0);
        }
        assert_eq!(
            death_awards(&events, victim)
                .iter()
                .map(|award| award.amount)
                .sum::<i32>(),
            132
        );
        assert!(
            !world.first_blood_claimed,
            "an undamaged death leaves First Blood available"
        );
    }
}

#[test]
fn kill_credit_a_shared_hero_kill_spends_first_blood_once() {
    use bota_proto::GoldAwardKind;
    let (mut world, ours, victim) = scene(2);
    for &hero in &ours {
        touch(&mut world, hero, victim, true);
    }
    let source = finisher(&mut world, false);
    finish(&mut world, victim, source);
    world.seats[1].respawn_left = 1;
    world.step();
    world.transform.get_mut(victim).unwrap().pos = FALL;
    world.take_modifier(victim, ModifierKind::FountainInvulnerable, Some(victim));
    let events = finish(&mut world, victim, ours[0]);
    let award = death_awards(&events, victim)
        .iter()
        .find(|award| award.kind == GoldAwardKind::KillBounty)
        .expect("later solo bounty");
    assert_eq!(
        award.amount, 133,
        "the shared kill already claimed First Blood"
    );
}

#[test]
fn kill_credit_trailing_team_uses_the_victim_to_earner_worth_ratio() {
    for (allies, victim_worth, assist) in [(2, 4000, 228), (3, 9000, 428)] {
        let (mut world, ours, victim) = scene(allies);
        for seat in &mut world.seats {
            seat.gold = if seat.team == Team::Dire {
                victim_worth
            } else {
                1000
            };
            seat.net_worth = seat.gold;
        }
        let before: Vec<_> = world.seats.iter().map(|seat| seat.gold).collect();
        finish(&mut world, victim, ours[0]);
        assert_eq!(world.seats[0].gold - before[0], 268 + assist);
        for (seat, before) in world.seats.iter().zip(&before).skip(2) {
            assert_eq!(seat.gold - before, assist);
        }
    }
}

#[test]
fn kill_credit_presence_aura_records_an_effective_debuff_on_a_hero() {
    let (mut world, ours, victim) = scene(2);
    let caster = ours[1];
    world.abilities.get_mut(caster).unwrap().slots[4].level = 1;
    world.put_modifier(
        caster,
        Modifier::new(ModifierKind::Broken, Some(victim), None, None),
    );
    world.tick_auras();
    assert!(
        !world.seats[1].hurt_by.iter().any(|held| held.by == 2),
        "a broken aura is not effective participation"
    );
    world.take_modifier(caster, ModifierKind::Broken, Some(victim));
    world.tick_auras();
    let held = world.seats[1]
        .hurt_by
        .iter()
        .find(|held| held.by == 2)
        .expect("an effective harmful aura records its hero source");
    assert_eq!(
        held.damage_at, None,
        "Presence records a debuff, not damage"
    );
    assert_eq!(world.seats[2].hero_damage, 0);
}

#[test]
fn kill_credit_flagbearer_reporting_keeps_bounty_and_each_bonus_component() {
    use bota_proto::GoldAwardKind;
    let (mut world, ours, _) = scene(2);
    let flag = world.spawn_creep(&bota_core::FLAGBEARER_CREEP, Team::Dire, FALL, 0, 0);
    world.settle();
    let before: Vec<_> = world.seats.iter().map(|seat| seat.gold).collect();
    let events = finish(&mut world, flag, ours[0]);
    let awards = death_awards(&events, flag);
    assert_eq!(
        awards
            .iter()
            .filter(|award| award.kind == GoldAwardKind::KillBounty)
            .count(),
        1
    );
    assert_eq!(
        awards
            .iter()
            .filter(|award| award.kind == GoldAwardKind::FlagbearerBonus)
            .count(),
        2
    );
    for index in [0, 2] {
        assert_eq!(
            awards
                .iter()
                .filter(|award| award.recipient == world.seats[index].slot)
                .map(|award| award.amount)
                .sum::<i32>(),
            world.seats[index].gold - before[index]
        );
    }
}

#[test]
fn kill_credit_shared_bounty_and_assist_reports_fit_all_enemy_seats() {
    let (mut world, ours, victim) = scene(9);
    for &hero in &ours {
        touch(&mut world, hero, victim, true);
    }
    let source = finisher(&mut world, true);
    let events = finish(&mut world, victim, source);
    assert_eq!(
        death_awards(&events, victim).len(),
        18,
        "nine shared bounty and nine assist credits"
    );
}

#[test]
fn kill_credit_ratio_payout_preserves_balance_limits_and_actual_receipts() {
    let (mut world, ours, victim) = scene(2);
    for seat in &mut world.seats {
        seat.gold = if seat.team == Team::Dire {
            1_000_000_000
        } else {
            1
        };
        seat.net_worth = seat.gold;
    }
    let before: Vec<_> = world.seats.iter().map(|seat| seat.gold).collect();
    let events = finish(&mut world, victim, ours[0]);
    for index in [0, 2] {
        assert_eq!(world.seats[index].gold, i32::MAX);
        let reported: i64 = death_awards(&events, victim)
            .iter()
            .filter(|award| award.recipient == world.seats[index].slot)
            .map(|award| i64::from(award.amount))
            .sum();
        assert_eq!(
            reported,
            i64::from(world.seats[index].gold) - i64::from(before[index])
        );
    }
}

#[test]
fn kill_credit_nonkill_finishes_do_not_pay_streak_experience() {
    for assisted in [false, true] {
        let (mut world, ours, victim) = scene(1);
        world.seats[1].level = 4;
        world.seats[1].streak = 3;
        if assisted {
            touch(&mut world, ours[0], victim, true);
        }
        let source = finisher(&mut world, true);
        let before = world.seats[0].xp;
        finish(&mut world, victim, source);
        assert_eq!(world.seats[0].xp - before, if assisted { 155 } else { 100 });
        assert_eq!(world.seats[1].streak, if assisted { 0 } else { 3 });
    }
}

#[test]
fn kill_credit_a_zero_damage_blow_earns_an_assist_but_not_the_kill() {
    use crate::game::{AppliedModifier, AppliedModifiers, AppliedOrigin};
    use bota_proto::ModifierSpec;
    let (mut world, ours, victim) = scene(1);
    let mut applied = AppliedModifiers::default();
    applied.push(AppliedModifier {
        spec: ModifierSpec {
            magic_resist: 10_000,
            ..ModifierSpec::NOMINAL
        },
        ticks_left: Some(900),
        origin: AppliedOrigin::Cheat,
    });
    world.applied.insert(victim, applied);
    world.settle();
    world.push_hit(Some(ours[0]), victim, 100, DamageKind::Magical, None);
    let events = world.step();
    assert!(events.iter().any(|event| matches!(event.kind(), EventKind::Damaged { target, amount: 0, .. } if target == wire_id(victim))));
    let entry = world.seats[1]
        .hurt_by
        .iter()
        .find(|entry| entry.by == 0)
        .expect("negated damage records participation");
    assert_eq!(
        entry.damage_at, None,
        "a fully negated blow cannot own the later kill"
    );
    world.applied.remove(victim);
    world.settle();
    world.transform.get_mut(ours[0]).unwrap().pos = FALL + Vec2::from_ints(-3000, 1000);
    let source = finisher(&mut world, true);
    let before = world.seats[0].gold;
    finish(&mut world, victim, source);
    assert_eq!(world.seats[0].kills, 0);
    assert_eq!(world.seats[0].assists, 1);
    assert_eq!(
        world.seats[0].gold - before,
        235,
        "133 shared base bounty and 102 assist gold"
    );
}

#[test]
fn kill_credit_souls_follow_the_physical_finisher_not_the_credited_hero() {
    for npc in [None, Some(false), Some(true)] {
        let (mut world, ours, victim) = scene(1);
        touch(&mut world, ours[0], victim, true);
        let source = npc
            .map(|tower| finisher(&mut world, tower))
            .unwrap_or(ours[0]);
        let before = world.souls_of(ours[0]);
        finish(&mut world, victim, source);
        assert_eq!(world.seats[0].kills, 1);
        assert_eq!(
            world.souls_of(ours[0]) - before,
            if npc.is_some() { 0 } else { 4 },
            "Necromastery listens to the physical OnDeath attacker"
        );
    }
}

#[test]
fn kill_credit_death_reports_the_physical_finisher() {
    let (mut world, ours, victim) = scene(1);
    touch(&mut world, ours[0], victim, true);
    let source = finisher(&mut world, true);
    let events = finish(&mut world, victim, source);
    assert!(events.iter().any(|event| matches!(event.kind(), EventKind::Died { unit, killer: Some(killer), .. } if unit == wire_id(victim) && killer == wire_id(source))), "Died.killer preserves the killing-blow unit");
    assert_eq!(world.seats[0].kills, 1);
}

#[test]
fn kill_credit_neutral_finishes_pay_no_enemy_bounty_or_experience_after_damage() {
    for count in [1, 2] {
        let (mut world, ours, victim) = scene(2);
        world.seats[1].streak = 3;
        for &hero in &ours[..count] {
            touch(&mut world, hero, victim, true);
        }
        for &hero in &ours {
            world.transform.get_mut(hero).unwrap().pos = FALL + Vec2::from_ints(-100, 0);
        }
        let source = world.spawn_unit(&MELEE_CREEP, Team::Neutral, FALL + Vec2::from_ints(600, 0));
        world.settle();
        world.set_order(source, UnitOrder::Stand);
        let before: Vec<_> = world
            .seats
            .iter()
            .map(|seat| (seat.gold, seat.xp))
            .collect();
        let scrolls = world
            .inventory
            .get(victim)
            .unwrap()
            .held()
            .filter(|stack| stack.id == bota_proto::ItemId::TOWN_PORTAL_SCROLL)
            .map(|stack| stack.charges)
            .sum::<u8>();
        finish(&mut world, victim, source);
        for index in [0, 2] {
            assert_eq!(
                (world.seats[index].gold, world.seats[index].xp),
                before[index]
            );
        }
        assert_eq!(world.seats[1].streak, 3);
        assert!(!world.first_blood_claimed);
        assert!(world.seats[1].respawn_left >= 26 * bota_core::rules::TICKS_PER_SECOND);
        assert_eq!(
            world
                .inventory
                .get(victim)
                .unwrap()
                .held()
                .filter(|stack| stack.id == bota_proto::ItemId::TOWN_PORTAL_SCROLL)
                .map(|stack| stack.charges)
                .sum::<u8>(),
            scrolls
        );
    }
}

#[test]
fn kill_credit_flesh_heap_uses_credit_and_debuffs_outside_radius_and_nearby_deaths() {
    for mode in [0, 1, 2] {
        let (mut world, ours, victim) = scene_for_hero(2, bota_proto::HeroId::PUDGE);
        let source = match mode {
            0 => {
                touch(&mut world, ours[0], victim, true);
                finisher(&mut world, true)
            }
            1 => {
                world.put_modifier(
                    victim,
                    Modifier::new(
                        ModifierKind::Rotting {
                            level: 1,
                            slow_pct: 20,
                            since: 0,
                        },
                        Some(ours[0]),
                        None,
                        Some(90),
                    ),
                );
                world.transform.get_mut(ours[0]).unwrap().pos = FALL + Vec2::from_ints(-3000, 1000);
                ours[1]
            }
            _ => {
                let neutral =
                    world.spawn_unit(&MELEE_CREEP, Team::Neutral, FALL + Vec2::from_ints(600, 0));
                world.settle();
                world.set_order(neutral, UnitOrder::Stand);
                neutral
            }
        };
        finish(&mut world, victim, source);
        let count = world
            .modifiers
            .get(ours[0])
            .unwrap()
            .count_of(ModifierKind::flesh_heap);
        assert_eq!(
            count, 1,
            "Flesh Heap credited/debuff/nearby condition {mode}"
        );
    }
}

#[test]
fn kill_credit_announces_credit_while_preserving_physical_attacker_privacy() {
    let (mut world, ours, victim) = scene(1);
    touch(&mut world, ours[0], victim, true);
    let source = finisher(&mut world, true);
    let mut events = finish(&mut world, victim, source);
    let event = events
        .iter_mut()
        .find(|event| matches!(event.kind(), EventKind::Died { .. }))
        .unwrap();
    // Explicitly exercise a finisher hidden from Dire, independent of fixture vision.
    let crate::game::Event::Fact { source_told_to, .. } = event else {
        unreachable!("a death is a fact");
    };
    *source_told_to = crate::game::EventVisibility::OneTeam(Team::Radiant);
    for side in [None, Some(Team::Radiant), Some(Team::Dire)] {
        let projected = crate::game_loop::visible_events(&events, side);
        let EventKind::Died {
            killer,
            credited_hero,
            ..
        } = projected
            .iter()
            .find(|event| matches!(event, EventKind::Died { .. }))
            .unwrap()
        else {
            unreachable!()
        };
        assert_eq!(
            *killer,
            if side == Some(Team::Dire) {
                None
            } else {
                Some(wire_id(source))
            }
        );
        assert_eq!(
            credited_hero.as_deref(),
            Some(&wire_id(ours[0])),
            "announced hero credit is public"
        );
    }
}

#[test]
fn kill_credit_ten_hero_fight_event_budget_roundtrips_without_drops() {
    use bota_proto::{
        FrameReader, MAX_EVENTS_PER_BATCH, MAX_ORDERS_PER_TICK, MAX_SEATS, ServerMsg,
        encode_frame_to_vec,
    };
    // Deliberately exceed legal cast throughput: all 64 orders from all ten
    // heroes release 20 Requiem lines, with both passes hitting all ten heroes.
    // Add an AoE from every modifier, every tree falling/growing, every order's
    // announcement, ten deaths and the maximum 29 level-ups for each hero.
    let lines = MAX_SEATS
        * MAX_ORDERS_PER_TICK
        * bota_core::rules::REQUIEM_LINES_MAX as usize
        * 2
        * MAX_SEATS;
    let modifiers = MAX_SEATS * bota_core::rules::MAX_EFFECTS_PER_UNIT * MAX_SEATS;
    let trees = bota_proto::MAX_STATIC_TREES + bota_proto::MAX_PLANTED_TREES;
    let orders = MAX_SEATS * MAX_ORDERS_PER_TICK;
    let deaths_and_levels = MAX_SEATS * usize::from(bota_core::rules::HERO_MAX_LEVEL);
    let total = lines + modifiers + trees + orders + deaths_and_levels;
    assert!(total < MAX_EVENTS_PER_BATCH);
    let mut events = vec![
        EventKind::Damaged {
            source: Some(bota_proto::EntityId {
                idx: 0,
                generation: 0
            }),
            target: bota_proto::EntityId {
                idx: 1,
                generation: 0
            },
            amount: 1,
            kind: bota_proto::DamageKind::Magical,
            inflictor: None,
            removal: false
        };
        total
    ];
    // Include all ten simultaneous death receipts, each with the full bound.
    for (slot, event) in events.iter_mut().take(MAX_SEATS).enumerate() {
        *event = EventKind::Died {
            unit: bota_proto::EntityId {
                idx: slot as u32,
                generation: 0,
            },
            killer: None,
            denied: false,
            gold: 0,
            bounty: Some(100),
            assists: 0,
            credited_hero: None,
            awards: Some(Box::new(vec![
                bota_proto::GoldAward {
                    recipient: bota_proto::SlotId(0),
                    team: Team::Radiant,
                    kind: bota_proto::GoldAwardKind::AssistGold,
                    amount: 1
                };
                bota_proto::MAX_DEATH_GOLD_AWARDS
            ])),
        };
    }
    let message = ServerMsg::Events { tick: 1, events };
    let frame = encode_frame_to_vec(&message).expect("the ten-hero budget fits a frame");
    let mut reader = FrameReader::new();
    reader.push(&frame);
    assert_eq!(
        reader.next_message::<ServerMsg>().unwrap(),
        Some(message),
        "every event and receipt survives"
    );
    println!(
        "ten-hero stress: {total} events < {MAX_EVENTS_PER_BATCH} cap; {} encoded bytes; no drops",
        frame.len()
    );
}
