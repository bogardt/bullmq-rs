use redis::aio::ConnectionManager;

use crate::error::BullmqResult;
use crate::scripts::ScriptLoader;

use super::key;

/// Add a parent job directly into the waiting-children state.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn add_parent_job(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    job_id: &str,
    name: &str,
    data: &str,
    timestamp: u64,
    opts_json: &str,
    max_events: u64,
    parent: Option<(&str, &str, &str)>,
) -> BullmqResult<String> {
    let (parent_dependencies_key, parent_key, parent_data) =
        parent.unwrap_or(("", "", ""));
    let keys = vec![
        key(prefix, queue_name, "waiting-children"),
        key(prefix, queue_name, "meta"),
        key(prefix, queue_name, "id"),
        key(prefix, queue_name, "events"),
        format!("{}:{}:{}", prefix, queue_name, job_id),
        parent_dependencies_key.to_string(),
    ];
    let args: Vec<Vec<u8>> = vec![
        name.as_bytes().to_vec(),
        data.as_bytes().to_vec(),
        timestamp.to_string().into_bytes(),
        job_id.as_bytes().to_vec(),
        opts_json.as_bytes().to_vec(),
        max_events.to_string().into_bytes(),
        parent_key.as_bytes().to_vec(),
        parent_data.as_bytes().to_vec(),
    ];
    let result = loader.invoke("addParentJob", conn, &keys, &args).await?;
    match result {
        redis::Value::BulkString(bytes) => Ok(String::from_utf8_lossy(&bytes).to_string()),
        redis::Value::SimpleString(s) => Ok(s),
        _ => Ok(job_id.to_string()),
    }
}
