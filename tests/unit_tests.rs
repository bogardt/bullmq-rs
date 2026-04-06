use bullmq_rs::*;
use std::collections::HashMap;
use std::time::Duration;

// ---------------------------------------------------------------------------
// JobState tests
// ---------------------------------------------------------------------------

#[test]
fn test_job_state_display() {
    assert_eq!(JobState::Wait.to_string(), "wait");
    assert_eq!(JobState::Delayed.to_string(), "delayed");
    assert_eq!(JobState::Active.to_string(), "active");
    assert_eq!(JobState::Completed.to_string(), "completed");
    assert_eq!(JobState::Failed.to_string(), "failed");
}

#[test]
fn test_job_state_parse() {
    assert_eq!("wait".parse::<JobState>().unwrap(), JobState::Wait);
    assert_eq!("delayed".parse::<JobState>().unwrap(), JobState::Delayed);
    assert_eq!("active".parse::<JobState>().unwrap(), JobState::Active);
    assert_eq!(
        "completed".parse::<JobState>().unwrap(),
        JobState::Completed
    );
    assert_eq!("failed".parse::<JobState>().unwrap(), JobState::Failed);
    assert!("invalid".parse::<JobState>().is_err());
}

#[test]
fn test_job_state_wait_variant() {
    // Wait
    assert_eq!(JobState::Wait.to_string(), "wait");
    assert_eq!("wait".parse::<JobState>().unwrap(), JobState::Wait);

    // Paused
    assert_eq!(JobState::Paused.to_string(), "paused");
    assert_eq!("paused".parse::<JobState>().unwrap(), JobState::Paused);

    // WaitingChildren
    assert_eq!(JobState::WaitingChildren.to_string(), "waiting-children");
    assert_eq!(
        "waiting-children".parse::<JobState>().unwrap(),
        JobState::WaitingChildren
    );
}

#[test]
fn test_job_state_serde_roundtrip() {
    let states = vec![
        (JobState::Wait, "\"wait\""),
        (JobState::Paused, "\"paused\""),
        (JobState::WaitingChildren, "\"waiting-children\""),
        (JobState::Delayed, "\"delayed\""),
        (JobState::Active, "\"active\""),
        (JobState::Completed, "\"completed\""),
        (JobState::Failed, "\"failed\""),
    ];
    for (state, expected_json) in states {
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, expected_json, "serializing {:?}", state);
        let restored: JobState = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, state, "deserializing {:?}", state);
    }
}

// ---------------------------------------------------------------------------
// Backoff tests
// ---------------------------------------------------------------------------

#[test]
fn test_backoff_fixed() {
    let strategy = BackoffStrategy::Fixed {
        delay: Duration::from_secs(5),
    };
    assert_eq!(strategy.delay_for_attempt(0), Duration::from_secs(5));
    assert_eq!(strategy.delay_for_attempt(1), Duration::from_secs(5));
    assert_eq!(strategy.delay_for_attempt(10), Duration::from_secs(5));
}

#[test]
fn test_backoff_exponential() {
    let strategy = BackoffStrategy::Exponential {
        base: Duration::from_secs(1),
        max: Duration::from_secs(30),
    };
    assert_eq!(strategy.delay_for_attempt(0), Duration::from_secs(1));
    assert_eq!(strategy.delay_for_attempt(1), Duration::from_secs(2));
    assert_eq!(strategy.delay_for_attempt(2), Duration::from_secs(4));
    assert_eq!(strategy.delay_for_attempt(3), Duration::from_secs(8));
    // Should cap at max
    assert_eq!(strategy.delay_for_attempt(10), Duration::from_secs(30));
}

#[test]
fn test_backoff_strategy_serialization() {
    let strategy = BackoffStrategy::Exponential {
        base: Duration::from_secs(1),
        max: Duration::from_secs(60),
    };
    let json = serde_json::to_string(&strategy).unwrap();
    let restored: BackoffStrategy = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.delay_for_attempt(0), Duration::from_secs(1));
    assert_eq!(restored.delay_for_attempt(5), Duration::from_secs(32));
}

// ---------------------------------------------------------------------------
// JobOptions tests
// ---------------------------------------------------------------------------

#[test]
fn test_job_options_default() {
    let opts = JobOptions::default();
    assert!(opts.priority.is_none());
    assert!(opts.delay.is_none());
    assert!(opts.attempts.is_none());
    assert!(opts.backoff.is_none());
    assert!(opts.ttl.is_none());
    assert!(opts.job_id.is_none());
}

