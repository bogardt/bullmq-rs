use redis::aio::ConnectionManager;

use crate::error::{BullmqError, BullmqResult};
use crate::scripts::ScriptLoader;

use super::key;

/// Result of an addJobScheduler call: the id and delay of the next iteration job.
#[derive(Debug)]
pub(crate) struct AddJobSchedulerResult {
    pub job_id: String,
    pub delay: u64,
}

/// Upsert a job scheduler and create its next iteration job.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn add_job_scheduler(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    scheduler_id: &str,
    next_millis: u64,
    scheduler_opts_json: &str,
    template_data_json: &str,
    template_opts_json: &str,
    delayed_opts_json: &str,
    timestamp: u64,
    producer_id: Option<&str>,
) -> BullmqResult<AddJobSchedulerResult> {
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
        key(prefix, queue_name, "active"),
    ];
    let producer_key = producer_id
        .map(|p| key(prefix, queue_name, p))
        .unwrap_or_default();
    let args: Vec<Vec<u8>> = vec![
        next_millis.to_string().into_bytes(),
        scheduler_opts_json.as_bytes().to_vec(),
        scheduler_id.as_bytes().to_vec(),
        template_data_json.as_bytes().to_vec(),
        template_opts_json.as_bytes().to_vec(),
        delayed_opts_json.as_bytes().to_vec(),
        timestamp.to_string().into_bytes(),
        format!("{}:{}:", prefix, queue_name).into_bytes(),
        producer_key.into_bytes(),
    ];

    let result = loader.invoke("addJobScheduler", conn, &keys, &args).await?;

    match result {
        redis::Value::Int(code) if code < 0 => Err(BullmqError::ScriptError(match code {
            -10 => format!(
                "addJobScheduler: job id collision for scheduler {}",
                scheduler_id
            ),
            -11 => format!(
                "addJobScheduler: job slots busy for scheduler {}",
                scheduler_id
            ),
            _ => format!("addJobScheduler returned unexpected code: {}", code),
        })),
        redis::Value::Array(items) if items.len() >= 2 => {
            let job_id = match &items[0] {
                redis::Value::BulkString(b) => String::from_utf8_lossy(b).to_string(),
                redis::Value::SimpleString(s) => s.clone(),
                other => {
                    return Err(BullmqError::ScriptError(format!(
                        "addJobScheduler returned unexpected job id: {:?}",
                        other
                    )))
                }
            };
            let delay = match &items[1] {
                redis::Value::Int(d) => (*d).max(0) as u64,
                redis::Value::BulkString(b) => String::from_utf8_lossy(b)
                    .parse::<f64>()
                    .unwrap_or(0.0)
                    .max(0.0) as u64,
                _ => 0,
            };
            Ok(AddJobSchedulerResult { job_id, delay })
        }
        other => Err(BullmqError::ScriptError(format!(
            "addJobScheduler returned unexpected value: {:?}",
            other
        ))),
    }
}
