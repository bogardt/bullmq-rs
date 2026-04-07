use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use redis::aio::ConnectionManager;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{BullmqError, BullmqResult};
use crate::queue_events::{QueueEvent, QueueEvents};
use crate::scripts::commands::key as build_key;
use crate::scripts::ScriptLoader;
use crate::types::{JobOptions, JobState};

/// Returns the current time as milliseconds since the Unix epoch.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Connection context injected by Queue/Worker. Enables active-handle methods
/// on Job instances (update_progress, log, change_delay, etc.).
pub(crate) struct JobContext {
    pub conn: ConnectionManager,
    pub scripts: Arc<ScriptLoader>,
    pub prefix: String,
    pub queue_name: String,
}

impl std::fmt::Debug for JobContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobContext")
            .field("prefix", &self.prefix)
            .field("queue_name", &self.queue_name)
            .finish_non_exhaustive()
    }
}

/// A job with a typed data payload.
///
/// Jobs are created by [`Queue::add`](crate::Queue::add) and processed by
/// [`Worker::start`](crate::Worker::start). The `data` field contains your
/// custom payload, which must implement `Serialize` and `DeserializeOwned`.
///
/// Field names in the Redis hash match BullMQ Node.js v5.x exactly for
/// wire compatibility (camelCase, abbreviations, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job<T> {
    /// Unique job identifier.
    pub id: String,
    /// Job name (used for categorization).
    pub name: String,
    /// The job payload.
    pub data: T,
    /// Current lifecycle state.
    pub state: JobState,
    /// Original creation options, serialized as JSON in the Redis hash.
    pub opts: JobOptions,
    /// When the job was created (ms since Unix epoch).
    pub timestamp: u64,
    /// Priority (lower = higher priority).
    pub priority: u32,
    /// Delay in milliseconds before the job becomes available.
    pub delay: u64,
    /// Number of attempts made so far. Maps to "atm" in the Redis hash.
    pub attempts_made: u32,
    /// Number of attempts started. Maps to "ats" in the Redis hash.
    pub attempts_started: u32,
    /// Job progress (flexible JSON value).
    pub progress: Option<serde_json::Value>,
    /// When the job started being processed (ms since Unix epoch).
    pub processed_on: Option<u64>,
    /// When the job finished — completed or failed (ms since Unix epoch).
    pub finished_on: Option<u64>,
    /// Reason for failure, if the job failed.
    pub failed_reason: Option<String>,
    /// Stack trace entries from failed processing attempts.
    pub stacktrace: Vec<String>,
    /// Return value from the handler (serialized as JSON).
    pub return_value: Option<serde_json::Value>,
    /// Identifier of the worker that processed this job. Maps to "pb" in the Redis hash.
    pub processed_by: Option<String>,
    /// Connection context injected by Queue/Worker. None for manually-created jobs.
    #[serde(skip)]
    pub(crate) ctx: Option<Arc<JobContext>>,
    /// Lock token assigned by the worker. Used by active-handle methods.
    #[serde(skip)]
    pub(crate) lock_token: Option<String>,
}

impl<T: Serialize + DeserializeOwned> Job<T> {
    /// Create a new job with the given name, data, and options.
    ///
    /// The job is always created in `JobState::Wait` — Lua scripts handle
    /// actual placement into the correct Redis data structure.
    pub fn new(id: String, name: String, data: T, opts: Option<JobOptions>) -> Self {
        let opts = opts.unwrap_or_default();
        let priority = opts.priority.unwrap_or(0);
        let delay = opts.delay.map(|d| d.as_millis() as u64).unwrap_or(0);

        Job {
            id,
            name,
            data,
            state: JobState::Wait,
            opts,
            timestamp: now_ms(),
            priority,
            delay,
            attempts_made: 0,
            attempts_started: 0,
            progress: None,
            processed_on: None,
            finished_on: None,
            failed_reason: None,
            stacktrace: Vec::new(),
            return_value: None,
            processed_by: None,
            ctx: None,
            lock_token: None,
        }
    }

