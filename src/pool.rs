//! Reusable node-set buffers for warm evaluations.
use std::{cell::RefCell, fmt};

/// The most buffers a pool keeps; further buffers given back are dropped.
const MAX_POOLED: usize = 64;

/// A pool of empty node-set buffers, shared by evaluations through
/// [`Context::with_pool`](crate::Context::with_pool).
///
/// The evaluator takes its intermediate node-set vectors from the pool and
/// gives them back when it is done with them, so warm evaluations reuse their
/// capacity instead of allocating. The node-set an evaluation returns is
/// always owned by the caller; a host may [`give`](Self::give) it back once it
/// has finished with it. Vectors returned by
/// [`Navigator::children_named`](crate::Navigator::children_named) are given
/// to the pool after use too, so a navigator that builds them from
/// [`take`](Self::take) has them recycled.
///
/// A pool is for one thread (`!Sync`) and holds at most 64 buffers.
pub struct NodeSetPool<T> {
    buffers: RefCell<Vec<Vec<T>>>,
}

impl<T> NodeSetPool<T> {
    pub fn new() -> Self {
        Self {
            buffers: RefCell::new(Vec::new()),
        }
    }
    /// An empty buffer: a pooled one if available, else `Vec::new()`.
    pub fn take(&self) -> Vec<T> {
        self.buffers.borrow_mut().pop().unwrap_or_default()
    }
    /// Clear `buffer` and keep it, with its capacity, for a later
    /// [`take`](Self::take). A buffer without capacity, or one given while the
    /// pool already holds 64, is dropped.
    pub fn give(&self, mut buffer: Vec<T>) {
        // Clear before borrowing, so dropping nodes never sees a borrowed pool.
        buffer.clear();
        if buffer.capacity() == 0 {
            return;
        }
        let mut buffers = self.buffers.borrow_mut();
        if buffers.len() < MAX_POOLED {
            buffers.push(buffer);
        }
    }
    /// The number of pooled buffers.
    pub fn len(&self) -> usize {
        self.buffers.borrow().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T> Default for NodeSetPool<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> fmt::Debug for NodeSetPool<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeSetPool")
            .field("len", &self.len())
            .finish()
    }
}
