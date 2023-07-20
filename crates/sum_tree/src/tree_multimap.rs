use std::{
    cmp::Ordering,
    collections::BTreeMap,
    fmt::Debug,
    iter,
    ops::{Bound, RangeBounds},
};

use crate::{Bias, Dimension, Edit, Item, KeyedItem, SeekTarget, SumTree, Summary};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeMultimap<K, V: KeyedItem>(SumTree<MapEntry<K, V>>)
where
    K: Clone + Debug + Default + Ord,
    V: Clone + Debug;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapEntry<K, V> {
    key: K,
    value: V,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct MapKey<K>(K);

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct MultimapKey<K, V>(K, V);

#[derive(Clone, Debug, Default)]
pub struct MapKeyRef<'a, K>(Option<&'a K>);

impl<K, V> TreeMultimap<K, V>
where
    K: Clone + Debug + Default + Ord,
    V: Clone + Debug + KeyedItem,
{
    pub fn from_ordered_entries(entries: impl IntoIterator<Item = (K, V)>) -> Self {
        let tree = SumTree::from_iter(
            entries
                .into_iter()
                .map(|(key, value)| MapEntry { key, value }),
            &(),
        );
        Self(tree)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get<'a>(&'a self, key: &'a K) -> impl Iterator<Item = &'a V> {
        let mut cursor = self.0.cursor::<MapKeyRef<'_, K>>();
        cursor.seek(&MapKeyRef(Some(key)), Bias::Left, &());

        iter::from_fn(move || {
            if let Some(item) = cursor.item() {
                cursor.next(&());
                if *key == item.key {
                    Some(&item.value)
                } else {
                    None
                }
            } else {
                None
            }
        })
    }

    pub fn contains_key<'a>(&self, key: &'a K) -> bool {
        self.get(key).next().is_some()
    }

    pub fn insert(&mut self, key: K, value: V) {
        self.0.insert_or_replace(MapEntry { key, value }, &());
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let mut removed = None;
        let mut cursor = self.0.cursor::<MapKeyRef<'_, K>>();
        let key = MapKeyRef(Some(key));
        let mut new_tree = cursor.slice(&key, Bias::Left, &());
        if key.cmp(&cursor.end(&()), &()) == Ordering::Equal {
            removed = Some(cursor.item().unwrap().value.clone());
            cursor.next(&());
        }
        new_tree.append(cursor.suffix(&()), &());
        drop(cursor);
        self.0 = new_tree;
        removed
    }

    pub fn remove_range(&mut self, start: &impl MapSeekTarget<K>, end: &impl MapSeekTarget<K>) {
        let start = MapSeekTargetAdaptor(start);
        let end = MapSeekTargetAdaptor(end);
        let mut cursor = self.0.cursor::<MapKeyRef<'_, K>>();
        let mut new_tree = cursor.slice(&start, Bias::Left, &());
        cursor.seek(&end, Bias::Left, &());
        new_tree.append(cursor.suffix(&()), &());
        drop(cursor);
        self.0 = new_tree;
    }

    /// Returns the key-value pair with the greatest key less than or equal to the given key.
    pub fn closest(&self, key: &K) -> Option<(&K, &V)> {
        let mut cursor = self.0.cursor::<MapKeyRef<'_, K>>();
        let key = MapKeyRef(Some(key));
        cursor.seek(&key, Bias::Right, &());
        cursor.prev(&());
        cursor.item().map(|item| (&item.key, &item.value))
    }

    pub fn range<'a, R>(&self, range: R) -> impl Iterator<Item = (&K, &V)>
    where
        K: 'a,
        R: RangeBounds<&'a K>,
    {
        let mut cursor = self.0.cursor::<MapKeyRef<'_, K>>();
        match range.start_bound() {
            Bound::Included(start) => {
                let start = MapKeyRef(Some(*start));
                cursor.seek(&start, Bias::Left, &());
            }
            Bound::Excluded(start) => {
                let start = MapKeyRef(Some(*start));
                cursor.seek(&start, Bias::Right, &());
            }
            Bound::Unbounded => cursor.next(&()),
        }
        cursor
            .map(|entry| (&entry.key, &entry.value))
            .take_while(move |(key, _)| match range.end_bound() {
                Bound::Included(end) => key <= end,
                Bound::Excluded(end) => key < end,
                Bound::Unbounded => true,
            })
    }

    pub fn iter_from<'a>(&'a self, from: &'a K) -> impl Iterator<Item = (&K, &V)> + '_ {
        let mut cursor = self.0.cursor::<MapKeyRef<'_, K>>();
        let from_key = MapKeyRef(Some(from));
        cursor.seek(&from_key, Bias::Left, &());

        cursor
            .into_iter()
            .map(|map_entry| (&map_entry.key, &map_entry.value))
    }

    pub fn update<F, T>(&mut self, key: &K, f: F) -> Option<T>
    where
        F: FnOnce(&mut V) -> T,
    {
        let mut cursor = self.0.cursor::<MapKeyRef<'_, K>>();
        let key = MapKeyRef(Some(key));
        let mut new_tree = cursor.slice(&key, Bias::Left, &());
        let mut result = None;
        if key.cmp(&cursor.end(&()), &()) == Ordering::Equal {
            let mut updated = cursor.item().unwrap().clone();
            result = Some(f(&mut updated.value));
            new_tree.push(updated, &());
            cursor.next(&());
        }
        new_tree.append(cursor.suffix(&()), &());
        drop(cursor);
        self.0 = new_tree;
        result
    }

    pub fn retain<F: FnMut(&K, &V) -> bool>(&mut self, mut predicate: F) {
        let mut new_map = SumTree::<MapEntry<K, V>>::default();

        let mut cursor = self.0.cursor::<MapKeyRef<'_, K>>();
        cursor.next(&());
        while let Some(item) = cursor.item() {
            if predicate(&item.key, &item.value) {
                new_map.push(item.clone(), &());
            }
            cursor.next(&());
        }
        drop(cursor);

        self.0 = new_map;
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> + '_ {
        self.0.iter().map(|entry| (&entry.key, &entry.value))
    }

    pub fn values(&self) -> impl Iterator<Item = &V> + '_ {
        self.0.iter().map(|entry| &entry.value)
    }

    pub fn insert_tree(&mut self, other: TreeMultimap<K, V>) {
        let edits = other
            .iter()
            .map(|(key, value)| {
                Edit::Insert(MapEntry {
                    key: key.to_owned(),
                    value: value.to_owned(),
                })
            })
            .collect();

        self.0.edit(edits, &());
    }
}

