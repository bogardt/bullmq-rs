use std::collections::HashMap;
use serde::{Deserialize, Serialize};
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
    Exponential {
        #[serde(with = "duration_millis")]
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
}

impl Default for WorkerOptions {
    fn default() -> Self {
        Self {
            concurrency: 1,
            lock_duration: Duration::from_secs(30),
            stalled_interval: Duration::from_secs(30),
            max_stalled_count: 1,
            skip_stalled_check: false,
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
