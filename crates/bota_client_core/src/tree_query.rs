//! Standing-tree queries over public static geometry and this seat's received cuts.
use bota_core::TreeIndex;
use bota_proto::{Fixed, Vec2};
pub struct TreeQuery<'a> {
    candidates: &'a [(u32, Vec2)],
    at: Vec2,
    radius: Fixed,
    search: u64,
}
impl<'a> TreeQuery<'a> {
    /// Prepay the two index searches before constructing this query.
    pub fn search_work(tree_count: usize) -> u64 {
        2 * (1 + u64::from(tree_count.max(1).ilog2()))
    }

    pub fn new(index: &'a TreeIndex, at: Vec2, radius: Fixed) -> Self {
        let radius = radius.max(Fixed::ZERO);
        Self {
            candidates: index.band(at, radius),
            at,
            radius,
            search: Self::search_work(index.len()),
        }
    }
    /// Upper work bound: two index searches, a fixed 64-word id set, candidate
    /// distance/cut tests and id extraction. O(log trees + candidates*log cuts + 64).
    pub fn work(&self, felled_len: usize) -> u64 {
        self.search
            + bota_proto::limits::MAX_STATIC_TREES.div_ceil(64) as u64
            + self.candidates.len() as u64 * (4 + u64::from(felled_len.max(1).ilog2()))
    }
    /// `felled` is the sorted received id list validated by ClientSession.
    pub fn ids(&self, felled: &[u32]) -> Vec<u32> {
        let mut present = [0u64; bota_proto::limits::MAX_STATIC_TREES.div_ceil(64)];
        for &(id, point) in self.candidates {
            if felled.binary_search(&id).is_err() && self.at.within(point, self.radius) {
                present[id as usize / 64] |= 1u64 << (id % 64);
            }
        }
        let mut ids = Vec::new();
        for (block, mut word) in present.into_iter().enumerate() {
            while word != 0 {
                let bit = word.trailing_zeros();
                ids.push((block * 64) as u32 + bit);
                word &= word - 1;
            }
        }
        ids
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexed_circle_results_keep_original_order_duplicates_and_observed_cuts() {
        let index = TreeIndex::new(&[
            Vec2::from_ints(-10, 0),
            Vec2::from_ints(10, 0),
            Vec2::ZERO,
            Vec2::from_ints(5, 12),
            Vec2::from_ints(3, 4),
            Vec2::ZERO,
        ])
        .unwrap();
        let query = TreeQuery::new(&index, Vec2::ZERO, Fixed::from_int(5));
        assert_eq!(query.ids(&[]), vec![2, 4, 5]);
        assert_eq!(query.ids(&[2]), vec![4, 5]);
        assert_eq!(
            TreeQuery::new(&index, Vec2::ZERO, Fixed::ZERO).ids(&[]),
            vec![2, 5]
        );
    }
}