#[test]
fn test_job_options_u32_priority() {
    let opts = JobOptions {
        priority: Some(42u32),
        ..Default::default()
    };
    assert_eq!(opts.priority, Some(42u32));

    // Ensure u32 max is representable
    let opts_max = JobOptions {
        priority: Some(u32::MAX),
        ..Default::default()
    };
    assert_eq!(opts_max.priority, Some(u32::MAX));
}

#[test]
fn test_job_options_serialization() {
    let opts = JobOptions {
        priority: Some(5),
        delay: Some(Duration::from_millis(1000)),
        attempts: Some(3),
        backoff: Some(BackoffStrategy::Fixed {
            delay: Duration::from_millis(2000),
        }),
        ttl: None,
        job_id: None,
    };
    let json = serde_json::to_string(&opts).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["priority"], 5);
    assert_eq!(parsed["delay"], 1000);
    assert_eq!(parsed["attempts"], 3);
    assert_eq!(parsed["backoff"]["type"], "fixed");
    assert_eq!(parsed["backoff"]["delay"], 2000);

    // Round-trip
    let restored: JobOptions = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.priority, Some(5));
    assert_eq!(restored.delay, Some(Duration::from_millis(1000)));
    assert_eq!(restored.attempts, Some(3));
}

// ---------------------------------------------------------------------------
// Job creation tests
// ---------------------------------------------------------------------------

#[test]
fn test_job_creation_defaults() {
    let job = Job::new("1".into(), "test".into(), "hello".to_string(), None);

    assert_eq!(job.id, "1");
    assert_eq!(job.name, "test");
    assert_eq!(job.data, "hello");
    assert_eq!(job.state, JobState::Wait);
    assert_eq!(job.priority, 0u32);
    assert!(job.timestamp > 0);
    assert_eq!(job.delay, 0);
    assert_eq!(job.attempts_made, 0);
    assert_eq!(job.attempts_started, 0);
    assert!(job.progress.is_none());
    assert!(job.processed_on.is_none());
    assert!(job.finished_on.is_none());
    assert!(job.failed_reason.is_none());
    assert!(job.stacktrace.is_empty());
    assert!(job.return_value.is_none());
    assert!(job.processed_by.is_none());
    // max_attempts is now accessed through opts
    assert_eq!(job.opts.attempts, None);
}

#[test]
fn test_job_creation_with_options() {
    let opts = JobOptions {
        priority: Some(5),
        delay: Some(Duration::from_secs(10)),
        attempts: Some(3),
        backoff: Some(BackoffStrategy::Fixed {
            delay: Duration::from_secs(2),
        }),
        ttl: Some(Duration::from_secs(60)),
        job_id: None,
    };

    let job = Job::new("2".into(), "delayed-job".into(), 42i32, Some(opts));

    // State is ALWAYS Wait, regardless of delay
    assert_eq!(job.state, JobState::Wait);
    assert_eq!(job.priority, 5u32);
    assert_eq!(job.delay, 10_000);

    // Options are stored in the opts field
    assert_eq!(job.opts.attempts, Some(3));
    assert!(job.opts.backoff.is_some());
    assert_eq!(
        job.opts.ttl,
        Some(Duration::from_secs(60))
    );
}

#[test]
fn test_job_serialization_roundtrip() {
    let job = Job::new("42".into(), "email".into(), "payload".to_string(), None);

    let hash = job.to_redis_hash().unwrap();
    let map: HashMap<String, String> = hash.into_iter().collect();

    // Verify BullMQ field names
    assert!(map.contains_key("name"));
    assert!(map.contains_key("data"));
    assert!(map.contains_key("opts"));
    assert!(map.contains_key("timestamp"));
    assert!(map.contains_key("delay"));
    assert!(map.contains_key("priority"));
    assert!(map.contains_key("atm"));
    assert!(map.contains_key("ats"));

    // Must NOT contain snake_case variants
    assert!(!map.contains_key("attempts_made"));
    assert!(!map.contains_key("attempts_started"));
    assert!(!map.contains_key("processed_on"));
    assert!(!map.contains_key("finished_on"));
    assert!(!map.contains_key("failed_reason"));
    assert!(!map.contains_key("return_value"));
    assert!(!map.contains_key("processed_by"));

    // Round-trip
    let restored = Job::<String>::from_redis_hash("42", &map).unwrap();
    assert_eq!(restored.id, "42");
    assert_eq!(restored.name, "email");
    assert_eq!(restored.data, "payload".to_string());
    assert_eq!(restored.priority, 0);
    assert_eq!(restored.delay, 0);
    assert_eq!(restored.attempts_made, 0);
    assert_eq!(restored.attempts_started, 0);
}

