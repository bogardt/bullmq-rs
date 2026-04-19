use redis::aio::ConnectionManager;

use crate::error::{BullmqError, BullmqResult};
use crate::scripts::ScriptLoader;

use super::key;

/// Move an active job back to the delayed state.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn move_to_delayed(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    job_id: &str,
    token: &str,
    timestamp: u64,
    delayed_timestamp: u64,
    max_events: u64,
    attempts_made: u32,
) -> BullmqResult<()> {
    let job_key = format!("{}:{}:{}", prefix, queue_name, job_id);
    let lock_key = format!("{}:lock", job_key);

    let keys = vec![
        key(prefix, queue_name, "active"),
        key(prefix, queue_name, "delayed"),
        key(prefix, queue_name, "events"),
        job_key,
        lock_key,
        key(prefix, queue_name, "meta"),
        key(prefix, queue_name, "marker"),
        key(prefix, queue_name, "stalled"),
    ];
    let args: Vec<Vec<u8>> = vec![
        token.as_bytes().to_vec(),
        timestamp.to_string().into_bytes(),
        job_id.as_bytes().to_vec(),
        delayed_timestamp.to_string().into_bytes(),
        max_events.to_string().into_bytes(),
        attempts_made.to_string().into_bytes(),
    ];

    let result = loader.invoke("moveToDelayed", conn, &keys, &args).await?;

    match result {
        redis::Value::Int(code) => {
            if code == 0 {
                Ok(())
            } else if code == -1 {
                Err(BullmqError::JobNotFound(job_id.to_string()))
            } else if code == -2 || code == -6 {
                Err(BullmqError::LockMismatch)
            } else {
                Err(BullmqError::ScriptError(format!(
                    "moveToDelayed returned unexpected code: {}",
                    code
                )))
            }
        }
        _ => Ok(()),
    }
}
