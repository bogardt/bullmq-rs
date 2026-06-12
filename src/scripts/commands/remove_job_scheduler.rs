use redis::aio::ConnectionManager;

use crate::error::BullmqResult;
use crate::scripts::ScriptLoader;

use super::key;

/// Remove a job scheduler, its metadata hash, and its pending iteration job.
///
/// Returns `true` when the scheduler existed and was removed.
pub(crate) async fn remove_job_scheduler(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    scheduler_id: &str,
) -> BullmqResult<bool> {
    let keys = vec![
        key(prefix, queue_name, "repeat"),
        key(prefix, queue_name, "delayed"),
        key(prefix, queue_name, "events"),
    ];
    let args: Vec<Vec<u8>> = vec![
        scheduler_id.as_bytes().to_vec(),
        format!("{}:{}:", prefix, queue_name).into_bytes(),
    ];

    let result = loader
        .invoke("removeJobScheduler", conn, &keys, &args)
        .await?;

    Ok(matches!(result, redis::Value::Int(0)))
}
