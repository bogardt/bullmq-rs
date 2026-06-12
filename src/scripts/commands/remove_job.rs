use redis::aio::ConnectionManager;

use crate::error::{BullmqError, BullmqResult};
use crate::scripts::ScriptLoader;

use super::key;

/// Remove a job from all states and delete all of its data, including the
/// deduplication key it owns. Mirrors Node `Scripts.remove` (removeJob-2.lua).
///
/// Fails when the job (or, with `remove_children`, one of its children) is
/// locked, or when the job is the current iteration of a job scheduler.
pub(crate) async fn remove_job(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    job_id: &str,
    remove_children: bool,
) -> BullmqResult<()> {
    let keys = vec![
        format!("{}:{}:{}", prefix, queue_name, job_id),
        key(prefix, queue_name, "repeat"),
    ];
    let args: Vec<Vec<u8>> = vec![
        job_id.as_bytes().to_vec(),
        if remove_children {
            b"1".to_vec()
        } else {
            b"0".to_vec()
        },
        format!("{}:{}:", prefix, queue_name).into_bytes(),
    ];

    let result = loader.invoke("removeJob", conn, &keys, &args).await?;

    match result {
        redis::Value::Int(1) => Ok(()),
        redis::Value::Int(0) => Err(BullmqError::Other(format!(
            "Job {} could not be removed because it is locked by another worker",
            job_id
        ))),
        redis::Value::Int(-8) => Err(BullmqError::Other(format!(
            "Job {} belongs to a job scheduler and cannot be removed directly",
            job_id
        ))),
        other => Err(BullmqError::ScriptError(format!(
            "removeJob returned unexpected value: {:?}",
            other
        ))),
    }
}
