//! Generation checked slots. Exhausted generations retire permanently.
use std::ops::{Index, IndexMut};
pub(crate) struct Slot<T> {
    generation: u64,
    value: Option<T>,
}
pub(super) struct Arena<T> {
    slots: Vec<Slot<T>>,
    free: Vec<usize>,
}
impl<T> Arena<T> {
    pub fn new() -> Self {
        Self {
            slots: vec![],
            free: vec![],
        }
    }
    pub fn len(&self) -> usize {
        self.slots.len()
    }
    /// Append-only publication preserves existing index-based module maps.
    pub fn push(&mut self, value: T) {
        self.slots.push(Slot {
            generation: 0,
            value: Some(value),
        })
    }
    pub fn generation(&self, index: usize) -> Option<u64> {
        self.slots
            .get(index)
            .and_then(|s| s.value.as_ref().map(|_| s.generation))
    }
    /// The live value at `index` if it has generation `generation`.
    #[cfg(not(feature = "execution-reference"))]
    #[inline(always)]
    pub fn get_live(&self, index: usize, generation: u64) -> Option<&T> {
        let slot = self.slots.get(index)?;
        if slot.generation != generation {
            return None;
        }
        slot.value.as_ref()
    }
    /// `get_live`, mutably.
    #[cfg(not(feature = "execution-reference"))]
    #[inline(always)]
    pub fn get_live_mut(&mut self, index: usize, generation: u64) -> Option<&mut T> {
        let slot = self.slots.get_mut(index)?;
        if slot.generation != generation {
            return None;
        }
        slot.value.as_mut()
    }
    pub fn get(&self, index: usize) -> Option<&T> {
        self.slots.get(index)?.value.as_ref()
    }
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.slots.iter().filter_map(|s| s.value.as_ref())
    }
    pub fn insert(&mut self, value: T) -> (usize, u64) {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index];
            debug_assert!(slot.value.is_none());
            slot.value = Some(value);
            (index, slot.generation)
        } else {
            let index = self.slots.len();
            self.push(value);
            (index, 0)
        }
    }
    pub fn remove(&mut self, index: usize, generation: u64) -> Option<T> {
        let slot = self.slots.get_mut(index)?;
        if slot.generation != generation {
            return None;
        }
        let value = slot.value.take()?;
        if let Some(next) = slot.generation.checked_add(1) {
            slot.generation = next;
            self.free.push(index)
        }
        Some(value)
    }
}
impl<T> Index<usize> for Arena<T> {
    type Output = T;
    #[inline(always)]
    fn index(&self, index: usize) -> &T {
        self.slots[index]
            .value
            .as_ref()
            .expect("validated live arena slot")
    }
}
impl<T> IndexMut<usize> for Arena<T> {
    #[inline(always)]
    fn index_mut(&mut self, index: usize) -> &mut T {
        self.slots[index]
            .value
            .as_mut()
            .expect("validated live arena slot")
    }
}
impl<'a, T> IntoIterator for &'a Arena<T> {
    type Item = &'a T;
    type IntoIter =
        std::iter::FilterMap<std::slice::Iter<'a, Slot<T>>, fn(&'a Slot<T>) -> Option<&'a T>>;
    fn into_iter(self) -> Self::IntoIter {
        self.slots.iter().filter_map(|s| s.value.as_ref())
    }
}

impl<T> Arena<T> {
    pub fn retire_from(&mut self, start: usize) {
        for index in start..self.slots.len() {
            let slot = &mut self.slots[index];
            slot.value = None;
            slot.generation = slot.generation.saturating_add(1);
        }
    }
    pub fn live_len(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.value.is_some())
            .count()
    }
}
impl<T> Arena<T> {
    pub fn pop_unpublished(&mut self) -> Option<T> {
        self.slots.pop()?.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t036_reuse_rejects_stale_generation() {
        let mut a = Arena::new();
        let (i, g) = a.insert(7);
        assert_eq!(a.remove(i, g), Some(7));
        let (j, h) = a.insert(9);
        assert_eq!(i, j);
        assert_ne!(g, h);
        assert_eq!(a.remove(i, g), None);
        assert_eq!(a.remove(j, h), Some(9));
    }
    #[test]
    fn t036_exhausted_generation_retires_slot() {
        let mut a = Arena::new();
        a.push(7);
        a.slots[0].generation = u64::MAX;
        assert_eq!(a.remove(0, u64::MAX), Some(7));
        assert_eq!(a.insert(9), (1, 0));
        assert_eq!(a.generation(0), None);
    }
}
