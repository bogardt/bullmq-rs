use redis::aio::ConnectionManager;

use crate::error::{BullmqError, BullmqResult};
use crate::scripts::ScriptLoader;

use super::key;

/// Remove up to `count` jobs of an obliterated queue.
///
/// Returns the number of remaining iterations: `0` when the queue is fully
/// obliterated, `1` when more jobs remain and the call must be repeated.
pub(crate) async fn obliterate(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    count: u64,
    force: bool,
) -> BullmqResult<i64> {
    let keys = vec![
        key(prefix, queue_name, "meta"),
        format!("{}:{}:", prefix, queue_name),
    ];
    let force_arg: &[u8] = if force { b"force" } else { b"" };
    let args: Vec<Vec<u8>> = vec![count.to_string().into_bytes(), force_arg.to_vec()];

    let result = loader.invoke("obliterate", conn, &keys, &args).await?;

    match result {
        redis::Value::Int(-1) => Err(BullmqError::Other(
            "Cannot obliterate non-paused queue".to_string(),
        )),
        redis::Value::Int(-2) => Err(BullmqError::Other(
            "Cannot obliterate queue with active jobs".to_string(),
        )),
        redis::Value::Int(cursor) => Ok(cursor),
        other => Err(BullmqError::ScriptError(format!(
            "obliterate returned unexpected value: {:?}",
            other
        ))),
    }
}
