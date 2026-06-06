use redis::aio::ConnectionManager;

use crate::error::{BullmqError, BullmqResult};
use crate::scripts::ScriptLoader;

use super::key;

/// Extend the lock on an active job.
///
/// Returns `Ok(())` on success, `Err(LockMismatch)` if the token doesn't match.
pub(crate) async fn extend_lock(
    loader: &ScriptLoader,
    conn: &mut ConnectionManager,
    prefix: &str,
    queue_name: &str,
    job_id: &str,
    token: &str,
    lock_duration: u64,
) -> BullmqResult<()> {
    let job_key = format!("{}:{}:{}", prefix, queue_name, job_id);
    let lock_key = format!("{}:lock", job_key);

    let keys = vec![lock_key, key(prefix, queue_name, "stalled")];
    let args: Vec<Vec<u8>> = vec![
        token.as_bytes().to_vec(),
        lock_duration.to_string().into_bytes(),
        job_id.as_bytes().to_vec(),
    ];

    let result = loader.invoke("extendLock", conn, &keys, &args).await?;

    match result {
        redis::Value::Int(code) => {
            if code == 0 {
                Ok(())
            } else {
                Err(BullmqError::LockMismatch)
            }
        }
        _ => Ok(()),
    }
}
