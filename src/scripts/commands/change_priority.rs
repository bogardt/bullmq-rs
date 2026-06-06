use redis::aio::ConnectionManager;

use crate::error::{BullmqError, BullmqResult};
use crate::scripts::ScriptLoader;

use super::key;

/// Change the priority of a job, moving it between wait and prioritized as needed.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn change_priority(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    job_id: &str,
    priority: u32,
    max_events: u64,
    timestamp: u64,
) -> BullmqResult<()> {
    let job_key = format!("{}:{}:{}", prefix, queue_name, job_id);
    let events_key = key(prefix, queue_name, "events");

    let keys = vec![
        key(prefix, queue_name, "meta"),
        key(prefix, queue_name, "prioritized"),
        key(prefix, queue_name, "wait"),
        key(prefix, queue_name, "paused"),
        key(prefix, queue_name, "active"),
        key(prefix, queue_name, "pc"),
        key(prefix, queue_name, "marker"),
    ];
    let args: Vec<Vec<u8>> = vec![
        priority.to_string().into_bytes(),
        job_id.as_bytes().to_vec(),
        job_key.into_bytes(),
        events_key.into_bytes(),
        timestamp.to_string().into_bytes(),
        max_events.to_string().into_bytes(),
    ];

    let result = loader.invoke("changePriority", conn, &keys, &args).await?;

    match result {
        redis::Value::Int(code) => {
            if code == 0 {
                Ok(())
            } else if code == -1 {
                Err(BullmqError::JobNotFound(job_id.to_string()))
            } else {
                Err(BullmqError::ScriptError(format!(
                    "changePriority returned unexpected code: {}",
                    code
                )))
            }
        }
        _ => Ok(()),
    }
}
