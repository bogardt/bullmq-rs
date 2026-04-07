use std::collections::HashMap;
use std::time::Duration;

use redis::aio::ConnectionManager;
use tokio::sync::{broadcast, watch};

use crate::connection::RedisConnection;
use crate::error::{BullmqError, BullmqResult};

/// A typed event from a BullMQ queue's event stream.
///
/// Parsed from Redis stream entries emitted by Lua scripts via XADD.
/// The stream key is `{prefix}:{queue}:events`.
#[derive(Debug, Clone, PartialEq)]
pub enum QueueEvent {
    Added {
        job_id: String,
        name: String,
    },
    Waiting {
        job_id: String,
        prev: Option<String>,
    },
    Active {
        job_id: String,
        prev: Option<String>,
    },
    Completed {
        job_id: String,
        return_value: serde_json::Value,
    },
    Failed {
        job_id: String,
        reason: String,
    },
    /// The `delay` field is an absolute Unix timestamp in milliseconds
    /// (not a relative duration). This matches what the Lua scripts emit.
    Delayed {
        job_id: String,
        delay: u64,
    },
    Progress {
        job_id: String,
        data: serde_json::Value,
    },
    Stalled {
        job_id: String,
    },
    Priority {
        job_id: String,
        priority: u32,
    },
    Removed {
        job_id: String,
        prev: Option<String>,
    },
    WaitingChildren {
        job_id: String,
    },
    Paused,
    Resumed,
    Drained,
    /// Custom or unrecognized events (e.g. from QueueEventsProducer).
    Unknown {
        event: String,
        fields: HashMap<String, String>,
    },
}

impl QueueEvent {
    /// Parse a stream entry's field-value map into a QueueEvent.
    pub fn parse(fields: &HashMap<String, String>) -> Self {
        let event_type = fields.get("event").map(|s| s.as_str()).unwrap_or("");
        let job_id = || fields.get("jobId").cloned().unwrap_or_default();
        let prev = || {
            fields
                .get("prev")
                .and_then(|s| if s.is_empty() { None } else { Some(s.clone()) })
        };

        match event_type {
            "added" => QueueEvent::Added {
                job_id: job_id(),
                name: fields.get("name").cloned().unwrap_or_default(),
            },
            "waiting" => QueueEvent::Waiting {
                job_id: job_id(),
                prev: prev(),
            },
            "active" => QueueEvent::Active {
                job_id: job_id(),
                prev: prev(),
            },
            "completed" => QueueEvent::Completed {
                job_id: job_id(),
                return_value: fields
                    .get("returnvalue")
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(serde_json::Value::Null),
            },
            "failed" => QueueEvent::Failed {
                job_id: job_id(),
                reason: fields.get("failedReason").cloned().unwrap_or_default(),
            },
            "delayed" => QueueEvent::Delayed {
                job_id: job_id(),
                delay: fields
                    .get("delay")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
            },
            "progress" => QueueEvent::Progress {
                job_id: job_id(),
                data: fields
                    .get("data")
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(serde_json::Value::Null),
            },
            "stalled" => QueueEvent::Stalled { job_id: job_id() },
            "priority" => QueueEvent::Priority {
                job_id: job_id(),
                priority: fields
                    .get("priority")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
            },
            "removed" => QueueEvent::Removed {
                job_id: job_id(),
                prev: prev(),
            },
            "waiting-children" => QueueEvent::WaitingChildren { job_id: job_id() },
            "paused" => QueueEvent::Paused,
            "resumed" => QueueEvent::Resumed,
            "drained" => QueueEvent::Drained,
            _ => QueueEvent::Unknown {
                event: event_type.to_string(),
                fields: fields.clone(),
            },
        }
    }
}

/// Parse the nested redis::Value from XREAD into (entry_id, fields) pairs.
fn parse_xread_response(value: &redis::Value) -> Vec<(String, HashMap<String, String>)> {
    let mut results = Vec::new();

    let streams = match value {
        redis::Value::Array(streams) => streams,
        _ => return results,
    };

    for stream in streams {
        let stream_parts = match stream {
            redis::Value::Array(parts) if parts.len() >= 2 => parts,
            _ => continue,
        };

        let entries = match &stream_parts[1] {
            redis::Value::Array(entries) => entries,
            _ => continue,
        };

        for entry in entries {
            let entry_parts = match entry {
                redis::Value::Array(parts) if parts.len() >= 2 => parts,
                _ => continue,
            };

            let entry_id = match &entry_parts[0] {
                redis::Value::BulkString(b) => String::from_utf8_lossy(b).to_string(),
                redis::Value::SimpleString(s) => s.clone(),
                _ => continue,
            };

            let field_values = match &entry_parts[1] {
                redis::Value::Array(fv) => fv,
                _ => continue,
            };

            let mut fields = HashMap::new();
            let mut i = 0;
            while i + 1 < field_values.len() {
                let k = match &field_values[i] {
                    redis::Value::BulkString(b) => String::from_utf8_lossy(b).to_string(),
                    redis::Value::SimpleString(s) => s.clone(),
                    _ => {
                        i += 2;
                        continue;
                    }
                };
                let v = match &field_values[i + 1] {
                    redis::Value::BulkString(b) => String::from_utf8_lossy(b).to_string(),
                    redis::Value::SimpleString(s) => s.clone(),
                    _ => String::new(),
                };
                fields.insert(k, v);
                i += 2;
            }

            results.push((entry_id, fields));
        }
    }

    results
}

