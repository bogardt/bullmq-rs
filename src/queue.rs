use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use std::sync::Arc;

use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::connection::RedisConnection;
use crate::error::{BullmqError, BullmqResult};
use crate::job::{cleanup_job, Job, JobContext};
use crate::scripts::commands::{
    add_delayed_job, add_job_scheduler, add_log, add_prioritized_job, add_standard_job,
    clean_jobs_in_set, extend_locks, get_job_scheduler, get_metrics, move_jobs_to_wait,
    move_to_active, obliterate, pause, remove_job_scheduler,
};
use crate::scripts::ScriptLoader;
use crate::types::{
    JobOptions, JobScheduler, JobSchedulerTemplate, JobState, Metrics, RepeatOptions,
    DEFAULT_MAX_EVENTS,
};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

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
    ///
    /// When [`JobOptions::deduplication`] is set and a deduplication key
    /// (`<prefix>:<queue>:de:<id>`) already exists, the add is deduplicated:
    /// no new job is created and the returned job carries the EXISTING job's
    /// ID (mirroring BullMQ Node, where `Queue.add` returns a job whose `id`
    /// is the deduplicated job's id).
    pub async fn add(&self, name: &str, data: T, opts: Option<JobOptions>) -> BullmqResult<Job<T>> {
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

        let mut job = Job::new(job_id, name.to_string(), data, opts);
        job.id = self.dispatch_add(&mut conn, &job).await?;
        job.ctx = Some(self.job_context());

        Ok(job)
    }

    /// Run the appropriate add script (delayed/prioritized/standard) for a job.
    ///
    /// Returns the id assigned by the script — the EXISTING job's id when the
    /// add was deduplicated, the job's own id otherwise.
    async fn dispatch_add(
        &self,
        conn: &mut ConnectionManager,
        job: &Job<T>,
    ) -> BullmqResult<String> {
        let data_json = serde_json::to_string(&job.data)?;
        let opts_json = serde_json::to_string(&job.opts)?;
        let timestamp = job.timestamp;

        let dedup_key = job
            .opts
            .deduplication
            .as_ref()
            .map(|d| self.key(&format!("de:{}", d.id)))
            .unwrap_or_default();

        let returned_id = if job.delay > 0 {
            // Delayed job: compute the delayed timestamp.
            let delayed_timestamp = timestamp + job.delay;
            add_delayed_job::add_delayed_job(
                &self.scripts,
                conn,
                &self.prefix,
                &self.name,
                &job.id,
                &job.name,
                &data_json,
                timestamp,
                &opts_json,
                DEFAULT_MAX_EVENTS,
                delayed_timestamp,
                &dedup_key,
            )
            .await?
        } else if job.priority > 0 {
            add_prioritized_job::add_prioritized_job(
                &self.scripts,
                conn,
                &self.prefix,
                &self.name,
                &job.id,
                &job.name,
                &data_json,
                timestamp,
                &opts_json,
                DEFAULT_MAX_EVENTS,
                &dedup_key,
            )
            .await?
        } else {
            add_standard_job::add_standard_job(
                &self.scripts,
                conn,
                &self.prefix,
                &self.name,
                &job.id,
                &job.name,
                &data_json,
                timestamp,
                &opts_json,
                DEFAULT_MAX_EVENTS,
                &dedup_key,
            )
            .await?
        };

        Ok(returned_id)
    }

    /// Add multiple jobs to the queue.
    ///
    /// Mirrors Node BullMQ `Queue.addBulk`: each job is a `(name, data, opts)`
    /// tuple following the same semantics as [`Queue::add`]. Auto-generated job
    /// IDs are reserved with a single `INCRBY` instead of one `INCR` per job.
    ///
    /// Returns the created jobs in input order with their assigned IDs.
    pub async fn add_bulk(
        &self,
        jobs: Vec<(String, T, Option<JobOptions>)>,
    ) -> BullmqResult<Vec<Job<T>>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }

        let mut conn = self.conn.clone();

        let auto_id_count = jobs
            .iter()
            .filter(|(_, _, opts)| opts.as_ref().and_then(|o| o.job_id.as_ref()).is_none())
            .count() as i64;

        let mut next_id: i64 = if auto_id_count > 0 {
            let last_id: i64 = redis::cmd("INCRBY")
                .arg(self.key("id"))
                .arg(auto_id_count)
                .query_async(&mut conn)
                .await?;
            last_id - auto_id_count + 1
        } else {
            0
        };

        let ctx = self.job_context();
        let mut created = Vec::with_capacity(jobs.len());

        for (name, data, opts) in jobs {
            let job_id = match opts.as_ref().and_then(|o| o.job_id.clone()) {
                Some(custom_id) => custom_id,
                None => {
                    let id = next_id;
                    next_id += 1;
                    id.to_string()
                }
            };

            let mut job = Job::new(job_id, name, data, opts);
            job.id = self.dispatch_add(&mut conn, &job).await?;
            job.ctx = Some(ctx.clone());
            created.push(job);
        }

        Ok(created)
    }

    /// Retry failed (or completed) jobs by moving them back to wait.
    ///
    /// Mirrors Node BullMQ `Queue.retryJobs`: loops the `moveJobsToWait`
    /// script, moving up to `count` jobs per iteration, until the cursor
    /// reaches 0. Only jobs finished at or before `timestamp` (default: now)
    /// are moved. `state` must be [`JobState::Failed`] or
    /// [`JobState::Completed`].
    pub async fn retry_jobs(
        &self,
        count: u64,
        state: JobState,
        timestamp: Option<u64>,
    ) -> BullmqResult<()> {
        if !matches!(state, JobState::Failed | JobState::Completed) {
            return Err(BullmqError::Other(format!(
                "retry_jobs only supports failed or completed states, got: {}",
                state
            )));
        }

        let timestamp = timestamp.unwrap_or_else(now_ms).to_string();
        self.move_jobs_to_wait(count, &state.to_string(), &timestamp)
            .await
    }

    /// Promote all delayed jobs to the wait list.
    ///
    /// Mirrors Node BullMQ `Queue.promoteJobs`: loops the `moveJobsToWait`
    /// script with the `delayed` state and an unbounded timestamp, moving up
    /// to `count` jobs per iteration, until the cursor reaches 0.
    pub async fn promote_jobs(&self, count: u64) -> BullmqResult<()> {
        // Node passes Number.MAX_VALUE so every delayed job matches.
        self.move_jobs_to_wait(count, "delayed", "1.7976931348623157e+308")
            .await
    }

    async fn move_jobs_to_wait(
        &self,
        count: u64,
        state: &str,
        timestamp: &str,
    ) -> BullmqResult<()> {
        // Node defaults count to 1000; count 0 would loop forever since the
        // script returns 1 whenever the remaining budget is exhausted.
        let count = if count == 0 { 1000 } else { count };
        let mut conn = self.conn.clone();

        loop {
            let cursor = move_jobs_to_wait::move_jobs_to_wait(
                &self.scripts,
                &mut conn,
                &self.prefix,
                &self.name,
                state,
                count,
                timestamp,
            )
            .await?;

            if cursor == 0 {
                return Ok(());
            }
        }
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
        cleanup_job(&mut conn, &self.prefix, &self.name, job_id).await
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
        let waiting_children: Vec<String> = redis::cmd("ZRANGE")
            .arg(self.key("waiting-children"))
            .arg(0i64)
            .arg(-1i64)
            .query_async(&mut conn)
            .await?;

        // Collect all unique IDs
        let all_ids: std::collections::HashSet<String> = wait
            .iter()
            .chain(paused.iter())
            .chain(active.iter())
            .chain(prioritized.iter())
            .chain(delayed.iter())
            .chain(completed.iter())
            .chain(failed.iter())
            .chain(waiting_children.iter())
            .cloned()
            .collect();

        for id in all_ids {
            cleanup_job(&mut conn, &self.prefix, &self.name, &id).await?;
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
            .arg(self.key("waiting-children"))
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

    /// Remove jobs in a given state older than the grace period.
    ///
    /// `grace` is the minimum age of the jobs to be removed, `limit` caps the
    /// number of removed jobs (`0` = unlimited), and `state` selects which
    /// set to clean: `Completed` (default in BullMQ), `Failed`, `Wait`,
    /// `Paused`, `Active`, `Delayed` or `Prioritized`.
    ///
    /// Returns the ids of the removed jobs.
    pub async fn clean(
        &self,
        grace: std::time::Duration,
        limit: u64,
        state: JobState,
    ) -> BullmqResult<Vec<String>> {
        if state == JobState::WaitingChildren {
            return Err(BullmqError::Other(
                "Cannot clean waiting-children jobs".to_string(),
            ));
        }

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let timestamp = now_ms.saturating_sub(grace.as_millis() as u64);
        let set_name = state.to_string();

        // Mirrors BullMQ: iterate in chunks of at most 10k removals per call.
        let max_count = if limit == 0 { u64::MAX } else { limit };
        let max_count_per_call = max_count.min(10_000);

        let mut conn = self.conn.clone();
        let mut deleted_job_ids = Vec::new();
        while (deleted_job_ids.len() as u64) < max_count {
            let job_ids = clean_jobs_in_set::clean_jobs_in_set(
                &self.scripts,
                &mut conn,
                &self.prefix,
                &self.name,
                &set_name,
                timestamp,
                max_count_per_call,
            )
            .await?;
            let batch_len = job_ids.len() as u64;
            deleted_job_ids.extend(job_ids);
            if batch_len < max_count_per_call {
                break;
            }
        }
        Ok(deleted_job_ids)
    }

    /// Completely destroy the queue and all of its contents.
    ///
    /// The queue is paused first, then all job data and queue keys are
    /// removed iteratively. Fails if the queue has active jobs, unless
    /// `force` is `true`.
    pub async fn obliterate(&self, force: bool) -> BullmqResult<()> {
        self.pause().await?;

        let mut conn = self.conn.clone();
        loop {
            let cursor = obliterate::obliterate(
                &self.scripts,
                &mut conn,
                &self.prefix,
                &self.name,
                1000,
                force,
            )
            .await?;
            if cursor == 0 {
                return Ok(());
            }
        }
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
    pub async fn add_log(&self, job_id: &str, log_line: &str) -> BullmqResult<u64> {
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
    pub async fn get_logs(&self, job_id: &str, start: i64, end: i64) -> BullmqResult<Vec<String>> {
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

    /// Get queue metrics for a finished state.
    ///
    /// `state` must be [`JobState::Completed`] or [`JobState::Failed`].
    /// `start` and `end` select a range of data points (`0` is the newest,
    /// `-1` the oldest), mirroring BullMQ's `Queue.getMetrics`.
    ///
    /// Metrics are only collected by workers configured with
    /// [`crate::WorkerBuilder::metrics`].
    pub async fn get_metrics(
        &self,
        state: JobState,
        start: i64,
        end: i64,
    ) -> BullmqResult<Metrics> {
        if state != JobState::Completed && state != JobState::Failed {
            return Err(BullmqError::Other(format!(
                "get_metrics only supports completed or failed states, got: {}",
                state
            )));
        }

        let mut conn = self.conn.clone();
        get_metrics::get_metrics(
            &self.scripts,
            &mut conn,
            &self.prefix,
            &self.name,
            &state.to_string(),
            start,
            end,
        )
        .await
    }

    /// Create or update a job scheduler (repeatable job) and schedule its
    /// next iteration.
    ///
    /// Mirrors Node BullMQ `Queue.upsertJobScheduler`. Exactly one of
    /// [`RepeatOptions::pattern`] or [`RepeatOptions::every`] must be set.
    /// When upserting over an existing scheduler, the pending iteration job
    /// is replaced (no orphan is left behind). After each iteration finishes,
    /// workers automatically schedule the following one until `limit` or
    /// `end_date` is reached, or the scheduler is removed.
    ///
    /// Returns the first iteration job, whose id has the shape
    /// `repeat:<scheduler_id>:<next_millis>`. Errors when the iteration
    /// `limit` or `end_date` is already exhausted (Node returns `undefined`
    /// in those cases).
    pub async fn upsert_job_scheduler(
        &self,
        scheduler_id: &str,
        repeat_opts: RepeatOptions,
        template: JobSchedulerTemplate<T>,
    ) -> BullmqResult<Job<T>> {
        if repeat_opts.pattern.is_some() && repeat_opts.every.is_some() {
            return Err(BullmqError::Other(
                "Both .pattern and .every options are defined for this repeatable job".into(),
            ));
        }
        if repeat_opts.pattern.is_none() && repeat_opts.every.is_none() {
            return Err(BullmqError::Other(
                "Either .pattern or .every options must be defined for this repeatable job".into(),
            ));
        }
        if repeat_opts.immediately && repeat_opts.start_date.is_some() {
            return Err(BullmqError::Other(
                "Both .immediately and .startDate options are defined for this repeatable job"
                    .into(),
            ));
        }

        let iteration_count = repeat_opts.count.map(|c| c + 1).unwrap_or(1);
        if let Some(limit) = repeat_opts.limit {
            if iteration_count > limit {
                return Err(BullmqError::Other(format!(
                    "Job scheduler {} reached its iteration limit",
                    scheduler_id
                )));
            }
        }

        let mut now = now_ms();
        if let Some(end_date) = repeat_opts.end_date {
            if now > end_date {
                return Err(BullmqError::Other(format!(
                    "Job scheduler {} is past its end date",
                    scheduler_id
                )));
            }
        }

        let template_opts = template.opts.unwrap_or_default();
        let prev_millis = template_opts.prev_millis.unwrap_or(0);
        if prev_millis > now {
            now = prev_millis;
        }

        let next_millis = match &repeat_opts.pattern {
            Some(pattern) => crate::repeat::next_pattern_millis(now, &repeat_opts, pattern)?
                .ok_or_else(|| {
                    BullmqError::Other(format!("No next occurrence for cron pattern '{}'", pattern))
                })?
                .max(now),
            // The Lua script computes nextMillis itself in 'every' mode.
            None => 0,
        };

        let job_name = template
            .name
            .clone()
            .unwrap_or_else(|| scheduler_id.to_string());
        let template_data_json = match &template.data {
            Some(data) => serde_json::to_string(data)?,
            None => "{}".to_string(),
        };

        let merged_opts = crate::repeat::build_iteration_opts(
            &template_opts,
            &repeat_opts,
            scheduler_id,
            iteration_count,
            if repeat_opts.pattern.is_some() {
                Some(next_millis)
            } else {
                None
            },
        );

        let scheduler_opts_json = scheduler_opts_json(&job_name, &repeat_opts)?;
        let template_opts_json = serde_json::to_string(&template_opts)?;
        let delayed_opts_json = serde_json::to_string(&merged_opts)?;

        let mut conn = self.conn.clone();
        let result = add_job_scheduler::add_job_scheduler(
            &self.scripts,
            &mut conn,
            &self.prefix,
            &self.name,
            scheduler_id,
            next_millis,
            &scheduler_opts_json,
            &template_data_json,
            &template_opts_json,
            &delayed_opts_json,
            now_ms(),
            None,
        )
        .await?;

        let data: T = match template.data {
            Some(data) => data,
            None => serde_json::from_str("{}").map_err(|_| {
                BullmqError::Other(
                    "JobSchedulerTemplate.data is required for this payload type".into(),
                )
            })?,
        };

        let mut opts = merged_opts;
        opts.job_id = Some(result.job_id.clone());
        opts.delay = Some(std::time::Duration::from_millis(result.delay));

        let mut job = Job::new(result.job_id, job_name, data, Some(opts));
        job.state = if result.delay > 0 {
            JobState::Delayed
        } else {
            JobState::Wait
        };
        job.repeat_job_key = Some(scheduler_id.to_string());
        job.ctx = Some(self.job_context());
        Ok(job)
    }

    /// Get a job scheduler's metadata, or `None` when it does not exist.
    pub async fn get_job_scheduler(&self, id: &str) -> BullmqResult<Option<JobScheduler>> {
        let mut conn = self.conn.clone();
        let Some((map, next)) = get_job_scheduler::get_job_scheduler(
            &self.scripts,
            &mut conn,
            &self.prefix,
            &self.name,
            id,
        )
        .await?
        else {
            return Ok(None);
        };
        if map.is_empty() {
            return Ok(None);
        }
        Ok(Some(scheduler_from_hash(id, &map, Some(next))))
    }

    /// List job schedulers from the repeat sorted set, ordered by next
    /// iteration timestamp (ascending). `start` and `end` are zero-based
    /// inclusive indexes (`0, -1` returns all).
    pub async fn get_job_schedulers(
        &self,
        start: i64,
        end: i64,
    ) -> BullmqResult<Vec<JobScheduler>> {
        let mut conn = self.conn.clone();
        let entries: Vec<(String, f64)> = redis::cmd("ZRANGE")
            .arg(self.key("repeat"))
            .arg(start)
            .arg(end)
            .arg("WITHSCORES")
            .query_async(&mut conn)
            .await?;

        let mut schedulers = Vec::with_capacity(entries.len());
        for (id, score) in entries {
            let map: HashMap<String, String> =
                conn.hgetall(self.key(&format!("repeat:{}", id))).await?;
            schedulers.push(scheduler_from_hash(&id, &map, Some(score as u64)));
        }
        Ok(schedulers)
    }

    /// Remove a job scheduler, its metadata, and its pending iteration job.
    ///
    /// Returns `true` when the scheduler existed and was removed.
    pub async fn remove_job_scheduler(&self, id: &str) -> BullmqResult<bool> {
        let mut conn = self.conn.clone();
        remove_job_scheduler::remove_job_scheduler(
            &self.scripts,
            &mut conn,
            &self.prefix,
            &self.name,
            id,
        )
        .await
    }

    /// Manually fetch the next job from the queue, moving it to active and
    /// acquiring a lock with the given `token` (mirrors `worker.getNextJob`
    /// in Node BullMQ, without blocking).
    ///
    /// The lock is held for 30 seconds. The caller is responsible for:
    /// - extending the lock while processing (see
    ///   [`Queue::extend_job_locks`]), and
    /// - finishing the job via [`Job::move_to_completed`] /
    ///   [`Job::move_to_failed`].
    ///
    /// If the job is neither finished nor its lock extended, it will be
    /// recovered by a worker's stalled-job checker after the lock expires.
    pub async fn get_next_job(&self, token: &str) -> BullmqResult<Option<Job<T>>> {
        let mut conn = self.conn.clone();
        let result = move_to_active::move_to_active(
            &self.scripts,
            &mut conn,
            &self.prefix,
            &self.name,
            token,
            30_000,
            now_ms(),
            DEFAULT_MAX_EVENTS,
            None,
        )
        .await?;

        let Some(job_id) = result.job_id else {
            return Ok(None);
        };
        let mut job = Job::from_redis_hash(&job_id, &result.job_data)?;
        job.state = JobState::Active;
        job.lock_token = Some(token.to_string());
        job.ctx = Some(self.job_context());
        Ok(Some(job))
    }

    /// Extend the locks of multiple jobs in one round trip (mirrors
    /// `scripts.extendLocks` in Node BullMQ). `job_ids` and `tokens` are
    /// matched by index and must have the same length.
    ///
    /// Returns the ids of the jobs whose lock could NOT be extended (missing
    /// lock or token mismatch); an empty vector means all succeeded.
    pub async fn extend_job_locks(
        &self,
        job_ids: &[String],
        tokens: &[String],
        duration: std::time::Duration,
    ) -> BullmqResult<Vec<String>> {
        if job_ids.len() != tokens.len() {
            return Err(BullmqError::Other(
                "extend_job_locks requires job_ids and tokens of the same length".into(),
            ));
        }
        let mut conn = self.conn.clone();
        extend_locks::extend_locks(
            &self.scripts,
            &mut conn,
            &self.prefix,
            &self.name,
            job_ids,
            tokens,
            duration.as_millis() as u64,
        )
        .await
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

/// Serialize the scheduler options stored in the `repeat:<id>` hash,
/// matching the fields Node passes to the addJobScheduler script.
fn scheduler_opts_json(name: &str, repeat: &RepeatOptions) -> BullmqResult<String> {
    let mut map = serde_json::Map::new();
    map.insert("name".into(), serde_json::Value::from(name));
    if let Some(start_date) = repeat.start_date {
        map.insert("startDate".into(), serde_json::Value::from(start_date));
    }
    if let Some(end_date) = repeat.end_date {
        map.insert("endDate".into(), serde_json::Value::from(end_date));
    }
    if let Some(ref tz) = repeat.tz {
        map.insert("tz".into(), serde_json::Value::from(tz.clone()));
    }
    if let Some(ref pattern) = repeat.pattern {
        map.insert("pattern".into(), serde_json::Value::from(pattern.clone()));
    }
    if let Some(every) = repeat.every {
        map.insert(
            "every".into(),
            serde_json::Value::from(every.as_millis() as u64),
        );
    }
    if let Some(limit) = repeat.limit {
        map.insert("limit".into(), serde_json::Value::from(limit));
    }
    // Node only forwards an explicit offset in 'every' mode.
    if repeat.every.is_some() {
        if let Some(offset) = repeat.offset {
            map.insert("offset".into(), serde_json::Value::from(offset));
        }
    }
    Ok(serde_json::to_string(&serde_json::Value::Object(map))?)
}

fn hash_num(map: &HashMap<String, String>, field: &str) -> Option<u64> {
    map.get(field)
        .and_then(|s| s.parse::<f64>().ok())
        .map(|f| f as u64)
}

/// Build a [`JobScheduler`] from the `repeat:<id>` hash fields, mirroring
/// Node's `transformSchedulerData`.
fn scheduler_from_hash(id: &str, map: &HashMap<String, String>, next: Option<u64>) -> JobScheduler {
    JobScheduler {
        id: id.to_string(),
        name: map.get("name").cloned().unwrap_or_default(),
        next,
        iteration_count: hash_num(map, "ic"),
        limit: hash_num(map, "limit"),
        start_date: hash_num(map, "startDate"),
        end_date: hash_num(map, "endDate"),
        tz: map.get("tz").cloned(),
        pattern: map.get("pattern").cloned(),
        every: hash_num(map, "every"),
        offset: hash_num(map, "offset"),
        template_data: map.get("data").and_then(|s| serde_json::from_str(s).ok()),
        template_opts: map.get("opts").and_then(|s| serde_json::from_str(s).ok()),
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
