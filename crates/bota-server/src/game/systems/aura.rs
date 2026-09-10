//! Effects handed out for standing near something.

use bota_proto::{Team, UnitKind};

use crate::game::rules;
use crate::game::{Auras, EntityAllocator, Reach, Status, Statuses, Table, Transform};

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
    /// Where a handed-out effect lands.
    pub statuses: &'a mut Table<Statuses>,
}

/// Puts every aura on everyone standing in it.
///
/// Standing in one hands the effect out afresh every tick, which is both how
/// it is put on and how it is held; walking out of it leaves it to run out on
/// its own, with nothing having to take it off.
pub fn aura_system(cx: AuraCx<'_>) {
    let AuraCx {
        entities,
        transform,
        team,
        kind,
        auras,
        statuses,
    } = cx;
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
            for entity in entities.iter() {
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
                let put = Status {
                    kind: aura.kind,
                    ticks_left: aura.ticks,
                };
                match statuses.get_mut(entity) {
                    Some(on_it) => on_it.put(put),
                    None => {
                        let mut on_it = Statuses::default();
                        on_it.put(put);
                        statuses.insert(entity, on_it);
                    }
                }
            }
        }
    }
}
