use redis::aio::ConnectionManager;

use crate::error::BullmqResult;
use crate::scripts::ScriptLoader;

use super::key;

/// Remove jobs from a specific state set/list, older than the given timestamp.
///
/// Returns the ids of the removed jobs.
pub(crate) async fn clean_jobs_in_set(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    set_name: &str,
    timestamp: u64,
    limit: u64,
) -> BullmqResult<Vec<String>> {
    let keys = vec![
        key(prefix, queue_name, set_name),
        key(prefix, queue_name, "events"),
        key(prefix, queue_name, "repeat"),
    ];
    let job_key_prefix = format!("{}:{}:", prefix, queue_name);
    let args: Vec<Vec<u8>> = vec![
        job_key_prefix.into_bytes(),
        timestamp.to_string().into_bytes(),
        limit.to_string().into_bytes(),
        set_name.as_bytes().to_vec(),
    ];

    let result = loader.invoke("cleanJobsInSet", conn, &keys, &args).await?;

    let mut job_ids = Vec::new();
    if let redis::Value::Array(items) = result {
        for item in items {
            match item {
                redis::Value::BulkString(b) => {
                    job_ids.push(String::from_utf8_lossy(&b).to_string())
                }
                redis::Value::SimpleString(s) => job_ids.push(s),
                _ => {}
            }
        }
    }
    Ok(job_ids)
}
