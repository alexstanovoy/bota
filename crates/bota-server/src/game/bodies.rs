//! Where every body stands, by bucket, for asking who is near a spot.

use bota_proto::{Fixed, Vec2};

use crate::game::{Entity, rules};

const BUCKETS: usize = rules::BUCKETS;

/// One body as the index has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Body {
    /// Which entity it is.
    pub entity: Entity,
    /// Where it stood when the index was laid.
    pub at: Vec2,
    /// Its collision size.
    pub radius: Fixed,
    /// How far it moved the tick before.
    pub delta: Vec2,
    /// Ticks running that it has not moved.
    pub still: u32,
    /// Whether it never moves at all.
    pub fixed: bool,
}

/// Every body of the world by the bucket it stands in, laid once a tick.
///
/// A bucket holds its bodies in entity order, and buckets are read in row
/// order, so what is asked for comes back the same on every run.
#[derive(Clone, Debug, Default)]
pub struct BodyIndex {
    /// Where each bucket's bodies begin in `items`, one more entry than
    /// there are buckets.
    start: Vec<u32>,
    /// Every body, grouped by bucket.
    items: Vec<Body>,
}

impl BodyIndex {
    /// An index with nobody in it.
    pub fn empty() -> BodyIndex {
        BodyIndex {
            start: vec![0; BUCKETS * BUCKETS + 1],
            items: Vec::new(),
        }
    }

    /// Lays the index out afresh from bodies given in entity order.
    pub fn lay(&mut self, bodies: &[Body]) {
        self.items.clear();
        self.items.extend_from_slice(bodies);
        self.items.sort_by_key(|body| bucket_of(body.at));
        self.start.clear();
        self.start.resize(BUCKETS * BUCKETS + 1, 0);
        for body in &self.items {
            self.start[bucket_of(body.at) + 1] += 1;
        }
        for b in 1..self.start.len() {
            self.start[b] += self.start[b - 1];
        }
    }

    /// Every body that stood within a reach of a spot when the index was
    /// laid, plus [`rules::BODY_INDEX_SLACK`] for what has moved since.
    pub fn near(&self, at: Vec2, reach: Fixed, mut each: impl FnMut(&Body)) {
        let pad = reach + rules::units(rules::BODY_INDEX_SLACK);
        let (bx0, by0) = bucket_coords(Vec2 {
            x: at.x - pad,
            y: at.y - pad,
        });
        let (bx1, by1) = bucket_coords(Vec2 {
            x: at.x + pad,
            y: at.y + pad,
        });
        for by in by0..=by1 {
            for bx in bx0..=bx1 {
                let b = by * BUCKETS + bx;
                let (from, to) = (self.start[b] as usize, self.start[b + 1] as usize);
                for body in &self.items[from..to] {
                    if body.at.within(at, pad + body.radius) {
                        each(body);
                    }
                }
            }
        }
    }
}

/// The bucket a position falls in.
fn bucket_of(at: Vec2) -> usize {
    let (bx, by) = bucket_coords(at);
    by * BUCKETS + bx
}

/// The bucket coordinates of a position, clamped to the map.
fn bucket_coords(at: Vec2) -> (usize, usize) {
    let last = BUCKETS as i32 - 1;
    let bx = (at.x.to_int() / rules::BUCKET_SIZE).clamp(0, last);
    let by = (at.y.to_int() / rules::BUCKET_SIZE).clamp(0, last);
    (bx as usize, by as usize)
}
