pub(crate) mod add_delayed_job;
pub(crate) mod add_job_scheduler;
pub(crate) mod add_log;
pub(crate) mod add_parent_job;
pub(crate) mod add_prioritized_job;
pub(crate) mod add_standard_job;
pub(crate) mod change_priority;
pub(crate) mod clean_jobs_in_set;
pub(crate) mod extend_lock;
pub(crate) mod extend_locks;
pub(crate) mod get_job_scheduler;
pub(crate) mod get_metrics;
pub(crate) mod move_job_from_active_to_wait;
pub(crate) mod move_jobs_to_wait;
pub(crate) mod move_stalled_jobs_to_wait;
pub(crate) mod move_to_active;
pub(crate) mod move_to_delayed;
pub(crate) mod move_to_finished;
pub(crate) mod obliterate;
pub(crate) mod pause;
pub(crate) mod promote;
pub(crate) mod remove_job;
pub(crate) mod remove_job_scheduler;
pub(crate) mod retry_job;
pub(crate) mod update_job_scheduler;

/// Build a Redis key: `{prefix}:{queue_name}:{suffix}`.
pub(crate) fn key(prefix: &str, queue_name: &str, suffix: &str) -> String {
    format!("{}:{}:{}", prefix, queue_name, suffix)
}
