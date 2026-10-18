//! Working out what each entity fights by, from scratch, every tick.

use bota_core::{
    Carried, Growth, ModifierKind, Ratio, Regen, Talent, UnitDef, regen, rules, soul_damage,
};
use bota_proto::{AbilityId, Attributes, Fixed, ModifierSpec};

use crate::game::{
    AbilityBook, AppliedModifiers, Def, EntityAllocator, Feast, FeastPhase, Health, Inventory,
    Level, MAX_COMBINED_MODIFIER_SCALE, Mana, Modifiers, StackKind, Stacks, Stats, Table, Upgrades,
    carried_bonus,
};

/// What working out stats reads and writes.
pub struct StatsCx<'a> {
    pub entities: &'a EntityAllocator,
    pub def: &'a Table<Def>,
    pub level: &'a Table<Level>,
    /// Upgrade intervals a creep spawned after.
    pub upgrades: &'a Table<Upgrades>,
    pub inventory: &'a Table<Inventory>,
    pub modifiers: &'a Table<Modifiers>,
    pub applied: &'a Table<AppliedModifiers>,
    /// What each entity has learned, for what its passives are worth.
    pub abilities: &'a Table<AbilityBook>,
    /// What each entity has kept of the deaths around it.
    pub stacks: &'a Table<Stacks>,
    /// The feast each entity has on it, for its haste.
    pub feast: &'a Table<Feast>,
    /// Where the answer goes.
    pub stats: &'a mut Table<Stats>,
    /// Health, which follows its maximum.
    pub health: &'a mut Table<Health>,
    /// Mana, which follows its maximum.
    pub mana: &'a mut Table<Mana>,
    /// Whether it is night, which every unit sees by.
    pub night: bool,
}

/// Rewrites every entity's [`Stats`] from the kind of unit it is, how far it
/// has been raised, and what is on it.
///
/// A pool follows its maximum: when the maximum moves, the pool keeps its
/// filled fraction, so a full pool stays full and a pool that held anything
/// is never left empty by the move alone. An entity with no stats behind it
/// yet has just been stood up, and stands up full.
pub fn derive_stats(cx: StatsCx<'_>) {
    let StatsCx {
        entities,
        def,
        level,
        upgrades,
        inventory,
        modifiers,
        applied,
        abilities,
        stacks,
        feast,
        stats,
        health,
        mana,
        night,
    } = cx;
    for entity in entities.iter() {
        let Some(Def(kind)) = def.get(entity) else {
            continue;
        };
        let levels = level.get(entity).map_or(0, |l| i32::from(l.0.max(1) - 1));
        let steps = upgrades.get(entity).map_or(0, |u| u.0 as i32);
        let mut now = raised(kind, levels, steps, night);
        // Applied changes are folded in first and additively. Items,
        // attributes and every multiplier below land on top of them.
        if let Some(on_it) = applied.get(entity) {
            fold_applied(&mut now, on_it);
        }
        let ranged = kind.projectile_speed.is_some();
        let carried = inventory
            .get(entity)
            .filter(|_| !kind.porter)
            .map(|bag| carried_bonus(bag, ranged));
        if let Some(carried) = carried {
            add_carried(&mut now, &carried);
        }
        let gathered = stacks.get(entity).copied().unwrap_or_default();
        if let Some(book) = abilities.get(entity) {
            now.attributes +=
                Attributes::all(rules::ATTRIBUTE_BONUS_POINTS * i32::from(book.attribute_bonus));
        }
        add_heap(&mut now, &gathered, abilities.get(entity));
        from_attributes(&mut now);
        if let Some(carried) = &carried {
            add_carried_shares(&mut now, carried);
        }
        now.damage += soul_damage(gathered.of(StackKind::Souls), levels + 1);
        if let Some(on_it) = modifiers.get(entity) {
            add_modifiers(&mut now, on_it);
        }
        if let Some(feast) = feast.get(entity) {
            add_feast(&mut now, feast, abilities.get(entity));
        }
        let max = Regen {
            hp: now.max_hp,
            mana: now.max_mana,
        };
        let regen = settled_regen(kind, carried, now.attributes, max, modifiers.get(entity));
        (now.hp_regen, now.mana_regen) = (regen.hp, regen.mana);
        follow_pools(
            stats.get(entity).copied(),
            &now,
            health.get_mut(entity),
            mana.get_mut(entity),
        );
        stats.insert(entity, now);
    }
}

