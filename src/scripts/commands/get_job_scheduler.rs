use std::collections::HashMap;

use redis::aio::ConnectionManager;

use crate::error::BullmqResult;
use crate::scripts::ScriptLoader;

use super::key;

fn value_to_string(value: &redis::Value) -> Option<String> {
    match value {
        redis::Value::BulkString(b) => Some(String::from_utf8_lossy(b).to_string()),
        redis::Value::SimpleString(s) => Some(s.clone()),
        _ => None,
    }
}

/// Get a job scheduler's metadata hash and next iteration timestamp (the
/// repeat zset score). Returns `None` when the scheduler does not exist.
pub(crate) async fn get_job_scheduler(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    scheduler_id: &str,
) -> BullmqResult<Option<(HashMap<String, String>, u64)>> {
    let keys = vec![key(prefix, queue_name, "repeat")];
    let args: Vec<Vec<u8>> = vec![scheduler_id.as_bytes().to_vec()];

    let result = loader.invoke("getJobScheduler", conn, &keys, &args).await?;

    let redis::Value::Array(items) = result else {
        return Ok(None);
    };
    if items.len() < 2 {
        return Ok(None);
    }

    let mut map = HashMap::new();
    if let redis::Value::Array(pairs) = &items[0] {
        let mut i = 0;
        while i + 1 < pairs.len() {
            if let (Some(k), Some(v)) = (value_to_string(&pairs[i]), value_to_string(&pairs[i + 1]))
            {
                map.insert(k, v);
            }
            i += 2;
        }
    }

    let score = value_to_string(&items[1])
        .and_then(|s| s.parse::<f64>().ok())
        .map(|f| f as u64)
        .unwrap_or(0);

    Ok(Some((map, score)))
}
