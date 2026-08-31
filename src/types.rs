use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Lifecycle state of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JobState {
    #[serde(rename = "wait")]
    Wait,
    #[serde(rename = "paused")]
    Paused,
    #[serde(rename = "prioritized")]
    Prioritized,
    #[serde(rename = "waiting-children")]
    WaitingChildren,
    #[serde(rename = "delayed")]
    Delayed,
    #[serde(rename = "active")]
    Active,
    #[serde(rename = "completed")]
    Completed,
    #[serde(rename = "failed")]
    Failed,
}

impl std::fmt::Display for JobState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobState::Wait => write!(f, "wait"),
            JobState::Paused => write!(f, "paused"),
            JobState::Prioritized => write!(f, "prioritized"),
            JobState::WaitingChildren => write!(f, "waiting-children"),
            JobState::Delayed => write!(f, "delayed"),
            JobState::Active => write!(f, "active"),
            JobState::Completed => write!(f, "completed"),
            JobState::Failed => write!(f, "failed"),
        }
    }
}

impl std::str::FromStr for JobState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "wait" => Ok(JobState::Wait),
            "paused" => Ok(JobState::Paused),
            "prioritized" => Ok(JobState::Prioritized),
            "waiting-children" => Ok(JobState::WaitingChildren),
            "delayed" => Ok(JobState::Delayed),
            "active" => Ok(JobState::Active),
            "completed" => Ok(JobState::Completed),
            "failed" => Ok(JobState::Failed),
            _ => Err(format!("Unknown job state: {}", s)),
        }
    }
}

/// Options for creating a job.
///
/// Serializes to JSON matching the BullMQ Node.js `opts` format.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobOptions {
    /// Job priority. Lower values = higher priority. Default is 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u32>,
    /// Delay before the job becomes available for processing (in milliseconds).
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "option_duration_millis",
        default
    )]
    pub delay: Option<Duration>,
    /// Maximum number of attempts (including the first). Default is 1 (no retry).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u32>,
    /// Backoff strategy for retries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backoff: Option<BackoffStrategy>,
    /// Time-to-live: job expires after this duration (in milliseconds).
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "option_duration_millis",
        default
    )]
    pub ttl: Option<Duration>,
    /// Custom job ID. Auto-generated if not provided.
    #[serde(rename = "jobId", skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// Deduplication options. Serialized as `de` to match the BullMQ wire format.
    #[serde(rename = "de", skip_serializing_if = "Option::is_none", default)]
    pub deduplication: Option<DeduplicationOptions>,
    /// Repeat options carried by every job produced by a job scheduler.
    /// Managed by [`crate::Queue::upsert_job_scheduler`]; not meant to be set
    /// directly on regular jobs.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub repeat: Option<RepeatOptions>,
    /// Iteration timestamp (ms) this scheduler-produced job was created for.
    /// Managed by the job scheduler.
    #[serde(
        rename = "prevMillis",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub prev_millis: Option<u64>,
    /// Id of the job scheduler that produced this job. Managed by the job
    /// scheduler (also stored as the `rjk` hash field).
    #[serde(
        rename = "repeatJobKey",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub repeat_job_key: Option<String>,
    /// Creation timestamp (ms) recorded in the serialized options by the job
    /// scheduler, mirroring BullMQ Node's merged options.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub timestamp: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct JobOptionsBuilder {
    pub priority: Option<u32>,
    pub delay: Option<Duration>,
    pub attempts: Option<u32>,
    pub backoff: Option<BackoffStrategy>,
    pub ttl: Option<Duration>,
    pub job_id: Option<String>,
    pub deduplication: Option<DeduplicationOptions>,
    pub repeat: Option<RepeatOptions>,
    pub prev_millis: Option<u64>,
    pub repeat_job_key: Option<String>,
    pub timestamp: Option<u64>,
}

impl JobOptionsBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn priority(mut self, priority: u32) -> Self {
        self.priority = Some(priority);
        self
    }
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }
    pub fn attempts(mut self, attempts: u32) -> Self {
        self.attempts = Some(attempts);
        self
    }
    pub fn backoff(mut self, backoff: BackoffStrategy) -> Self {
        self.backoff = Some(backoff);
        self
    }
    pub fn ttl(mut self, ttl: Duration) -> Self {
        self.ttl = Some(ttl);
        self
    }
    pub fn job_id(mut self, job_id: String) -> Self {
        self.job_id = Some(job_id);
        self
    }
    pub fn deduplication(mut self, deduplication: DeduplicationOptions) -> Self {
        self.deduplication = Some(deduplication);
        self
    }
    pub fn repeat(mut self, repeat: RepeatOptions) -> Self {
        self.repeat = Some(repeat);
        self
    }
    pub fn prev_millis(mut self, prev_millis: u64) -> Self {
        self.prev_millis = Some(prev_millis);
        self
    }
    pub fn repeat_job_key(mut self, repeat_job_key: String) -> Self {
        self.repeat_job_key = Some(repeat_job_key);
        self
    }
    pub fn timestamp(mut self, timestamp: u64) -> Self {
        self.timestamp = Some(timestamp);
        self
    }
    pub fn build(self) -> JobOptions {
        JobOptions {
            priority: self.priority,
            delay: self.delay,
            attempts: self.attempts,
            backoff: self.backoff,
            ttl: self.ttl,
            job_id: self.job_id,
            deduplication: self.deduplication,
            repeat: self.repeat,
            prev_millis: self.prev_millis,
            repeat_job_key: self.repeat_job_key,
            timestamp: self.timestamp,
        }
    }
}

