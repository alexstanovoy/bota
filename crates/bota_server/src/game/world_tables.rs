//! The one list of what a [`World`](crate::game::World) holds, and what of it the fingerprint covers.

/// Declares the world: its fields, [`World::new`] and [`World::hash`] come out of the one list, so a
/// part of the world cannot be left out of the fingerprint by forgetting it.
///
/// - `state`: whole-match values, eaten in the order written.
/// - `tables`: the component tables, eaten entity by entity, each row after the number of its table.
/// - `derived`: tables worked out afresh from the rest every tick; a copy of what is hashed.
/// - `scratch`: caches, buffers and what the map and the rules fix; none of it is state.
///
/// A field of `state` or `tables` is read by [`StateHash`](crate::engine::StateHash) and, in tests,
/// proven to move the fingerprint when it changes. `state` and `scratch` fields start from their
/// initialiser, which may name the fields before it.
macro_rules! world_tables {
    (
        $(#[$meta:meta])*
        pub struct World;

        state {
            $($(#[$sm:meta])* $svis:vis $sname:ident : $sty:ty = $sinit:expr;)*
        }

        tables {
            $($(#[$tm:meta])* $tvis:vis $tname:ident : $comp:ty;)*
        }

        derived {
            $($(#[$dm:meta])* $dvis:vis $dname:ident : $dcomp:ty;)*
        }

        scratch {
            $($(#[$xm:meta])* $xvis:vis $xname:ident : $xty:ty = $xinit:expr;)*
        }
    ) => {
        $(#[$meta])*
        pub struct World {
            $($(#[$sm])* $svis $sname: $sty,)*
            $($(#[$tm])* $tvis $tname: $crate::engine::Table<$comp>,)*
            $($(#[$dm])* $dvis $dname: $crate::engine::Table<$dcomp>,)*
            $($(#[$xm])* $xvis $xname: $xty,)*
        }

        impl World {
            /// A world at tick zero with nothing in it, on the Dota map.
            pub fn new() -> World {
                $(let $sname = $sinit;)*
                $(let $xname = $xinit;)*
                World {
                    $($sname,)*
                    $($tname: $crate::engine::Table::new(),)*
                    $($dname: $crate::engine::Table::new(),)*
                    $($xname,)*
                }
            }

            /// A fingerprint of everything a tick acts on: the match-wide state, then every live entity
            /// with a row for each table it has.
            pub fn hash(&self) -> u64 {
                /// The number each hashed table is told by.
                #[allow(non_camel_case_types, dead_code)]
                #[repr(u8)]
                enum Row {
                    $($tname),*
                }

                let mut fnv = $crate::engine::Fnv::new();
                $($crate::engine::StateHash::state_hash(&self.$sname, &mut fnv);)*
                for entity in self.entities.iter() {
                    fnv.entity(entity);
                    $(
                        if let Some(row) = self.$tname.get(entity) {
                            fnv.u8(Row::$tname as u8);
                            $crate::engine::StateHash::state_hash(row, &mut fnv);
                        }
                    )*
                }
                fnv.done()
            }
        }

        #[cfg(test)]
        mod fingerprint {
            use super::*;
            use $crate::engine::Specimen;

            #[test]
            fn every_part_of_the_world_moves_the_fingerprint() {
                $({
                    let mut world = World::new();
                    world.$sname = <$sty as Specimen>::low();
                    let low = world.hash();
                    world.$sname = <$sty as Specimen>::high();
                    assert_ne!(
                        low,
                        world.hash(),
                        "the fingerprint ignores {}",
                        stringify!($sname)
                    );
                })*
                $({
                    let mut world = World::new();
                    let entity = world.spawn();
                    let none = world.hash();
                    world.$tname.insert(entity, <$comp as Specimen>::low());
                    let low = world.hash();
                    world.$tname.insert(entity, <$comp as Specimen>::high());
                    let high = world.hash();
                    assert!(
                        none != low && low != high && none != high,
                        "the fingerprint ignores {}",
                        stringify!($tname)
                    );
                })*
            }
        }
    };
}
pub(crate) use world_tables;
