//! Effects handed out for standing near something.

use bota_proto::{Team, UnitKind};

use crate::game::{
    Auras, Entity, EntityAllocator, Modifier, Modifiers, Reach, Spots, Stats, Table, Transform,
    Upgrades, is_structure, rules,
};

use super::modifiers::resisted_ticks;

/// What handing out effects reads and writes.
pub struct AuraCx<'a> {
    pub entities: &'a EntityAllocator,
    pub transform: &'a Table<Transform>,
    pub team: &'a Table<Team>,
    /// For the auras that reach only some kinds.
    pub kind: &'a Table<UnitKind>,
    pub auras: &'a Table<Auras>,
    /// Upgrade intervals each source spawned after, for the auras that grow.
    pub upgrades: &'a Table<Upgrades>,
    /// Status resistance, for the timed effects an aura may hand out.
    pub stats: &'a Table<Stats>,
    /// Where a handed-out effect lands.
    pub modifiers: &'a mut Table<Modifiers>,
    /// Reused buffer every entity is sorted into by where it stands.
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
        upgrades,
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
        let steps = upgrades.get(source).map_or(0, |Upgrades(steps)| *steps);
        for aura in handed {
            let handed_kind = aura.kind.grown(steps);
            let reach = rules::units(aura.radius);
            for entity in spots.around(from, i64::from(reach.raw).abs()) {
                if team.get(entity).copied() != Some(side) {
                    continue;
                }
                let wanted = match aura.reaches {
                    Reach::All => true,
                    Reach::Heroes => kind.get(entity).copied() == Some(UnitKind::Hero),
                    Reach::Units => kind.get(entity).copied().is_some_and(|kind| {
                        kind != UnitKind::Hero && kind != UnitKind::Ward && !is_structure(kind)
                    }),
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
                        kind: handed_kind,
                        source: Some(source),
                        ticks_left: Some(resisted_ticks(stats, entity, handed_kind, aura.ticks)),
                    });
                }
            }
        }
    }
}
