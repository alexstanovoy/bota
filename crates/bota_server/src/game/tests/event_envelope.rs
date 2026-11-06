//! Internal representations preserve the public wire kind and existing dealer policy.

use crate::game::{Event, EventVisibility};
use bota_proto::{EntityId, EventKind, ItemId, Team, Vec2};

#[test]
fn item_fact_preserves_its_public_point_and_source_audience() {
    let caster = EntityId {
        idx: 7,
        generation: 1,
    };
    let at = Vec2::from_ints(1500, 1000);
    let fact = Event::ItemUsed {
        caster: Some(caster),
        item: ItemId::HEALING_SALVE,
        at: Some(at),
        target: None,
        visible_to: EventVisibility::Everyone,
        point_told_to: EventVisibility::Everyone,
        source_told_to: EventVisibility::OneTeam(Team::Radiant),
    };
    for (viewer, source) in [
        (None, Some(caster)),
        (Some(Team::Radiant), Some(caster)),
        (Some(Team::Dire), None),
    ] {
        assert_eq!(
            crate::game_loop::visible_events(std::slice::from_ref(&fact), viewer),
            vec![EventKind::ItemUsed {
                caster: source,
                item: ItemId::HEALING_SALVE,
                at: Some(at),
                target: None,
            }]
        );
    }
}

#[test]
fn ordinary_facts_keep_their_wire_kind_and_team_audience() {
    let kind = EventKind::LevelUp {
        unit: EntityId {
            idx: 7,
            generation: 1,
        },
        level: 2,
    };
    let fact = Event::Fact {
        kind: kind.clone(),
        visible_to: EventVisibility::OneTeam(Team::Radiant),
        gold_hidden_from: None,
        source_told_to: EventVisibility::Everyone,
    };
    for viewer in [None, Some(Team::Radiant)] {
        assert_eq!(
            crate::game_loop::visible_events(std::slice::from_ref(&fact), viewer),
            vec![kind.clone()]
        );
    }
    assert!(crate::game_loop::visible_events(&[fact], Some(Team::Dire)).is_empty());
}
