// Copyright (C) 2025 Bonsai Software, Inc.
// This file is part of Noshtastic, and is licensed under the
// GNU General Public License, version 3 or later. See the LICENSE file
// or <https://www.gnu.org/licenses/> for details.

use std::collections::HashSet;
use std::collections::VecDeque;

#[derive(Debug)]
pub struct LruSet<T> {
    set: HashSet<T>,
    deque: VecDeque<T>,
    capacity: usize,
}

impl<T: Eq + std::hash::Hash + Clone> LruSet<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            set: HashSet::new(),
            deque: VecDeque::new(),
            capacity,
        }
    }

    pub fn insert(&mut self, value: T) {
        if self.set.contains(&value) {
            return;
        }
        if self.deque.len() == self.capacity {
            if let Some(oldest) = self.deque.pop_front() {
                self.set.remove(&oldest);
            } else {
                panic!("unable to pop_front");
            }
        }
        self.set.insert(value.clone());
        self.deque.push_back(value);
    }

    pub fn contains(&self, value: &T) -> bool {
        self.set.contains(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lruset() {
        let mut recent_ids = LruSet::new(10);

        for i in 0..20 {
            recent_ids.insert(i);
        }

        assert!(!recent_ids.contains(&5));
        assert!(!recent_ids.contains(&9));
        assert!(recent_ids.contains(&10));
        assert!(recent_ids.contains(&15));
    }
}
