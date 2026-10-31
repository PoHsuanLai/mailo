//! A bounded memory of the last few answers, for a lookup that is dear to repeat.
//!
//! Newest in, oldest out: a [`put`](Recent::put) under a key already held replaces it and makes
//! it the newest, and one past the capacity drops the oldest. Reading does not reorder, so a
//! `&self` lookup is all a reader on the thread that draws needs. The sizes in use are tens to a
//! few hundred, so a walk over them is cheaper than hashing and keeps the type free of `Hash`.
//!
//! A value, not a global: whoever owns one decides how long it lives and who shares it, and two
//! of them share nothing.

use std::collections::VecDeque;

/// The last `capacity` answers put, by key.
#[derive(Debug, Clone)]
pub struct Recent<K, V> {
    items: VecDeque<(K, V)>,
    capacity: usize,
}

impl<K: PartialEq, V> Recent<K, V> {
    /// An empty memory holding at most `capacity` answers. A capacity of zero remembers nothing.
    pub fn new(capacity: usize) -> Self {
        Recent {
            items: VecDeque::new(),
            capacity,
        }
    }

    /// The answer held under `key`, if there is one.
    pub fn get(&self, key: &K) -> Option<&V> {
        self.items
            .iter()
            .find(|(held, _)| held == key)
            .map(|(_, value)| value)
    }

    /// Hold `value` under `key` as the newest, replacing what `key` had and dropping the oldest
    /// answers beyond the capacity.
    pub fn put(&mut self, key: K, value: V) {
        self.items.retain(|(held, _)| *held != key);
        self.items.push_back((key, value));
        while self.items.len() > self.capacity {
            self.items.pop_front();
        }
    }

    /// How many answers are held.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether nothing is held.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::Recent;

    #[test]
    fn an_answer_put_is_found_and_a_missing_key_is_not() {
        let mut recent = Recent::new(4);
        recent.put("a", 1);
        assert_eq!(recent.get(&"a"), Some(&1));
        assert_eq!(recent.get(&"b"), None);
    }

    #[test]
    fn the_oldest_goes_first_once_past_the_capacity() {
        let mut recent = Recent::new(2);
        recent.put(1, "one");
        recent.put(2, "two");
        recent.put(3, "three");
        assert_eq!(recent.len(), 2);
        assert_eq!(recent.get(&1), None);
        assert_eq!(recent.get(&2), Some(&"two"));
        assert_eq!(recent.get(&3), Some(&"three"));
    }

    #[test]
    fn putting_under_a_held_key_replaces_it_and_makes_it_the_newest() {
        let mut recent = Recent::new(2);
        recent.put(1, "one");
        recent.put(2, "two");
        recent.put(1, "uno");
        assert_eq!(recent.len(), 2);
        // 2 is now the oldest, so it is what the next put drops.
        recent.put(3, "three");
        assert_eq!(recent.get(&2), None);
        assert_eq!(recent.get(&1), Some(&"uno"));
    }

    #[test]
    fn reading_does_not_keep_an_answer_alive() {
        let mut recent = Recent::new(2);
        recent.put(1, ());
        recent.put(2, ());
        assert!(recent.get(&1).is_some());
        recent.put(3, ());
        assert!(recent.get(&1).is_none());
    }

    #[test]
    fn a_capacity_of_zero_remembers_nothing() {
        let mut recent = Recent::new(0);
        recent.put(1, ());
        assert!(recent.is_empty());
        assert_eq!(recent.get(&1), None);
    }
}
