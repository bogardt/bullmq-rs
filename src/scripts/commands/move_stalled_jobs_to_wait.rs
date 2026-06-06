use redis::aio::ConnectionManager;

use crate::error::BullmqResult;
use crate::scripts::ScriptLoader;

use super::key;

/// Result of the stalled jobs check.
#[derive(Debug)]
pub(crate) struct StalledJobsResult {
    /// Number of jobs that were stalled and moved back to wait.
    pub stalled: u64,
    /// Number of jobs that exceeded max stall count and were failed.
    pub failed: u64,
}

/// Check for stalled jobs and move them back to wait or fail them.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn move_stalled_jobs_to_wait(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    max_stalled_count: u32,
    stalled_interval: u64,
    timestamp: u64,
    max_events: u64,
) -> BullmqResult<StalledJobsResult> {
    let job_key_prefix = format!("{}:{}:", prefix, queue_name);

    let keys = vec![
        key(prefix, queue_name, "wait"),
        key(prefix, queue_name, "active"),
        key(prefix, queue_name, "prioritized"),
        key(prefix, queue_name, "stalled"),
        key(prefix, queue_name, "stalled-check"),
        key(prefix, queue_name, "meta"),
        key(prefix, queue_name, "paused"),
        key(prefix, queue_name, "events"),
    ];
    let args: Vec<Vec<u8>> = vec![
        max_stalled_count.to_string().into_bytes(),
        stalled_interval.to_string().into_bytes(),
        timestamp.to_string().into_bytes(),
        max_events.to_string().into_bytes(),
        job_key_prefix.into_bytes(),
        key(prefix, queue_name, "marker").into_bytes(),
        key(prefix, queue_name, "pc").into_bytes(),
    ];

    let result = loader
        .invoke("moveStalledJobsToWait", conn, &keys, &args)
        .await?;

    match result {
        redis::Value::Array(items) if items.len() >= 2 => {
            let stalled = match &items[0] {
                redis::Value::Int(n) => *n as u64,
                _ => 0,
            };
            let failed = match &items[1] {
                redis::Value::Int(n) => *n as u64,
                _ => 0,
            };
            Ok(StalledJobsResult { stalled, failed })
        }
        _ => Ok(StalledJobsResult {
            stalled: 0,
            failed: 0,
        }),
    }
}
