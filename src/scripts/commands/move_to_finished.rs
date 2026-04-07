use redis::aio::ConnectionManager;

use crate::error::{BullmqError, BullmqResult};
use crate::scripts::ScriptLoader;

use super::key;
use super::move_to_active::{parse_move_to_active_result, MoveToActiveResult};

/// Result of a moveToFinished call.
#[derive(Debug)]
pub(crate) enum MoveToFinishedResult {
    /// The job was successfully finished (no next job fetched).
    Done,
    /// The job was finished and a next job was fetched.
    NextJob(MoveToActiveResult),
}

/// Move a job from active to completed or failed, and optionally fetch the next job.
///
/// `target_state` should be `"completed"` or `"failed"`.
/// `result_data` is the return value (for completed) or failure reason (for failed).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn move_to_finished(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    job_id: &str,
    token: &str,
    timestamp: u64,
    result_data: &str,
    target_state: &str,
    max_events: u64,
    fetch_next: bool,
    lock_duration: u64,
    attempts_made: u32,
) -> BullmqResult<MoveToFinishedResult> {
    let job_key = format!("{}:{}:{}", prefix, queue_name, job_id);
    let lock_key = format!("{}:lock", job_key);
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
        key(prefix, queue_name, target_state),
        key(prefix, queue_name, "meta"),
        key(prefix, queue_name, "pc"),
        key(prefix, queue_name, "marker"),
        job_key,
        lock_key,
    ];
    let args: Vec<Vec<u8>> = vec![
        token.as_bytes().to_vec(),
        timestamp.to_string().into_bytes(),
        result_data.as_bytes().to_vec(),
        target_state.as_bytes().to_vec(),
        max_events.to_string().into_bytes(),
        if fetch_next {
            b"1".to_vec()
        } else {
            b"0".to_vec()
        },
        lock_duration.to_string().into_bytes(),
        job_id.as_bytes().to_vec(),
        attempts_made.to_string().into_bytes(),
        job_key_prefix.into_bytes(),
    ];

    let result = loader.invoke("moveToFinished", conn, &keys, &args).await?;

    match &result {
        redis::Value::Int(code) => {
            let code = *code;
            if code == 0 {
                Ok(MoveToFinishedResult::Done)
            } else if code == -1 {
                Err(BullmqError::JobNotFound(job_id.to_string()))
            } else if code == -2 || code == -6 {
                Err(BullmqError::LockMismatch)
            } else {
                Err(BullmqError::ScriptError(format!(
                    "moveToFinished returned unexpected code: {}",
                    code
                )))
            }
        }
        redis::Value::Array(_) => {
            let next = parse_move_to_active_result(result)?;
            Ok(MoveToFinishedResult::NextJob(next))
        }
        _ => Ok(MoveToFinishedResult::Done),
    }
}