// ---------------------------------------------------------------------------
// v2-specific tests
// ---------------------------------------------------------------------------

#[test]
fn test_job_v2_fields() {
    let job = Job::new("1".into(), "test".into(), "data".to_string(), None);

    // Verify new v2 fields exist and have correct defaults
    assert_eq!(job.attempts_started, 0);
    assert!(job.stacktrace.is_empty());
    assert!(job.processed_by.is_none());

    // Verify opts field is stored
    let _opts: &JobOptions = &job.opts;
}

#[test]
fn test_job_v2_redis_hash_field_names() {
    use serde_json::json;

    let mut job = Job::new("1".into(), "test".into(), "data".to_string(), None);
    job.attempts_made = 3;
    job.attempts_started = 4;
    job.processed_on = Some(1700000000000);
    job.finished_on = Some(1700000001000);
    job.failed_reason = Some("timeout".into());
    job.return_value = Some(json!({"result": "ok"}));
    job.stacktrace = vec!["Error: timeout".into(), "  at process".into()];
    job.processed_by = Some("worker-1".into());

    let hash = job.to_redis_hash().unwrap();
    let map: HashMap<String, String> = hash.into_iter().collect();

    // BullMQ-compatible field names
    assert_eq!(map.get("atm").unwrap(), "3");
    assert_eq!(map.get("ats").unwrap(), "4");
    assert_eq!(map.get("processedOn").unwrap(), "1700000000000");
    assert_eq!(map.get("finishedOn").unwrap(), "1700000001000");
    assert_eq!(map.get("pb").unwrap(), "worker-1");

    // failedReason is stored as JSON string
    let failed_reason: String = serde_json::from_str(map.get("failedReason").unwrap()).unwrap();
    assert_eq!(failed_reason, "timeout");

    // returnvalue (all lowercase) is stored as JSON
    let return_val: serde_json::Value =
        serde_json::from_str(map.get("returnvalue").unwrap()).unwrap();
    assert_eq!(return_val, json!({"result": "ok"}));

    // stacktrace is stored as JSON array
    let st: Vec<String> = serde_json::from_str(map.get("stacktrace").unwrap()).unwrap();
    assert_eq!(st.len(), 2);
    assert_eq!(st[0], "Error: timeout");

    // Must NOT have snake_case keys
    assert!(!map.contains_key("attempts_made"));
    assert!(!map.contains_key("attempts_started"));
    assert!(!map.contains_key("processed_on"));
    assert!(!map.contains_key("finished_on"));
    assert!(!map.contains_key("failed_reason"));
    assert!(!map.contains_key("return_value"));
    assert!(!map.contains_key("processed_by"));
}

#[test]
fn test_job_v2_roundtrip() {
    use serde_json::json;

    let opts = JobOptions {
        priority: Some(10),
        delay: Some(Duration::from_millis(5000)),
        attempts: Some(5),
        backoff: Some(BackoffStrategy::Exponential {
            base: Duration::from_millis(1000),
            max: Duration::from_millis(30000),
        }),
        ttl: Some(Duration::from_millis(60000)),
        job_id: Some("custom-id".into()),
    };

    let mut job = Job::new("custom-id".into(), "process".into(), json!({"key": "value"}), Some(opts));
    job.attempts_made = 2;
    job.attempts_started = 3;
    job.processed_on = Some(1700000000000);
    job.finished_on = Some(1700000001000);
    job.failed_reason = Some("network error".into());
    job.return_value = Some(json!(42));
    job.stacktrace = vec!["Error: network error".into()];
    job.processed_by = Some("worker-abc".into());
    job.progress = Some(json!(75));

    let hash = job.to_redis_hash().unwrap();
    let map: HashMap<String, String> = hash.into_iter().collect();

    let restored = Job::<serde_json::Value>::from_redis_hash("custom-id", &map).unwrap();

    assert_eq!(restored.id, "custom-id");
    assert_eq!(restored.name, "process");
    assert_eq!(restored.data, json!({"key": "value"}));
    assert_eq!(restored.priority, 10);
    assert_eq!(restored.delay, 5000);
    assert_eq!(restored.attempts_made, 2);
    assert_eq!(restored.attempts_started, 3);
    assert_eq!(restored.processed_on, Some(1700000000000));
    assert_eq!(restored.finished_on, Some(1700000001000));
    assert_eq!(restored.failed_reason, Some("network error".into()));
    assert_eq!(restored.return_value, Some(json!(42)));
    assert_eq!(restored.stacktrace, vec!["Error: network error".to_string()]);
    assert_eq!(restored.processed_by, Some("worker-abc".into()));
    assert_eq!(restored.progress, Some(json!(75)));

    // Verify opts survived the roundtrip
    assert_eq!(restored.opts.priority, Some(10));
    assert_eq!(restored.opts.delay, Some(Duration::from_millis(5000)));
    assert_eq!(restored.opts.attempts, Some(5));
    assert_eq!(restored.opts.ttl, Some(Duration::from_millis(60000)));
    assert_eq!(restored.opts.job_id, Some("custom-id".into()));
    assert!(restored.opts.backoff.is_some());
}

