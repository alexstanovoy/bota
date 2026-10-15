//! Deterministic scalar and geometry types.
//!
//! Every non-integral quantity on the wire is fixed-point. Rendering code is
//! free to convert to `f32`.

use core::fmt;
use core::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

/// A fixed-point scalar in Q16.16 format.
///
/// The integral range is `-32768..=32767` and the resolution is `1/65536`.
/// The operators debug-assert on overflow and saturate in release.
#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fixed {
    /// Raw Q16.16 representation. Multiply by `2^-16` to get world units.
    pub raw: i32,
}

impl Fixed {
    /// Number of fractional bits in the raw representation.
    pub const FRAC_BITS: u32 = 16;

    /// One whole world unit.
    pub const ONE: Fixed = Fixed {
        raw: 1 << Self::FRAC_BITS,
    };

    pub const ZERO: Fixed = Fixed { raw: 0 };

    /// Smallest representable value.
    pub const MIN: Fixed = Fixed { raw: i32::MIN };

    /// Largest representable value.
    pub const MAX: Fixed = Fixed { raw: i32::MAX };

    /// Smallest step between two distinct values.
    pub const EPSILON: Fixed = Fixed { raw: 1 };

    /// A whole number of world units.
    pub const fn from_int(units: i32) -> Fixed {
        Fixed::from_i64((units as i64) << Self::FRAC_BITS)
    }

    /// A ratio of two integers, such as `Fixed::from_ratio(3, 10)` for `0.3`.
    ///
    /// Panics when `den` is zero.
    pub const fn from_ratio(num: i32, den: i32) -> Fixed {
        Fixed::from_i64(((num as i64) << Self::FRAC_BITS) / den as i64)
    }

    /// Whole world units, rounded towards negative infinity.
    pub const fn to_int(self) -> i32 {
        self.raw >> Self::FRAC_BITS
    }

    /// The fractional part, always in `0..1`.
    pub const fn frac(self) -> Fixed {
        Fixed {
            raw: self.raw & ((1 << Self::FRAC_BITS) - 1),
        }
    }

    /// Distance from zero.
    pub const fn abs(self) -> Fixed {
        Fixed::from_i64((self.raw as i64).abs())
    }

    /// The value squared, as a raw Q32.32 integer: the units of
    /// [`Vec2::distance_squared`].
    pub const fn squared_raw(self) -> i64 {
        (self.raw as i64) * (self.raw as i64)
    }

    /// The sum, or `None` when it does not fit.
    pub const fn checked_add(self, rhs: Fixed) -> Option<Fixed> {
        Fixed::checked_from_i64(self.raw as i64 + rhs.raw as i64)
    }

    /// The difference, or `None` when it does not fit.
    pub const fn checked_sub(self, rhs: Fixed) -> Option<Fixed> {
        Fixed::checked_from_i64(self.raw as i64 - rhs.raw as i64)
    }

    /// The product, or `None` when it does not fit.
    pub const fn checked_mul(self, rhs: Fixed) -> Option<Fixed> {
        let wide = (self.raw as i64) * (rhs.raw as i64);
        Fixed::checked_from_i64(wide >> Self::FRAC_BITS)
    }

    /// The quotient, or `None` when it does not fit or `rhs` is zero.
    pub const fn checked_div(self, rhs: Fixed) -> Option<Fixed> {
        if rhs.raw == 0 {
            return None;
        }
        let wide = ((self.raw as i64) << Self::FRAC_BITS) / rhs.raw as i64;
        Fixed::checked_from_i64(wide)
    }

    /// The value as an `f32`, for rendering.
    ///
    /// Nothing that feeds the simulation may go through this.
    #[expect(
        clippy::float_arithmetic,
        reason = "converts a wire value for rendering, never for simulation"
    )]
    pub fn to_f32(self) -> f32 {
        self.raw as f32 / (1u32 << Self::FRAC_BITS) as f32
    }

    /// Narrows a wide intermediate, saturating past the ends of the range.
    const fn from_i64(wide: i64) -> Fixed {
        debug_assert!(
            wide >= i32::MIN as i64 && wide <= i32::MAX as i64,
            "fixed-point overflow"
        );
        if wide < i32::MIN as i64 {
            Fixed::MIN
        } else if wide > i32::MAX as i64 {
            Fixed::MAX
        } else {
            Fixed { raw: wide as i32 }
        }
    }

    /// Narrows a wide intermediate, or `None` past the ends of the range.
    const fn checked_from_i64(wide: i64) -> Option<Fixed> {
        if wide < i32::MIN as i64 || wide > i32::MAX as i64 {
            None
        } else {
            Some(Fixed { raw: wide as i32 })
        }
    }
}

