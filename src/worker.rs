use std::collections::HashSet;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::{watch, Mutex, Semaphore};

use crate::connection::RedisConnection;
use crate::error::{BullmqError, BullmqResult};
use crate::job::{Job, JobContext};

/// Type alias for the on_completed callback.
type CompletedCallback = Arc<dyn Fn(&Job<serde_json::Value>) + Send + Sync>;
/// Type alias for the on_failed callback.
type FailedCallback = Arc<dyn Fn(&Job<serde_json::Value>, &BullmqError) + Send + Sync>;
use crate::scripts::commands::{extend_lock, move_stalled_jobs_to_wait, move_to_active, move_to_delayed, move_to_finished};
use crate::scripts::ScriptLoader;
use crate::types::{WorkerOptions, DEFAULT_MAX_EVENTS};

/// A worker that processes jobs from a queue.
///
/// Use [`WorkerBuilder`] to create a worker, then call [`Worker::start`] with
/// an async handler function.
///
/// # Example
/// ```rust,no_run
/// use bullmq_rs::{WorkerBuilder, RedisConnection};
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Serialize, Deserialize, Debug, Clone)]
/// struct MyJob { message: String }
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let worker = WorkerBuilder::new("my_queue")
///     .connection(RedisConnection::new("redis://127.0.0.1:6379"))
///     .concurrency(5)
///     .build::<MyJob>();
///
/// let handle = worker.start(|job| async move {
///     println!("Processing: {:?}", job.data);
///     Ok(())
/// }).await?;
///
/// // Later: graceful shutdown
/// handle.shutdown();
/// handle.wait().await?;
/// # Ok(())
/// # }
/// ```
pub struct Worker<T> {
    name: String,
    prefix: String,
    conn: RedisConnection,
    scripts: Arc<ScriptLoader>,
    options: WorkerOptions,
    on_completed: Option<CompletedCallback>,
    on_failed: Option<FailedCallback>,
    _phantom: PhantomData<T>,
}

/// Handle to a running worker, used for shutdown.
pub struct WorkerHandle {
    shutdown_tx: watch::Sender<bool>,
    join_handle: tokio::task::JoinHandle<()>,
}

impl WorkerHandle {
    /// Signal the worker to stop after finishing current jobs.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Wait for the worker to fully stop.
    pub async fn wait(self) -> BullmqResult<()> {
        self.join_handle
            .await
            .map_err(|e| BullmqError::Other(format!("Worker task panicked: {}", e)))?;
        Ok(())
    }
}

/// Returns the current time as milliseconds since the Unix epoch.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

