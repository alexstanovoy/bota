//! Points sorted along x, for asking which stand within a square about a
//! spot.

use bota_proto::Vec2;

/// Items at points, sorted by x, then y, then item once laid.
///
/// Answers only for what was pushed before it was last sorted, and says
/// nothing of distance: a caller asking about a circle checks each item
/// the square gives back.
pub struct Spots<T> {
    /// The raw x, the raw y and the item of every row.
    rows: Vec<(i32, i32, T)>,
}

impl<T> Default for Spots<T> {
    fn default() -> Self {
        Spots { rows: Vec::new() }
    }
}

impl<T: Copy + Ord> Spots<T> {
    /// Forgets every item, keeping the room.
    pub fn clear(&mut self) {
        self.rows.clear();
    }

    /// Adds an item at a point.
    pub fn push(&mut self, at: Vec2, item: T) {
        self.rows.push((at.x.raw, at.y.raw, item));
    }

    /// Sorts what was pushed, readying it to be asked.
    pub fn sort(&mut self) {
        self.rows.sort_unstable();
    }

    /// Every item within a square about a spot, `reach` raw units from its
    /// centre to each side, by x.
    pub fn around(&self, at: Vec2, reach: i64) -> impl Iterator<Item = T> + '_ {
        debug_assert!(self.rows.is_sorted(), "asked before it was sorted");
        let (x, y) = (i64::from(at.x.raw), i64::from(at.y.raw));
        let first = self
            .rows
            .partition_point(|&(row_x, _, _)| i64::from(row_x) < x - reach);
        self.rows[first..]
            .iter()
            .take_while(move |&&(row_x, _, _)| i64::from(row_x) <= x + reach)
            .filter(move |&&(_, row_y, _)| (i64::from(row_y) - y).abs() <= reach)
            .map(|&(_, _, item)| item)
    }
}
