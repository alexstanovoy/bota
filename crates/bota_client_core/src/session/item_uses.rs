//! Retained application and refusal facts for the seat's two controlled bodies.
use crate::orders::{ControlledUnit, IssuedOrder};
use bota_proto::{EntityId, EventKind, ItemId, Order, RejectReason};

/// The latest received item application by one controlled body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemUseObservation {
    pub tick: u32,
    pub source: EntityId,
    pub item: ItemId,
}
/// A received refusal, correlated with the locally numbered request it rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemUseRefusal {
    pub sequence: u32,
    pub source: EntityId,
    pub issued_at: u32,
    pub order: Order,
    pub reason: RejectReason,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Request {
    sequence: u32,
    source: EntityId,
    issued_at: u32,
    order: Order,
}
/// Fixed storage. Unrelated events cannot evict facts; replacing a body clears its history.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ItemUses {
    bodies: [Option<EntityId>; 2],
    requests: [Option<Request>; 2],
    applied: [Option<ItemUseObservation>; 2],
    refused: [Option<ItemUseRefusal>; 2],
}
impl ItemUses {
    pub fn bind(&mut self, bodies: [Option<EntityId>; 2]) {
        for (index, body) in bodies.into_iter().enumerate() {
            if self.bodies[index] != body {
                self.bodies[index] = body;
                self.requests[index] = None;
                self.applied[index] = None;
                self.refused[index] = None;
            }
        }
    }
    pub fn record(&mut self, sequence: u32, seen: u32, issued: IssuedOrder, source: EntityId) {
        let index = ControlledUnit::of(issued.unit).index();
        if self.bodies[index] != Some(source) {
            return;
        }
        self.requests[index] = Some(Request {
            sequence,
            source,
            issued_at: seen,
            order: issued.order,
        });
        self.refused[index] = None;
    }
    pub fn reject(&mut self, sequence: u32, reason: RejectReason) {
        for (index, request) in self.requests.iter().enumerate() {
            if let Some(request) = request
                && request.sequence == sequence
                && self.refused[index].is_none()
            {
                self.refused[index] = Some(ItemUseRefusal {
                    sequence,
                    source: request.source,
                    issued_at: request.issued_at,
                    order: request.order,
                    reason,
                });
            }
        }
    }
    pub fn events(&mut self, tick: u32, events: &[EventKind]) {
        for event in events {
            let EventKind::ItemUsed {
                caster: Some(source),
                item,
                ..
            } = *event
            else {
                continue;
            };
            for (index, body) in self.bodies.iter().enumerate() {
                if *body == Some(source) {
                    self.applied[index] = Some(ItemUseObservation { tick, source, item });
                }
            }
        }
    }
    pub fn applications(&self) -> [Option<ItemUseObservation>; 2] {
        self.applied
    }
    pub fn refusals(&self) -> [Option<ItemUseRefusal>; 2] {
        self.refused
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bota_proto::{ItemSlot, Target};
    fn source(index: u32) -> EntityId {
        EntityId {
            idx: index,
            generation: 0,
        }
    }
    fn issued(unit: Option<EntityId>) -> IssuedOrder {
        IssuedOrder {
            unit,
            order: Order::Use {
                slot: ItemSlot(0),
                target: Target::None,
            },
            queue: false,
        }
    }
    #[test]
    fn only_the_correlated_request_can_be_refused_and_a_new_body_replaces_old_facts() {
        let mut uses = ItemUses::default();
        let hero = source(7);
        let courier = source(8);
        uses.bind([Some(hero), Some(courier)]);
        uses.record(10, 20, issued(None), hero);
        uses.record(11, 20, issued(Some(courier)), courier);
        uses.reject(9, RejectReason::Disabled);
        assert_eq!(uses.refusals(), [None, None]);
        uses.reject(10, RejectReason::OnCooldown);
        assert_eq!(uses.refusals()[0].unwrap().reason, RejectReason::OnCooldown);
        assert_eq!(uses.refusals()[1], None);
        uses.record(12, 21, issued(None), hero);
        uses.reject(10, RejectReason::EmptySlot);
        assert_eq!(uses.refusals(), [None, None]);
        uses.reject(11, RejectReason::Disabled);
        assert_eq!(uses.refusals()[1].unwrap().source, courier);
        let replacement = EntityId {
            generation: 1,
            ..courier
        };
        uses.bind([Some(hero), Some(replacement)]);
        uses.record(13, 21, issued(Some(replacement)), replacement);
        uses.reject(11, RejectReason::Disabled);
        assert_eq!(uses.refusals(), [None, None]);
    }
    #[test]
    fn received_applications_need_no_local_request_and_survive_unrelated_events() {
        let mut uses = ItemUses::default();
        let own = source(7);
        uses.bind([Some(own), None]);
        let event = |caster, item| EventKind::ItemUsed {
            caster: Some(caster),
            item,
            at: None,
            target: None,
        };
        uses.events(11, &[event(source(8), ItemId::MANGO)]);
        assert_eq!(uses.applications(), [None, None]);
        uses.events(11, &[event(own, ItemId::MANGO)]);
        let applied = uses.applications();
        assert_eq!(applied[0].unwrap().item, ItemId::MANGO);
        uses.events(
            12,
            &vec![
                EventKind::TreeGrew {
                    at: bota_proto::Vec2::ZERO
                };
                crate::MAX_RECENT_EVENTS + 1
            ],
        );
        assert_eq!(uses.applications(), applied);
        uses.record(1, 12, issued(None), own);
        uses.reject(1, RejectReason::Disabled);
        assert_eq!(uses.applications(), applied);
        uses.events(13, &[event(own, ItemId::CLARITY)]);
        assert_eq!(uses.applications()[0].unwrap().item, ItemId::CLARITY);
        uses.bind([
            Some(EntityId {
                generation: 1,
                ..own
            }),
            None,
        ]);
        assert_eq!(uses.applications(), [None, None]);
        assert_eq!(uses.refusals(), [None, None]);
    }
}
