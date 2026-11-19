//! Immutable range index for public map trees; ids remain their original input indices.
use bota_proto::{Fixed, Vec2};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeIndex {
    rows: Vec<(u32, Vec2)>,
}
impl TreeIndex {
    /// O(n log n), once per map; rejects more than the protocol's static-tree limit.
    pub fn new(points: &[Vec2]) -> Option<Self> {
        if points.len() > bota_proto::limits::MAX_STATIC_TREES {
            return None;
        }
        let mut rows: Vec<_> = points
            .iter()
            .enumerate()
            .map(|(id, &point)| (id as u32, point))
            .collect();
        rows.sort_unstable_by_key(|&(id, point)| (point.x.raw, id));
        Some(Self { rows })
    }
    /// O(log n), an inclusive x band containing every possible circle result.
    pub fn band(&self, at: Vec2, radius: Fixed) -> &[(u32, Vec2)] {
        let low = i64::from(at.x.raw) - i64::from(radius.raw.max(0));
        let high = i64::from(at.x.raw) + i64::from(radius.raw.max(0));
        let first = self
            .rows
            .partition_point(|&(_, point)| i64::from(point.x.raw) < low);
        let end = self
            .rows
            .partition_point(|&(_, point)| i64::from(point.x.raw) <= high);
        &self.rows[first..end]
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bands_preserve_original_ids_and_include_both_boundaries() {
        let index =
            TreeIndex::new(&[Vec2::from_ints(10, 0), Vec2::from_ints(-10, 0), Vec2::ZERO]).unwrap();
        assert_eq!(
            index
                .band(Vec2::ZERO, Fixed::from_int(10))
                .iter()
                .map(|row| row.0)
                .collect::<Vec<_>>(),
            vec![1, 2, 0]
        );
        assert_eq!(index.band(Vec2::ZERO, Fixed::ZERO), &[(2, Vec2::ZERO)]);
        assert!(
            TreeIndex::new(&vec![Vec2::ZERO; bota_proto::limits::MAX_STATIC_TREES + 1]).is_none()
        );
    }
}