impl<K, V> Into<BTreeMap<K, V>> for &TreeMultimap<K, V>
where
    K: Clone + Debug + Default + Ord,
    V: Clone + Debug + KeyedItem,
{
    fn into(self) -> BTreeMap<K, V> {
        self.iter()
            .map(|(replica_id, count)| (replica_id.clone(), count.clone()))
            .collect()
    }
}

impl<K, V> From<&BTreeMap<K, V>> for TreeMultimap<K, V>
where
    K: Clone + Debug + Default + Ord,
    V: Clone + Debug + KeyedItem,
{
    fn from(value: &BTreeMap<K, V>) -> Self {
        TreeMultimap::from_ordered_entries(value.into_iter().map(|(k, v)| (k.clone(), v.clone())))
    }
}

#[derive(Debug)]
struct MapSeekTargetAdaptor<'a, T>(&'a T);

impl<'a, K: Debug + Clone + Default + Ord, T: MapSeekTarget<K>>
    SeekTarget<'a, MapKey<K>, MapKeyRef<'a, K>> for MapSeekTargetAdaptor<'_, T>
{
    fn cmp(&self, cursor_location: &MapKeyRef<K>, _: &()) -> Ordering {
        if let Some(key) = &cursor_location.0 {
            MapSeekTarget::cmp_cursor(self.0, key)
        } else {
            Ordering::Greater
        }
    }
}

pub trait MapSeekTarget<K>: Debug {
    fn cmp_cursor(&self, cursor_location: &K) -> Ordering;
}

impl<K: Debug + Ord> MapSeekTarget<K> for K {
    fn cmp_cursor(&self, cursor_location: &K) -> Ordering {
        self.cmp(cursor_location)
    }
}

