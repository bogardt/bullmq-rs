use redis::aio::ConnectionManager;

use crate::error::BullmqResult;
use crate::scripts::ScriptLoader;

use super::key;

/// Pause or resume a queue.
///
/// When `pause` is `true`, jobs are moved from wait to paused and the queue is marked paused.
/// When `pause` is `false`, jobs are moved from paused to wait and the queue is resumed.
pub(crate) async fn pause_queue(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    pause: bool,
    max_events: u64,
) -> BullmqResult<()> {
    let keys = vec![
        key(prefix, queue_name, "wait"),
        key(prefix, queue_name, "paused"),
        key(prefix, queue_name, "meta"),
        key(prefix, queue_name, "events"),
        key(prefix, queue_name, "delayed"),
        key(prefix, queue_name, "prioritized"),
        key(prefix, queue_name, "marker"),
    ];
    let action = if pause { "pause" } else { "resume" };
    let args: Vec<Vec<u8>> = vec![
        action.as_bytes().to_vec(),
        max_events.to_string().into_bytes(),
    ];

    loader.invoke("pause", conn, &keys, &args).await?;
    Ok(())
}
