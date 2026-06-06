use redis::aio::ConnectionManager;

use crate::error::BullmqResult;
use crate::scripts::ScriptLoader;

use super::key;

/// Add a log entry to a job's log list.
///
/// Returns the current log count after insertion.
pub(crate) async fn add_log(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    job_id: &str,
    log_line: &str,
    max_log_count: u64,
) -> BullmqResult<u64> {
    let job_key = format!("{}:{}:{}", prefix, queue_name, job_id);
    let logs_key = format!("{}:logs", job_key);

    let keys = vec![logs_key, key(prefix, queue_name, "meta")];
    let args: Vec<Vec<u8>> = vec![
        log_line.as_bytes().to_vec(),
        max_log_count.to_string().into_bytes(),
    ];

    let result = loader.invoke("addLog", conn, &keys, &args).await?;

    match result {
        redis::Value::Int(n) => Ok(n as u64),
        _ => Ok(0),
    }
}
