//! Opaque cleanup authority shared by browser process owners.

use std::sync::Arc;

/// Trusted authority retained until exact browser process-tree and profile
/// cleanup completes. The engine cannot inspect the wrapped value; every
/// launch and cancellation path keeps it with the process cleanup authority.
#[derive(Clone)]
pub struct HostCleanupLease {
    _authority: Arc<dyn Send + Sync>,
}

impl HostCleanupLease {
    /// Wrap one trusted, process-internal cleanup authority. Dropping the
    /// final engine clone is the only operation the engine performs on it.
    pub fn new<T>(authority: T) -> Self
    where
        T: Send + Sync + 'static,
    {
        Self {
            _authority: Arc::new(authority),
        }
    }
}

impl std::fmt::Debug for HostCleanupLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostCleanupLease")
            .field("opaque", &true)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Authority(Arc<AtomicUsize>);
    impl Drop for Authority {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn authority_is_retained_until_the_final_lease_drops() {
        let drops = Arc::new(AtomicUsize::new(0));
        let lease = HostCleanupLease::new(Authority(drops.clone()));
        let retained = lease.clone();
        drop(lease);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(retained);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn debug_does_not_expose_authority_contents() {
        let lease = HostCleanupLease::new("private authority");
        assert_eq!(format!("{lease:?}"), "HostCleanupLease { opaque: true }");
    }
}
