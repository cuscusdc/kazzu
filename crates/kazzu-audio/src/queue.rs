//! Play queue: items, play order (shuffle), repeat and navigation.
//! Pure logic, no audio; the engine drives it.

use kazzu_core::{QueueItem, QueueItemId, RepeatMode};

#[derive(Debug)]
pub struct Queue {
    items: Vec<QueueItem>,
    /// Play order: indices into `items`. Identity unless shuffled.
    order: Vec<usize>,
    /// Position in `order` of the audible item.
    current: Option<usize>,
    repeat: RepeatMode,
    shuffle: bool,
    rng: XorShift,
}

impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}

impl Queue {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            order: Vec::new(),
            current: None,
            repeat: RepeatMode::Off,
            shuffle: false,
            rng: XorShift::from_time(),
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    /// Replaces the queue; `start` indexes `items` (given order).
    pub fn load(&mut self, items: Vec<QueueItem>, start: usize) {
        self.items = items;
        self.order = (0..self.items.len()).collect();
        self.current = if self.items.is_empty() {
            None
        } else {
            Some(start.min(self.items.len() - 1))
        };
        if self.shuffle {
            self.reshuffle();
        }
    }

    pub fn enqueue(&mut self, items: Vec<QueueItem>) {
        for item in items {
            self.order.push(self.items.len());
            self.items.push(item);
        }
    }

    /// Inserts right after the current item in play order.
    pub fn play_next(&mut self, items: Vec<QueueItem>) {
        let first = self.current.map_or(self.order.len(), |c| c + 1);
        for (at, item) in (first..).zip(items) {
            let idx = self.items.len();
            self.items.push(item);
            self.order.insert(at, idx);
        }
    }

    /// Removes an item. Returns true when it was the current one (the current
    /// position then points at the item that followed it, if any).
    pub fn remove(&mut self, id: QueueItemId) -> bool {
        let Some(idx) = self.items.iter().position(|i| i.id == id) else {
            return false;
        };
        let Some(pos) = self.order.iter().position(|&i| i == idx) else {
            return false;
        };
        self.items.remove(idx);
        self.order.remove(pos);
        for i in &mut self.order {
            if *i > idx {
                *i -= 1;
            }
        }
        let was_current = self.current == Some(pos);
        self.current = match self.current {
            _ if self.order.is_empty() => None,
            Some(c) if c > pos => Some(c - 1),
            Some(c) if c == pos => Some(c.min(self.order.len() - 1)),
            other => other,
        };
        was_current
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.order.clear();
        self.current = None;
    }

    /// Move an item in playback order while preserving the current item.
    pub fn move_before(&mut self, id: QueueItemId, before: QueueItemId) -> bool {
        let (Some(from), Some(to)) = (self.order_position(id), self.order_position(before)) else {
            return false;
        };
        if from == to || from + 1 == to {
            return false;
        }
        let current = self.current_id();
        let index = self.order.remove(from);
        self.order.insert(if from < to { to - 1 } else { to }, index);
        self.current = current.and_then(|id| self.order_position(id));
        if !self.shuffle {
            self.items = self.ordered_items();
            self.order = (0..self.items.len()).collect();
        }
        true
    }

    pub fn get(&self, id: QueueItemId) -> Option<&QueueItem> {
        self.items.iter().find(|i| i.id == id)
    }

    /// Replaces the stream of an item (e.g. a refreshed, re-signed URL).
    pub fn set_stream(&mut self, id: QueueItemId, stream: kazzu_core::StreamSource) -> bool {
        match self.items.iter_mut().find(|i| i.id == id) {
            Some(item) => {
                item.stream = stream;
                true
            }
            None => false,
        }
    }

    pub fn current(&self) -> Option<&QueueItem> {
        self.current.map(|p| &self.items[self.order[p]])
    }

    pub fn current_id(&self) -> Option<QueueItemId> {
        self.current().map(|i| i.id)
    }

    /// Position of the current item in play order.
    pub fn current_position(&self) -> Option<usize> {
        self.current
    }

    pub fn set_current(&mut self, id: QueueItemId) -> bool {
        match self.order_position(id) {
            Some(p) => {
                self.current = Some(p);
                true
            }
            None => false,
        }
    }

    fn order_position(&self, id: QueueItemId) -> Option<usize> {
        self.order.iter().position(|&i| self.items[i].id == id)
    }

    fn id_at(&self, pos: usize) -> QueueItemId {
        self.items[self.order[pos]].id
    }