#[test]
fn test_job_v2_from_redis_hash_tolerates_unknown_fields() {
    let mut map = HashMap::new();
    map.insert("name".into(), "test".into());
    map.insert("data".into(), "\"hello\"".into());
    map.insert("timestamp".into(), "1700000000000".into());
    // Add unknown fields that BullMQ Lua scripts may set
    map.insert("stc".into(), "0".into());
    map.insert("rjk".into(), "some-key".into());

    let job = Job::<String>::from_redis_hash("1", &map).unwrap();
    assert_eq!(job.name, "test");
    assert_eq!(job.data, "hello".to_string());
    // Unknown fields are silently ignored
}

#[test]
fn test_job_v2_from_redis_hash_missing_optional_fields() {
    let mut map = HashMap::new();
    map.insert("name".into(), "test".into());
    map.insert("data".into(), "\"hello\"".into());
    map.insert("timestamp".into(), "1700000000000".into());

    let job = Job::<String>::from_redis_hash("1", &map).unwrap();
    assert_eq!(job.priority, 0);
    assert_eq!(job.delay, 0);
    assert_eq!(job.attempts_made, 0);
    assert_eq!(job.attempts_started, 0);
    assert!(job.progress.is_none());
    assert!(job.processed_on.is_none());
    assert!(job.finished_on.is_none());
    assert!(job.failed_reason.is_none());
    assert!(job.stacktrace.is_empty());
    assert!(job.return_value.is_none());
    assert!(job.processed_by.is_none());
    // Default opts when none in hash
    assert!(job.opts.priority.is_none());
}

// ---------------------------------------------------------------------------
// WorkerOptions tests
// ---------------------------------------------------------------------------

#[test]
fn test_worker_options_v2_defaults() {
    let opts = WorkerOptions::default();
    assert_eq!(opts.concurrency, 1);
    assert_eq!(opts.lock_duration, Duration::from_secs(30));
    assert_eq!(opts.stalled_interval, Duration::from_secs(30));
    assert_eq!(opts.max_stalled_count, 1);
    assert!(!opts.skip_stalled_check);
}

// ---------------------------------------------------------------------------
// RedisConnection tests
// ---------------------------------------------------------------------------

#[test]
fn test_redis_connection_default() {
    let conn = RedisConnection::default();
    assert_eq!(conn.url(), "redis://127.0.0.1:6379");
}

#[test]
fn test_redis_connection_custom() {
    let conn = RedisConnection::new("redis://myhost:6380/1");
    assert_eq!(conn.url(), "redis://myhost:6380/1");
}

// ---------------------------------------------------------------------------
// Error tests
// ---------------------------------------------------------------------------

#[test]
fn test_error_display() {
    let err = BullmqError::JobNotFound("123".into());
    assert_eq!(err.to_string(), "Job not found: 123");

    let err = BullmqError::WorkerClosed;
    assert_eq!(err.to_string(), "Worker has been shut down");

    let err = BullmqError::Other("custom error".into());
    assert_eq!(err.to_string(), "custom error");
}

#[test]
fn test_error_from_redis() {
    let redis_err = redis::RedisError::from((
        redis::ErrorKind::Io,
        "connection refused",
    ));
    let err: BullmqError = redis_err.into();
    assert!(matches!(err, BullmqError::Redis(_)));
    assert!(err.to_string().contains("connection refused"));
}

#[test]
fn test_error_new_variants() {
    let err = BullmqError::LockMismatch;
    assert_eq!(
        err.to_string(),
        "Lock token mismatch: job was stalled and recovered"
    );

    let err = BullmqError::QueuePaused;
    assert_eq!(err.to_string(), "Queue is paused");

    let err = BullmqError::ScriptError("NOSCRIPT No matching script".into());
    assert_eq!(
        err.to_string(),
        "Script error: NOSCRIPT No matching script"
    );
}
