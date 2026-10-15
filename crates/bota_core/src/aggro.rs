//! Whom a unit picks to attack when nobody told it: the class order, and
//! the order within a class.

/// The class order a unit ranks its targets by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriorityOrder {
    /// Heroes and ordinary units, then siege creeps, then buildings, then
    /// wards. What everything but a siege creep uses.
    Normal,
    /// Buildings, then siege creeps, then everything else, then wards.
    SiegeFirst,
}

/// The priority class of a target, before the class order is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetClass {
    /// Anything not in another class, a barracks included.
    Unit,
    /// A siege creep.
    Siege,
    /// A tower, an ancient or a fountain.
    Building,
    /// A ward.
    Ward,
}

/// Where a class sits in an order. Lower is taken first.
#[inline]
pub const fn class_rank(class: TargetClass, order: PriorityOrder) -> u8 {
    match order {
        PriorityOrder::Normal => match class {
            TargetClass::Unit => 0,
            TargetClass::Siege => 1,
            TargetClass::Building => 2,
            TargetClass::Ward => 3,
        },
        PriorityOrder::SiegeFirst => match class {
            TargetClass::Building => 0,
            TargetClass::Siege => 1,
            TargetClass::Unit => 2,
            TargetClass::Ward => 3,
        },
    }
}
