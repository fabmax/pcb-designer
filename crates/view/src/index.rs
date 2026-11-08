//! Spatial index over object bounding boxes (R*-tree).

use model::{ObjectId, Point, Rect};
use rstar::{AABB, RTree, RTreeObject};
use rustc_hash::FxHashMap;

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    id: ObjectId,
    env: AABB<[i64; 2]>,
}

impl RTreeObject for Entry {
    type Envelope = AABB<[i64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.env
    }
}

fn aabb(r: Rect) -> AABB<[i64; 2]> {
    AABB::from_corners([r.min.x, r.min.y], [r.max.x, r.max.y])
}

#[derive(Default)]
pub struct SpatialIndex {
    tree: RTree<Entry>,
    bounds: FxHashMap<ObjectId, Rect>,
}

impl SpatialIndex {
    pub fn clear(&mut self) {
        self.tree = RTree::new();
        self.bounds.clear();
    }

    /// Insert or update an object's bounds.
    pub fn set(&mut self, id: ObjectId, r: Rect) {
        if let Some(old) = self.bounds.insert(id, r) {
            if old == r {
                return;
            }
            self.tree.remove(&Entry { id, env: aabb(old) });
        }
        self.tree.insert(Entry { id, env: aabb(r) });
    }

    pub fn remove(&mut self, id: ObjectId) {
        if let Some(old) = self.bounds.remove(&id) {
            self.tree.remove(&Entry { id, env: aabb(old) });
        }
    }

    pub fn bounds(&self, id: ObjectId) -> Option<Rect> {
        self.bounds.get(&id).copied()
    }

    pub fn len(&self) -> usize {
        self.bounds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bounds.is_empty()
    }

    /// Objects whose bounds are within `tol` of `p`.
    pub fn near(&self, p: Point, tol: i64) -> Vec<ObjectId> {
        let env = AABB::from_corners([p.x - tol, p.y - tol], [p.x + tol, p.y + tol]);
        self.tree.locate_in_envelope_intersecting(env).map(|e| e.id).collect()
    }

    /// Objects fully inside `r` (`crossing = false`) or touching it (`crossing = true`).
    pub fn in_rect(&self, r: Rect, crossing: bool) -> Vec<ObjectId> {
        let env = aabb(r);
        if crossing {
            self.tree.locate_in_envelope_intersecting(env).map(|e| e.id).collect()
        } else {
            self.tree.locate_in_envelope(env).map(|e| e.id).collect()
        }
    }

    pub fn ids(&self) -> Vec<ObjectId> {
        self.bounds.keys().copied().collect()
    }

    /// Bounds of everything.
    pub fn total(&self) -> Option<Rect> {
        Rect::union_all(self.bounds.values().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::Id;

    #[test]
    fn set_move_remove() {
        let mut idx = SpatialIndex::default();
        let a = ObjectId::Wire(Id::new());
        idx.set(a, Rect::around(Point::new(0, 0), 10));
        assert_eq!(idx.near(Point::new(5, 5), 0), vec![a]);
        idx.set(a, Rect::around(Point::new(100, 0), 10));
        assert!(idx.near(Point::new(5, 5), 0).is_empty());
        assert_eq!(idx.in_rect(Rect::around(Point::new(100, 0), 20), false), vec![a]);
        assert!(idx.in_rect(Rect::around(Point::new(100, 0), 5), false).is_empty());
        assert_eq!(idx.in_rect(Rect::around(Point::new(100, 0), 5), true), vec![a]);
        idx.remove(a);
        assert!(idx.is_empty());
    }
}