/// Repeat options for a job scheduler (repeatable job).
///
/// Exactly one of [`pattern`](Self::pattern) (cron) or [`every`](Self::every)
/// must be set. Serializes to JSON matching the BullMQ Node.js `repeat`
/// options format.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepeatOptions {
    /// Cron pattern (cron-parser syntax: 5 fields, or 6 fields with leading
    /// seconds), e.g. `"* * * * *"` for every minute.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pattern: Option<String>,
    /// Fixed interval between iterations.
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "option_duration_millis",
        default
    )]
    pub every: Option<Duration>,
    /// Maximum total number of iterations.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub limit: Option<u64>,
    /// Earliest time (ms since Unix epoch) for the first iteration.
    #[serde(rename = "startDate", skip_serializing_if = "Option::is_none", default)]
    pub start_date: Option<u64>,
    /// Time (ms since Unix epoch) after which no more iterations are produced.
    #[serde(rename = "endDate", skip_serializing_if = "Option::is_none", default)]
    pub end_date: Option<u64>,
    /// IANA timezone used to evaluate the cron `pattern` (e.g. "Europe/Paris").
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tz: Option<String>,
    /// Produce the first iteration immediately instead of waiting for the next
    /// cron match. Only valid with `pattern`, and incompatible with
    /// `start_date`. Never serialized, mirroring BullMQ Node.
    #[serde(skip, default)]
    pub immediately: bool,
    /// Offset in milliseconds within `every` slots. Managed by the scheduler.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub offset: Option<u64>,
    /// Iteration counter. Managed by the scheduler.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub count: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct RepeatOptionsBuilder {
    pattern: Option<String>,
    every: Option<Duration>,
    limit: Option<u64>,
    start_date: Option<u64>,
    end_date: Option<u64>,
    tz: Option<String>,
    immediately: bool,
    offset: Option<u64>,
    count: Option<u64>,
}

impl RepeatOptionsBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn pattern(mut self, pattern: &str) -> Self {
        self.pattern = Some(pattern.to_string());
        self
    }
    pub fn every(mut self, every: Duration) -> Self {
        self.every = Some(every);
        self
    }
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }
    pub fn start_date(mut self, start_date: u64) -> Self {
        self.start_date = Some(start_date);
        self
    }
    pub fn end_date(mut self, end_date: u64) -> Self {
        self.end_date = Some(end_date);
        self
    }
    pub fn tz(mut self, tz: &str) -> Self {
        self.tz = Some(tz.to_string());
        self
    }
    pub fn immediately(mut self, immediately: bool) -> Self {
        self.immediately = immediately;
        self
    }
    pub fn offset(mut self, offset: u64) -> Self {
        self.offset = Some(offset);
        self
    }
    pub fn count(mut self, count: u64) -> Self {
        self.count = Some(count);
        self
    }
    pub fn build(self) -> RepeatOptions {
        RepeatOptions {
            pattern: self.pattern,
            every: self.every,
            limit: self.limit,
            start_date: self.start_date,
            end_date: self.end_date,
            tz: self.tz,
            immediately: self.immediately,
            offset: self.offset,
            count: self.count,
        }
    }
}

/// Job template applied to every job produced by a job scheduler.
#[derive(Debug, Clone)]
pub struct JobSchedulerTemplate<T> {
    /// Name of the produced jobs. Defaults to the scheduler id.
    pub name: Option<String>,
    /// Data payload of the produced jobs.
    pub data: Option<T>,
    /// Options applied to the produced jobs.
    pub opts: Option<JobOptions>,
}

impl<T> Default for JobSchedulerTemplate<T> {
    fn default() -> Self {
        Self {
            name: None,
            data: None,
            opts: None,
        }
    }
}

/// Metadata of a job scheduler as returned by
/// [`crate::Queue::get_job_scheduler`].
#[derive(Debug, Clone, Default)]
pub struct JobScheduler {
    /// Scheduler id (the `key` in BullMQ Node).
    pub id: String,
    /// Name of the produced jobs.
    pub name: String,
    /// Timestamp (ms) of the next scheduled iteration.
    pub next: Option<u64>,
    /// Number of iterations produced so far.
    pub iteration_count: Option<u64>,
    /// Maximum total number of iterations.
    pub limit: Option<u64>,
    /// Earliest time (ms since Unix epoch) for the first iteration.
    pub start_date: Option<u64>,
    /// Time (ms since Unix epoch) after which no more iterations are produced.
    pub end_date: Option<u64>,
    /// IANA timezone used to evaluate the cron pattern.
    pub tz: Option<String>,
    /// Cron pattern.
    pub pattern: Option<String>,
    /// Fixed interval between iterations (ms).
    pub every: Option<u64>,
    /// Offset in milliseconds within `every` slots.
    pub offset: Option<u64>,
    /// Template data payload of the produced jobs.
    pub template_data: Option<serde_json::Value>,
    /// Template options of the produced jobs.
    pub template_opts: Option<JobOptions>,
}

