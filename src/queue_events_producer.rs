use redis::aio::ConnectionManager;

use crate::connection::RedisConnection;
use crate::error::BullmqResult;
use crate::types::DEFAULT_MAX_EVENTS;

/// Publishes custom events to a BullMQ queue's event stream.
///
/// Events are published via XADD and appear as `QueueEvent::Unknown`
/// on the consumer side (unless they use a built-in event name).
pub struct QueueEventsProducer {
    name: String,
    prefix: String,
    conn: ConnectionManager,
}

/// Builder for creating a [`QueueEventsProducer`].
pub struct QueueEventsProducerBuilder {
    name: String,
    connection: RedisConnection,
    prefix: String,
}

impl QueueEventsProducerBuilder {
    /// Create a new builder for the given queue name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            connection: RedisConnection::default(),
            prefix: "bull".to_string(),
        }
    }

    /// Set the Redis connection configuration.
    pub fn connection(mut self, conn: RedisConnection) -> Self {
        self.connection = conn;
        self
    }

    /// Set a custom key prefix (default: "bull").
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Build the producer, establishing a Redis connection.
    pub async fn build(self) -> BullmqResult<QueueEventsProducer> {
        let conn = self.connection.get_manager().await?;
        Ok(QueueEventsProducer {
            name: self.name,
            prefix: self.prefix,
            conn,
        })
    }
}

impl QueueEventsProducer {
    /// Publish a custom event to the queue's event stream.
    ///
    /// Uses the default max stream length (10,000).
    pub async fn publish(&self, event_name: &str, fields: &[(&str, &str)]) -> BullmqResult<()> {
        self.publish_with_max_len(event_name, fields, DEFAULT_MAX_EVENTS)
            .await
    }

    /// Publish a custom event with a specific max stream length.
    pub async fn publish_with_max_len(
        &self,
        event_name: &str,
        fields: &[(&str, &str)],
        max_len: u64,
    ) -> BullmqResult<()> {
        let mut conn = self.conn.clone();
        let events_key = format!("{}:{}:events", self.prefix, self.name);

        let mut cmd = redis::cmd("XADD");
        cmd.arg(&events_key)
            .arg("MAXLEN")
            .arg("~")
            .arg(max_len)
            .arg("*")
            .arg("event")
            .arg(event_name);

        for (k, v) in fields {
            cmd.arg(*k).arg(*v);
        }

        cmd.query_async::<String>(&mut conn).await?;
        Ok(())
    }
}