    /// Serialize the job into a list of (field, value) pairs for Redis `HSET`.
    ///
    /// The field names match BullMQ Node.js v5.x exactly:
    /// - `atm` for attempts_made
    /// - `ats` for attempts_started
    /// - `processedOn`, `finishedOn`, `failedReason` (camelCase)
    /// - `returnvalue` (all lowercase)
    /// - `pb` for processed_by
    pub fn to_redis_hash(&self) -> BullmqResult<Vec<(String, String)>> {
        let data_json = serde_json::to_string(&self.data)?;
        let opts_json = serde_json::to_string(&self.opts)?;

        let mut fields = vec![
            ("name".into(), self.name.clone()),
            ("data".into(), data_json),
            ("opts".into(), opts_json),
            ("timestamp".into(), self.timestamp.to_string()),
            ("delay".into(), self.delay.to_string()),
            ("priority".into(), self.priority.to_string()),
            ("atm".into(), self.attempts_made.to_string()),
            ("ats".into(), self.attempts_started.to_string()),
        ];

        if let Some(ref progress) = self.progress {
            fields.push(("progress".into(), serde_json::to_string(progress)?));
        }
        if let Some(processed_on) = self.processed_on {
            fields.push(("processedOn".into(), processed_on.to_string()));
        }
        if let Some(finished_on) = self.finished_on {
            fields.push(("finishedOn".into(), finished_on.to_string()));
        }
        if let Some(ref reason) = self.failed_reason {
            fields.push(("failedReason".into(), serde_json::to_string(reason)?));
        }
        if let Some(ref val) = self.return_value {
            fields.push(("returnvalue".into(), serde_json::to_string(val)?));
        }
        if !self.stacktrace.is_empty() {
            fields.push((
                "stacktrace".into(),
                serde_json::to_string(&self.stacktrace)?,
            ));
        }
        if let Some(ref pb) = self.processed_by {
            fields.push(("pb".into(), pb.clone()));
        }

        Ok(fields)
    }

    /// Deserialize a job from a Redis hash (field-value map).
    ///
    /// Reads BullMQ Node.js field names and tolerates unknown fields
    /// (e.g., `stc` for stalled counter, managed by Lua scripts).
    pub fn from_redis_hash(id: &str, map: &HashMap<String, String>) -> BullmqResult<Self> {
        let get = |key: &str| -> BullmqResult<&String> {
            map.get(key)
                .ok_or_else(|| BullmqError::Other(format!("Missing field '{}' in job hash", key)))
        };

        let data: T = serde_json::from_str(get("data")?)?;

        let opts: JobOptions = match map.get("opts") {
            Some(s) => serde_json::from_str(s)?,
            None => JobOptions::default(),
        };

        let timestamp: u64 = get("timestamp")?
            .parse()
            .map_err(|_| BullmqError::Other("Invalid timestamp".into()))?;
        let priority: u32 = map
            .get("priority")
            .map(|s| s.parse())
            .transpose()
            .map_err(|_| BullmqError::Other("Invalid priority".into()))?
            .unwrap_or(0);
        let delay: u64 = map
            .get("delay")
            .map(|s| s.parse())
            .transpose()
            .map_err(|_| BullmqError::Other("Invalid delay".into()))?
            .unwrap_or(0);
        let attempts_made: u32 = map
            .get("atm")
            .map(|s| s.parse())
            .transpose()
            .map_err(|_| BullmqError::Other("Invalid atm".into()))?
            .unwrap_or(0);
        let attempts_started: u32 = map
            .get("ats")
            .map(|s| s.parse())
            .transpose()
            .map_err(|_| BullmqError::Other("Invalid ats".into()))?
            .unwrap_or(0);

        let progress = map
            .get("progress")
            .map(|s| serde_json::from_str(s))
            .transpose()?;
        let processed_on = map
            .get("processedOn")
            .map(|s| s.parse::<u64>())
            .transpose()
            .map_err(|_| BullmqError::Other("Invalid processedOn".into()))?;
        let finished_on = map
            .get("finishedOn")
            .map(|s| s.parse::<u64>())
            .transpose()
            .map_err(|_| BullmqError::Other("Invalid finishedOn".into()))?;
        let failed_reason = map.get("failedReason").map(|s| {
            serde_json::from_str(s).unwrap_or_else(|_| s.clone())
        });
        let return_value = map
            .get("returnvalue")
            .map(|s| serde_json::from_str(s))
            .transpose()?;
        let stacktrace: Vec<String> = map
            .get("stacktrace")
            .map(|s| serde_json::from_str(s))
            .transpose()?
            .unwrap_or_default();
        let processed_by = map.get("pb").cloned();

        // Determine state: prefer explicit "state" field, otherwise infer Wait.
        let state: JobState = match map.get("state") {
            Some(s) => s.parse().map_err(|e: String| BullmqError::Other(e))?,
            None => JobState::Wait,
        };

        Ok(Job {
            id: id.to_string(),
            name: get("name")?.clone(),
            data,
            state,
            opts,
            timestamp,
            priority,
            delay,
            attempts_made,
            attempts_started,
            progress,
            processed_on,
            finished_on,
            failed_reason,
            stacktrace,
            return_value,
            processed_by,
            ctx: None,
            lock_token: None,
        })
    }

