//! Effects handed out for standing near something.

use bota_proto::{Team, UnitKind};

use crate::game::rules;
use crate::game::{
    Auras, Entity, EntityAllocator, Modifier, Modifiers, Reach, Spots, Stats, Table, Transform,
};

use super::modifiers::resisted_ticks;

/// What handing out effects reads and writes.
pub struct AuraCx<'a> {
    /// Which entities exist.
    pub entities: &'a EntityAllocator,
    /// Where each entity stands.
    pub transform: &'a Table<Transform>,
    /// Which side each entity is on.
    pub team: &'a Table<Team>,
    /// What kind of thing each entity is, for the auras that reach only some
    /// of them.
    pub kind: &'a Table<UnitKind>,
    /// What each entity hands out.
    pub auras: &'a Table<Auras>,
    /// Status resistance, for the timed effects an aura may hand out.
    pub stats: &'a Table<Stats>,
    /// Where a handed-out effect lands.
    pub modifiers: &'a mut Table<Modifiers>,
    /// Reused room every entity is sorted by where it stands in.
    pub spots: &'a mut Spots<Entity>,
}

/// Puts every aura on every ally of its source standing in it, the source
/// included.
///
/// Standing in one puts the effect on afresh every tick; walking out leaves it
/// to run out on its own.
pub fn aura_system(cx: AuraCx<'_>) {
    let AuraCx {
        entities,
        transform,
        team,
        kind,
        auras,
        stats,
        modifiers,
        spots,
    } = cx;
    spots.clear();
    for entity in entities.iter() {
        if let Some(at) = transform.get(entity) {
            spots.push(at.pos, entity);
        }
    }
    spots.sort();
    for source in entities.iter() {
        let Some(Auras(handed)) = auras.get(source).copied() else {
            continue;
        };
        let (Some(from), Some(side)) = (
            transform.get(source).map(|t| t.pos),
            team.get(source).copied(),
        ) else {
            continue;
        };
        for aura in handed {
            let reach = rules::units(aura.radius);
            for entity in spots.around(from, i64::from(reach.raw).abs()) {
                if team.get(entity).copied() != Some(side) {
                    continue;
                }
                let wanted = match aura.reaches {
                    Reach::All => true,
                    Reach::Heroes => kind.get(entity).copied() == Some(UnitKind::Hero),
                };
                if !wanted {
                    continue;
                }
                if !transform
                    .get(entity)
                    .is_some_and(|at| at.pos.within(from, reach))
                {
                    continue;
                }
                if let Some(on_it) = modifiers.get_mut(entity) {
                    on_it.put(Modifier {
                        kind: aura.kind,
                        source: Some(source),
                        ticks_left: Some(resisted_ticks(stats, entity, aura.kind, aura.ticks)),
                    });
                }
            }
        }
    }
}