impl<T: Serialize + DeserializeOwned + Clone + Send + Sync + 'static> Worker<T> {
    /// Start processing jobs with the given async handler.
    ///
    /// The handler receives a `Job<T>` and must return a `Result<(), Box<dyn Error>>`.
    /// On success the job moves to completed; on error it is retried (if attempts
    /// remain) or moved to failed.
    ///
    /// Returns a [`WorkerHandle`] that can be used to shut down the worker.
    pub async fn start<F, Fut>(self, handler: F) -> BullmqResult<WorkerHandle>
    where
        F: Fn(Job<T>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>> + Send,
    {
        // Create two independent ConnectionManager instances.
        let blocking_conn = self.conn.get_manager().await?;
        let cmd_conn = self.conn.get_manager().await?;

        // Generate a unique worker token.
        let token = uuid::Uuid::new_v4().to_string();

        // Shared state for active job tracking (used by lock extender).
        let active_jobs: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

        // Shutdown channel.
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let handler = Arc::new(handler);
        let semaphore = Arc::new(Semaphore::new(self.options.concurrency));
        let scripts = self.scripts.clone();
        let name = self.name.clone();
        let prefix = self.prefix.clone();
        let on_completed = self.on_completed.clone();
        let on_failed = self.on_failed.clone();
        let lock_duration_ms = self.options.lock_duration.as_millis() as u64;
        let stalled_interval = self.options.stalled_interval;
        let max_stalled_count = self.options.max_stalled_count;
        let skip_stalled_check = self.options.skip_stalled_check;
        let concurrency = self.options.concurrency;

        // Channel for fast-path: moveToFinished can return the next job.
        // Capacity matches concurrency so concurrent handler completions don't drop prefetched jobs.
        let (next_job_tx, next_job_rx) = tokio::sync::mpsc::channel::<move_to_active::MoveToActiveResult>(concurrency);
        let next_job_rx = Arc::new(Mutex::new(next_job_rx));

        let join_handle = tokio::spawn({
            let scripts = scripts.clone();
            let name = name.clone();
            let prefix = prefix.clone();
            let token = token.clone();
            let active_jobs = active_jobs.clone();
            let shutdown_rx_main = shutdown_rx.clone();

            // Spawn the lock extender background task.
            let lock_extender_handle = {
                let scripts = scripts.clone();
                let name = name.clone();
                let prefix = prefix.clone();
                let token = token.clone();
                let active_jobs = active_jobs.clone();
                let mut shutdown_rx = shutdown_rx.clone();
                let mut cmd_conn = cmd_conn.clone();
                let extend_interval = Duration::from_millis(lock_duration_ms / 2);

                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = tokio::time::sleep(extend_interval) => {},
                            _ = shutdown_rx.changed() => break,
                        }

                        let job_ids: Vec<String> = {
                            let set = active_jobs.lock().await;
                            set.iter().cloned().collect()
                        };

                        for job_id in &job_ids {
                            if let Err(e) = extend_lock::extend_lock(
                                &scripts,
                                &mut cmd_conn,
                                &prefix,
                                &name,
                                job_id,
                                &token,
                                lock_duration_ms,
                            )
                            .await
                            {
                                match e {
                                    BullmqError::LockMismatch => {
                                        tracing::warn!(
                                            "Lock mismatch extending lock for job {}, it may have been stalled",
                                            job_id
                                        );
                                    }
                                    _ => {
                                        tracing::warn!("Error extending lock for job {}: {}", job_id, e);
                                    }
                                }
                            }
                        }
                    }
                    tracing::debug!("Lock extender stopped");
                })
            };

            // Spawn the stalled job checker background task.
            let stalled_checker_handle = if !skip_stalled_check {
                let scripts = scripts.clone();
                let name = name.clone();
                let prefix = prefix.clone();
                let mut shutdown_rx = shutdown_rx.clone();
                let mut cmd_conn = cmd_conn.clone();

                Some(tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = tokio::time::sleep(stalled_interval) => {},
                            _ = shutdown_rx.changed() => break,
                        }

                        let timestamp = now_ms();
                        match move_stalled_jobs_to_wait::move_stalled_jobs_to_wait(
                            &scripts,
                            &mut cmd_conn,
                            &prefix,
                            &name,
                            max_stalled_count,
                            stalled_interval.as_millis() as u64,
                            timestamp,
                            DEFAULT_MAX_EVENTS,
                        )
                        .await
                        {
                            Ok(result) => {
                                if result.stalled > 0 || result.failed > 0 {
                                    tracing::info!(
                                        "Stalled jobs check: {} recovered, {} failed",
                                        result.stalled,
                                        result.failed
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Error checking stalled jobs: {}", e);
                            }
                        }
                    }
                    tracing::debug!("Stalled job checker stopped");
                }))
            } else {
                None
            };

            async move {
                let mut shutdown_rx = shutdown_rx_main;
                let mut blocking_conn = blocking_conn;
                let mut cmd_conn = cmd_conn;
                let marker_key = format!("{}:{}:marker", prefix, name);

                // Bootstrap: add a marker so BZPOPMIN picks up pre-existing jobs.
                // Idempotent — if "0" already exists, score is unchanged.
                let _: redis::RedisResult<i64> = redis::cmd("ZADD")
                    .arg(&marker_key)
                    .arg(0i64)
                    .arg("0")
                    .query_async(&mut cmd_conn)
                    .await;

                loop {
                    // Check for shutdown.
                    if *shutdown_rx.borrow() {
                        break;
                    }

                    // Check fast-path channel first (next job from moveToFinished).
                    let fast_path_result = {
                        let mut rx = next_job_rx.lock().await;
                        rx.try_recv().ok()
                    };

                    let move_result = if let Some(result) = fast_path_result {
                        // We got a job from the fast-path channel.
                        Some(result)
                    } else {
                        // BZPOPMIN on the marker key with 5-second timeout.
                        let bzpopmin_result: redis::RedisResult<redis::Value> =
                            redis::cmd("BZPOPMIN")
                                .arg(&marker_key)
                                .arg(5.0_f64)
                                .query_async(&mut blocking_conn)
                                .await;

                        // Parse BZPOPMIN response manually:
                        // - Nil or empty array = timeout
                        // - Array of [key, member, score] = got a marker
                        let parsed = match &bzpopmin_result {
                            Ok(redis::Value::Array(items)) if items.len() >= 3 => {
                                let member = match &items[1] {
                                    redis::Value::BulkString(b) => String::from_utf8_lossy(b).to_string(),
                                    redis::Value::SimpleString(s) => s.clone(),
                                    _ => String::new(),
                                };
                                let score = match &items[2] {
                                    redis::Value::BulkString(b) => String::from_utf8_lossy(b).parse::<f64>().unwrap_or(0.0),
                                    redis::Value::Double(f) => *f,
                                    redis::Value::Int(i) => *i as f64,
                                    _ => 0.0,
                                };
                                Some((member, score))
                            }
                            Ok(redis::Value::Nil) | Ok(redis::Value::Array(_)) => None,
                            Ok(_) => None,
                            Err(_) => None,
                        };

                        // Check if the original result was a real error (not a timeout)
                        if let Err(ref e) = bzpopmin_result {
                            // Timeouts and type errors (Nil response) are normal for BZPOPMIN
                            let msg = e.to_string();
                            if msg.contains("timed out") || msg.contains("response was nil") ||
                               msg.contains("not compatible") {
                                continue;
                            }
                            tracing::warn!("BZPOPMIN error: {}, retrying after delay", e);
                            tokio::select! {
                                _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                                _ = shutdown_rx.changed() => break,
                            }
                            continue;
                        }

                        match parsed {
                            Some((member, score)) => {
                                if member == "0" {
                                    // Score 0 means a job is available: call moveToActive.
                                    let timestamp = now_ms();
                                    match move_to_active::move_to_active(
                                        &scripts,
                                        &mut cmd_conn,
                                        &prefix,
                                        &name,
                                        &token,
                                        lock_duration_ms,
                                        timestamp,
                                        DEFAULT_MAX_EVENTS,
                                    )
                                    .await
                                    {
                                        Ok(result) => {
                                            if result.job_id.is_some() {
                                                Some(result)
                                            } else {
                                                None
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!("moveToActive error: {}", e);
                                            None
                                        }
                                    }
                                } else if member == "1" {
                                    // Score is a timestamp for a delayed job.
                                    // Sleep until that time, then call moveToActive (which promotes delayed jobs).
                                    let now = now_ms();
                                    let target = score as u64;
                                    if target > now {
                                        let wait_ms = (target - now).min(5_000);
                                        tokio::select! {
                                            _ = tokio::time::sleep(Duration::from_millis(wait_ms)) => {},
                                            _ = shutdown_rx.changed() => break,
                                        }
                                    }
                                    // After sleeping, call moveToActive to promote and fetch delayed jobs
                                    let timestamp = now_ms();
                                    match move_to_active::move_to_active(
                                        &scripts,
                                        &mut cmd_conn,
                                        &prefix,
                                        &name,
                                        &token,
                                        lock_duration_ms,
                                        timestamp,
                                        DEFAULT_MAX_EVENTS,
                                    )
                                    .await
                                    {
                                        Ok(result) if result.job_id.is_some() => Some(result),
                                        Ok(_) => { continue; }
                                        Err(e) => {
                                            tracing::warn!("moveToActive error after delay: {}", e);
                                            continue;
                                        }
                                    }
                                } else {
                                    // Unknown marker member, treat as a regular job marker.
                                    // Call moveToActive just in case.
                                    let timestamp = now_ms();
                                    match move_to_active::move_to_active(
                                        &scripts,
                                        &mut cmd_conn,
                                        &prefix,
                                        &name,
                                        &token,
                                        lock_duration_ms,
                                        timestamp,
                                        DEFAULT_MAX_EVENTS,
                                    )
                                    .await
                                    {
                                        Ok(result) if result.job_id.is_some() => Some(result),
                                        Ok(_) => None,
                                        Err(e) => {
                                            tracing::warn!("moveToActive error: {}", e);
                                            None
                                        }
                                    }
                                }
                            }
                            None => {
                                // Timeout — try moveToActive as recovery.
                                // Handles lost markers (Redis failover, manual deletion, etc.)
                                let timestamp = now_ms();
                                match move_to_active::move_to_active(
                                    &scripts,
                                    &mut cmd_conn,
                                    &prefix,
                                    &name,
                                    &token,
                                    lock_duration_ms,
                                    timestamp,
                                    DEFAULT_MAX_EVENTS,
                                )
                                .await
                                {
                                    Ok(result) if result.job_id.is_some() => Some(result),
                                    Ok(_) => { continue; }
                                    Err(e) => {
                                        tracing::debug!("moveToActive recovery check: {}", e);
                                        continue;
                                    }
                                }
                            }
                        }
                    };

                    // If we have a job result from moveToActive, process it.
                    if let Some(result) = move_result {
                        if let Some(job_id) = result.job_id {
                            // Deserialize the job from the hash data returned by moveToActive.
                            let mut job: Job<T> = match Job::from_redis_hash(&job_id, &result.job_data) {
                                Ok(j) => j,
                                Err(e) => {
                                    tracing::error!("Failed to deserialize job {}: {}", job_id, e);
                                    continue;
                                }
                            };

                            // Inject connection context, lock token, and active state
                            // for active-handle methods.
                            job.ctx = Some(Arc::new(JobContext {
                                conn: cmd_conn.clone(),
                                scripts: scripts.clone(),
                                prefix: prefix.clone(),
                                queue_name: name.clone(),
                            }));
                            job.lock_token = Some(token.clone());
                            job.state = crate::types::JobState::Active;

                            // Acquire a semaphore permit (may block if at concurrency limit).
                            let permit = tokio::select! {
                                p = semaphore.clone().acquire_owned() => {
                                    match p {
                                        Ok(permit) => permit,
                                        Err(_) => break, // Semaphore closed
                                    }
                                },
                                _ = shutdown_rx.changed() => break,
                            };

                            // Spawn the job processing task.
                            let handler = handler.clone();
                            let scripts = scripts.clone();
                            let mut task_conn = cmd_conn.clone();
                            let task_prefix = prefix.clone();
                            let task_name = name.clone();
                            let task_token = token.clone();
                            let on_completed = on_completed.clone();
                            let on_failed = on_failed.clone();
                            let active_jobs = active_jobs.clone();
                            let next_job_tx = next_job_tx.clone();
                            let task_lock_duration = lock_duration_ms;

                            tokio::spawn(async move {
                                let _permit = permit;
                                let job_id = job.id.clone();

                                // Register job ID in active_jobs set.
                                {
                                    let mut set = active_jobs.lock().await;
                                    set.insert(job_id.clone());
                                }

                                // Run the user handler.
                                match handler(job.clone()).await {
                                    Ok(()) => {
                                        // Success: call moveToFinished(completed) with fetchNext=true.
                                        let timestamp = now_ms();
                                        let attempts = job.attempts_made + 1;

                                        match move_to_finished::move_to_finished(
                                            &scripts,
                                            &mut task_conn,
                                            &task_prefix,
                                            &task_name,
                                            &job_id,
                                            &task_token,
                                            timestamp,
                                            "{}",  // return value (empty JSON object)
                                            "completed",
                                            DEFAULT_MAX_EVENTS,
                                            true,  // fetchNext
                                            task_lock_duration,
                                            attempts,
                                        )
                                        .await
                                        {
                                            Ok(move_to_finished::MoveToFinishedResult::NextJob(next)) => {
                                                // Fast-path: send next job via channel.
                                                if next.job_id.is_some() {
                                                    if let Err(e) = next_job_tx.try_send(next) {
                                                        tracing::warn!(
                                                            "Fast-path channel full, prefetched job will be recovered via stalled check: {:?}",
                                                            e.into_inner().job_id
                                                        );
                                                    }
                                                }
                                            }
                                            Ok(move_to_finished::MoveToFinishedResult::Done) => {}
                                            Err(BullmqError::LockMismatch) => {
                                                tracing::warn!(
                                                    "Lock mismatch finishing job {} (completed) - job may have been stalled",
                                                    job_id
                                                );
                                            }
                                            Err(e) => {
                                                tracing::error!(
                                                    "Error moving job {} to completed: {}",
                                                    job_id,
                                                    e
                                                );
                                            }
                                        }

                                        // Invoke on_completed callback.
                                        if let Some(ref cb) = on_completed {
                                            if let Ok(val_job) = convert_job_to_value(&job) {
                                                cb(&val_job);
                                            }
                                        }

                                        tracing::debug!("Job {} completed", job_id);
                                    }
                                    Err(err) => {
                                        let new_attempts = job.attempts_made + 1;
                                        let max_attempts = job.opts.attempts.unwrap_or(1);

                                        if new_attempts < max_attempts {
                                            // Retries remaining: call moveToDelayed with backoff.
                                            let backoff_delay = job
                                                .opts
                                                .backoff
                                                .as_ref()
                                                .map(|b| b.delay_for_attempt(job.attempts_made))
                                                .unwrap_or(Duration::from_secs(1));

                                            let timestamp = now_ms();
                                            let delayed_timestamp =
                                                timestamp + backoff_delay.as_millis() as u64;

                                            match move_to_delayed::move_to_delayed(
                                                &scripts,
                                                &mut task_conn,
                                                &task_prefix,
                                                &task_name,
                                                &job_id,
                                                &task_token,
                                                timestamp,
                                                delayed_timestamp,
                                                DEFAULT_MAX_EVENTS,
                                                new_attempts,
                                            )
                                            .await
                                            {
                                                Ok(()) => {
                                                    tracing::warn!(
                                                        "Job {} failed (attempt {}/{}), retrying in {:?}: {}",
                                                        job_id,
                                                        new_attempts,
                                                        max_attempts,
                                                        backoff_delay,
                                                        err
                                                    );
                                                }
                                                Err(BullmqError::LockMismatch) => {
                                                    tracing::warn!(
                                                        "Lock mismatch moving job {} to delayed - job may have been stalled",
                                                        job_id
                                                    );
                                                }
                                                Err(e) => {
                                                    tracing::error!(
                                                        "Error moving job {} to delayed: {}",
                                                        job_id,
                                                        e
                                                    );
                                                }
                                            }

                                            // Invoke on_failed callback even for retries.
                                            if let Some(ref cb) = on_failed {
                                                if let Ok(val_job) = convert_job_to_value(&job) {
                                                    let bullmq_err = BullmqError::Other(err.to_string());
                                                    cb(&val_job, &bullmq_err);
                                                }
                                            }
                                        } else {
                                            // No retries left: call moveToFinished(failed).
                                            let timestamp = now_ms();
                                            let failed_reason = err.to_string();

                                            match move_to_finished::move_to_finished(
                                                &scripts,
                                                &mut task_conn,
                                                &task_prefix,
                                                &task_name,
                                                &job_id,
                                                &task_token,
                                                timestamp,
                                                &failed_reason,
                                                "failed",
                                                DEFAULT_MAX_EVENTS,
                                                false, // don't fetch next on failure
                                                task_lock_duration,
                                                new_attempts,
                                            )
                                            .await
                                            {
                                                Ok(_) => {}
                                                Err(BullmqError::LockMismatch) => {
                                                    tracing::warn!(
                                                        "Lock mismatch finishing job {} (failed) - job may have been stalled",
                                                        job_id
                                                    );
                                                }
                                                Err(e) => {
                                                    tracing::error!(
                                                        "Error moving job {} to failed: {}",
                                                        job_id,
                                                        e
                                                    );
                                                }
                                            }

                                            // Invoke on_failed callback.
                                            if let Some(ref cb) = on_failed {
                                                if let Ok(val_job) = convert_job_to_value(&job) {
                                                    let bullmq_err = BullmqError::Other(err.to_string());
                                                    cb(&val_job, &bullmq_err);
                                                }
                                            }

                                            tracing::error!(
                                                "Job {} permanently failed after {} attempts: {}",
                                                job_id,
                                                new_attempts,
                                                err
                                            );
                                        }
                                    }
                                }

                                // Unregister job ID from active_jobs.
                                {
                                    let mut set = active_jobs.lock().await;
                                    set.remove(&job_id);
                                }
                            });
                        }
                    }
                }

                // Graceful shutdown: wait for all in-flight jobs by acquiring all permits.
                tracing::info!("Worker '{}' shutting down, waiting for in-flight jobs...", name);
                let _ = semaphore.acquire_many(concurrency as u32).await;
                tracing::info!("Worker '{}' shut down", name);

                // Stop background tasks.
                lock_extender_handle.abort();
                if let Some(handle) = stalled_checker_handle {
                    handle.abort();
                }
            }
        });

        Ok(WorkerHandle {
            shutdown_tx,
            join_handle,
        })
    }
}