/// Health and mana mended each tick by a unit's kind, what it carries, its
/// attributes and the modifiers on it.
fn settled_regen(
    kind: &UnitDef,
    carried: Option<Carried>,
    attributes: Attributes,
    max: Regen,
    on_it: Option<&Modifiers>,
) -> Regen {
    regen(
        Regen {
            hp: kind.hp_regen,
            mana: kind.mana_regen,
        },
        carried.map_or(Regen::NONE, |carried| Regen {
            hp: carried.hp_regen,
            mana: carried.mana_regen,
        }),
        attributes,
        max,
        on_it
            .into_iter()
            .flat_map(|on_it| on_it.active().map(|held| held.kind)),
    )
}

/// Moves the pools with their maximums: each keeps its filled fraction, and
/// a unit with no stats behind it yet stands up full.
fn follow_pools(
    before: Option<Stats>,
    now: &Stats,
    health: Option<&mut Health>,
    mana: Option<&mut Mana>,
) {
    if let Some(hp) = health {
        hp.hp = before.map_or(now.max_hp, |before| {
            follow(hp.hp, before.max_hp, now.max_hp)
        });
    }
    if let Some(mp) = mana {
        mp.mana = before.map_or(now.max_mana, |before| {
            follow(mp.mana, before.max_mana, now.max_mana)
        });
    }
}

/// Adds the haste of a feast that is running, the talent's attack speed
/// with it when its caster has learned it.
fn add_feast(now: &mut Stats, feast: &Feast, book: Option<&AbilityBook>) {
    if feast.phase != FeastPhase::Feasting {
        return;
    }
    let level = usize::from(feast.level);
    now.attack_speed += rules::FEAST_ATTACK_SPEED[level];
    if book.is_some_and(|book| book.has_talent(Talent::FeastAttackSpeed)) {
        now.attack_speed += rules::TALENT_FEAST_ATTACK_SPEED;
    }
    now.move_speed = scaled(now.move_speed, 100 + rules::FEAST_MOVE_PCT[level]);
}

/// Adds what the inventory proper carries as shares of what the attributes
/// already brought: of the maximum mana, and of the base pace and what
/// agility adds to it, nothing else.
fn add_carried_shares(now: &mut Stats, carried: &Carried) {
    if carried.mana_pct != 0 {
        now.max_mana += now.max_mana * Fixed::from_ratio(carried.mana_pct, 100);
    }
    if carried.base_attack_speed_pct != 0 {
        let base = rules::BASE_ATTACK_SPEED + agility_pace(now.attributes);
        now.attack_speed += base * carried.base_attack_speed_pct / 100;
    }
}

/// Adds what the inventory proper carries.
fn add_carried(now: &mut Stats, carried: &Carried) {
    now.attributes += carried.attributes;
    now.max_hp += Fixed::from_int(carried.hp);
    now.max_mana += Fixed::from_int(carried.mana);
    now.damage += carried.damage;
    now.damage_to_creeps += carried.damage_to_creeps;
    now.attack_speed += carried.attack_speed;
    now.armor += carried.armor;
    now.move_speed += Fixed::from_int(carried.move_speed + carried.lace_speed);
    now.evasion = carried.evasion;
    now.pierce = carried.pierce;
    now.pierce_damage = carried.pierce_damage;
    if now.projectile_speed.is_none() {
        now.attack_range += Fixed::from_int(carried.melee_range);
    }
}

/// What the flesh heap has kept, worth strength by the heap's level once
/// the heap is learned at all; the heap's magic resistance multiplies with
/// what is already there.
fn add_heap(now: &mut Stats, gathered: &Stacks, book: Option<&AbilityBook>) {
    let heap = book.map_or(0, |book| {
        book.slots
            .iter()
            .find(|slot| slot.id == AbilityId::FLESH_HEAP)
            .map_or(0, |slot| slot.level)
    });
    let Some(level) = heap.checked_sub(1).map(usize::from) else {
        return;
    };
    let kept = gathered.of(StackKind::FleshHeap) as i32;
    now.attributes.strength += rules::FLESH_HEAP_STRENGTH_PER_STACK[level] * Fixed::from_int(kept);
    let let_through = (rules::NOMINAL_BP - now.magic_resist_bp)
        * (100 - rules::FLESH_HEAP_MAGIC_RESIST_PCT[level])
        / 100;
    now.magic_resist_bp = rules::NOMINAL_BP - let_through;
}

