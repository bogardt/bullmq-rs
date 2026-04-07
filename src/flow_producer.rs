use std::sync::Arc;

use crate::connection::RedisConnection;
use crate::error::{BullmqError, BullmqResult};
use crate::job::Job;
use crate::scripts::ScriptLoader;
use crate::types::JobOptions;

/// Producer for BullMQ flows.
pub struct FlowProducer {
    connection: RedisConnection,
    prefix: String,
    scripts: Arc<ScriptLoader>,
}

/// Builder for creating a [`FlowProducer`].
pub struct FlowProducerBuilder {
    connection: Option<RedisConnection>,
    prefix: String,
}

/// A flow job definition.
#[derive(Debug, Clone)]
pub struct FlowJob<T = serde_json::Value> {
    pub name: String,
    pub queue_name: String,
    pub data: T,
    pub prefix: Option<String>,
    pub opts: Option<JobOptions>,
    pub children: Vec<FlowJob<T>>,
}

/// A node in a flow tree.
#[derive(Debug, Clone)]
pub struct FlowNode<T = serde_json::Value> {
    pub job: Job<T>,
    pub children: Vec<FlowNode<T>>,
}

impl FlowProducerBuilder {
    /// Create a new flow producer builder.
    pub fn new() -> Self {
        Self {
            connection: None,
            prefix: "bull".to_string(),
        }
    }

    /// Set the Redis connection configuration.
    pub fn connection(mut self, conn: RedisConnection) -> Self {
        self.connection = Some(conn);
        self
    }

    /// Set a custom key prefix (default: "bull").
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Build the flow producer.
    pub async fn build(self) -> BullmqResult<FlowProducer> {
        let connection = self.connection.ok_or_else(|| {
            BullmqError::Other("FlowProducerBuilder requires a Redis connection".into())
        })?;

        Ok(FlowProducer {
            connection,
            prefix: self.prefix,
            scripts: Arc::new(ScriptLoader::new()),
        })
    }
}

impl FlowProducer {
    /// Add a flow to Redis.
    pub async fn add<T>(&self, _job: FlowJob<T>) -> BullmqResult<FlowNode<T>> {
        let _ = (&self.connection, &self.prefix, &self.scripts);
        Err(BullmqError::NotImplemented(
            "FlowProducer::add is not yet implemented".into(),
        ))
    }
}
