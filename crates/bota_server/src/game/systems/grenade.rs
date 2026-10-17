//! The Blood Grenade: thrown at a spot, it bursts there, and the shops sell
//! it from a stock that refills over time.

use bota_core::{
    ITEMS, ModifierKind, clamp_to_map, facing_towards, item_def, kind_def, move_towards, per_tick,
    rules,
};
use bota_proto::{DamageKind, ItemId, Target, Team, UnitKind};

use crate::game::{Entity, Grenade, Modifier, ShopStock, World};

/// Every side's shop as the match starts: each item sold from a limited
/// stock at its starting count, by side, then in catalog order.
pub fn fresh_stock() -> Vec<ShopStock> {
    let mut stock = Vec::new();
    for team in [Team::Radiant, Team::Dire] {
        for (index, def) in ITEMS.iter().enumerate() {
            let Some(rule) = def.stock else {
                continue;
            };
            stock.push(ShopStock {
                team,
                item: ItemId(index as u16),
                left: rule.initial,
                restock_left: if rule.initial < rule.max {
                    rule.restock_ticks
                } else {
                    0
                },
            });
        }
    }
    stock
}

impl World {
    /// Whether one side's shop has an item to sell right now. One sold
    /// without limit always has.
    pub fn in_stock(&self, team: Team, item: ItemId) -> bool {
        self.shop_stock
            .iter()
            .find(|held| held.team == team && held.item == item)
            .is_none_or(|held| held.left > 0)
    }

    /// Takes one of an item off a side's shelf. One sold without limit is
    /// left alone. A shelf that was full starts refilling.
    pub fn take_from_stock(&mut self, team: Team, item: ItemId) {
        let Some(rule) = item_def(item).and_then(|def| def.stock) else {
            return;
        };
        let held = self
            .shop_stock
            .iter_mut()
            .find(|held| held.team == team && held.item == item)
            .expect("every stocked item has a shelf on every side");
        assert!(held.left > 0);
        if held.left == rule.max {
            assert_eq!(held.restock_left, 0);
            held.restock_left = rule.restock_ticks;
        }
        held.left -= 1;
    }

    /// Runs every shelf that is not full one tick closer to gaining one
    /// more.
    pub fn restock_shops(&mut self) {
        for held in self.shop_stock.iter_mut() {
            let rule = item_def(held.item)
                .and_then(|def| def.stock)
                .expect("a shelf holds a stocked item");
            if held.left >= rule.max {
                assert_eq!(held.restock_left, 0);
                continue;
            }
            assert!(held.restock_left > 0);
            held.restock_left -= 1;
            if held.restock_left == 0 {
                held.left += 1;
                if held.left < rule.max {
                    held.restock_left = rule.restock_ticks;
                }
            }
        }
    }

    /// Throws a Blood Grenade from its user at a spot within its reach.
    pub fn throw_grenade(&mut self, user: Entity, target: Target) -> bool {
        let Target::Pos(aim) = target else {
            return false;
        };
        let (Some(from), Some(side)) = (
            self.transform.get(user).map(|t| t.pos),
            self.team.get(user).copied(),
        ) else {
            return false;
        };
        let aim = clamp_to_map(aim);
        let reach = item_def(ItemId::BLOOD_GRENADE).map_or(0, |def| def.range);
        if !self.alive(user) || !from.within(aim, rules::units(reach)) {
            return false;
        }
        let damage_amp_bp = self.outgoing_damage_amp_bp(Some(user), DamageKind::Magical);
        let grenade = self.spawn_placed(from, facing_towards(from, aim), side);
        self.grenade.insert(
            grenade,
            Grenade {
                owner: user,
                aim,
                damage_amp_bp,
            },
        );
        true
    }

    /// Flies every Blood Grenade one tick on, bursting the ones that land.
    pub fn tick_grenades(&mut self) {
        let entities = self.take_entity_snapshot();
        for entity in entities.iter().copied() {
            let Some(shot) = self.grenade.get(entity).copied() else {
                continue;
            };
            let Some(at) = self.transform.get_mut(entity) else {
                self.grenade.remove(entity);
                self.discard(entity);
                continue;
            };
            let next = move_towards(
                at.pos,
                shot.aim,
                per_tick(rules::units(rules::GRENADE_SPEED)),
            );
            at.pos = next;
            if next == shot.aim {
                self.burst(entity, &shot);
                self.grenade.remove(entity);
                self.discard(entity);
            }
        }
        self.recycle_entity_snapshot(entities);
    }

    /// What a Blood Grenade does where it lands: every enemy hero and creep
    /// within its radius takes its debuff, then its impact.
    ///
    /// A debuff already on keeps the beat it started with and runs its full
    /// time again.
    fn burst(&mut self, grenade: Entity, shot: &Grenade) {
        let reach = rules::units(rules::GRENADE_RADIUS);
        let struck: Vec<Entity> = self
            .entities
            .iter()
            .filter(|other| {
                self.hostile(grenade, *other)
                    && self
                        .kind
                        .get(*other)
                        .copied()
                        .is_some_and(|kind| kind == UnitKind::Hero || kind_def(kind).creep)
                    && self
                        .transform
                        .get(*other)
                        .is_some_and(|t| t.pos.within(shot.aim, reach))
            })
            .collect();
        for other in struck {
            let since = self.grenade_beat(other).unwrap_or(self.tick);
            self.put_modifier(
                other,
                Modifier::new(
                    ModifierKind::BloodGrenade { since },
                    Some(shot.owner),
                    Some(rules::GRENADE_DEBUFF_TICKS),
                ),
            );
            self.push_hit_with_amp(
                Some(shot.owner),
                other,
                rules::GRENADE_IMPACT_DAMAGE,
                DamageKind::Magical,
                shot.damage_amp_bp,
            );
        }
    }

    /// The tick the Blood Grenade debuff on an entity started its beat on,
    /// while one is on.
    fn grenade_beat(&self, entity: Entity) -> Option<u32> {
        self.modifiers
            .get(entity)?
            .active()
            .find_map(|held| match held.kind {
                ModifierKind::BloodGrenade { since } => Some(since),
                _ => None,
            })
    }
}
