//! Working out what each entity fights by, from scratch, every tick.

use bota_core::{
    Buff, Carried, Growth, ModifierKind, NeutralAbility, Odds, Regen, Stat, Talent, UnitDef,
    agility_armor, agility_attack_speed, intelligence_magic_resist_bp, primary_damage, regen,
    rules, soul_damage, spell_buff,
};
use bota_proto::{Attributes, Fixed, ModifierSpec};

use crate::game::{
    AbilityBook, AppliedModifiers, Def, EntityAllocator, Health, Inventory, Level,
    MAX_COMBINED_MODIFIER_SCALE, Mana, Modifiers, StatBonus, Stats, Table, Upgrades, carried_bonus,
};

/// What working out stats reads and writes.
pub struct StatsCx<'a> {
    pub entities: &'a EntityAllocator,
    pub def: &'a Table<Def>,
    pub level: &'a Table<Level>,
    /// Upgrade intervals a creep spawned after.
    pub upgrades: &'a Table<Upgrades>,
    /// Health and damage a creep was raised with beyond its kind.
    pub bonus: &'a Table<StatBonus>,
    pub inventory: &'a Table<Inventory>,
    pub modifiers: &'a Table<Modifiers>,
    pub applied: &'a Table<AppliedModifiers>,
    /// What each entity has learned, for what its passives are worth.
    pub abilities: &'a Table<AbilityBook>,
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
        bonus,
        inventory,
        modifiers,
        applied,
        abilities,
        stats,
        health,
        mana,
        night,
    } = cx;
    for entity in entities.iter() {
        let Some(Def(kind)) = def.get(entity) else {
            continue;
        };
        let held = |of: fn(u32) -> ModifierKind| {
            modifiers.get(entity).map_or(0, |on_it| on_it.count_of(of))
        };
        let levels = level.get(entity).map_or(0, |l| i32::from(l.0.max(1) - 1));
        // A lane creep's upgrades come with its spawn; a neutral's are its
        // hidden neutral upgrade.
        let raised_by =
            upgrades.get(entity).map_or(0, |u| u.0) + held(ModifierKind::neutral_upgrade);
        let steps = i32::try_from(raised_by).unwrap_or(i32::MAX);
        let mut now = raised(kind, levels, steps, night);
        if let Some(bonus) = bonus.get(entity) {
            now.max_hp += Fixed::from_int(bonus.max_hp);
            now.damage += bonus.damage;
        }
        // A broken unit's own passives do nothing: what it has gathered
        // is worth nothing while it lasts.
        let broken = modifiers
            .get(entity)
            .is_some_and(|on_it| on_it.active().any(|held| held.kind == ModifierKind::Broken));
        // Applied changes are folded in first and additively. Items,
        // attributes and every multiplier below land on top of them. A
        // world whose paces are scaled keeps its limits in proportion.
        let pace_scale_bp = applied
            .get(entity)
            .map_or(rules::NOMINAL_BP, |on_it| fold_applied(&mut now, on_it));
        let ranged = kind.projectile_speed.is_some();
        let carried = inventory
            .get(entity)
            .filter(|_| !kind.porter)
            .map(|bag| carried_bonus(bag, ranged));
        if let Some(carried) = carried {
            add_carried(&mut now, &carried);
        }
        if let Some(book) = abilities.get(entity) {
            now.attributes +=
                Attributes::all(rules::ATTRIBUTE_BONUS_POINTS * i32::from(book.attribute_bonus));
        }
        if !broken {
            add_heap(&mut now, held(ModifierKind::flesh_heap));
        }
        from_attributes(&mut now);
        let base_armor =
            Fixed::from_ratio(kind.armor * 2 + kind.per_upgrade.armor_halves * steps, 2)
                + agility_armor(now.attributes);
        if let Some(carried) = &carried {
            add_carried_shares(&mut now, carried);
        }
        if !broken {
            now.damage += soul_damage(held(ModifierKind::souls), levels + 1);
        }
        // Every percent change of the pace is summed and taken of the flat
        // pace at once, as Dota works movement speed out.
        let mut pace_pct = 0;
        if let Some(on_it) = modifiers.get(entity) {
            let hero = kind.kind == bota_proto::UnitKind::Hero;
            add_modifiers(&mut now, &mut pace_pct, on_it, base_armor, hero);
        }
        if let Some(on_it) = modifiers.get(entity) {
            add_feast(&mut now, &mut pace_pct, on_it, abilities.get(entity));
        }
        now.move_speed = paced(now.move_speed, pace_pct, pace_scale_bp, kind.unbounded_pace);
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
/// with it when its caster has learned it; its pace goes to `pace_pct`.
fn add_feast(now: &mut Stats, pace_pct: &mut i32, on_it: &Modifiers, book: Option<&AbilityBook>) {
    let Some(level) = on_it.active().find_map(|held| match held.kind {
        ModifierKind::Feasting { level, .. } => Some(usize::from(level)),
        _ => None,
    }) else {
        return;
    };
    now.attack_speed += rules::FEAST_ATTACK_SPEED[level];
    if book.is_some_and(|book| book.has_talent(Talent::FeastAttackSpeed)) {
        now.attack_speed += rules::TALENT_FEAST_ATTACK_SPEED;
    }
    *pace_pct += rules::FEAST_MOVE_PCT[level];
}

/// Adds what the inventory proper carries as shares of what the attributes
/// already brought: of the maximum mana, and of the base pace and what
/// agility adds to it, nothing else.
fn add_carried_shares(now: &mut Stats, carried: &Carried) {
    if carried.mana_pct != 0 {
        now.max_mana += now.max_mana * Fixed::from_ratio(carried.mana_pct, 100);
    }
    if carried.base_attack_speed_pct != 0 {
        let base = rules::BASE_ATTACK_SPEED + agility_attack_speed(now.attributes);
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
    if now.projectile_speed.is_none() {
        now.attack_range += Fixed::from_int(carried.melee_range);
    }
}

/// What the flesh heap has kept, worth strength by the stack. Only a hero
/// whose innate it is keeps any.
fn add_heap(now: &mut Stats, kept: u32) {
    let kept = i32::try_from(kept).unwrap_or(i32::MAX);
    now.attributes.strength += rules::FLESH_HEAP_STRENGTH_PER_STACK * Fixed::from_int(kept);
}

/// Adds what every active modifier on a unit is worth to its stats, each
/// percent change of its pace to `pace_pct` (a slow taken off, a haste
/// added). What holds a unit still, what burns it and what it hands out are
/// read where they are acted on, not here. `base_armor` is what a share of
/// its base armor is taken of.
fn add_modifiers(
    now: &mut Stats,
    pace_pct: &mut i32,
    on_it: &Modifiers,
    base_armor: Fixed,
    hero: bool,
) {
    for held in on_it.active() {
        match held.kind {
            ModifierKind::Spell {
                ability: NeutralAbility::Purge,
                amount,
                ..
            } => {
                *pace_pct -= i32::from(amount).min(100);
            }
            ModifierKind::Spell { ability, level, .. } => {
                if let Some(buff) = spell_buff(ability) {
                    add_buff(now, pace_pct, &buff, level, base_armor, hero);
                }
            }
            ModifierKind::Slowed { pct } => *pace_pct -= pct.clamp(0, 100),
            ModifierKind::Rotting { level } => {
                *pace_pct -= rules::ROT_SLOW_PCT[usize::from(level)].clamp(0, 100);
            }
            ModifierKind::BloodGrenade { .. } => *pace_pct -= rules::GRENADE_SLOW_PCT,
            ModifierKind::Hastened { pct } => *pace_pct += pct.max(0),
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
            ModifierKind::Fortified => now.glyph = true,
            ModifierKind::Mending { .. }
            | ModifierKind::Clarity { .. }
            | ModifierKind::Fountain { .. }
            | ModifierKind::Stunned
            | ModifierKind::Feared
            | ModifierKind::Burning { .. }
            | ModifierKind::Dismembered { .. }
            | ModifierKind::MeatShield { .. }
            | ModifierKind::Shadowraze { .. }
            | ModifierKind::Rot { .. }
            | ModifierKind::Rooted
            | ModifierKind::Disarmed
            | ModifierKind::Broken
            | ModifierKind::BackdoorProtection
            | ModifierKind::TimedLife
            | ModifierKind::Illusion
            | ModifierKind::NeutralUpgrade { .. }
            | ModifierKind::FleshHeap { .. }
            | ModifierKind::Souls { .. }
            | ModifierKind::Feasting { .. }
            | ModifierKind::FeastLinger { .. } => {}
        }
    }
}

/// What a spell's buff is worth at a level: everything of it but what it
/// does to regeneration, which [`bota_core::regen`] reads.
fn add_buff(
    now: &mut Stats,
    pace_pct: &mut i32,
    buff: &Buff,
    level: u8,
    base_armor: Fixed,
    hero: bool,
) {
    let of = |stat| buff.get(stat, level);
    *pace_pct += of(Stat::MovePct);
    now.attack_speed += of(Stat::AttackSpeed);
    now.armor += Fixed::from_int(of(Stat::Armor));
    now.armor += base_armor * Fixed::from_ratio(of(Stat::BaseArmorPct), 100);
    let resist = of(if hero {
        Stat::MagicResistPct
    } else {
        Stat::MagicResistCreepPct
    });
    if resist != 0 {
        let let_through = (rules::NOMINAL_BP - now.magic_resist_bp) * (100 - resist) / 100;
        now.magic_resist_bp = rules::NOMINAL_BP - let_through;
    }
    now.damage += now.damage * of(Stat::DamagePct) / 100 + of(Stat::Damage);
    let outgoing = of(Stat::OutgoingPct) * rules::NOMINAL_BP / 100;
    now.physical_amp_bp += outgoing;
    now.magic_amp_bp += outgoing;
    now.pure_amp_bp += outgoing;
    now.lifesteal_pct += of(Stat::LifestealPct);
    if of(Stat::MaxHpPct) != 0 {
        now.max_hp = scaled(now.max_hp, 100 + of(Stat::MaxHpPct));
    }
    now.magic_taken_pct += of(Stat::MagicTakenPct);
    now.cooldown_rate_bp -= of(Stat::CooldownPct) * rules::NOMINAL_BP / 100;
    let accuracy = u8::try_from(of(Stat::AccuracyPct).clamp(0, 100)).unwrap_or(0);
    now.accuracy = now.accuracy.or(Odds::percent(accuracy));
}

/// Folds everything applied to a unit into a freshly raised stat block.
///
/// Every family is summed as a delta first and written once, so several
/// sources never compound. Resistances are added in their own units and
/// scales as deltas of the nominal.
fn fold_applied(now: &mut Stats, applied: &AppliedModifiers) -> i32 {
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
    // Held to its bounds once intelligence is in (`from_attributes`): what
    // is applied adds to the base resistance the attribute adds to.
    now.magic_resist_bp = now.magic_resist_bp.saturating_add(magic_resist);
    now.status_resist_bp = add(now.status_resist_bp, status_resist, 0, nominal - 1);
    now.physical_amp_bp = add(now.physical_amp_bp, physical, 0, most);
    now.magic_amp_bp = add(now.magic_amp_bp, magical, 0, most);
    now.pure_amp_bp = add(now.pure_amp_bp, pure, 0, most);
    now.cooldown_rate_bp = add(now.cooldown_rate_bp, cooldown, 1, most);
    now.mana_cost_rate_bp = add(now.mana_cost_rate_bp, mana_cost, 1, most);
    let pace_scale = combined_scale(move_speed);
    now.move_speed = scaled_bp(now.move_speed, pace_scale);
    now.max_hp = scaled_bp(now.max_hp, combined_scale(max_hp));
    now.max_mana = scaled_bp(now.max_mana, combined_scale(max_mana));
    pace_scale
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
    // Into the base resistance, which what multiplies resistance reads; the
    // base with what is applied to it is held between none and all.
    now.magic_resist_bp = now
        .magic_resist_bp
        .saturating_add(intelligence_magic_resist_bp(has))
        .clamp(0, rules::NOMINAL_BP);
    now.armor += agility_armor(has);
    now.attack_speed += agility_attack_speed(has);
    now.damage += primary_damage(has, now.primary);
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
        attack_speed: g.attack_speed * levels + per.attack_speed * steps,
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
        attack_speed: kind.attack_speed + up.attack_speed,
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
        evasion: Odds::NEVER,
        lifesteal_pct: 0,
        magic_taken_pct: 0,
        accuracy: Odds::NEVER,
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

/// Movement speed as Dota works it out: the flat pace (the unit's own, its
/// boots and what else adds whole units) times one plus every percent change
/// summed, held within [`rules::MIN_MOVE_SPEED`] and
/// [`rules::MAX_MOVE_SPEED`], both at the pace scale applied to the unit
/// (nominal unless its world's paces are scaled); a unit whose pace is
/// `unbounded` (Dota's courier) keeps only the lower limit. A unit with no
/// pace of its own (a building, a ward) stays still.
fn paced(flat: Fixed, pct: i32, scale_bp: i32, unbounded: bool) -> Fixed {
    if flat <= Fixed::ZERO {
        return flat;
    }
    let limit = |speed: i32| scaled_bp(Fixed::from_int(speed), scale_bp);
    let pace = scaled(flat, (100 + pct).max(0)).max(limit(rules::MIN_MOVE_SPEED));
    if unbounded {
        pace
    } else {
        pace.min(limit(rules::MAX_MOVE_SPEED))
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
