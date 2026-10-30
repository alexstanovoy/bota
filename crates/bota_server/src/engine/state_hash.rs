//! What the world fingerprint is made of: a type that holds simulation state says field by field
//! what of it goes in.
//!
//! A type implements [`StateHash`] next to where it is defined, with [`state_hash!`] for a plain
//! struct (it names every field, so a field added later is a compile error until it is placed) or by
//! hand for an enum or a type with something to leave out. What an implementation eats must tell any
//! two values apart that the simulation tells apart; it eats a count before a collection and a flag
//! before an optional value, so that no two values share a byte stream.

use std::collections::VecDeque;

use bota_proto::{Angle, Fixed, Team, UnitKind, Vec2};

use crate::engine::{Entity, Fnv};

/// A value that goes into the world fingerprint.
pub trait StateHash {
    /// Eats everything of the value the simulation acts on.
    fn state_hash(&self, fnv: &mut Fnv);
}

/// Two values of a type that tell it apart in every field, for the test that proves each part of the
/// world moves the fingerprint.
#[cfg(test)]
pub trait Specimen: Sized {
    /// One value, the same every time.
    fn low() -> Self;
    /// Another, differing from [`Specimen::low`] in every field.
    fn high() -> Self;
}

/// Implements [`StateHash`] (and, in tests, [`Specimen`]) for a struct, naming every field it holds,
/// for a tuple struct of one field as `Name(0)`, or for an enum of unit variants as `Name as u8:`
/// followed by two of its variants.
macro_rules! state_hash {
    ($ty:ident { $($field:ident),+ $(,)? }) => {
        impl $crate::engine::StateHash for $ty {
            fn state_hash(&self, fnv: &mut $crate::engine::Fnv) {
                let $ty { $($field),+ } = self;
                $($crate::engine::StateHash::state_hash($field, fnv);)+
            }
        }

        #[cfg(test)]
        impl $crate::engine::Specimen for $ty {
            fn low() -> Self {
                $ty { $($field: $crate::engine::Specimen::low()),+ }
            }

            fn high() -> Self {
                $ty { $($field: $crate::engine::Specimen::high()),+ }
            }
        }
    };
    ($ty:ident (0)) => {
        impl $crate::engine::StateHash for $ty {
            fn state_hash(&self, fnv: &mut $crate::engine::Fnv) {
                let $ty(held) = self;
                $crate::engine::StateHash::state_hash(held, fnv);
            }
        }

        #[cfg(test)]
        impl $crate::engine::Specimen for $ty {
            fn low() -> Self {
                $ty($crate::engine::Specimen::low())
            }

            fn high() -> Self {
                $ty($crate::engine::Specimen::high())
            }
        }
    };
    ($ty:ident as u8 : $low:expr, $high:expr) => {
        impl $crate::engine::StateHash for $ty {
            fn state_hash(&self, fnv: &mut $crate::engine::Fnv) {
                fnv.u8(*self as u8);
            }
        }

        #[cfg(test)]
        impl $crate::engine::Specimen for $ty {
            fn low() -> Self {
                $low
            }

            fn high() -> Self {
                $high
            }
        }
    };
}
pub(crate) use state_hash;

macro_rules! number {
    ($($ty:ident),+) => {$(
        impl StateHash for $ty {
            fn state_hash(&self, fnv: &mut Fnv) {
                for byte in self.to_le_bytes() {
                    fnv.u8(byte);
                }
            }
        }

        #[cfg(test)]
        impl Specimen for $ty {
            fn low() -> Self {
                1
            }

            fn high() -> Self {
                2
            }
        }
    )+};
}
number!(u8, u16, u32, u64, i8, i32);

/// The items a match plays with are part of the match: how many ids it holds.
impl StateHash for bota_core::Catalogue {
    fn state_hash(&self, fnv: &mut Fnv) {
        self.items().state_hash(fnv);
    }
}

impl StateHash for usize {
    fn state_hash(&self, fnv: &mut Fnv) {
        fnv.u64(*self as u64);
    }
}

impl StateHash for bool {
    fn state_hash(&self, fnv: &mut Fnv) {
        fnv.some(*self);
    }
}

impl<T: StateHash> StateHash for Option<T> {
    fn state_hash(&self, fnv: &mut Fnv) {
        fnv.some(self.is_some());
        if let Some(held) = self {
            held.state_hash(fnv);
        }
    }
}

impl<T: StateHash> StateHash for [T] {
    fn state_hash(&self, fnv: &mut Fnv) {
        fnv.u64(self.len() as u64);
        for held in self {
            held.state_hash(fnv);
        }
    }
}