/// Job deduplication options.
///
/// While a deduplication key (`<prefix>:<queue>:de:<id>`) exists, further adds
/// with the same `id` are deduplicated and return the existing job's ID.
/// Without a `ttl`, the key lives until the deduplicated job completes or
/// fails; with a `ttl`, it expires after that duration instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeduplicationOptions {
    /// Identifier used to deduplicate jobs.
    pub id: String,
    /// Time-to-live for the deduplication key (in milliseconds).
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "option_duration_millis",
        default
    )]
    pub ttl: Option<Duration>,
}

/// Backoff strategy for job retries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BackoffStrategy {
    /// Fixed delay between retries.
    Fixed {
        #[serde(with = "duration_millis")]
        delay: Duration,
    },
    /// Exponential backoff with a maximum delay cap.
    ///
    /// Serializes as `delay` to match BullMQ Node.js's `BackoffOptions`;
    /// `max` is a bullmq-rs extension only honored by bullmq-rs workers.
    Exponential {
        #[serde(with = "duration_millis", rename = "delay")]
        base: Duration,
        #[serde(with = "duration_millis")]
        max: Duration,
    },
}

impl BackoffStrategy {
    /// Calculate the delay for a given attempt number (0-indexed).
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        match self {
            BackoffStrategy::Fixed { delay } => *delay,
            BackoffStrategy::Exponential { base, max } => {
                let delay = base.as_millis() as u64 * 2u64.saturating_pow(attempt);
                let max_ms = max.as_millis() as u64;
                Duration::from_millis(delay.min(max_ms))
            }
        }
    }
}

/// Rate limiter options for a worker.
///
/// Limits the number of jobs moved to active per `duration` window, matching
/// the BullMQ Node.js `limiter` worker option (shared `limiter` key in Redis).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimiterOptions {
    /// Maximum number of jobs processed per window.
    pub max: u64,
    /// Length of the rate-limit window.
    pub duration: Duration,
}

/// Metrics collection options for a worker.
///
/// When set, the worker records per-minute finished-job counts in the
/// `metrics:<state>` keys, matching the BullMQ Node.js `metrics` worker option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricsOptions {
    /// Maximum number of per-minute data points retained.
    pub max_data_points: u64,
}

/// Metadata of collected queue metrics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetricsMeta {
    /// Total number of finished jobs counted.
    pub count: u64,
    /// Timestamp (ms) of the last collected data point.
    pub prev_ts: u64,
    /// Job count at the last collected data point.
    pub prev_count: u64,
}

/// Queue metrics as returned by `Queue::get_metrics`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metrics {
    /// Collection metadata.
    pub meta: MetricsMeta,
    /// Per-minute data points (newest first).
    pub data: Vec<i64>,
    /// Number of data points available.
    pub count: u64,
}

/// Options for creating a worker.
#[derive(Debug, Clone)]
pub struct WorkerOptions {
    /// Number of jobs to process concurrently. Default is 1.
    pub concurrency: usize,
    /// Duration a job lock is held before it can be considered stalled. Default is 30s.
    pub lock_duration: Duration,
    /// How often to check for stalled jobs. Default is 30s.
    pub stalled_interval: Duration,
    /// Maximum number of times a job can be recovered from stalled state. Default is 1.
    pub max_stalled_count: u32,
    /// Whether to skip the stalled-job check entirely. Default is false.
    pub skip_stalled_check: bool,
    /// Rate limiter: max jobs per duration window. Default is `None` (no limit).
    pub limiter: Option<RateLimiterOptions>,
    /// Metrics collection. Default is `None` (no metrics collected).
    pub metrics: Option<MetricsOptions>,
}

impl Default for WorkerOptions {
    fn default() -> Self {
        Self {
            concurrency: 1,
            lock_duration: Duration::from_secs(30),
            stalled_interval: Duration::from_secs(30),
            max_stalled_count: 1,
            skip_stalled_check: false,
            limiter: None,
            metrics: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JobDependencies {
    pub processed: HashMap<String, serde_json::Value>,
    pub unprocessed: Vec<String>,
}

/// Default maximum number of events to keep in the events stream.
pub(crate) const DEFAULT_MAX_EVENTS: u64 = 10_000;

mod duration_millis {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_millis() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let ms = u64::deserialize(d)?;
        Ok(Duration::from_millis(ms))
    }
}

mod option_duration_millis {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(dur) => s.serialize_u64(dur.as_millis() as u64),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        let opt = Option::<u64>::deserialize(d)?;
        Ok(opt.map(Duration::from_millis))
    }
}
