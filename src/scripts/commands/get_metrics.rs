use redis::aio::ConnectionManager;

use crate::error::{BullmqError, BullmqResult};
use crate::scripts::ScriptLoader;
use crate::types::{Metrics, MetricsMeta};

use super::key;

fn value_to_i64(value: &redis::Value) -> i64 {
    match value {
        redis::Value::Int(i) => *i,
        redis::Value::BulkString(b) => String::from_utf8_lossy(b).parse().unwrap_or(0),
        redis::Value::SimpleString(s) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

/// Fetch queue metrics for a finished state (`"completed"` or `"failed"`).
pub(crate) async fn get_metrics(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    target_state: &str,
    start: i64,
    end: i64,
) -> BullmqResult<Metrics> {
    let metrics_suffix = format!("metrics:{}", target_state);
    let keys = vec![
        key(prefix, queue_name, &metrics_suffix),
        key(prefix, queue_name, &format!("{}:data", metrics_suffix)),
    ];
    let args: Vec<Vec<u8>> = vec![start.to_string().into_bytes(), end.to_string().into_bytes()];

    let result = loader.invoke("getMetrics", conn, &keys, &args).await?;

    let items = match result {
        redis::Value::Array(items) if items.len() >= 3 => items,
        other => {
            return Err(BullmqError::ScriptError(format!(
                "getMetrics returned unexpected value: {:?}",
                other
            )))
        }
    };

    let meta = match &items[0] {
        redis::Value::Array(fields) if fields.len() >= 3 => MetricsMeta {
            count: value_to_i64(&fields[0]) as u64,
            prev_ts: value_to_i64(&fields[1]) as u64,
            prev_count: value_to_i64(&fields[2]) as u64,
        },
        _ => MetricsMeta::default(),
    };

    let data = match &items[1] {
        redis::Value::Array(points) => points.iter().map(value_to_i64).collect(),
        _ => Vec::new(),
    };

    let count = value_to_i64(&items[2]) as u64;

    Ok(Metrics { meta, data, count })
}
