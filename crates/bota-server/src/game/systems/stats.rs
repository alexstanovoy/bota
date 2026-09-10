//! Working out what each entity fights by, from scratch, every tick.

use bota_proto::Fixed;

use crate::game::rules;
use crate::game::{
    AbilityBook, Def, EntityAllocator, Growth, Health, Inventory, Level, Mana, StackKind, Stacks,
    Stats, StatusKind, Statuses, Table, UnitDef, Upgrades,
};

/// What working out stats reads and writes.
///
/// The set is wide enough that naming the tables one by one runs past what is
/// readable at a call site; gathered here, the access a system takes is still
/// declared, and still checked when [`World::step`] hands the tables over.
///
/// [`World::step`]: crate::game::World::step
pub struct StatsCx<'a> {
    /// Which entities exist.
    pub entities: &'a EntityAllocator,
    /// Which kind of unit each entity is.
    pub def: &'a Table<Def>,
    /// Hero levels.
    pub level: &'a Table<Level>,
    /// Upgrade intervals a creep spawned after.
    pub upgrades: &'a Table<Upgrades>,
    /// What each entity carries.
    pub inventory: &'a Table<Inventory>,
    /// What is on each entity.
    pub statuses: &'a Table<Statuses>,
    /// What each entity has learned, for what its passives are worth.
    pub abilities: &'a Table<AbilityBook>,
    /// What each entity has kept of the deaths around it.
    pub stacks: &'a Table<Stacks>,
    /// Where the answer goes.
    pub stats: &'a mut Table<Stats>,
    /// Health, which follows its maximum.
    pub health: &'a mut Table<Health>,
    /// Mana, which follows its maximum.
    pub mana: &'a mut Table<Mana>,
}

/// Rewrites every entity's [`Stats`] from the kind of unit it is, how far it
/// has been raised, and what is on it.
///
/// A pool follows its maximum: when the maximum moves, the pool keeps its
/// filled fraction, and a pool that held anything is never left empty by the
/// move alone. An entity with no stats behind it yet has just been stood up,
/// and stands up full.
pub fn derive_stats(cx: StatsCx<'_>) {
    let StatsCx {
        entities,
        def,
        level,
        upgrades,
        inventory,
        statuses,
        abilities,
        stacks,
        stats,
        health,
        mana,
    } = cx;
    for entity in entities.iter() {
        let Some(Def(kind)) = def.get(entity) else {
            continue;
        };
        let levels = level.get(entity).map_or(0, |l| i32::from(l.0.max(1) - 1));
        let steps = upgrades.get(entity).map_or(0, |u| u.0 as i32);
        let mut now = raised(kind, levels, steps);
        if let Some(bag) = inventory.get(entity).filter(|_| !kind.porter) {
            let carried = crate::game::carried_bonus(bag);
            now.attributes += carried.attributes;
            now.max_hp += Fixed::from_int(carried.hp);
            now.max_mana += Fixed::from_int(carried.mana);
            now.hp_regen += carried.hp_regen;
            now.mana_regen += carried.mana_regen;
            now.damage += carried.damage;
            now.damage_to_creeps += carried.damage_to_creeps;
            now.attack_speed += carried.attack_speed;
            now.armor += carried.armor;
            now.move_speed += Fixed::from_int(carried.move_speed);
        }
        from_attributes(&mut now);
        // What the flesh heap has kept is worth health, and knowing it at all
        // is worth holding magic off.
        let heap = abilities.get(entity).map_or(0, |book| {
            book.slots
                .iter()
                .find(|slot| slot.id == crate::game::ability::FLESH_HEAP)
                .map_or(0, |slot| slot.level)
        });
        let gathered = stacks.get(entity).copied().unwrap_or_default();
        if heap > 0 {
            let kept = gathered.of(StackKind::FleshHeap);
            now.max_hp += Fixed::from_int(rules::FLESH_HEAP_HP * kept as i32);
            now.magic_resist_pct += rules::FLESH_HEAP_RESIST_PCT[usize::from(heap - 1)];
        }
        // Every soul gathered is worth attack damage for as long as it is
        // held.
        now.damage += rules::DAMAGE_PER_SOUL * gathered.of(StackKind::Souls) as i32;
        if let Some(on_it) = statuses.get(entity) {
            for status in on_it.active() {
                match status.kind {
                    StatusKind::Haste { speed } => now.attack_speed += speed,
                    StatusKind::Mending { per_tick, .. } => {
                        now.hp_regen += Fixed::from_ratio(per_tick, 100);
                    }
                    StatusKind::Clarity { per_tick, .. } => {
                        now.mana_regen += Fixed::from_ratio(per_tick, 100);
                    }
                    StatusKind::Fountain {
                        hp_per_tick,
                        mana_per_tick,
                    } => {
                        now.hp_regen += Fixed::from_ratio(hp_per_tick, 100);
                        now.mana_regen += Fixed::from_ratio(mana_per_tick, 100);
                    }
                    StatusKind::Slowed { pct } => {
                        now.move_speed = scaled(now.move_speed, (100 - pct).clamp(0, 100));
                    }
                    StatusKind::Hastened { pct } => {
                        now.move_speed = scaled(now.move_speed, 100 + pct.max(0));
                    }
                    StatusKind::ArmorBroken { armor } => {
                        now.armor -= Fixed::from_int(armor);
                    }
                    StatusKind::Guarded {
                        armor,
                        hp_per_second,
                    } => {
                        now.armor += Fixed::from_int(armor);
                        now.hp_regen += per_second(hp_per_second);
                    }
                    StatusKind::Inspired { hp_per_second } => {
                        now.hp_regen += per_second(hp_per_second);
                    }
                    StatusKind::Shielded => now.invulnerable = true,
                    StatusKind::Phased => now.phased = true,
                    // What holds a unit still and what burns it are read
                    // where they are acted on, not here.
                    StatusKind::Stunned | StatusKind::Burning { .. } => {}
                }
            }
        }
        now.attack_interval = swing_interval(now.attack_interval, now.attack_speed);
        let before = stats.get(entity).copied();
        if let Some(hp) = health.get_mut(entity) {
            hp.hp = match before {
                Some(before) => follow(hp.hp, before.max_hp, now.max_hp),
                None => now.max_hp,
            };
        }
        if let Some(mp) = mana.get_mut(entity) {
            mp.mana = match before {
                Some(before) => follow(mp.mana, before.max_mana, now.max_mana),
                None => now.max_mana,
            };
        }
        stats.insert(entity, now);
    }
}

