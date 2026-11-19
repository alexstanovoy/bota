//! A team ability that holds charges and gets them back one at a time: Scan, and since Dota 7.42
//! the Glyph of Fortification. The same rules serve both (the game's server code runs one scheme
//! for the two): a spent charge starts the wait for the next unless one runs already, a charge
//! arrives when its wait ends, and no wait runs while every charge is held. Times are ticks of
//! the game clock.

/// How a team ability's charges come back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChargeRule {
    /// The most charges held.
    pub max: u8,
    /// Ticks one charge takes to come back.
    pub recharge: u32,
}

/// What a side holds of a charged team ability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Charges {
    /// Charges held, at most the rule's most.
    pub charges: u8,
    /// The game-clock tick the next charge arrives; none while every charge is held.
    pub next_charge: Option<u32>,
}

impl Charges {
    /// No charge held, the first arriving at game-clock tick `first`: what the pregame leaves.
    pub const fn first_at(first: u32) -> Self {
        Self {
            charges: 0,
            next_charge: Some(first),
        }
    }

    /// Lets the charges arrive that are due by `clock` under `rule`, one a wait, as many as have
    /// fallen due (a clock that jumped brings several at once). The wait for the next starts
    /// where the last one ended, and none runs once every charge is held.
    pub fn catch_up(&mut self, clock: u32, rule: ChargeRule) {
        assert!(self.charges <= rule.max, "more charges than the rule holds");
        assert!(
            self.next_charge.is_none() || self.charges < rule.max,
            "a wait runs only below the most charges"
        );
        // Each round adds a charge and the loop ends at the most: `rule.max` rounds at the
        // very most.
        while let Some(due) = self.next_charge.filter(|due| clock >= *due) {
            self.charges += 1;
            self.next_charge = (self.charges < rule.max).then(|| due + rule.recharge);
        }
    }

    /// Spends one charge at `clock` under `rule`, if there is one: the ledger must be caught up.
    /// A wait that is not running starts; one that is running goes on.
    #[must_use]
    pub fn spend(&mut self, clock: u32, rule: ChargeRule) -> bool {
        if self.charges == 0 {
            return false;
        }
        self.charges -= 1;
        self.next_charge.get_or_insert(clock + rule.recharge);
        true
    }

    /// Ticks until the next charge from `clock`; none while every charge is held.
    pub fn next_in(&self, clock: u32) -> Option<u32> {
        self.next_charge.map(|due| due.saturating_sub(clock))
    }
}
