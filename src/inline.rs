//! A list of at most `N` items kept inline, with no heap allocation (T1b): the per-enemy lists
//! (rules, statuses) and an attack task's frames are bounded by the validator or by the kinds
//! there are, so they need no `Vec`. The unused places hold `T::default()`; they are never read.

use std::ops::{Deref, DerefMut};

/// At most `N` items, in order.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InlineVec<T, const N: usize> {
    items: [T; N],
    len: u8,
}

/// Compares the live items only: the unused places keep what `pop` and `remove` left there.
#[cfg(test)]
impl<T: PartialEq, const N: usize> PartialEq for InlineVec<T, N> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<T: Copy + Default, const N: usize> Default for InlineVec<T, N> {
    fn default() -> Self {
        InlineVec::new()
    }
}

impl<T: Copy + Default, const N: usize> InlineVec<T, N> {
    pub(crate) fn new() -> Self {
        const { assert!(N <= u8::MAX as usize) };
        InlineVec {
            items: [T::default(); N],
            len: 0,
        }
    }

    /// Adds `item` at the end; false, changing nothing, if the list is full.
    #[must_use]
    pub(crate) fn try_push(&mut self, item: T) -> bool {
        let len = usize::from(self.len);
        if len == N {
            return false;
        }
        self.items[len] = item;
        self.len += 1;
        true
    }

    /// Adds `item` at the end. A list is sized for the most there can be, so a full one is a bug:
    /// it panics, as any other overflow does (determinism rules).
    pub(crate) fn push(&mut self, item: T) {
        assert!(self.try_push(item), "an inline list of {N} is full");
    }

    pub(crate) fn pop(&mut self) -> Option<T> {
        self.len = self.len.checked_sub(1)?;
        Some(self.items[usize::from(self.len)])
    }

    /// Removes the item at `index`, moving the ones after it down.
    pub(crate) fn remove(&mut self, index: usize) -> T {
        let len = usize::from(self.len);
        assert!(index < len, "remove index {index} of {len}");
        let item = self.items[index];
        self.items.copy_within(index + 1..len, index);
        self.len -= 1;
        item
    }

    /// Keeps the items `keep` accepts, in order.
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&T) -> bool) {
        let mut kept = 0;
        for i in 0..usize::from(self.len) {
            if keep(&self.items[i]) {
                self.items[kept] = self.items[i];
                kept += 1;
            }
        }
        self.len = u8::try_from(kept).expect("at most the length it had");
    }
}

impl<T: Copy + Default, const N: usize> FromIterator<T> for InlineVec<T, N> {
    /// Panics if there are more than `N` items, as `push` does.
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut list = InlineVec::new();
        for item in iter {
            list.push(item);
        }
        list
    }
}

impl<'a, T, const N: usize> IntoIterator for &'a InlineVec<T, N> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T, const N: usize> IntoIterator for &'a mut InlineVec<T, N> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<T, const N: usize> Deref for InlineVec<T, N> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.items[..usize::from(self.len)]
    }
}

impl<T, const N: usize> DerefMut for InlineVec<T, N> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.items[..usize::from(self.len)]
    }
}

#[cfg(test)]
mod tests {
    use super::InlineVec;

    #[test]
    fn it_behaves_as_a_vec_up_to_its_size() {
        let mut v: InlineVec<u32, 3> = InlineVec::new();
        assert!(v.is_empty());
        v.push(1);
        v.push(2);
        assert!(v.try_push(3));
        assert!(!v.try_push(4));
        assert_eq!(&*v, &[1, 2, 3]);
        assert_eq!(v.remove(0), 1);
        assert_eq!(&*v, &[2, 3]);
        v.retain(|&x| x != 2);
        assert_eq!(&*v, &[3]);
        assert_eq!(v.pop(), Some(3));
        assert_eq!(v.pop(), None);
    }

    #[test]
    #[should_panic(expected = "full")]
    fn a_full_list_panics_on_push() {
        let mut v: InlineVec<u8, 1> = InlineVec::new();
        v.push(1);
        v.push(2);
    }
}
