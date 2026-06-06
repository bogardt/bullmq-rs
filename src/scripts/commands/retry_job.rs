use redis::aio::ConnectionManager;

use crate::error::{BullmqError, BullmqResult};
use crate::scripts::ScriptLoader;

use super::key;

/// Retry a job by moving it from active back to wait or prioritized.
///
/// `push_cmd` should be `"RPUSH"` (retried jobs go to the back) or `"LPUSH"`.
#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
pub(crate) async fn retry_job(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    job_id: &str,
    token: &str,
    push_cmd: &str,
    max_events: u64,
    attempts_made: u32,
    priority: u32,
) -> BullmqResult<()> {
    let job_key = format!("{}:{}:{}", prefix, queue_name, job_id);

    let keys = vec![
        key(prefix, queue_name, "active"),
        key(prefix, queue_name, "wait"),
        key(prefix, queue_name, "stalled"),
        key(prefix, queue_name, "paused"),
        key(prefix, queue_name, "meta"),
        key(prefix, queue_name, "events"),
        key(prefix, queue_name, "delayed"),
        key(prefix, queue_name, "prioritized"),
        key(prefix, queue_name, "pc"),
        key(prefix, queue_name, "marker"),
        job_key,
    ];
    let args: Vec<Vec<u8>> = vec![
        token.as_bytes().to_vec(),
        push_cmd.as_bytes().to_vec(),
        job_id.as_bytes().to_vec(),
        max_events.to_string().into_bytes(),
        attempts_made.to_string().into_bytes(),
        priority.to_string().into_bytes(),
    ];

    let result = loader.invoke("retryJob", conn, &keys, &args).await?;

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
                    "retryJob returned unexpected code: {}",
                    code
                )))
            }
        }
        _ => Ok(()),
    }
}
