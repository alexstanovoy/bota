//! Typed reads over facts a seat was told. Unknown private fields stay unknown.
use bota_proto::{AbilityView, ItemView, TalentView, Team, UnitKind, UnitView};

/// Unspent ordinary skill points of an observed friendly hero. Talents spend their own points.
/// O(n²) for at most MAX_ABILITY_SLOTS slots: shared rank groups count once.
pub fn ability_points(unit: &UnitView, talent: Option<&TalentView>, viewer: Team) -> Option<u8> {
    if unit.team != viewer {
        return None;
    }
    if unit.kind != UnitKind::Hero {
        return Some(0);
    }
    let attribute = talent?.attribute_bonus;
    Some(unspent_points(unit.level, &unit.abilities, attribute))
}
/// O(n²), bounded by the ability-slot limit; the same learn-group registry as the simulation.
pub fn unspent_points(level: u8, abilities: &[AbilityView], attribute_bonus: u8) -> u8 {
    let mut spent = u16::from(attribute_bonus);
    for (at, ability) in abilities
        .iter()
        .take(bota_proto::limits::MAX_ABILITY_SLOTS)
        .enumerate()
    {
        if bota_core::ability_def(ability.id).is_some_and(|def| def.granted_at.is_some()) {
            continue;
        }
        let group = bota_core::learn_group(ability.id);
        if !abilities[..at]
            .iter()
            .any(|other| bota_core::learn_group(other.id) == group)
        {
            spent = spent.saturating_add(u16::from(ability.level));
        }
    }
    u16::from(level)
        .saturating_sub(spent)
        .min(u16::from(u8::MAX)) as u8
}
/// Sum of item catalogue costs, independent of remaining charges (installed game build 6975).
/// O(STASH_SLOTS), over the receiving player's storage only. None is not an empty stash.
pub fn stash_value(stash: Option<&[Option<ItemView>]>) -> Option<i32> {
    let mut value = 0i32;
    for item in stash?.iter().take(bota_core::rules::STASH_SLOTS).flatten() {
        let def = bota_core::item_def(item.id)?;
        value = value.saturating_add(def.cost);
    }
    Some(value)
}

/// Remaining public stock of the receiving team's shop. O(stock entries).
/// None means a missing limited-stock observation; -1 means an unlimited item.
pub fn item_stock(
    stock: &[bota_proto::StockView],
    team: Team,
    item: bota_proto::ItemId,
) -> Option<i32> {
    let def = bota_core::item_def(item)?;
    if def.stock.is_none() {
        return Some(-1);
    }
    stock
        .iter()
        .take(bota_proto::limits::MAX_VIEW_STOCK)
        .find(|row| row.team == team && row.item == item)
        .map(|row| i32::from(row.left))
}

impl crate::EntityTrack {
    /// A currently observed attack target. O(1); the folded aggro record names visible units only.
    pub fn attack_target(&self) -> Option<bota_proto::EntityId> {
        if !self.visible || self.unit.hp <= 0 {
            return None;
        }
        self.fight.and_then(|fight| fight.target)
    }
}
/// O(1) linear prediction from observed displacement. Before a second sample the observed
/// velocity is zero; a malformed or unrepresentable prediction is unknown.
#[expect(
    clippy::float_arithmetic,
    reason = "observed velocity is projected and rounded to fixed coordinates"
)]
pub fn project_observed_position(
    position: bota_proto::Vec2,
    velocity: Option<crate::VelocityEstimate>,
    delay: f64,
) -> Option<bota_proto::Vec2> {
    if !delay.is_finite() {
        return None;
    }
    let Some(velocity) = velocity else {
        return Some(position);
    };
    if velocity.elapsed_ticks == 0 || !delay.is_finite() {
        return None;
    }
    let ratio =
        delay * f64::from(bota_core::rules::TICKS_PER_SECOND) / f64::from(velocity.elapsed_ticks);
    let component = |origin: i32, delta: i32| {
        let origin = f64::from(origin) / 65536.0;
        let delta = f64::from(delta) / 65536.0;
        let raw = ((origin + delta * ratio) * 65536.0).round_ties_even();
        (raw.is_finite() && raw >= f64::from(i32::MIN) && raw <= f64::from(i32::MAX))
            .then_some(bota_proto::Fixed { raw: raw as i32 })
    };
    Some(bota_proto::Vec2 {
        x: component(position.x.raw, velocity.delta.x.raw)?,
        y: component(position.y.raw, velocity.delta.y.raw)?,
    })
}

