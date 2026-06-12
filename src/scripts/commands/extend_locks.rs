use redis::aio::ConnectionManager;

use crate::error::BullmqResult;
use crate::scripts::ScriptLoader;

use super::key;

/// Extend the locks of multiple jobs in one round trip.
///
/// Returns the ids of the jobs whose lock could NOT be extended (missing lock
/// or token mismatch).
pub(crate) async fn extend_locks(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    job_ids: &[String],
    tokens: &[String],
    duration_ms: u64,
) -> BullmqResult<Vec<String>> {
    let keys = vec![key(prefix, queue_name, "stalled")];
    let args: Vec<Vec<u8>> = vec![
        format!("{}:{}:", prefix, queue_name).into_bytes(),
        serde_json::to_vec(tokens)?,
        serde_json::to_vec(job_ids)?,
        duration_ms.to_string().into_bytes(),
    ];

    let result = loader.invoke("extendLocks", conn, &keys, &args).await?;

    let mut failed = Vec::new();
    if let redis::Value::Array(items) = result {
        for item in items {
            match item {
                redis::Value::BulkString(b) => failed.push(String::from_utf8_lossy(&b).to_string()),
                redis::Value::SimpleString(s) => failed.push(s),
                _ => {}
            }
        }
    }
    Ok(failed)
}
