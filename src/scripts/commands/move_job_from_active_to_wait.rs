use redis::aio::ConnectionManager;

use crate::error::{BullmqError, BullmqResult};
use crate::scripts::ScriptLoader;

use super::key;

/// Move a job back from active to wait (or prioritized) after a manual
/// rate-limit, without consuming an attempt.
///
/// Returns the rate limiter key PTTL in milliseconds (0 when no rate limit
/// is active), mirroring Node `Scripts.moveJobFromActiveToWait`.
pub(crate) async fn move_job_from_active_to_wait(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    job_id: &str,
    token: &str,
) -> BullmqResult<u64> {
    let job_key = format!("{}:{}:{}", prefix, queue_name, job_id);

    let keys = vec![
        key(prefix, queue_name, "active"),
        key(prefix, queue_name, "wait"),
        key(prefix, queue_name, "stalled"),
        key(prefix, queue_name, "paused"),
        key(prefix, queue_name, "meta"),
        key(prefix, queue_name, "limiter"),
        key(prefix, queue_name, "prioritized"),
        key(prefix, queue_name, "marker"),
        key(prefix, queue_name, "events"),
    ];
    let args: Vec<Vec<u8>> = vec![
        job_id.as_bytes().to_vec(),
        token.as_bytes().to_vec(),
        job_key.into_bytes(),
    ];

    let result = loader
        .invoke("moveJobFromActiveToWait", conn, &keys, &args)
        .await?;

    match result {
        redis::Value::Int(code) if code >= 0 => Ok(code as u64),
        redis::Value::Int(-1) => Err(BullmqError::JobNotFound(job_id.to_string())),
        redis::Value::Int(-2) | redis::Value::Int(-6) => Err(BullmqError::LockMismatch),
        other => Err(BullmqError::ScriptError(format!(
            "moveJobFromActiveToWait returned unexpected value: {:?}",
            other
        ))),
    }
}
