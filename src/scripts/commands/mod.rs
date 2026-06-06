pub(crate) mod add_delayed_job;
pub(crate) mod add_log;
pub(crate) mod add_parent_job;
pub(crate) mod add_prioritized_job;
pub(crate) mod add_standard_job;
pub(crate) mod change_priority;
pub(crate) mod extend_lock;
pub(crate) mod move_stalled_jobs_to_wait;
pub(crate) mod move_to_active;
pub(crate) mod move_to_delayed;
pub(crate) mod move_to_finished;
pub(crate) mod pause;
pub(crate) mod promote;
pub(crate) mod retry_job;

/// Build a Redis key: `{prefix}:{queue_name}:{suffix}`.
pub(crate) fn key(prefix: &str, queue_name: &str, suffix: &str) -> String {
    format!("{}:{}:{}", prefix, queue_name, suffix)
}
