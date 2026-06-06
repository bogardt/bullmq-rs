use std::time::Duration;

use redis::aio::ConnectionManager;
use redis::aio::ConnectionManagerConfig;

use crate::error::BullmqResult;

/// Redis connection configuration.
///
/// # Example
/// ```
/// use bullmq_rs::RedisConnection;
///
/// let conn = RedisConnection::new("redis://127.0.0.1:6379");
/// ```
#[derive(Debug, Clone)]
pub struct RedisConnection {
    url: String,
}

impl RedisConnection {
    /// Create a new connection configuration from a Redis URL.
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }

    /// Get the Redis URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Create a `ConnectionManager` for resilient Redis connections.
    pub(crate) async fn get_manager(&self) -> BullmqResult<ConnectionManager> {
        self.get_manager_with_response_timeout(None).await
    }

    /// Create a `ConnectionManager` with an optional per-command response timeout.
    pub(crate) async fn get_manager_with_response_timeout(
        &self,
        response_timeout: Option<Duration>,
    ) -> BullmqResult<ConnectionManager> {
        let client = redis::Client::open(self.url.as_str())?;
        let config = ConnectionManagerConfig::new()
            .set_max_delay(Duration::from_millis(5_000))
            .set_response_timeout(response_timeout);
        let manager = ConnectionManager::new_with_config(client, config).await?;
        Ok(manager)
    }
}

impl Default for RedisConnection {
    fn default() -> Self {
        Self::new("redis://127.0.0.1:6379")
    }
}