    /// Get the connection context, or error if this job was created without one.
    fn ctx(&self) -> BullmqResult<&JobContext> {
        self.ctx.as_ref().map(|arc| arc.as_ref()).ok_or_else(|| {
            BullmqError::Other(
                "Job has no connection context (created outside a Queue/Worker)".into(),
            )
        })
    }

    /// Build a Redis key for this job's queue.
    fn queue_key(&self, suffix: &str) -> BullmqResult<String> {
        let ctx = self.ctx()?;
        Ok(build_key(&ctx.prefix, &ctx.queue_name, suffix))
    }

    /// Update the job's progress in Redis and locally.
    pub async fn update_progress(&mut self, progress: serde_json::Value) -> BullmqResult<()> {
        let ctx = self.ctx()?;
        let mut conn = ctx.conn.clone();
        let job_key = self.queue_key(&self.id)?;
        let events_key = self.queue_key("events")?;

        // Check job still exists to avoid creating orphan hashes
        let exists: bool = redis::cmd("EXISTS")
            .arg(&job_key)
            .query_async(&mut conn)
            .await?;
        if !exists {
            return Err(BullmqError::JobNotFound(self.id.clone()));
        }

        let progress_json = serde_json::to_string(&progress)?;

        redis::cmd("HSET")
            .arg(&job_key)
            .arg("progress")
            .arg(&progress_json)
            .query_async::<i64>(&mut conn)
            .await?;

        redis::cmd("XADD")
            .arg(&events_key)
            .arg("MAXLEN")
            .arg("~")
            .arg(10_000u64)
            .arg("*")
            .arg("event")
            .arg("progress")
            .arg("jobId")
            .arg(&self.id)
            .arg("data")
            .arg(&progress_json)
            .query_async::<String>(&mut conn)
            .await?;

        self.progress = Some(progress);
        Ok(())
    }

    /// Append a log line to this job's log list in Redis.
    ///
    /// Returns the total log count after insertion.
    pub async fn log(&self, row: &str) -> BullmqResult<u64> {
        let ctx = self.ctx()?;
        let mut conn = ctx.conn.clone();
        crate::scripts::commands::add_log::add_log(
            &ctx.scripts,
            &mut conn,
            &ctx.prefix,
            &ctx.queue_name,
            &self.id,
            row,
            0,
        )
        .await
    }

    /// Update the job's data payload in Redis and locally.
    pub async fn update_data(&mut self, data: T) -> BullmqResult<()> {
        let ctx = self.ctx()?;
        let mut conn = ctx.conn.clone();
        let job_key = self.queue_key(&self.id)?;

        let data_json = serde_json::to_string(&data)?;

        redis::cmd("HSET")
            .arg(&job_key)
            .arg("data")
            .arg(&data_json)
            .query_async::<i64>(&mut conn)
            .await?;

        self.data = data;
        Ok(())
    }

    /// Delete all log entries for this job from Redis.
    pub async fn clear_logs(&self) -> BullmqResult<()> {
        let ctx = self.ctx()?;
        let mut conn = ctx.conn.clone();
        let job_key = self.queue_key(&self.id)?;
        let logs_key = format!("{}:logs", job_key);

        redis::cmd("DEL")
            .arg(&logs_key)
            .query_async::<i64>(&mut conn)
            .await?;

        Ok(())
    }

