use std::collections::HashMap;

use redis::aio::ConnectionManager;

use crate::error::BullmqResult;
use crate::scripts::ScriptLoader;

use super::key;

/// Result of a moveToActive call.
#[derive(Debug)]
pub(crate) struct MoveToActiveResult {
    /// The job ID, or `None` if no job was available.
    pub job_id: Option<String>,
    /// The job's hash fields as key-value pairs (only present when a job is returned).
    pub job_data: HashMap<String, String>,
}

/// Move the next waiting/prioritized job to active state, acquiring a lock.
///
/// Returns the job ID and all hash fields on success, or `None` if no job available.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn move_to_active(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    token: &str,
    lock_duration: u64,
    timestamp: u64,
    max_events: u64,
) -> BullmqResult<MoveToActiveResult> {
    let job_key_prefix = format!("{}:{}:", prefix, queue_name);
    let keys = vec![
        key(prefix, queue_name, "wait"),
        key(prefix, queue_name, "active"),
        key(prefix, queue_name, "prioritized"),
        key(prefix, queue_name, "events"),
        key(prefix, queue_name, "stalled"),
        key(prefix, queue_name, "limiter"),
        key(prefix, queue_name, "delayed"),
        key(prefix, queue_name, "paused"),
        key(prefix, queue_name, "meta"),
        key(prefix, queue_name, "pc"),
        key(prefix, queue_name, "marker"),
    ];
    let args: Vec<Vec<u8>> = vec![
        token.as_bytes().to_vec(),
        lock_duration.to_string().into_bytes(),
        timestamp.to_string().into_bytes(),
        max_events.to_string().into_bytes(),
        job_key_prefix.into_bytes(),
    ];
    let result = loader.invoke("moveToActive", conn, &keys, &args).await?;
    parse_move_to_active_result(result)
}

/// Parse the Lua return value into a `MoveToActiveResult`.
pub(crate) fn parse_move_to_active_result(value: redis::Value) -> BullmqResult<MoveToActiveResult> {
    match value {
        redis::Value::Array(items) if !items.is_empty() => {
            // First element is the job ID
            let job_id = match &items[0] {
                redis::Value::BulkString(bytes) => String::from_utf8_lossy(bytes).to_string(),
                redis::Value::SimpleString(s) => s.clone(),
                _ => {
                    return Ok(MoveToActiveResult {
                        job_id: None,
                        job_data: HashMap::new(),
                    })
                }
            };

            // Remaining elements are key-value pairs from HGETALL
            let mut job_data = HashMap::new();
            let mut i = 1;
            while i + 1 < items.len() {
                let k = match &items[i] {
                    redis::Value::BulkString(b) => String::from_utf8_lossy(b).to_string(),
                    redis::Value::SimpleString(s) => s.clone(),
                    _ => {
                        i += 2;
                        continue;
                    }
                };
                let v = match &items[i + 1] {
                    redis::Value::BulkString(b) => String::from_utf8_lossy(b).to_string(),
                    redis::Value::SimpleString(s) => s.clone(),
                    _ => String::new(),
                };
                job_data.insert(k, v);
                i += 2;
            }

            Ok(MoveToActiveResult {
                job_id: Some(job_id),
                job_data,
            })
        }
        _ => Ok(MoveToActiveResult {
            job_id: None,
            job_data: HashMap::new(),
        }),
    }
}