/// Convert a `Job<T>` to `Job<serde_json::Value>` for type-erased callbacks.
fn convert_job_to_value<T: Serialize>(job: &Job<T>) -> BullmqResult<Job<serde_json::Value>> {
    let value = serde_json::to_value(&job.data)?;
    Ok(Job {
        id: job.id.clone(),
        name: job.name.clone(),
        data: value,
        state: job.state,
        opts: job.opts.clone(),
        timestamp: job.timestamp,
        priority: job.priority,
        delay: job.delay,
        attempts_made: job.attempts_made,
        attempts_started: job.attempts_started,
        progress: job.progress.clone(),
        processed_on: job.processed_on,
        finished_on: job.finished_on,
        failed_reason: job.failed_reason.clone(),
        stacktrace: job.stacktrace.clone(),
        return_value: job.return_value.clone(),
        processed_by: job.processed_by.clone(),
        ctx: None,
        lock_token: None,
    })
}

/// Builder for creating a [`Worker`].
pub struct WorkerBuilder {
    name: String,
    connection: RedisConnection,
    prefix: String,
    options: WorkerOptions,
    on_completed: Option<CompletedCallback>,
    on_failed: Option<FailedCallback>,
}

impl WorkerBuilder {
    /// Create a new worker builder for the given queue name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            connection: RedisConnection::default(),
            prefix: "bull".to_string(),
            options: WorkerOptions::default(),
            on_completed: None,
            on_failed: None,
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