    /// Query Redis to determine the current state of this job.
    ///
    /// Checks key membership in BullMQ order: completed, failed, delayed,
    /// active, wait, paused, prioritized, waiting-children.
    /// Lists use LPOS (O(N), Redis 6.0.6+), sorted sets use ZSCORE (O(log N)).
    pub async fn get_state(&mut self) -> BullmqResult<JobState> {
        let ctx = self.ctx()?;
        let mut conn = ctx.conn.clone();
        let job_id = self.id.clone();

        // Sorted sets: use ZSCORE
        let checks_zset: &[(JobState, &str)] = &[
            (JobState::Completed, "completed"),
            (JobState::Failed, "failed"),
            (JobState::Delayed, "delayed"),
        ];
        for (state, suffix) in checks_zset {
            let key = build_key(&ctx.prefix, &ctx.queue_name, suffix);
            let score: Option<f64> = redis::cmd("ZSCORE")
                .arg(&key)
                .arg(&job_id)
                .query_async(&mut conn)
                .await?;
            if score.is_some() {
                self.state = *state;
                return Ok(*state);
            }
        }

        // Lists: use LPOS (Redis 6.0.6+)
        let checks_list: &[(JobState, &str)] = &[
            (JobState::Active, "active"),
            (JobState::Wait, "wait"),
            (JobState::Paused, "paused"),
        ];
        for (state, suffix) in checks_list {
            let key = build_key(&ctx.prefix, &ctx.queue_name, suffix);
            let pos: redis::Value = redis::cmd("LPOS")
                .arg(&key)
                .arg(&job_id)
                .query_async(&mut conn)
                .await?;
            if !matches!(pos, redis::Value::Nil) {
                self.state = *state;
                return Ok(*state);
            }
        }

        // Sorted set: prioritized
        {
            let key = build_key(&ctx.prefix, &ctx.queue_name, "prioritized");
            let score: Option<f64> = redis::cmd("ZSCORE")
                .arg(&key)
                .arg(&job_id)
                .query_async(&mut conn)
                .await?;
            if score.is_some() {
                self.state = JobState::Prioritized;
                return Ok(JobState::Prioritized);
            }
        }

        // Sorted set: waiting-children
        {
            let key = build_key(&ctx.prefix, &ctx.queue_name, "waiting-children");
            let score: Option<f64> = redis::cmd("ZSCORE")
                .arg(&key)
                .arg(&job_id)
                .query_async(&mut conn)
                .await?;
            if score.is_some() {
                self.state = JobState::WaitingChildren;
                return Ok(JobState::WaitingChildren);
            }
        }

        Err(BullmqError::JobNotFound(self.id.clone()))
    }

    pub async fn is_completed(&mut self) -> BullmqResult<bool> {
        Ok(self.get_state().await? == JobState::Completed)
    }

    pub async fn is_failed(&mut self) -> BullmqResult<bool> {
        Ok(self.get_state().await? == JobState::Failed)
    }

    pub async fn is_delayed(&mut self) -> BullmqResult<bool> {
        Ok(self.get_state().await? == JobState::Delayed)
    }

    pub async fn is_active(&mut self) -> BullmqResult<bool> {
        Ok(self.get_state().await? == JobState::Active)
    }

    pub async fn is_waiting(&mut self) -> BullmqResult<bool> {
        Ok(self.get_state().await? == JobState::Wait)
    }

    pub async fn is_paused(&mut self) -> BullmqResult<bool> {
        Ok(self.get_state().await? == JobState::Paused)
    }

    pub async fn is_prioritized(&mut self) -> BullmqResult<bool> {
        Ok(self.get_state().await? == JobState::Prioritized)
    }