/// Adds what every active modifier on a unit is worth to its stats. What
/// holds a unit still, what burns it and what it hands out are read where
/// they are acted on, not here.
fn add_modifiers(now: &mut Stats, on_it: &Modifiers) {
    for held in on_it.active() {
        match held.kind {
            ModifierKind::Slowed { pct } => {
                now.move_speed = scaled(now.move_speed, (100 - pct).clamp(0, 100));
            }
            ModifierKind::BloodGrenade { .. } => {
                now.move_speed = scaled(now.move_speed, 100 - rules::GRENADE_SLOW_PCT);
            }
            ModifierKind::Hastened { pct } => {
                now.move_speed = scaled(now.move_speed, 100 + pct.max(0));
            }
            ModifierKind::ArmorBroken { armor } => {
                now.armor -= armor;
            }
            ModifierKind::MagicBroken { pct } => {
                let let_through =
                    i64::from(rules::NOMINAL_BP - now.magic_resist_bp) * i64::from(100 + pct) / 100;
                now.magic_resist_bp = (i64::from(rules::NOMINAL_BP) - let_through)
                    .clamp(rules::MIN_MAGIC_RESIST_BP.into(), rules::NOMINAL_BP.into())
                    as i32;
            }
            ModifierKind::Guarded { armor, .. } => now.armor += Fixed::from_int(armor),
            ModifierKind::Inspired {
                magic_resist_pct, ..
            } => {
                let let_through =
                    (rules::NOMINAL_BP - now.magic_resist_bp) * (100 - magic_resist_pct) / 100;
                now.magic_resist_bp = rules::NOMINAL_BP - let_through;
            }
            ModifierKind::Shielded => now.invulnerable = true,
            ModifierKind::Phased => now.phased = true,
            ModifierKind::Mending { .. }
            | ModifierKind::Clarity { .. }
            | ModifierKind::Fountain { .. }
            | ModifierKind::Stunned
            | ModifierKind::Feared
            | ModifierKind::Burning { .. }
            | ModifierKind::Shadowraze { .. }
            | ModifierKind::Rot { .. } => {}
        }
    }
}

/// Folds everything applied to a unit into a freshly raised stat block.
///
/// Every family is summed as a delta first and written once, so several
/// sources never compound. Resistances are added in their own units and
/// scales as deltas of the nominal.
fn fold_applied(now: &mut Stats, applied: &AppliedModifiers) {
    let nominal = rules::NOMINAL_BP;
    let mut sums = [0i32; 10];
    for held in applied.iter() {
        let spec = held.spec;
        let deltas = [
            spec.magic_resist,
            spec.status_resist,
            spec.physical_damage - nominal,
            spec.magic_damage - nominal,
            spec.pure_damage - nominal,
            spec.cooldown_rate - nominal,
            spec.mana_cost_rate - nominal,
            spec.move_speed - nominal,
            spec.max_hp - nominal,
            spec.max_mana - nominal,
        ];
        for (sum, delta) in sums.iter_mut().zip(deltas) {
            *sum = sum.saturating_add(delta);
        }
    }
    let [
        magic_resist,
        status_resist,
        physical,
        magical,
        pure,
        cooldown,
        mana_cost,
        move_speed,
        max_hp,
        max_mana,
    ] = sums;
    let add = |value: i32, delta: i32, least: i32, most: i32| {
        value.saturating_add(delta).clamp(least, most)
    };
    let most = MAX_COMBINED_MODIFIER_SCALE;
    now.magic_resist_bp = add(now.magic_resist_bp, magic_resist, 0, nominal);
    now.status_resist_bp = add(now.status_resist_bp, status_resist, 0, nominal - 1);
    now.physical_amp_bp = add(now.physical_amp_bp, physical, 0, most);
    now.magic_amp_bp = add(now.magic_amp_bp, magical, 0, most);
    now.pure_amp_bp = add(now.pure_amp_bp, pure, 0, most);
    now.cooldown_rate_bp = add(now.cooldown_rate_bp, cooldown, 1, most);
    now.mana_cost_rate_bp = add(now.mana_cost_rate_bp, mana_cost, 1, most);
    now.move_speed = scaled_bp(now.move_speed, combined_scale(move_speed));
    now.max_hp = scaled_bp(now.max_hp, combined_scale(max_hp));
    now.max_mana = scaled_bp(now.max_mana, combined_scale(max_mana));
}

/// What several additive scale deltas come to, kept inside the bounds one
/// source may carry.
fn combined_scale(total_delta: i32) -> i32 {
    rules::NOMINAL_BP
        .saturating_add(total_delta)
        .clamp(ModifierSpec::MIN_SCALE, ModifierSpec::MAX_SCALE)
}