impl Add for Fixed {
    type Output = Fixed;
    fn add(self, rhs: Fixed) -> Fixed {
        Fixed::from_i64(self.raw as i64 + rhs.raw as i64)
    }
}

impl Sub for Fixed {
    type Output = Fixed;
    fn sub(self, rhs: Fixed) -> Fixed {
        Fixed::from_i64(self.raw as i64 - rhs.raw as i64)
    }
}

impl Neg for Fixed {
    type Output = Fixed;
    fn neg(self) -> Fixed {
        Fixed::from_i64(-(self.raw as i64))
    }
}

/// Rounds towards negative infinity.
impl Mul for Fixed {
    type Output = Fixed;
    fn mul(self, rhs: Fixed) -> Fixed {
        let wide = (self.raw as i64) * (rhs.raw as i64);
        Fixed::from_i64(wide >> Fixed::FRAC_BITS)
    }
}

/// Rounds towards negative infinity. Panics when `rhs` is zero.
impl Div for Fixed {
    type Output = Fixed;
    fn div(self, rhs: Fixed) -> Fixed {
        let wide = ((self.raw as i64) << Fixed::FRAC_BITS) / rhs.raw as i64;
        Fixed::from_i64(wide)
    }
}

impl AddAssign for Fixed {
    fn add_assign(&mut self, rhs: Fixed) {
        *self = *self + rhs;
    }
}

impl SubAssign for Fixed {
    fn sub_assign(&mut self, rhs: Fixed) {
        *self = *self - rhs;
    }
}

impl fmt::Display for Fixed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let wide = self.raw as i64;
        let sign = if wide < 0 { "-" } else { "" };
        let magnitude = wide.unsigned_abs();
        let whole = magnitude >> Fixed::FRAC_BITS;
        let frac = magnitude & ((1 << Fixed::FRAC_BITS) - 1);
        // Five digits resolve 1/65536 without inventing precision.
        let decimals = (frac * 100_000) >> Fixed::FRAC_BITS;
        write!(f, "{sign}{whole}.{decimals:05}")
    }
}

impl fmt::Debug for Fixed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

/// A rotation stored in binary radians, or "brads".
///
/// Used for unit facing and projectile heading.
#[derive(
    Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct Angle {
    /// Rotation where a full turn is 65536.
    ///
    /// Zero points along the positive X axis and the value grows
    /// counter-clockwise. Overflow wraps.
    pub brads: u16,
}

/// A position or offset on the map, in world units.
///
/// The map spans `0..=18432` on both axes, with the Radiant fountain in the
/// lower left corner and the Dire fountain in the upper right.
#[derive(
    Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct Vec2 {
    /// Horizontal coordinate, growing towards the Dire side of the map.
    pub x: Fixed,
    /// Vertical coordinate, growing towards the Dire side of the map.
    pub y: Fixed,
}

impl Vec2 {
    /// The origin, in the Radiant corner of the map.
    pub const ZERO: Vec2 = Vec2 {
        x: Fixed::ZERO,
        y: Fixed::ZERO,
    };

    /// A position from whole world units.
    pub const fn from_ints(x: i32, y: i32) -> Vec2 {
        Vec2 {
            x: Fixed::from_int(x),
            y: Fixed::from_int(y),
        }
    }

    /// The squared length, as a raw Q32.32 integer: the units of
    /// [`Fixed::squared_raw`].
    pub const fn length_squared(self) -> i64 {
        self.x.squared_raw() + self.y.squared_raw()
    }

    /// The squared distance to another position, in the units of
    /// [`length_squared`](Vec2::length_squared).
    pub const fn distance_squared(self, other: Vec2) -> i64 {
        let dx = (self.x.raw as i64) - (other.x.raw as i64);
        let dy = (self.y.raw as i64) - (other.y.raw as i64);
        dx * dx + dy * dy
    }

    /// Whether `other` is no further away than `radius`.
    pub const fn within(self, other: Vec2, radius: Fixed) -> bool {
        self.distance_squared(other) <= radius.squared_raw()
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, rhs: Vec2) -> Vec2 {
        Vec2 {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, rhs: Vec2) -> Vec2 {
        Vec2 {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
        }
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2 {
            x: -self.x,
            y: -self.y,
        }
    }
}

impl Mul<Fixed> for Vec2 {
    type Output = Vec2;
    fn mul(self, rhs: Fixed) -> Vec2 {
        Vec2 {
            x: self.x * rhs,
            y: self.y * rhs,
        }
    }
}

impl AddAssign for Vec2 {
    fn add_assign(&mut self, rhs: Vec2) {
        *self = *self + rhs;
    }
}

impl SubAssign for Vec2 {
    fn sub_assign(&mut self, rhs: Vec2) {
        *self = *self - rhs;
    }
}
