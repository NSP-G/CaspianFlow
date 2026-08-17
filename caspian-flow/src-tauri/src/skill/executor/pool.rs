//! Execution pool — limits concurrent skill executions.
//!
//! Uses a [`tokio::sync::Semaphore`] to enforce a maximum number of
//! concurrent subprocess executions. When the pool is full, new
//! `acquire()` calls will wait until a permit is released.

use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::types::{ExecutorError, ExecutorResult};

/// A pool that limits the number of concurrent skill executions.
///
/// Clone-safe: cloning an `ExecutionPool` shares the same underlying
/// semaphore, so clones respect the same concurrency limit.
pub struct ExecutionPool {
    semaphore: Arc<Semaphore>,
    max_concurrent: usize,
}

impl ExecutionPool {
    /// Create a new pool with the given maximum concurrent executions.
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            max_concurrent,
        }
    }

    /// Acquire a permit from the pool.
    ///
    /// This will wait if all permits are currently held.
    /// The permit is released when the returned `SemaphorePermit` is dropped.
    pub async fn acquire(&self) -> ExecutorResult<tokio::sync::SemaphorePermit<'_>> {
        self.semaphore
            .acquire()
            .await
            .map_err(|_| ExecutorError::PoolExhausted {
                max_concurrent: self.max_concurrent,
            })
    }

    /// Get the maximum number of concurrent executions.
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// Get the number of available permits.
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }
}

impl Clone for ExecutionPool {
    fn clone(&self) -> Self {
        Self {
            semaphore: Arc::clone(&self.semaphore),
            max_concurrent: self.max_concurrent,
        }
    }
}

impl std::fmt::Debug for ExecutionPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionPool")
            .field("max_concurrent", &self.max_concurrent)
            .field("available_permits", &self.available_permits())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_acquire_release() {
        let pool = ExecutionPool::new(2);
        let _permit1 = pool.acquire().await.unwrap();
        let _permit2 = pool.acquire().await.unwrap();
        // Drop one to free a slot
        drop(_permit1);
        let _permit3 = pool.acquire().await.unwrap();
    }

    #[tokio::test]
    async fn test_max_concurrent() {
        let pool = ExecutionPool::new(4);
        assert_eq!(pool.max_concurrent(), 4);
    }

    #[tokio::test]
    async fn test_available_permits() {
        let pool = ExecutionPool::new(3);
        assert_eq!(pool.available_permits(), 3);
        let _permit = pool.acquire().await.unwrap();
        assert_eq!(pool.available_permits(), 2);
    }

    #[tokio::test]
    async fn test_clone_shares_semaphore() {
        let pool = ExecutionPool::new(2);
        let pool_clone = pool.clone();

        let _permit = pool.acquire().await.unwrap();
        assert_eq!(pool_clone.available_permits(), 1);
    }

    #[tokio::test]
    async fn test_concurrent_execution() {
        let pool = ExecutionPool::new(2);
        let pool_clone = pool.clone();

        // Spawn a task that holds a permit briefly
        let handle = tokio::spawn(async move {
            let _permit = pool_clone.acquire().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        });

        // This should succeed immediately (2 permits, 1 held)
        let _permit = pool.acquire().await.unwrap();

        handle.await.unwrap();
    }

    #[test]
    fn test_debug_format() {
        let pool = ExecutionPool::new(2);
        let debug = format!("{pool:?}");
        assert!(debug.contains("ExecutionPool"));
        assert!(debug.contains("max_concurrent"));
    }
}
