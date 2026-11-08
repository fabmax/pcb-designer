//! Ordered id → object collection, serialized as a JSON array.

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::id::Id;

pub trait HasId {
    fn id(&self) -> Id;
}

/// Insertion-ordered collection keyed by [`Id`]. Order is preserved across save/load and
/// across remove + re-insert at the old index, so undo restores files byte for byte.
#[derive(Clone, Debug, PartialEq)]
pub struct Store<T> {
    items: IndexMap<Id, T>,
}

impl<T> Default for Store<T> {
    fn default() -> Self {
        Self { items: IndexMap::new() }
    }
}

impl<T: HasId> Store<T> {
    /// Insert or replace. A replaced item keeps its position; a new item is appended.
    pub fn insert(&mut self, item: T) -> Option<T> {
        self.items.insert(item.id(), item)
    }

    /// Insert a new item at `index` (clamped), or replace in place if it already exists.
    pub fn insert_at(&mut self, index: usize, item: T) {
        let id = item.id();
        if let Some(slot) = self.items.get_mut(&id) {
            *slot = item;
        } else {
            let index = index.min(self.items.len());
            self.items.shift_insert(index, id, item);
        }
    }

    /// Replace an existing item in place (O(1)), returning its index and the old item.
    pub fn replace(&mut self, item: T) -> Option<(usize, T)> {
        let (index, _, slot) = self.items.get_full_mut(&item.id())?;
        Some((index, std::mem::replace(slot, item)))
    }

    /// Remove, returning the item and its former index.
    pub fn remove(&mut self, id: Id) -> Option<(usize, T)> {
        self.items.shift_remove_full(&id).map(|(i, _, t)| (i, t))
    }

    pub fn get(&self, id: Id) -> Option<&T> {
        self.items.get(&id)
    }

    pub fn get_mut(&mut self, id: Id) -> Option<&mut T> {
        self.items.get_mut(&id)
    }

    pub fn contains(&self, id: Id) -> bool {
        self.items.contains_key(&id)
    }

    pub fn index_of(&self, id: Id) -> Option<usize> {
        self.items.get_index_of(&id)
    }
}

impl<T> Store<T> {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.values()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.items.values_mut()
    }

    pub fn ids(&self) -> impl Iterator<Item = Id> + '_ {
        self.items.keys().copied()
    }
}

impl<T: HasId> FromIterator<T> for Store<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut s = Self::default();
        for t in iter {
            s.insert(t);
        }
        s
    }
}

impl<T: Serialize> Serialize for Store<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.items.values())
    }
}

impl<'de, T: Deserialize<'de> + HasId> Deserialize<'de> for Store<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let items = Vec::<T>::deserialize(d)?;
        let mut store = Store::default();
        for item in items {
            if store.insert(item).is_some() {
                return Err(serde::de::Error::custom("duplicate id in collection"));
            }
        }
        Ok(store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Item {
        id: Id,
        v: u32,
    }
    impl HasId for Item {
        fn id(&self) -> Id {
            self.id
        }
    }

    #[test]
    fn remove_and_reinsert_restores_order() {
        let items: Vec<Item> = (0..5).map(|i| Item { id: Id::from_u128(i + 1), v: i as u32 }).collect();
        let mut s: Store<Item> = items.iter().cloned().collect();
        let original = s.clone();
        let (idx, item) = s.remove(Id::from_u128(3)).unwrap();
        assert_eq!(idx, 2);
        s.insert_at(idx, item);
        assert_eq!(s, original);
    }

    #[test]
    fn serializes_as_array_and_rejects_duplicates() {
        let s: Store<Item> = [Item { id: Id::from_u128(1), v: 7 }].into_iter().collect();
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.starts_with('['));
        let back: Store<Item> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        let dup = format!("[{0},{0}]", serde_json::to_string(s.iter().next().unwrap()).unwrap());
        assert!(serde_json::from_str::<Store<Item>>(&dup).is_err());
    }
}
