//! What a death pays and what may be put out by its own side.

use bota_proto::{Fixed, UnitKind};

use crate::kind_def;

/// Whether one of a side's own units is worn down far enough for that side
/// to put it out: below its kind's
/// [`deny_below_pct`](crate::KindDef::deny_below_pct) of what it can hold.
#[inline]
pub fn deniable(kind: UnitKind, hp: Fixed, max_hp: Fixed) -> bool {
    let Some(share) = kind_def(kind).deny_below_pct else {
        return false;
    };
    i64::from(hp.raw) * 100 < i64::from(max_hp.raw) * i64::from(share)
}
