//! Shared limits for the descendants of one top-level plugin handle.

use super::memory::runtime_error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use wasmer::RuntimeError;

/// Maximum nesting below the top-level plugin instance.
pub(crate) const MAX_DEPTH: usize = 8;
/// Maximum child-module attempts across the complete invocation tree.
pub(crate) const MAX_MODULE_CALLS: usize = 64;

/// A depth bound and a shared, nonrenewable child-module allowance.
#[derive(Clone, Debug)]
pub(crate) struct InvocationBudget {
    depth: usize,
    remaining: Arc<AtomicUsize>,
}

impl InvocationBudget {
    /// Start the independent invocation tree of a top-level handle.
    pub(crate) fn new() -> Self {
        Self {
            depth: 0,
            remaining: Arc::new(AtomicUsize::new(MAX_MODULE_CALLS)),
        }
    }

    /// Reserve an attempt before loading or constructing a child module.
    /// Failed calls and dropped children never restore the shared allowance.
    pub(crate) fn child(&self) -> Result<Self, RuntimeError> {
        self.remaining
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                remaining.checked_sub(1)
            })
            .map_err(|_| runtime_error("WASM nested module call limit exceeded"))?;
        if self.depth >= MAX_DEPTH {
            return Err(runtime_error("WASM module invocation depth limit exceeded"));
        }
        Ok(Self {
            depth: self.depth + 1,
            remaining: Arc::clone(&self.remaining),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nesting_is_bounded_and_failed_attempts_are_not_refunded() {
        let root = InvocationBudget::new();
        let mut nested = root.clone();
        for _ in 0..MAX_DEPTH {
            nested = nested.child().unwrap();
        }
        assert!(nested
            .child()
            .unwrap_err()
            .to_string()
            .contains("depth limit"));
        drop(nested);
        for _ in 0..MAX_MODULE_CALLS - MAX_DEPTH - 1 {
            root.child().unwrap();
        }
        assert!(root.child().unwrap_err().to_string().contains("call limit"));
    }

    #[test]
    fn siblings_and_their_descendants_share_one_allowance() {
        let root = InvocationBudget::new();
        let left = root.child().unwrap();
        let right = root.child().unwrap();
        for _ in 0..MAX_MODULE_CALLS - 2 {
            left.child().unwrap();
        }
        for exhausted in [&root, &left, &right] {
            assert!(exhausted
                .child()
                .unwrap_err()
                .to_string()
                .contains("call limit"));
        }
        assert!(InvocationBudget::new().child().is_ok());
    }
}