    /// Remove this job from the queue.
    ///
    /// Removes the job ID from all state lists/sets and deletes the job hash,
    /// lock key, and logs key. Not atomic (multiple Redis commands).
    pub async fn remove(&self) -> BullmqResult<()> {
        let ctx = self.ctx()?;
        let mut conn = ctx.conn.clone();
        let job_id = &self.id;

        // Remove from lists (LREM)
        for suffix in &["wait", "active", "paused"] {
            let key = build_key(&ctx.prefix, &ctx.queue_name, suffix);
            redis::cmd("LREM")
                .arg(&key)
                .arg(0i64)
                .arg(job_id)
                .query_async::<i64>(&mut conn)
                .await?;
        }

        // Remove from sorted sets (ZREM)
        for suffix in &["prioritized", "delayed", "completed", "failed"] {
            let key = build_key(&ctx.prefix, &ctx.queue_name, suffix);
            redis::cmd("ZREM")
                .arg(&key)
                .arg(job_id)
                .query_async::<i64>(&mut conn)
                .await?;
        }

        // Delete the job hash, lock key, and logs key
        let job_key = self.queue_key(job_id)?;
        let lock_key = format!("{}:lock", job_key);
        let logs_key = format!("{}:logs", job_key);
        redis::cmd("DEL")
            .arg(&job_key)
            .arg(&lock_key)
            .arg(&logs_key)
            .query_async::<i64>(&mut conn)
            .await?;

        Ok(())
    }

    /// Move this active job back to delayed with a new delay.
    ///
    /// Only valid when the job is active and has a lock token (inside a worker handler).
    pub async fn change_delay(&mut self, delay_ms: u64) -> BullmqResult<()> {
        if self.state != JobState::Active {
            return Err(BullmqError::Other(format!(
                "change_delay requires job to be active, but job {} is in state {}",
                self.id, self.state
            )));
        }

        let token = self.lock_token.as_deref().ok_or_else(|| {
            BullmqError::Other(
                "change_delay requires a lock token (only available in worker handler)".into(),
            )
        })?;

        let ctx = self.ctx()?;
        let mut conn = ctx.conn.clone();
        let timestamp = now_ms();
        let delayed_timestamp = timestamp + delay_ms;

        crate::scripts::commands::move_to_delayed::move_to_delayed(
            &ctx.scripts,
            &mut conn,
            &ctx.prefix,
            &ctx.queue_name,
            &self.id,
            token,
            timestamp,
            delayed_timestamp,
            10_000,
            self.attempts_made,
        )
        .await?;

        self.delay = delay_ms;
        Ok(())
    }

    /// Change the priority of this job.
    ///
    /// If priority > 0, moves job to the prioritized sorted set.
    /// If priority == 0, moves job back to the wait list.
    pub async fn change_priority(&mut self, priority: u32) -> BullmqResult<()> {
        let ctx = self.ctx()?;
        let mut conn = ctx.conn.clone();
        let timestamp = now_ms();

        crate::scripts::commands::change_priority::change_priority(
            &ctx.scripts,
            &mut conn,
            &ctx.prefix,
            &ctx.queue_name,
            &self.id,
            priority,
            10_000,
            timestamp,
        )
        .await?;

        self.priority = priority;
        self.opts.priority = if priority > 0 { Some(priority) } else { None };
        self.state = if priority > 0 {
            JobState::Prioritized
        } else {
            JobState::Wait
        };
        Ok(())
    }

    /// Promote this job from delayed to wait (or prioritized if priority > 0).
    pub async fn promote(&mut self) -> BullmqResult<()> {
        let ctx = self.ctx()?;
        let mut conn = ctx.conn.clone();
        let timestamp = now_ms();

        crate::scripts::commands::promote::promote(
            &ctx.scripts,
            &mut conn,
            &ctx.prefix,
            &ctx.queue_name,
            &self.id,
            10_000,
            timestamp,
        )
        .await?;

        self.state = JobState::Wait;
        self.delay = 0;
        Ok(())
    }

    /// Retry this active job by moving it back to wait or prioritized.
    ///
    /// Only works on active jobs with a lock token (inside a worker handler).
    pub async fn retry(&self) -> BullmqResult<()> {
        let token = self.lock_token.as_deref().ok_or_else(|| {
            BullmqError::Other(
                "retry requires a lock token (only available in worker handler)".into(),
            )
        })?;

        let ctx = self.ctx()?;
        let mut conn = ctx.conn.clone();

        crate::scripts::commands::retry_job::retry_job(
            &ctx.scripts,
            &mut conn,
            &ctx.prefix,
            &ctx.queue_name,
            &self.id,
            token,
            "RPUSH",
            10_000,
            self.attempts_made,
            self.priority,
        )
        .await
    }