/// A value at a basis-point scale, where [`rules::NOMINAL_BP`] leaves it as
/// it is.
fn scaled_bp(value: Fixed, bp: i32) -> Fixed {
    let raw = i64::from(value.raw) * i64::from(bp) / i64::from(rules::NOMINAL_BP);
    Fixed {
        raw: raw.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
    }
}

/// What the three attributes are worth, added to whatever already stands.
///
/// Read after everything that adds attributes and before anything that reads
/// what they pay for.
fn from_attributes(now: &mut Stats) {
    let has = now.attributes;
    now.max_hp += Fixed::from_int(rules::HP_PER_STRENGTH) * has.strength;
    now.max_mana += Fixed::from_int(rules::MANA_PER_INTELLIGENCE) * has.intelligence;
    now.armor += rules::ARMOR_PER_AGILITY * has.agility;
    now.attack_speed += agility_pace(has);
    if let Some(primary) = now.primary {
        now.damage += (has.of(primary) * Fixed::from_int(rules::DAMAGE_PER_PRIMARY)).to_int();
    }
}

/// Attack speed the agility attribute is worth.
fn agility_pace(has: Attributes) -> i32 {
    (has.agility * Fixed::from_int(rules::ATTACK_SPEED_PER_AGILITY)).to_int()
}

/// The plain form of a kind raised by `levels` levels and `steps` upgrades,
/// seeing by night or by day.
fn raised(kind: &UnitDef, levels: i32, steps: i32, night: bool) -> Stats {
    let gained = |g: &Growth, per: &Growth| Growth {
        attributes: g.attributes.scaled(Fixed::from_int(levels))
            + per.attributes.scaled(Fixed::from_int(steps)),
        hp: g.hp * levels + per.hp * steps,
        mana: g.mana * levels + per.mana * steps,
        damage: g.damage * levels + per.damage * steps,
        armor_halves: g.armor_halves * levels + per.armor_halves * steps,
        gold_halves: g.gold_halves * levels + per.gold_halves * steps,
        xp: g.xp * levels + per.xp * steps,
    };
    let up = gained(&kind.per_level, &kind.per_upgrade);
    Stats {
        attributes: kind.attributes + up.attributes,
        primary: kind.primary,
        max_hp: Fixed::from_int(kind.max_hp + up.hp),
        max_mana: Fixed::from_int(kind.max_mana + up.mana),
        hp_regen: Fixed::ZERO,
        mana_regen: Fixed::ZERO,
        damage: kind.damage + up.damage,
        damage_spread: kind.damage_spread,
        damage_to_creeps: 0,
        attack_range: Fixed::from_int(kind.attack_range),
        acquisition: Fixed::from_int(kind.acquisition),
        attack_time: kind.attack_time,
        attack_speed: kind.attack_speed,
        attack_point: kind.attack_point,
        attack_backswing: kind.attack_backswing,
        projectile_speed: kind.projectile_speed.map(Fixed::from_int),
        armor: Fixed::from_ratio(kind.armor * 2 + up.armor_halves, 2),
        magic_resist_bp: kind.magic_resist_pct * 100,
        status_resist_bp: 0,
        physical_amp_bp: rules::NOMINAL_BP,
        magic_amp_bp: rules::NOMINAL_BP,
        pure_amp_bp: rules::NOMINAL_BP,
        cooldown_rate_bp: rules::NOMINAL_BP,
        mana_cost_rate_bp: rules::NOMINAL_BP,
        evasion: Ratio::NEVER,
        pierce: Ratio::NEVER,
        pierce_damage: 0,
        move_speed: Fixed::from_int(kind.move_speed),
        turn_rate: kind.turn_rate,
        vision: Fixed::from_int(if night {
            kind.night_vision
        } else {
            kind.day_vision
        }),
        true_sight: Fixed::from_int(kind.true_sight),
        hides: kind.hides,
        flies: kind.flies,
        phased: false,
        invulnerable: kind.invulnerable,
        backdoor: false,
        glyph: false,
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

/// A speed taken to a percent of itself, worked out in `i64` and clamped
/// to what a [`Fixed`] holds.
fn scaled(speed: Fixed, pct: i32) -> Fixed {
    let raw = i64::from(speed.raw) * i64::from(pct) / 100;
    Fixed {
        raw: raw.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
    }
}