    /// Set the number of jobs to process concurrently (default: 1).
    pub fn concurrency(mut self, n: usize) -> Self {
        self.options.concurrency = n.max(1);
        self
    }

    /// Set the duration a job lock is held before it can be considered stalled (default: 30s).
    pub fn lock_duration(mut self, d: Duration) -> Self {
        self.options.lock_duration = d;
        self
    }

    /// Set how often to check for stalled jobs (default: 30s).
    pub fn stalled_interval(mut self, d: Duration) -> Self {
        self.options.stalled_interval = d;
        self
    }

    /// Set the maximum number of times a job can be recovered from stalled state (default: 1).
    pub fn max_stalled_count(mut self, n: u32) -> Self {
        self.options.max_stalled_count = n;
        self
    }

    /// Set whether to skip the stalled-job check entirely (default: false).
    pub fn skip_stalled_check(mut self, skip: bool) -> Self {
        self.options.skip_stalled_check = skip;
        self
    }

    /// Set a callback invoked when a job completes successfully.
    pub fn on_completed<F>(mut self, f: F) -> Self
    where
        F: Fn(&Job<serde_json::Value>) + Send + Sync + 'static,
    {
        self.on_completed = Some(Arc::new(f));
        self
    }

    /// Set a callback invoked when a job permanently fails.
    pub fn on_failed<F>(mut self, f: F) -> Self
    where
        F: Fn(&Job<serde_json::Value>, &BullmqError) + Send + Sync + 'static,
    {
        self.on_failed = Some(Arc::new(f));
        self
    }

    /// Build the worker.
    ///
    /// Does not establish any Redis connections yet -- connections are created
    /// lazily when [`Worker::start`] is called.
    pub fn build<T: Serialize + DeserializeOwned + Send + Sync + 'static>(self) -> Worker<T> {
        let scripts = Arc::new(ScriptLoader::new());
        Worker {
            name: self.name,
            prefix: self.prefix,
            conn: self.connection,
            scripts,
            options: self.options,
            on_completed: self.on_completed,
            on_failed: self.on_failed,
            _phantom: PhantomData,
        }
    }
}
