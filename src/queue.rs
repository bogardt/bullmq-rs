use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use std::sync::Arc;

use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::connection::RedisConnection;
use crate::error::{BullmqError, BullmqResult};
use crate::job::{Job, JobContext};
use crate::scripts::commands::{
    add_delayed_job, add_log, add_prioritized_job, add_standard_job, pause,
};
use crate::scripts::ScriptLoader;
use crate::types::{JobOptions, JobState, DEFAULT_MAX_EVENTS};

/// A typed job queue backed by Redis.
///
/// Use [`QueueBuilder`] to create a queue instance.
///
/// # Example
/// ```rust,no_run
/// use bullmq_rs::{QueueBuilder, RedisConnection, JobOptions};
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Serialize, Deserialize)]
/// struct MyJob { url: String }
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let queue = QueueBuilder::new("downloads")
///     .connection(RedisConnection::new("redis://127.0.0.1:6379"))
///     .build::<MyJob>()
///     .await?;
///
/// queue.add("fetch", MyJob { url: "https://example.com".into() }, None).await?;
/// # Ok(())
/// # }
/// ```
pub struct Queue<T> {
    pub(crate) name: String,
    pub(crate) prefix: String,
    pub(crate) conn: ConnectionManager,
    pub(crate) scripts: Arc<ScriptLoader>,
    _phantom: PhantomData<T>,
}