impl<T: StateHash> StateHash for Vec<T> {
    fn state_hash(&self, fnv: &mut Fnv) {
        self.as_slice().state_hash(fnv);
    }
}

impl<T: StateHash> StateHash for VecDeque<T> {
    fn state_hash(&self, fnv: &mut Fnv) {
        fnv.u64(self.len() as u64);
        for held in self {
            held.state_hash(fnv);
        }
    }
}

/// A fixed number of values: the number is part of the type.
impl<T: StateHash, const N: usize> StateHash for [T; N] {
    fn state_hash(&self, fnv: &mut Fnv) {
        for held in self {
            held.state_hash(fnv);
        }
    }
}

impl<A: StateHash, B: StateHash> StateHash for (A, B) {
    fn state_hash(&self, fnv: &mut Fnv) {
        self.0.state_hash(fnv);
        self.1.state_hash(fnv);
    }
}

impl StateHash for Entity {
    fn state_hash(&self, fnv: &mut Fnv) {
        fnv.entity(*self);
    }
}

impl StateHash for Fixed {
    fn state_hash(&self, fnv: &mut Fnv) {
        fnv.fixed(*self);
    }
}

impl StateHash for Vec2 {
    fn state_hash(&self, fnv: &mut Fnv) {
        fnv.vec2(*self);
    }
}

impl StateHash for Angle {
    fn state_hash(&self, fnv: &mut Fnv) {
        fnv.angle(*self);
    }
}

impl StateHash for Team {
    fn state_hash(&self, fnv: &mut Fnv) {
        fnv.team(*self);
    }
}

impl StateHash for UnitKind {
    fn state_hash(&self, fnv: &mut Fnv) {
        fnv.kind(*self);
    }
}

#[cfg(test)]
mod specimens {
    use super::*;

    impl Specimen for bool {
        fn low() -> Self {
            false
        }

        fn high() -> Self {
            true
        }
    }

    impl Specimen for bota_core::Catalogue {
        fn low() -> Self {
            bota_core::Catalogue::first(1).expect("a catalogue of one item")
        }

        fn high() -> Self {
            bota_core::Catalogue::first(2).expect("a catalogue of two items")
        }
    }

    impl Specimen for usize {
        fn low() -> Self {
            1
        }

        fn high() -> Self {
            2
        }
    }

    impl<T: Specimen> Specimen for Option<T> {
        fn low() -> Self {
            None
        }

        fn high() -> Self {
            Some(T::high())
        }
    }

    impl<T: Specimen> Specimen for Vec<T> {
        fn low() -> Self {
            Vec::new()
        }

        fn high() -> Self {
            vec![T::high()]
        }
    }

    impl<T: Specimen> Specimen for VecDeque<T> {
        fn low() -> Self {
            VecDeque::new()
        }

        fn high() -> Self {
            VecDeque::from([T::high()])
        }
    }

    impl<T: Specimen, const N: usize> Specimen for [T; N] {
        fn low() -> Self {
            std::array::from_fn(|_| T::low())
        }

        fn high() -> Self {
            std::array::from_fn(|_| T::high())
        }
    }

    impl<A: Specimen, B: Specimen> Specimen for (A, B) {
        fn low() -> Self {
            (A::low(), B::low())
        }

        fn high() -> Self {
            (A::high(), B::high())
        }
    }

    impl Specimen for Entity {
        fn low() -> Self {
            crate::engine::EntityAllocator::new().alloc()
        }

        fn high() -> Self {
            let mut entities = crate::engine::EntityAllocator::new();
            entities.alloc();
            entities.alloc()
        }
    }

    impl Specimen for Fixed {
        fn low() -> Self {
            Fixed::ONE
        }

        fn high() -> Self {
            Fixed::from_int(2)
        }
    }

    impl Specimen for Vec2 {
        fn low() -> Self {
            Vec2::from_ints(1, 2)
        }

        fn high() -> Self {
            Vec2::from_ints(3, 4)
        }
    }

    impl Specimen for Angle {
        fn low() -> Self {
            Angle { brads: 1 }
        }

        fn high() -> Self {
            Angle { brads: 2 }
        }
    }

    impl Specimen for Team {
        fn low() -> Self {
            Team::Radiant
        }

        fn high() -> Self {
            Team::Dire
        }
    }

    impl Specimen for UnitKind {
        fn low() -> Self {
            UnitKind::Hero
        }

        fn high() -> Self {
            UnitKind::CreepMelee
        }
    }
}