/// Consumes events from a BullMQ queue's Redis event stream.
///
/// Uses `XREAD BLOCK` in a background task and delivers events
/// through a `tokio::broadcast` channel.
///
/// Not `Clone` — wrap in `Arc<QueueEvents>` to share across tasks.
pub struct QueueEvents {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    prefix: String,
    tx: broadcast::Sender<(QueueEvent, String)>,
    shutdown_tx: watch::Sender<bool>,
    join_handle: Option<tokio::task::JoinHandle<()>>,
}

impl QueueEvents {
    /// Create a new subscriber for events.
    pub fn subscribe(&self) -> broadcast::Receiver<(QueueEvent, String)> {
        self.tx.subscribe()
    }

    /// Signal the background XREAD loop to stop.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Wait for the background task to exit.
    pub async fn wait(&mut self) -> BullmqResult<()> {
        if let Some(handle) = self.join_handle.take() {
            handle
                .await
                .map_err(|e| BullmqError::Other(format!("QueueEvents task panicked: {}", e)))?;
        }
        Ok(())
    }

    /// Signal shutdown and wait for the background task to exit.
    pub async fn close(&mut self) -> BullmqResult<()> {
        self.shutdown();
        self.wait().await
    }
}

/// Builder for creating a [`QueueEvents`].
pub struct QueueEventsBuilder {
    name: String,
    connection: RedisConnection,
    prefix: String,
    last_event_id: String,
    blocking_timeout: u64,
    capacity: usize,
}

impl QueueEventsBuilder {
    /// Create a new builder for the given queue name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            connection: RedisConnection::default(),
            prefix: "bull".to_string(),
            last_event_id: "$".to_string(),
            blocking_timeout: 10_000,
            capacity: 256,
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

    /// Resume from a specific stream ID instead of "$" (latest).
    pub fn last_event_id(mut self, id: impl Into<String>) -> Self {
        self.last_event_id = id.into();
        self
    }

    /// Set the XREAD BLOCK timeout in milliseconds (default: 10000).
    pub fn blocking_timeout(mut self, ms: u64) -> Self {
        self.blocking_timeout = ms;
        self
    }

    /// Set the broadcast channel capacity (default: 256).
    pub fn capacity(mut self, cap: usize) -> Self {
        self.capacity = cap;
        self
    }

    /// Build the QueueEvents, establishing a dedicated Redis connection
    /// and starting the background XREAD loop.
    pub async fn build(self) -> BullmqResult<QueueEvents> {
        let mut conn: ConnectionManager = self.connection.get_manager().await?;
        let events_key = format!("{}:{}:events", self.prefix, self.name);
        let (tx, _) = broadcast::channel(self.capacity);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let tx_clone = tx.clone();
        let blocking_timeout = self.blocking_timeout;
        let mut last_id = self.last_event_id;

        let join_handle = tokio::spawn(async move {
            let mut shutdown_rx = shutdown_rx;

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let result: redis::RedisResult<redis::Value> = redis::cmd("XREAD")
                    .arg("BLOCK")
                    .arg(blocking_timeout)
                    .arg("STREAMS")
                    .arg(&events_key)
                    .arg(&last_id)
                    .query_async(&mut conn)
                    .await;

                match result {
                    Ok(redis::Value::Nil) => {
                        // Timeout — no new entries.
                        continue;
                    }
                    Ok(value) => {
                        let entries = parse_xread_response(&value);
                        for (entry_id, fields) in entries {
                            last_id = entry_id.clone();
                            let event = QueueEvent::parse(&fields);
                            let _ = tx_clone.send((event, entry_id));
                        }
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("response was nil") || msg.contains("not compatible") {
                            continue;
                        }
                        tracing::warn!("QueueEvents XREAD error: {}", e);
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                            _ = shutdown_rx.changed() => break,
                        }
                    }
                }
            }
            tracing::debug!("QueueEvents loop stopped");
        });

        Ok(QueueEvents {
            name: self.name,
            prefix: self.prefix,
            tx,
            shutdown_tx,
            join_handle: Some(join_handle),
        })
    }
}
