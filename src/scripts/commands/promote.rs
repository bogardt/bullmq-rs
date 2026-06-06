use redis::aio::ConnectionManager;

use crate::error::{BullmqError, BullmqResult};
use crate::scripts::ScriptLoader;

use super::key;

/// Promote a delayed job to wait or prioritized.
pub(crate) async fn promote(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    job_id: &str,
    max_events: u64,
    timestamp: u64,
) -> BullmqResult<()> {
    let job_key = format!("{}:{}:{}", prefix, queue_name, job_id);

    let keys = vec![
        key(prefix, queue_name, "delayed"),
        key(prefix, queue_name, "wait"),
        key(prefix, queue_name, "paused"),
        key(prefix, queue_name, "meta"),
        key(prefix, queue_name, "prioritized"),
        key(prefix, queue_name, "active"),
        key(prefix, queue_name, "pc"),
        key(prefix, queue_name, "events"),
        key(prefix, queue_name, "marker"),
    ];
    let args: Vec<Vec<u8>> = vec![
        job_id.as_bytes().to_vec(),
        job_key.into_bytes(),
        timestamp.to_string().into_bytes(),
        max_events.to_string().into_bytes(),
    ];

    let result = loader.invoke("promote", conn, &keys, &args).await?;

    match result {
        redis::Value::Int(code) => {
            if code == 0 {
                Ok(())
            } else if code == -1 {
                Err(BullmqError::Other(format!(
                    "Job {} is not in delayed state",
                    job_id
                )))
            } else {
                Err(BullmqError::ScriptError(format!(
                    "promote returned unexpected code: {}",
                    code
                )))
            }
        }
        _ => Ok(()),
    }
}
