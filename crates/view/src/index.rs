//! Spatial index over object bounding boxes (R*-tree).

use std::hash::Hash;

use model::{ObjectId, Point, Rect};
use rstar::{AABB, RTree, RTreeObject};
use rustc_hash::FxHashMap;

/// Keys stored in the index.
pub trait Key: Copy + Eq + Hash + PartialEq + std::fmt::Debug {}
impl<T: Copy + Eq + Hash + std::fmt::Debug> Key for T {}

#[derive(Clone, Debug, PartialEq)]
struct Entry<K> {
    id: K,
    env: AABB<[i64; 2]>,
}

impl<K: Key> RTreeObject for Entry<K> {
    type Envelope = AABB<[i64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.env
    }
}

fn aabb(r: Rect) -> AABB<[i64; 2]> {
    AABB::from_corners([r.min.x, r.min.y], [r.max.x, r.max.y])
}

pub struct SpatialIndex<K: Key = ObjectId> {
    tree: RTree<Entry<K>>,
    bounds: FxHashMap<K, Rect>,
}

impl<K: Key> Default for SpatialIndex<K> {
    fn default() -> Self {
        SpatialIndex { tree: RTree::new(), bounds: FxHashMap::default() }
    }
}

impl<K: Key> SpatialIndex<K> {
    pub fn clear(&mut self) {
        self.tree = RTree::new();
        self.bounds.clear();
    }

    /// Insert or update an object's bounds.
    pub fn set(&mut self, id: K, r: Rect) {
        if let Some(old) = self.bounds.insert(id, r) {
            if old == r {
                return;
            }
            self.tree.remove(&Entry { id, env: aabb(old) });
        }
        self.tree.insert(Entry { id, env: aabb(r) });
    }

    pub fn remove(&mut self, id: K) {
        if let Some(old) = self.bounds.remove(&id) {
            self.tree.remove(&Entry { id, env: aabb(old) });
        }
    }

    pub fn bounds(&self, id: K) -> Option<Rect> {
        self.bounds.get(&id).copied()
    }

    pub fn len(&self) -> usize {
        self.bounds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bounds.is_empty()
    }

    /// Objects whose bounds are within `tol` of `p`.
    pub fn near(&self, p: Point, tol: i64) -> Vec<K> {
        let env = AABB::from_corners([p.x - tol, p.y - tol], [p.x + tol, p.y + tol]);
        self.tree.locate_in_envelope_intersecting(env).map(|e| e.id).collect()
    }

    /// Objects fully inside `r` (`crossing = false`) or touching it (`crossing = true`).
    pub fn in_rect(&self, r: Rect, crossing: bool) -> Vec<K> {
        let env = aabb(r);
        if crossing {
            self.tree.locate_in_envelope_intersecting(env).map(|e| e.id).collect()
        } else {
            self.tree.locate_in_envelope(env).map(|e| e.id).collect()
        }
    }

    pub fn ids(&self) -> Vec<K> {
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
        let mut idx: SpatialIndex = SpatialIndex::default();
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
