//! Event queue abstraction for interior mutability.
//!
//! Encapsulates the `RefCell<VecDeque<T>>` pattern to provide a cleaner
//! API for event collection in callback contexts.

use std::cell::RefCell;
use std::collections::VecDeque;

/// A thread-local event queue that can be written to from callbacks.
///
/// This encapsulates the interior mutability pattern, making it explicit
/// that this is designed for use in callback contexts where we can't
/// pass mutable references.
pub struct EventQueue<T> {
    inner: RefCell<VecDeque<T>>,
}

impl<T> EventQueue<T> {
    /// Create a new empty event queue.
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(VecDeque::new()),
        }
    }

    /// Push an event to the queue.
    ///
    /// This uses interior mutability so it can be called from callbacks.
    pub fn push(&self, event: T) {
        self.inner.borrow_mut().push_back(event);
    }

    /// Drain all events from the queue.
    pub fn drain(&self) -> Vec<T> {
        self.inner.borrow_mut().drain(..).collect()
    }
}

impl<T> Default for EventQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}