/// A finite path expressed through the ordinary ordered movement queue. O(MAX_QUEUED_ORDERS).
/// The first waypoint interrupts; each following one waits for its predecessor to finish.
pub fn movement_path_orders(points: &[bota_proto::Vec2]) -> Option<Vec<crate::IssuedOrder>> {
    if points.len() > bota_core::rules::MAX_QUEUED_ORDERS {
        return None;
    }
    Some(
        points
            .iter()
            .enumerate()
            .map(|(at, &point)| {
                let order = crate::IssuedOrder::hero(bota_proto::Order::Move {
                    target: bota_proto::Target::Pos(point),
                });
                if at == 0 { order } else { order.queued() }
            })
            .collect(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NearbyCreeps {
    Lane,
    Neutral,
}
/// Visible living creeps sorted by squared planar distance then identity. O(n log n),
/// n <= MAX_VIEW_UNITS. Pass the current seat view, never remembered bodies.
pub fn nearby_creeps<'a>(
    actor: &UnitView,
    visible: impl Iterator<Item = &'a UnitView>,
    radius: bota_proto::Fixed,
    kind: NearbyCreeps,
    enemies: bool,
) -> Vec<bota_proto::EntityId> {
    let mut found: Vec<_> = visible
        .take(bota_proto::limits::MAX_VIEW_UNITS)
        .filter(|unit| {
            let definition = bota_core::kind_def(unit.kind);
            let eligible = match kind {
                NearbyCreeps::Lane => {
                    definition.lane_creep
                        && (unit.team != actor.team) == enemies
                        && unit.team != Team::Neutral
                }
                NearbyCreeps::Neutral => unit.team == Team::Neutral && definition.creep,
            };
            eligible && unit.id != actor.id && unit.hp > 0 && actor.pos.within(unit.pos, radius)
        })
        .map(|unit| (actor.pos.distance_squared(unit.pos), unit.id))
        .collect();
    found.sort_unstable();
    found.into_iter().map(|(_, id)| id).collect()
}

/// O(1). Received armor can exceed the core's signed intermediate domain; reject it
/// before evaluating physical mitigation. This is a client observation boundary, not a rule change.
pub fn observed_physical_damage(amount: i32, armor: bota_proto::Fixed) -> Option<i32> {
    let factor = 100 * i128::from(bota_proto::Fixed::ONE.raw)
        + 2 * i128::from(bota_core::rules::ARMOR_SCALE) * i128::from(armor.raw).abs();
    let product = i128::from(amount) * factor;
    if armor < bota_proto::Fixed::ZERO
        && !(i128::from(i64::MIN)..=i128::from(i64::MAX)).contains(&product)
    {
        return None;
    }
    Some(bota_core::mitigate(
        amount,
        bota_proto::DamageKind::Physical,
        armor,
        0,
    ))
}
/// O(1) incoming damage from the seat's received statistics, unknown if a physical
/// intermediate would exceed the core projection domain.
pub fn observed_incoming_damage(
    unit: &UnitView,
    amount: i32,
    kind: bota_proto::DamageKind,
) -> Option<i32> {
    match kind {
        bota_proto::DamageKind::Physical => observed_physical_damage(amount, unit.armor),
        _ => Some(bota_core::mitigate(
            amount,
            kind,
            unit.armor,
            bota_core::magic_resist_bp(unit.magic_resist),
        )),
    }
}
