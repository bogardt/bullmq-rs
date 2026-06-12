use redis::aio::ConnectionManager;

use crate::error::BullmqResult;
use crate::scripts::ScriptLoader;

use super::key;

/// Schedule the next iteration of a job scheduler after the current iteration
/// finished. Returns the id of the created delayed job, or `None` when the
/// scheduler no longer exists, the producer is stale, or the next iteration
/// already exists.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn update_job_scheduler(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    scheduler_id: &str,
    next_millis: u64,
    template_data_json: &str,
    delayed_opts_json: &str,
    timestamp: u64,
    producer_id: &str,
) -> BullmqResult<Option<String>> {
    let keys = vec![
        key(prefix, queue_name, "repeat"),
        key(prefix, queue_name, "delayed"),
        key(prefix, queue_name, "wait"),
        key(prefix, queue_name, "paused"),
        key(prefix, queue_name, "meta"),
        key(prefix, queue_name, "prioritized"),
        key(prefix, queue_name, "marker"),
        key(prefix, queue_name, "id"),
        key(prefix, queue_name, "events"),
        key(prefix, queue_name, "pc"),
        key(prefix, queue_name, producer_id),
        key(prefix, queue_name, "active"),
    ];
    let args: Vec<Vec<u8>> = vec![
        next_millis.to_string().into_bytes(),
        scheduler_id.as_bytes().to_vec(),
        template_data_json.as_bytes().to_vec(),
        delayed_opts_json.as_bytes().to_vec(),
        timestamp.to_string().into_bytes(),
        format!("{}:{}:", prefix, queue_name).into_bytes(),
        producer_id.as_bytes().to_vec(),
    ];

    let result = loader
        .invoke("updateJobScheduler", conn, &keys, &args)
        .await?;

    match result {
        redis::Value::BulkString(b) => Ok(Some(String::from_utf8_lossy(&b).to_string())),
        redis::Value::SimpleString(s) => Ok(Some(s)),
        _ => Ok(None),
    }
}