/// What the three attributes are worth, added to whatever already stands.
///
/// Read after everything that adds attributes and before anything that reads
/// what they pay for.
fn from_attributes(now: &mut Stats) {
    let has = now.attributes;
    now.max_hp += Fixed::from_int(rules::HP_PER_STRENGTH) * has.strength;
    now.hp_regen += rules::HP_REGEN_PER_STRENGTH * has.strength;
    now.max_mana += Fixed::from_int(rules::MANA_PER_INTELLIGENCE) * has.intelligence;
    now.mana_regen += rules::MANA_REGEN_PER_INTELLIGENCE * has.intelligence;
    now.armor += rules::ARMOR_PER_AGILITY * has.agility;
    now.attack_speed += (has.agility * Fixed::from_int(rules::ATTACK_SPEED_PER_AGILITY)).to_int();
    if let Some(primary) = now.primary {
        now.damage += (has.of(primary) * Fixed::from_int(rules::DAMAGE_PER_PRIMARY)).to_int();
    }
}

/// The wait between two attacks at a given attack speed.
///
/// Never shorter than a tick: two attacks in one tick is a swing that never
/// happened.
fn swing_interval(interval: u32, speed: i32) -> u32 {
    let speed = speed.clamp(rules::MIN_ATTACK_SPEED, rules::MAX_ATTACK_SPEED);
    let scaled = i64::from(interval) * i64::from(rules::BASE_ATTACK_SPEED) / i64::from(speed);
    scaled.max(1) as u32
}

/// The plain form of a kind raised by `levels` levels and `steps` upgrades.
fn raised(kind: &UnitDef, levels: i32, steps: i32) -> Stats {
    let gained = |g: &Growth, per: &Growth| Growth {
        attributes: g.attributes.scaled(Fixed::from_int(levels))
            + per.attributes.scaled(Fixed::from_int(steps)),
        hp: g.hp * levels + per.hp * steps,
        mana: g.mana * levels + per.mana * steps,
        damage: g.damage * levels + per.damage * steps,
        armor_halves: g.armor_halves * levels + per.armor_halves * steps,
        gold: g.gold * levels + per.gold * steps,
        xp: g.xp * levels + per.xp * steps,
    };
    let up = gained(&kind.per_level, &kind.per_upgrade);
    Stats {
        attributes: kind.attributes + up.attributes,
        primary: kind.primary,
        max_hp: Fixed::from_int(kind.max_hp + up.hp),
        max_mana: Fixed::from_int(kind.max_mana + up.mana),
        hp_regen: kind.hp_regen,
        mana_regen: kind.mana_regen,
        damage: kind.damage + up.damage,
        damage_to_creeps: 0,
        attack_range: Fixed::from_int(kind.attack_range),
        acquisition: Fixed::from_int(kind.acquisition),
        attack_interval: kind.attack_interval,
        attack_speed: rules::BASE_ATTACK_SPEED,
        attack_point: kind.attack_point,
        attack_backswing: kind.attack_backswing,
        projectile_speed: kind.projectile_speed.map(Fixed::from_int),
        armor: Fixed::from_ratio(kind.armor * 2 + up.armor_halves, 2),
        magic_resist_pct: kind.magic_resist_pct,
        move_speed: Fixed::from_int(kind.move_speed),
        turn_rate: kind.turn_rate,
        vision: Fixed::from_int(kind.vision),
        true_sight: Fixed::from_int(kind.true_sight),
        hides: kind.hides,
        flies: kind.flies,
        phased: false,
        invulnerable: kind.invulnerable,
    }
}

/// What a pool holds once its maximum has moved.
///
/// The filled fraction is kept, worked out wide in raw units: a pool times a
/// pool is past what a [`Fixed`] holds. The result stays within
/// `Fixed::EPSILON..=now` when anything was held, and within `0..=now`
/// otherwise.
fn follow(held: Fixed, was: Fixed, now: Fixed) -> Fixed {
    if now <= Fixed::ZERO {
        return Fixed::ZERO;
    }
    if held <= Fixed::ZERO || was <= Fixed::ZERO {
        return held.clamp(Fixed::ZERO, now);
    }
    let kept = i64::from(held.raw) * i64::from(now.raw) / i64::from(was.raw);
    Fixed {
        raw: kept.clamp(i64::from(Fixed::EPSILON.raw), i64::from(now.raw)) as i32,
    }
}

/// Mending per tick, from hundredths of a point a second.
fn per_second(hundredths: i32) -> Fixed {
    Fixed::from_ratio(hundredths, 100 * rules::TICKS_PER_SECOND as i32)
}

/// A speed taken to a percent of itself.
///
/// Worked out wide: a speed in fixed point is already millions of raw units,
/// and a hundredth of it would overflow the width it is kept in.
fn scaled(speed: Fixed, pct: i32) -> Fixed {
    let raw = i64::from(speed.raw) * i64::from(pct) / 100;
    Fixed {
        raw: raw.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
    }
}