    /// Item that follows `id`. `natural` means "the track ended by itself",
    /// where repeat-one replays the same item; a manual skip ignores it.
    pub fn next_after(&self, id: QueueItemId, natural: bool) -> Option<QueueItemId> {
        let pos = self.order_position(id)?;
        if natural && self.repeat == RepeatMode::One {
            return Some(id);
        }
        if pos + 1 < self.order.len() {
            Some(self.id_at(pos + 1))
        } else if self.repeat == RepeatMode::All && !self.order.is_empty() {
            Some(self.id_at(0))
        } else {
            None
        }
    }

    pub fn previous_before(&self, id: QueueItemId) -> Option<QueueItemId> {
        let pos = self.order_position(id)?;
        if pos > 0 {
            Some(self.id_at(pos - 1))
        } else if self.repeat == RepeatMode::All && !self.order.is_empty() {
            Some(self.id_at(self.order.len() - 1))
        } else {
            None
        }
    }

    pub fn first_id(&self) -> Option<QueueItemId> {
        (!self.order.is_empty()).then(|| self.id_at(0))
    }

    pub fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
    }

    pub fn set_shuffle(&mut self, enabled: bool) {
        if enabled == self.shuffle {
            return;
        }
        self.shuffle = enabled;
        if enabled {
            self.reshuffle();
        } else {
            let current_idx = self.current.map(|p| self.order[p]);
            self.order = (0..self.items.len()).collect();
            self.current = current_idx;
        }
    }

    /// Current item first, the rest in random order (Fisher–Yates).
    fn reshuffle(&mut self) {
        let current_idx = self.current.map(|p| self.order[p]);
        let mut rest: Vec<usize> = (0..self.items.len())
            .filter(|&i| Some(i) != current_idx)
            .collect();
        for i in (1..rest.len()).rev() {
            let j = (self.rng.next() % (i as u64 + 1)) as usize;
            rest.swap(i, j);
        }
        self.order = current_idx.into_iter().chain(rest).collect();
        self.current = current_idx.map(|_| 0);
    }

    /// Items in play order.
    pub fn ordered_items(&self) -> Vec<QueueItem> {
        self.order.iter().map(|&i| self.items[i].clone()).collect()
    }
}

/// Tiny non-cryptographic PRNG for shuffling (no extra dependency).
#[derive(Debug)]
struct XorShift(u64);

impl XorShift {
    fn from_time() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kazzu_core::{StreamSource, Track, TrackSource};

    fn item(n: u32) -> QueueItem {
        let path = format!("/t/{n}.flac");
        QueueItem::new(
            Track::new(n.to_string(), format!("T{n}"), TrackSource::Local { path: path.clone().into() }),
            StreamSource::file(path),
        )
    }

    fn ids(q: &Queue) -> Vec<String> {
        q.ordered_items().iter().map(|i| i.track.id.to_string()).collect()
    }

    #[test]
    fn navigation_and_repeat() {
        let mut q = Queue::new();
        let items: Vec<_> = (1..=3).map(item).collect();
        let (a, b, c) = (items[0].id, items[1].id, items[2].id);
        q.load(items, 0);
        assert_eq!(q.current_id(), Some(a));
        assert_eq!(q.next_after(a, true), Some(b));
        assert_eq!(q.next_after(c, true), None);
        assert_eq!(q.previous_before(a), None);
        q.set_repeat(RepeatMode::All);
        assert_eq!(q.next_after(c, true), Some(a));
        assert_eq!(q.previous_before(a), Some(c));
        q.set_repeat(RepeatMode::One);
        assert_eq!(q.next_after(b, true), Some(b));
        assert_eq!(q.next_after(b, false), Some(c));
    }

    #[test]
    fn play_next_and_remove() {
        let mut q = Queue::new();
        let items: Vec<_> = (1..=3).map(item).collect();
        let b = items[1].id;
        q.load(items, 0);
        q.play_next(vec![item(9)]);
        assert_eq!(ids(&q), ["1", "9", "2", "3"]);
        assert!(!q.remove(b));
        assert_eq!(ids(&q), ["1", "9", "3"]);
        let cur = q.current_id().unwrap();
        assert!(q.remove(cur));
        assert_eq!(q.current().unwrap().track.id.as_str(), "9");
    }

    #[test]
    fn shuffle_keeps_current_first_and_restores_order() {
        let mut q = Queue::new();
        let items: Vec<_> = (1..=20).map(item).collect();
        q.load(items, 5);
        let cur = q.current_id().unwrap();
        q.set_shuffle(true);
        assert_eq!(q.current_position(), Some(0));
        assert_eq!(q.current_id(), Some(cur));
        let mut sorted = ids(&q);
        sorted.sort_by_key(|s| s.parse::<u32>().unwrap());
        assert_eq!(sorted, (1..=20).map(|n| n.to_string()).collect::<Vec<_>>());
        q.set_shuffle(false);
        assert_eq!(q.current_position(), Some(5));
        assert_eq!(q.current_id(), Some(cur));
    }
}