    /// Wait until this job is finished (completed or failed).
    ///
    /// Subscribes to the QueueEvents broadcast channel and filters for
    /// matching Completed/Failed events. Also checks Redis immediately
    /// for the race condition where the job already finished before
    /// subscribing.
    pub async fn wait_until_finished(
        &self,
        queue_events: &QueueEvents,
        ttl: Option<Duration>,
    ) -> BullmqResult<serde_json::Value> {
        let ctx = self.ctx()?;
        let mut conn = ctx.conn.clone();
        let job_id = self.id.clone();

        // 1. Subscribe first (before checking Redis) to avoid missing events.
        let mut rx = queue_events.subscribe();

        // 2. Race condition guard: check if already finished.
        if let Some(result) = self
            .check_finished(&mut conn, &ctx.prefix, &ctx.queue_name, &job_id)
            .await?
        {
            return result;
        }

        // 3. Wait for matching event with optional timeout.
        let wait_fut = async {
            loop {
                match rx.recv().await {
                    Ok((
                        QueueEvent::Completed {
                            job_id: eid,
                            return_value,
                        },
                        _,
                    )) if eid == job_id => {
                        return Ok(return_value);
                    }
                    Ok((
                        QueueEvent::Failed {
                            job_id: eid,
                            reason,
                        },
                        _,
                    )) if eid == job_id => {
                        return Err(BullmqError::Other(reason));
                    }
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // Missed events — fall back to Redis check.
                        if let Some(result) = self
                            .check_finished(&mut conn, &ctx.prefix, &ctx.queue_name, &job_id)
                            .await?
                        {
                            return result;
                        }
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Err(BullmqError::Other("QueueEvents closed".into()));
                    }
                }
            }
        };

        match ttl {
            Some(duration) => tokio::time::timeout(duration, wait_fut)
                .await
                .unwrap_or(Err(BullmqError::Other("Job wait timed out".into()))),
            None => wait_fut.await,
        }
    }

    /// Check Redis to see if this job is already in completed or failed state.
    async fn check_finished(
        &self,
        conn: &mut ConnectionManager,
        prefix: &str,
        queue_name: &str,
        job_id: &str,
    ) -> BullmqResult<Option<BullmqResult<serde_json::Value>>> {
        let completed_key = build_key(prefix, queue_name, "completed");
        let failed_key = build_key(prefix, queue_name, "failed");
        let job_key = build_key(prefix, queue_name, job_id);

        // Check completed
        let score: Option<f64> = redis::cmd("ZSCORE")
            .arg(&completed_key)
            .arg(job_id)
            .query_async(conn)
            .await?;
        if score.is_some() {
            let rv: Option<String> = redis::cmd("HGET")
                .arg(&job_key)
                .arg("returnvalue")
                .query_async(conn)
                .await?;
            let value = rv
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(serde_json::Value::Null);
            return Ok(Some(Ok(value)));
        }

        // Check failed
        let score: Option<f64> = redis::cmd("ZSCORE")
            .arg(&failed_key)
            .arg(job_id)
            .query_async(conn)
            .await?;
        if score.is_some() {
            let reason: Option<String> = redis::cmd("HGET")
                .arg(&job_key)
                .arg("failedReason")
                .query_async(conn)
                .await?;
            // failedReason is stored as a plain string in Redis, not JSON-encoded.
            let reason = reason.unwrap_or_else(|| "Unknown failure".to_string());
            return Ok(Some(Err(BullmqError::Other(reason))));
        }

        Ok(None)
    }

    /// Get this job's dependencies.
    ///
    /// **Not yet implemented.** Requires Flows subsystem.
    pub async fn get_dependencies(&self) -> BullmqResult<serde_json::Value> {
        Err(BullmqError::NotImplemented(
            "getDependencies requires Flows (not yet implemented)".into(),
        ))
    }

    /// Get the return values of this job's children.
    ///
    /// **Not yet implemented.** Requires Flows subsystem.
    pub async fn get_children_values(&self) -> BullmqResult<serde_json::Value> {
        Err(BullmqError::NotImplemented(
            "getChildrenValues requires Flows (not yet implemented)".into(),
        ))
    }
}
