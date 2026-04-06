use std::collections::HashMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{BullmqError, BullmqResult};
use crate::types::{JobOptions, JobState};

/// Returns the current time as milliseconds since the Unix epoch.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
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
            fields.push(("stacktrace".into(), serde_json::to_string(&self.stacktrace)?));
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
            map.get(key).ok_or_else(|| {
                BullmqError::Other(format!("Missing field '{}' in job hash", key))
            })
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
        let failed_reason = map
            .get("failedReason")
            .map(|s| serde_json::from_str(s))
            .transpose()?;
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
        })
    }
}
