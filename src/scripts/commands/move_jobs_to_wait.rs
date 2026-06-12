use redis::aio::ConnectionManager;

use crate::error::{BullmqError, BullmqResult};
use crate::scripts::ScriptLoader;

use super::key;

/// Move up to `count` jobs from `state` (failed, completed or delayed) back to
/// wait/paused. Returns 1 when more jobs may remain, 0 when done.
pub(crate) async fn move_jobs_to_wait(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    state: &str,
    count: u64,
    timestamp: &str,
) -> BullmqResult<i64> {
    let keys = vec![
        key(prefix, queue_name, ""),
        key(prefix, queue_name, "events"),
        key(prefix, queue_name, state),
        key(prefix, queue_name, "wait"),
        key(prefix, queue_name, "paused"),
        key(prefix, queue_name, "meta"),
        key(prefix, queue_name, "active"),
        key(prefix, queue_name, "marker"),
    ];
    let args: Vec<Vec<u8>> = vec![
        count.to_string().into_bytes(),
        timestamp.as_bytes().to_vec(),
        state.as_bytes().to_vec(),
    ];

    let result = loader.invoke("moveJobsToWait", conn, &keys, &args).await?;

    match result {
        redis::Value::Int(cursor) => Ok(cursor),
        other => Err(BullmqError::ScriptError(format!(
            "moveJobsToWait returned unexpected value: {:?}",
            other
        ))),
    }
}