impl<T: Serialize + DeserializeOwned + Send + Sync + 'static> Queue<T> {
    /// Build a JobContext from this queue's connection info.
    fn job_context(&self) -> Arc<JobContext> {
        Arc::new(JobContext {
            conn: self.conn.clone(),
            scripts: self.scripts.clone(),
            prefix: self.prefix.clone(),
            queue_name: self.name.clone(),
        })
    }

    fn normalize_job_states(states: &[JobState]) -> Vec<JobState> {
        let requested = if states.is_empty() {
            vec![
                JobState::Wait,
                JobState::Active,
                JobState::Delayed,
                JobState::Prioritized,
                JobState::Completed,
                JobState::Failed,
                JobState::WaitingChildren,
            ]
        } else {
            states.to_vec()
        };

        let mut normalized = Vec::new();
        let mut seen = HashSet::new();

        for state in requested {
            if seen.insert(state) {
                normalized.push(state);
            }

            if state == JobState::Wait && seen.insert(JobState::Paused) {
                normalized.push(JobState::Paused);
            }
        }

        normalized
    }

    async fn get_job_ids(
        &self,
        state: JobState,
        start: i64,
        end: i64,
        asc: bool,
    ) -> BullmqResult<Vec<String>> {
        let mut conn = self.conn.clone();

        match state {
            JobState::Wait | JobState::Active | JobState::Paused => {
                let mut ids: Vec<String> = if asc {
                    let modified_start = if start == -1 { 0 } else { -(start + 1) };
                    let modified_end = if end == -1 { 0 } else { -(end + 1) };

                    redis::cmd("LRANGE")
                        .arg(self.key(&state.to_string()))
                        .arg(modified_end)
                        .arg(modified_start)
                        .query_async(&mut conn)
                        .await?
                } else {
                    redis::cmd("LRANGE")
                        .arg(self.key(&state.to_string()))
                        .arg(start)
                        .arg(end)
                        .query_async(&mut conn)
                        .await?
                };

                if asc {
                    ids.reverse();
                }

                Ok(ids)
            }
            JobState::Prioritized
            | JobState::WaitingChildren
            | JobState::Delayed
            | JobState::Completed
            | JobState::Failed => {
                let command = if asc { "ZRANGE" } else { "ZREVRANGE" };
                let ids: Vec<String> = redis::cmd(command)
                    .arg(self.key(&state.to_string()))
                    .arg(start)
                    .arg(end)
                    .query_async(&mut conn)
                    .await?;

                Ok(ids)
            }
        }
    }

    /// Add a job to the queue.
    ///
    /// Dispatches to the appropriate Lua script based on job options:
    /// - `delay > 0` uses `addDelayedJob`
    /// - `priority > 0` uses `addPrioritizedJob`
    /// - otherwise uses `addStandardJob`
    ///
    /// Returns the created job with its assigned ID.
    pub async fn add(
        &self,
        name: &str,
        data: T,
        opts: Option<JobOptions>,
    ) -> BullmqResult<Job<T>> {
        let mut conn = self.conn.clone();

        // Generate job ID: custom if provided, otherwise INCR the id counter.
        let job_id: String = match opts.as_ref().and_then(|o| o.job_id.clone()) {
            Some(custom_id) => custom_id,
            None => {
                let id: i64 = redis::cmd("INCR")
                    .arg(self.key("id"))
                    .query_async(&mut conn)
                    .await?;
                id.to_string()
            }
        };

        let job = Job::new(job_id.clone(), name.to_string(), data, opts);

        let data_json = serde_json::to_string(&job.data)?;
        let opts_json = serde_json::to_string(&job.opts)?;
        let timestamp = job.timestamp;

        if job.delay > 0 {
            // Delayed job: compute the delayed timestamp.
            let delayed_timestamp = timestamp + job.delay;
            add_delayed_job::add_delayed_job(
                &self.scripts,
                &mut conn,
                &self.prefix,
                &self.name,
                &job_id,
                name,
                &data_json,
                timestamp,
                &opts_json,
                DEFAULT_MAX_EVENTS,
                delayed_timestamp,
            )
            .await?;
        } else if job.priority > 0 {
            // Prioritized job.
            add_prioritized_job::add_prioritized_job(
                &self.scripts,
                &mut conn,
                &self.prefix,
                &self.name,
                &job_id,
                name,
                &data_json,
                timestamp,
                &opts_json,
                DEFAULT_MAX_EVENTS,
            )
            .await?;
        } else {
            // Standard job.
            add_standard_job::add_standard_job(
                &self.scripts,
                &mut conn,
                &self.prefix,
                &self.name,
                &job_id,
                name,
                &data_json,
                timestamp,
                &opts_json,
                DEFAULT_MAX_EVENTS,
            )
            .await?;
        }

        let mut job = job;
        job.ctx = Some(self.job_context());

        Ok(job)
    }

    /// Get a job by its ID.
    pub async fn get_job(&self, job_id: &str) -> BullmqResult<Option<Job<T>>> {
        let mut conn = self.conn.clone();
        let job_key = self.key(job_id);

        let map: HashMap<String, String> = conn.hgetall(&job_key).await?;
        if map.is_empty() {
            return Ok(None);
        }

        let mut job = Job::from_redis_hash(job_id, &map)?;
        job.ctx = Some(self.job_context());
        Ok(Some(job))
    }

    /// Get the number of jobs in each state.
    ///
    /// Uses the correct Redis data structure for each BullMQ v5.x key:
    /// - `wait` and `paused` and `active` are Lists (LLEN)
    /// - `prioritized`, `delayed`, `completed`, `failed` are Sorted Sets (ZCARD)
    pub async fn get_job_counts(&self) -> BullmqResult<HashMap<JobState, u64>> {
        let mut conn = self.conn.clone();
        let mut counts = HashMap::new();

        // Lists: LLEN
        let wait: u64 = redis::cmd("LLEN")
            .arg(self.key("wait"))
            .query_async(&mut conn)
            .await?;
        let paused: u64 = redis::cmd("LLEN")
            .arg(self.key("paused"))
            .query_async(&mut conn)
            .await?;
        let active: u64 = redis::cmd("LLEN")
            .arg(self.key("active"))
            .query_async(&mut conn)
            .await?;

        // Sorted sets: ZCARD
        let prioritized: u64 = redis::cmd("ZCARD")
            .arg(self.key("prioritized"))
            .query_async(&mut conn)
            .await?;
        let delayed: u64 = redis::cmd("ZCARD")
            .arg(self.key("delayed"))
            .query_async(&mut conn)
            .await?;
        let completed: u64 = redis::cmd("ZCARD")
            .arg(self.key("completed"))
            .query_async(&mut conn)
            .await?;
        let failed: u64 = redis::cmd("ZCARD")
            .arg(self.key("failed"))
            .query_async(&mut conn)
            .await?;
        let waiting_children: u64 = redis::cmd("ZCARD")
            .arg(self.key("waiting-children"))
            .query_async(&mut conn)
            .await?;

        counts.insert(JobState::Wait, wait);
        counts.insert(JobState::Paused, paused);
        counts.insert(JobState::Active, active);
        counts.insert(JobState::Prioritized, prioritized);
        counts.insert(JobState::Delayed, delayed);
        counts.insert(JobState::Completed, completed);
        counts.insert(JobState::Failed, failed);
        counts.insert(JobState::WaitingChildren, waiting_children);

        Ok(counts)
    }

    /// Total jobs waiting to be processed.
    ///
    /// Matches BullMQ Node.js `Queue.count()` by including wait, paused,
    /// delayed, prioritized, and waiting-children.
    pub async fn count(&self) -> BullmqResult<u64> {
        let counts = self.get_job_counts().await?;

        Ok(counts.get(&JobState::Wait).copied().unwrap_or(0)
            + counts.get(&JobState::Paused).copied().unwrap_or(0)
            + counts.get(&JobState::Delayed).copied().unwrap_or(0)
            + counts.get(&JobState::Prioritized).copied().unwrap_or(0)
            + counts.get(&JobState::WaitingChildren).copied().unwrap_or(0))
    }

    /// Number of jobs in the waiting state, including paused jobs.
    pub async fn get_waiting_count(&self) -> BullmqResult<u64> {
        let counts = self.get_job_counts().await?;

        Ok(counts.get(&JobState::Wait).copied().unwrap_or(0)
            + counts.get(&JobState::Paused).copied().unwrap_or(0))
    }

    /// Number of jobs in the active state.
    pub async fn get_active_count(&self) -> BullmqResult<u64> {
        let counts = self.get_job_counts().await?;
        Ok(counts.get(&JobState::Active).copied().unwrap_or(0))
    }

    /// Number of jobs in the delayed state.
    pub async fn get_delayed_count(&self) -> BullmqResult<u64> {
        let counts = self.get_job_counts().await?;
        Ok(counts.get(&JobState::Delayed).copied().unwrap_or(0))
    }

    /// Number of jobs in the completed state.
    pub async fn get_completed_count(&self) -> BullmqResult<u64> {
        let counts = self.get_job_counts().await?;
        Ok(counts.get(&JobState::Completed).copied().unwrap_or(0))
    }

    /// Number of jobs in the failed state.
    pub async fn get_failed_count(&self) -> BullmqResult<u64> {
        let counts = self.get_job_counts().await?;
        Ok(counts.get(&JobState::Failed).copied().unwrap_or(0))
    }

    /// Number of jobs in the prioritized state.
    pub async fn get_prioritized_count(&self) -> BullmqResult<u64> {
        let counts = self.get_job_counts().await?;
        Ok(counts.get(&JobState::Prioritized).copied().unwrap_or(0))
    }

    /// Number of jobs in the waiting-children state.
    pub async fn get_waiting_children_count(&self) -> BullmqResult<u64> {
        let counts = self.get_job_counts().await?;
        Ok(counts.get(&JobState::WaitingChildren).copied().unwrap_or(0))
    }

    /// Get jobs from one or more states with BullMQ-compatible ordering.
    pub async fn get_jobs(
        &self,
        states: &[JobState],
        start: i64,
        end: i64,
        asc: bool,
    ) -> BullmqResult<Vec<Job<T>>> {
        let query_states = Self::normalize_job_states(states);
        let mut conn = self.conn.clone();
        let ctx = self.job_context();
        let mut seen = HashSet::new();
        let mut jobs = Vec::new();

        for state in query_states {
            let job_ids = self.get_job_ids(state, start, end, asc).await?;

            for job_id in job_ids {
                if !seen.insert(job_id.clone()) {
                    continue;
                }

                let map: HashMap<String, String> = conn.hgetall(self.key(&job_id)).await?;
                if map.is_empty() {
                    continue;
                }

                let mut job = Job::from_redis_hash(&job_id, &map)?;
                job.state = state;
                job.ctx = Some(ctx.clone());
                jobs.push(job);
            }
        }

        Ok(jobs)
    }

    /// Get waiting jobs, including paused jobs, oldest first.
    pub async fn get_waiting(&self, start: i64, end: i64) -> BullmqResult<Vec<Job<T>>> {
        self.get_jobs(&[JobState::Wait], start, end, true).await
    }

    /// Get active jobs, oldest first.
    pub async fn get_active(&self, start: i64, end: i64) -> BullmqResult<Vec<Job<T>>> {
        self.get_jobs(&[JobState::Active], start, end, true).await
    }

    /// Get delayed jobs, earliest scheduled first.
    pub async fn get_delayed(&self, start: i64, end: i64) -> BullmqResult<Vec<Job<T>>> {
        self.get_jobs(&[JobState::Delayed], start, end, true).await
    }

    /// Get prioritized jobs, highest priority first.
    pub async fn get_prioritized(&self, start: i64, end: i64) -> BullmqResult<Vec<Job<T>>> {
        self.get_jobs(&[JobState::Prioritized], start, end, true)
            .await
    }

    /// Get completed jobs, newest first.
    pub async fn get_completed(&self, start: i64, end: i64) -> BullmqResult<Vec<Job<T>>> {
        self.get_jobs(&[JobState::Completed], start, end, false)
            .await
    }

    /// Get failed jobs, newest first.
    pub async fn get_failed(&self, start: i64, end: i64) -> BullmqResult<Vec<Job<T>>> {
        self.get_jobs(&[JobState::Failed], start, end, false).await
    }

    /// Get waiting-children jobs, lowest score first.
    pub async fn get_waiting_children(&self, start: i64, end: i64) -> BullmqResult<Vec<Job<T>>> {
        self.get_jobs(&[JobState::WaitingChildren], start, end, true)
            .await
    }

    /// Remove a job by its ID from all state lists/sets and delete its hash,
    /// lock key, and logs key.
    pub async fn remove(&self, job_id: &str) -> BullmqResult<()> {
        let mut conn = self.conn.clone();

        // Remove from lists (LREM)
        redis::cmd("LREM")
            .arg(self.key("wait"))
            .arg(0i64)
            .arg(job_id)
            .query_async::<i64>(&mut conn)
            .await?;
        redis::cmd("LREM")
            .arg(self.key("active"))
            .arg(0i64)
            .arg(job_id)
            .query_async::<i64>(&mut conn)
            .await?;
        redis::cmd("LREM")
            .arg(self.key("paused"))
            .arg(0i64)
            .arg(job_id)
            .query_async::<i64>(&mut conn)
            .await?;

        // Remove from sorted sets (ZREM)
        redis::cmd("ZREM")
            .arg(self.key("prioritized"))
            .arg(job_id)
            .query_async::<i64>(&mut conn)
            .await?;
        redis::cmd("ZREM")
            .arg(self.key("delayed"))
            .arg(job_id)
            .query_async::<i64>(&mut conn)
            .await?;
        redis::cmd("ZREM")
            .arg(self.key("completed"))
            .arg(job_id)
            .query_async::<i64>(&mut conn)
            .await?;
        redis::cmd("ZREM")
            .arg(self.key("failed"))
            .arg(job_id)
            .query_async::<i64>(&mut conn)
            .await?;

        // Delete the job hash, lock key, and logs key
        let job_key = self.key(job_id);
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

    /// Remove all jobs from the queue (drain).
    ///
    /// Gets all job IDs from all state lists and sorted sets, deletes all
    /// job hashes + lock keys + log keys, then deletes all state keys.
    pub async fn drain(&self) -> BullmqResult<()> {
        let mut conn = self.conn.clone();

        // Get all job IDs from lists (LRANGE)
        let wait: Vec<String> = redis::cmd("LRANGE")
            .arg(self.key("wait"))
            .arg(0i64)
            .arg(-1i64)
            .query_async(&mut conn)
            .await?;
        let paused: Vec<String> = redis::cmd("LRANGE")
            .arg(self.key("paused"))
            .arg(0i64)
            .arg(-1i64)
            .query_async(&mut conn)
            .await?;
        let active: Vec<String> = redis::cmd("LRANGE")
            .arg(self.key("active"))
            .arg(0i64)
            .arg(-1i64)
            .query_async(&mut conn)
            .await?;

        // Get all job IDs from sorted sets (ZRANGE)
        let prioritized: Vec<String> = redis::cmd("ZRANGE")
            .arg(self.key("prioritized"))
            .arg(0i64)
            .arg(-1i64)
            .query_async(&mut conn)
            .await?;
        let delayed: Vec<String> = redis::cmd("ZRANGE")
            .arg(self.key("delayed"))
            .arg(0i64)
            .arg(-1i64)
            .query_async(&mut conn)
            .await?;
        let completed: Vec<String> = redis::cmd("ZRANGE")
            .arg(self.key("completed"))
            .arg(0i64)
            .arg(-1i64)
            .query_async(&mut conn)
            .await?;
        let failed: Vec<String> = redis::cmd("ZRANGE")
            .arg(self.key("failed"))
            .arg(0i64)
            .arg(-1i64)
            .query_async(&mut conn)
            .await?;

        // Collect all unique IDs
        let all_ids: Vec<&String> = wait
            .iter()
            .chain(paused.iter())
            .chain(active.iter())
            .chain(prioritized.iter())
            .chain(delayed.iter())
            .chain(completed.iter())
            .chain(failed.iter())
            .collect();

        // Delete all job hashes, lock keys, and log keys
        for id in &all_ids {
            let job_key = self.key(id);
            let lock_key = format!("{}:lock", job_key);
            let logs_key = format!("{}:logs", job_key);
            redis::cmd("DEL")
                .arg(&job_key)
                .arg(&lock_key)
                .arg(&logs_key)
                .query_async::<i64>(&mut conn)
                .await?;
        }

        // Delete all state keys and the ID counter
        redis::cmd("DEL")
            .arg(self.key("wait"))
            .arg(self.key("paused"))
            .arg(self.key("active"))
            .arg(self.key("prioritized"))
            .arg(self.key("delayed"))
            .arg(self.key("completed"))
            .arg(self.key("failed"))
            .arg(self.key("id"))
            .query_async::<i64>(&mut conn)
            .await?;

        Ok(())
    }

    /// Update the progress of a job.
    ///
    /// Accepts a flexible JSON value and also publishes a progress event
    /// to the queue's events stream via XADD.
    pub async fn update_progress(
        &self,
        job_id: &str,
        progress: serde_json::Value,
    ) -> BullmqResult<()> {
        let mut conn = self.conn.clone();
        let job_key = self.key(job_id);

        let exists: bool = redis::cmd("EXISTS")
            .arg(&job_key)
            .query_async(&mut conn)
            .await?;
        if !exists {
            return Err(BullmqError::JobNotFound(job_id.to_string()));
        }

        let progress_json = serde_json::to_string(&progress)?;

        // Update the progress field in the job hash
        redis::cmd("HSET")
            .arg(&job_key)
            .arg("progress")
            .arg(&progress_json)
            .query_async::<i64>(&mut conn)
            .await?;

        // Publish progress event to the events stream
        redis::cmd("XADD")
            .arg(self.key("events"))
            .arg("MAXLEN")
            .arg("~")
            .arg(DEFAULT_MAX_EVENTS)
            .arg("*")
            .arg("event")
            .arg("progress")
            .arg("jobId")
            .arg(job_id)
            .arg("data")
            .arg(&progress_json)
            .query_async::<String>(&mut conn)
            .await?;

        Ok(())
    }

    /// Pause the queue.
    ///
    /// Moves jobs from the wait list to paused and marks the queue as paused
    /// in the meta hash.
    pub async fn pause(&self) -> BullmqResult<()> {
        let mut conn = self.conn.clone();
        pause::pause_queue(
            &self.scripts,
            &mut conn,
            &self.prefix,
            &self.name,
            true,
            DEFAULT_MAX_EVENTS,
        )
        .await
    }

    /// Resume the queue.
    ///
    /// Moves jobs from paused back to wait and removes the paused marker
    /// from the meta hash.
    pub async fn resume(&self) -> BullmqResult<()> {
        let mut conn = self.conn.clone();
        pause::pause_queue(
            &self.scripts,
            &mut conn,
            &self.prefix,
            &self.name,
            false,
            DEFAULT_MAX_EVENTS,
        )
        .await
    }

    /// Check if the queue is currently paused.
    ///
    /// Returns `true` if the `paused` field exists in the queue's meta hash.
    pub async fn is_paused(&self) -> BullmqResult<bool> {
        let mut conn = self.conn.clone();
        let paused: bool = redis::cmd("HEXISTS")
            .arg(self.key("meta"))
            .arg("paused")
            .query_async(&mut conn)
            .await?;
        Ok(paused)
    }

    /// Add a log entry to a job's log list.
    ///
    /// Returns the current log count after insertion.
    pub async fn add_log(
        &self,
        job_id: &str,
        log_line: &str,
    ) -> BullmqResult<u64> {
        let mut conn = self.conn.clone();
        add_log::add_log(
            &self.scripts,
            &mut conn,
            &self.prefix,
            &self.name,
            job_id,
            log_line,
            // 0 means unlimited (Lua script checks > 0 before LTRIM).
            0,
        )
        .await
    }

    /// Get log entries for a job.
    ///
    /// Returns log lines from `start` to `end` (inclusive, 0-based).
    /// Use `start=0, end=-1` to get all logs.
    pub async fn get_logs(
        &self,
        job_id: &str,
        start: i64,
        end: i64,
    ) -> BullmqResult<Vec<String>> {
        let mut conn = self.conn.clone();
        let job_key = self.key(job_id);
        let logs_key = format!("{}:logs", job_key);

        let logs: Vec<String> = redis::cmd("LRANGE")
            .arg(&logs_key)
            .arg(start)
            .arg(end)
            .query_async(&mut conn)
            .await?;

        Ok(logs)
    }

    /// Get the queue name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Build a Redis key with the queue prefix.
    pub(crate) fn key(&self, suffix: &str) -> String {
        format!("{}:{}:{}", self.prefix, self.name, suffix)
    }
}

/// Builder for creating a [`Queue`].
pub struct QueueBuilder {
    name: String,
    connection: RedisConnection,
    prefix: String,
}

impl QueueBuilder {
    /// Create a new queue builder with the given queue name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            connection: RedisConnection::default(),
            prefix: "bull".to_string(),
        }
    }

    /// Set the Redis connection configuration.
    pub fn connection(mut self, conn: RedisConnection) -> Self {
        self.connection = conn;
        self
    }

    /// Set a custom key prefix (default: "bull").
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Build the queue, establishing the Redis connection and loading Lua scripts.
    pub async fn build<T: Serialize + DeserializeOwned + Send + Sync + 'static>(
        self,
    ) -> BullmqResult<Queue<T>> {
        let conn = self.connection.get_manager().await?;
        let scripts = Arc::new(ScriptLoader::new());
        Ok(Queue {
            name: self.name,
            prefix: self.prefix,
            conn,
            scripts,
            _phantom: PhantomData,
        })
    }
}