impl<K, V> Default for TreeMultimap<K, V>
where
    K: Clone + Debug + Default + Ord,
    V: Clone + Debug + KeyedItem,
{
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<K, V> Item for MapEntry<K, V>
where
    K: Clone + Debug + Default + Ord,
    V: Clone + KeyedItem,
{
    type Summary = MultimapKey<K, V::Key>;

    fn summary(&self) -> Self::Summary {
        self.key()
    }
}

impl<K, V: KeyedItem> KeyedItem for MapEntry<K, V>
where
    K: Clone + Debug + Default + Ord,
    V: Clone,
{
    type Key = MultimapKey<K, V::Key>;

    fn key(&self) -> Self::Key {
        MultimapKey(self.key.clone(), self.value.key())
    }
}

impl<K> Summary for MapKey<K>
where
    K: Clone + Debug + Default,
{
    type Context = ();

    fn add_summary(&mut self, summary: &Self, _: &()) {
        *self = summary.clone()
    }
}

impl<'a, K> Dimension<'a, MapKey<K>> for MapKeyRef<'a, K>
where
    K: Clone + Debug + Default + Ord,
{
    fn add_summary(&mut self, summary: &'a MapKey<K>, _: &()) {
        self.0 = Some(&summary.0)
    }
}

impl<'a, K> SeekTarget<'a, MapKey<K>, MapKeyRef<'a, K>> for MapKeyRef<'_, K>
where
    K: Clone + Debug + Default + Ord,
{
    fn cmp(&self, cursor_location: &MapKeyRef<K>, _: &()) -> Ordering {
        Ord::cmp(&self.0, &cursor_location.0)
    }
}

impl<K, V> Summary for MultimapKey<K, V>
where
    K: Clone + Debug + Default,
    V: Clone + Debug + Default,
{
    type Context = ();

    fn add_summary(&mut self, summary: &Self, _: &()) {
        *self = summary.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl KeyedItem for &str {
        type Key = Self;

        fn key(&self) -> Self::Key {
            self
        }
    }

    #[test]
    fn test_basic() {
        let mut map = TreeMultimap::default();
        assert_eq!(map.iter().collect::<Vec<_>>(), vec![]);

        map.insert(3, "c");
        assert_eq!(map.get(&3).next(), Some(&"c"));
        assert_eq!(map.iter().collect::<Vec<_>>(), vec![(&3, &"c")]);

        map.insert(1, "a");
        assert_eq!(map.get(&1).next(), Some(&"a"));
        assert_eq!(map.iter().collect::<Vec<_>>(), vec![(&1, &"a"), (&3, &"c")]);

        map.insert(2, "b");
        assert_eq!(map.get(&2).next(), Some(&"b"));
        assert_eq!(map.get(&1).next(), Some(&"a"));
        assert_eq!(map.get(&3).next(), Some(&"c"));
        assert_eq!(
            map.iter().collect::<Vec<_>>(),
            vec![(&1, &"a"), (&2, &"b"), (&3, &"c")]
        );

        assert_eq!(map.closest(&0), None);
        assert_eq!(map.closest(&1), Some((&1, &"a")));
        assert_eq!(map.closest(&10), Some((&3, &"c")));

        map.remove(&2);
        assert_eq!(map.get(&2).next(), None);
        assert_eq!(map.iter().collect::<Vec<_>>(), vec![(&1, &"a"), (&3, &"c")]);

        assert_eq!(map.closest(&2), Some((&1, &"a")));

        map.remove(&3);
        assert_eq!(map.get(&3).next(), None);
        assert_eq!(map.iter().collect::<Vec<_>>(), vec![(&1, &"a")]);

        map.remove(&1);
        assert_eq!(map.get(&1).next(), None);
        assert_eq!(map.iter().collect::<Vec<_>>(), vec![]);

        map.insert(4, "d");
        map.insert(5, "e");
        map.insert(6, "f");
        map.retain(|key, _| *key % 2 == 0);
        assert_eq!(map.iter().collect::<Vec<_>>(), vec![(&4, &"d"), (&6, &"f")]);
    }

    #[test]
    fn test_iter_from() {
        let mut map = TreeMultimap::default();

        map.insert("a", 1);
        map.insert("b", 2);
        map.insert("baa", 3);
        map.insert("baaab", 4);
        map.insert("c", 5);

        let result = map
            .iter_from(&"ba")
            .take_while(|(key, _)| key.starts_with(&"ba"))
            .collect::<Vec<_>>();

        assert_eq!(result.len(), 2);
        assert!(result.iter().find(|(k, _)| k == &&"baa").is_some());
        assert!(result.iter().find(|(k, _)| k == &&"baaab").is_some());

        let result = map
            .iter_from(&"c")
            .take_while(|(key, _)| key.starts_with(&"c"))
            .collect::<Vec<_>>();

        assert_eq!(result.len(), 1);
        assert!(result.iter().find(|(k, _)| k == &&"c").is_some());
    }

    #[test]
    fn test_insert_tree() {
        let mut map = TreeMultimap::default();
        map.insert("a", 1);
        map.insert("b", 2);
        map.insert("c", 3);

        let mut other = TreeMultimap::default();
        other.insert("a", 2);
        other.insert("b", 2);
        other.insert("d", 4);

        map.insert_tree(other);

        assert_eq!(map.iter().count(), 4);
        assert_eq!(map.get(&"a").next(), Some(&2));
        assert_eq!(map.get(&"b").next(), Some(&2));
        assert_eq!(map.get(&"c").next(), Some(&3));
        assert_eq!(map.get(&"d").next(), Some(&4));
    }

    #[test]
    fn test_remove_between_and_path_successor() {
        use std::path::{Path, PathBuf};

        #[derive(Debug)]
        pub struct PathDescendants<'a>(&'a Path);

        impl MapSeekTarget<PathBuf> for PathDescendants<'_> {
            fn cmp_cursor(&self, key: &PathBuf) -> Ordering {
                if key.starts_with(&self.0) {
                    Ordering::Greater
                } else {
                    self.0.cmp(key)
                }
            }
        }

        let mut map = TreeMultimap::default();

        map.insert(PathBuf::from("a"), 1);
        map.insert(PathBuf::from("a/a"), 1);
        map.insert(PathBuf::from("b"), 2);
        map.insert(PathBuf::from("b/a/a"), 3);
        map.insert(PathBuf::from("b/a/a/a/b"), 4);
        map.insert(PathBuf::from("c"), 5);
        map.insert(PathBuf::from("c/a"), 6);

        map.remove_range(
            &PathBuf::from("b/a"),
            &PathDescendants(&PathBuf::from("b/a")),
        );

        assert_eq!(map.get(&PathBuf::from("a")).next(), Some(&1));
        assert_eq!(map.get(&PathBuf::from("a/a")).next(), Some(&1));
        assert_eq!(map.get(&PathBuf::from("b")).next(), Some(&2));
        assert_eq!(map.get(&PathBuf::from("b/a/a")).next(), None);
        assert_eq!(map.get(&PathBuf::from("b/a/a/a/b")).next(), None);
        assert_eq!(map.get(&PathBuf::from("c")).next(), Some(&5));
        assert_eq!(map.get(&PathBuf::from("c/a")).next(), Some(&6));

        map.remove_range(&PathBuf::from("c"), &PathDescendants(&PathBuf::from("c")));

        assert_eq!(map.get(&PathBuf::from("a")).next(), Some(&1));
        assert_eq!(map.get(&PathBuf::from("a/a")).next(), Some(&1));
        assert_eq!(map.get(&PathBuf::from("b")).next(), Some(&2));
        assert_eq!(map.get(&PathBuf::from("c")).next(), None);
        assert_eq!(map.get(&PathBuf::from("c/a")).next(), None);

        map.remove_range(&PathBuf::from("a"), &PathDescendants(&PathBuf::from("a")));

        assert_eq!(map.get(&PathBuf::from("a")).next(), None);
        assert_eq!(map.get(&PathBuf::from("a/a")).next(), None);
        assert_eq!(map.get(&PathBuf::from("b")).next(), Some(&2));

        map.remove_range(&PathBuf::from("b"), &PathDescendants(&PathBuf::from("b")));

        assert_eq!(map.get(&PathBuf::from("b")).next(), None);
    }
}
