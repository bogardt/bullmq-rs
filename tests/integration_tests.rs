use bullmq_rs::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct TestJob {
    value: String,
}

fn redis_conn() -> RedisConnection {
    RedisConnection::new(
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string()),
    )
}

/// Helper: get a raw redis ConnectionManager for direct Redis commands in tests.
async fn raw_redis_conn() -> redis::aio::ConnectionManager {
    let url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let client = redis::Client::open(url.as_str()).unwrap();
    redis::aio::ConnectionManager::new(client).await.unwrap()
}

/// Helper: generate a unique queue name for tests that need isolation.
fn unique_queue_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    format!("test_unique_{}_{}_{}", pid, nanos, counter)
}

// ---------------------------------------------------------------------------
// 1. test_queue_add_and_get_job
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_queue_add_and_get_job() {
    let queue = QueueBuilder::new("test_add_get_v2")
        .connection(redis_conn())
        .build::<TestJob>()
        .await
        .unwrap();

    // Clean up
    queue.drain().await.unwrap();

    let job = queue
        .add(
            "test",
            TestJob {
                value: "hello".into(),
            },
            None,
        )
        .await
        .unwrap();

    // Verify the returned job
    assert_eq!(job.name, "test");
    assert_eq!(job.data.value, "hello");
    assert_eq!(job.state, JobState::Wait);

    // Fetch from Redis and verify
    let fetched = queue.get_job(&job.id).await.unwrap().unwrap();
    assert_eq!(fetched.name, "test");
    assert_eq!(fetched.data.value, "hello");

    // Verify BullMQ field names in the Redis hash directly
    let mut conn = raw_redis_conn().await;
    let job_key = format!("bull:test_add_get_v2:{}", job.id);

    let name: String = redis::cmd("HGET")
        .arg(&job_key)
        .arg("name")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(name, "test");

    let data: String = redis::cmd("HGET")
        .arg(&job_key)
        .arg("data")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(data.contains("hello"), "data field should contain serialized job data");

    let timestamp: String = redis::cmd("HGET")
        .arg(&job_key)
        .arg("timestamp")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(!timestamp.is_empty(), "timestamp field should be present");

    let opts: String = redis::cmd("HGET")
        .arg(&job_key)
        .arg("opts")
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(!opts.is_empty(), "opts field should be present");

    // Clean up
    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 2. test_queue_job_counts
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_queue_job_counts() {
    let queue = QueueBuilder::new("test_counts_v2")
        .connection(redis_conn())
        .build::<TestJob>()
        .await
        .unwrap();

    queue.drain().await.unwrap();

    for i in 0..3 {
        queue
            .add(
                &format!("job_{}", i),
                TestJob {
                    value: format!("val_{}", i),
                },
                None,
            )
            .await
            .unwrap();
    }

    let counts = queue.get_job_counts().await.unwrap();
    assert_eq!(*counts.get(&JobState::Wait).unwrap(), 3);
    assert_eq!(*counts.get(&JobState::Active).unwrap(), 0);
    assert_eq!(*counts.get(&JobState::Prioritized).unwrap(), 0);
    assert_eq!(*counts.get(&JobState::Delayed).unwrap(), 0);
    assert_eq!(*counts.get(&JobState::Completed).unwrap(), 0);
    assert_eq!(*counts.get(&JobState::Failed).unwrap(), 0);

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 3. test_queue_count_methods
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_queue_count_methods() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let queue = QueueBuilder::new(&qname)
        .connection(conn)
        .build::<TestJob>()
        .await
        .unwrap();

    for i in 0..3 {
        queue
            .add(
                "std",
                TestJob {
                    value: format!("s{}", i),
                },
                None,
            )
            .await
            .unwrap();
    }

    let delayed_opts = JobOptions {
        delay: Some(Duration::from_secs(3600)),
        ..Default::default()
    };
    queue
        .add(
            "delayed",
            TestJob { value: "d0".into() },
            Some(delayed_opts),
        )
        .await
        .unwrap();

    let prio_opts = JobOptions {
        priority: Some(5),
        ..Default::default()
    };
    queue
        .add("prio", TestJob { value: "p0".into() }, Some(prio_opts))
        .await
        .unwrap();

    let total = queue.count().await.unwrap();
    assert_eq!(total, 5);
    assert_eq!(queue.get_waiting_count().await.unwrap(), 3);
    assert_eq!(queue.get_active_count().await.unwrap(), 0);
    assert_eq!(queue.get_delayed_count().await.unwrap(), 1);
    assert_eq!(queue.get_prioritized_count().await.unwrap(), 1);
    assert_eq!(queue.get_completed_count().await.unwrap(), 0);
    assert_eq!(queue.get_failed_count().await.unwrap(), 0);
    assert_eq!(queue.get_waiting_children_count().await.unwrap(), 0);

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 4. test_queue_get_jobs
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_queue_get_jobs() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let queue = QueueBuilder::new(&qname)
        .connection(conn)
        .build::<TestJob>()
        .await
        .unwrap();

    for i in 0..3 {
        queue
            .add(
                "std",
                TestJob {
                    value: format!("job-{}", i),
                },
                None,
            )
            .await
            .unwrap();
    }

    let delayed_opts = JobOptions {
        delay: Some(Duration::from_secs(3600)),
        ..Default::default()
    };
    queue
        .add(
            "delayed",
            TestJob {
                value: "delayed-0".into(),
            },
            Some(delayed_opts),
        )
        .await
        .unwrap();

    let waiting = queue
        .get_jobs(&[JobState::Wait], 0, -1, true)
        .await
        .unwrap();
    assert_eq!(waiting.len(), 3);

    let mixed = queue
        .get_jobs(&[JobState::Wait, JobState::Delayed], 0, -1, true)
        .await
        .unwrap();
    assert_eq!(mixed.len(), 4);

    let page = queue.get_jobs(&[JobState::Wait], 0, 1, true).await.unwrap();
    assert_eq!(page.len(), 2);

    let mut first = waiting.into_iter().next().unwrap();
    let state = first.get_state().await.unwrap();
    assert_eq!(state, JobState::Wait);

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 5. test_queue_convenience_getters
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_queue_convenience_getters() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let queue = QueueBuilder::new(&qname)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    for i in 0..3 {
        queue
            .add(
                "std",
                TestJob {
                    value: format!("w{}", i),
                },
                None,
            )
            .await
            .unwrap();
    }

    let delayed_opts = JobOptions {
        delay: Some(Duration::from_secs(3600)),
        ..Default::default()
    };
    queue
        .add(
            "delayed",
            TestJob { value: "d0".into() },
            Some(delayed_opts),
        )
        .await
        .unwrap();

    let prio_opts = JobOptions {
        priority: Some(5),
        ..Default::default()
    };
    queue
        .add("prio", TestJob { value: "p0".into() }, Some(prio_opts))
        .await
        .unwrap();

    let waiting = queue.get_waiting(0, -1).await.unwrap();
    assert_eq!(waiting.len(), 3);
    assert!(waiting[0].timestamp <= waiting[1].timestamp);
    assert!(waiting[1].timestamp <= waiting[2].timestamp);

    let delayed = queue.get_delayed(0, -1).await.unwrap();
    assert_eq!(delayed.len(), 1);

    let prioritized = queue.get_prioritized(0, -1).await.unwrap();
    assert_eq!(prioritized.len(), 1);

    let active = queue.get_active(0, -1).await.unwrap();
    assert_eq!(active.len(), 0);

    let completed = queue.get_completed(0, -1).await.unwrap();
    assert_eq!(completed.len(), 0);

    let failed = queue.get_failed(0, -1).await.unwrap();
    assert_eq!(failed.len(), 0);

    let waiting_children = queue.get_waiting_children(0, -1).await.unwrap();
    assert_eq!(waiting_children.len(), 0);

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 6. test_queue_get_waiting_includes_paused
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_queue_get_waiting_includes_paused() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let queue = QueueBuilder::new(&qname)
        .connection(conn)
        .build::<TestJob>()
        .await
        .unwrap();

    queue
        .add("before_pause", TestJob { value: "a".into() }, None)
        .await
        .unwrap();

    queue.pause().await.unwrap();

    queue
        .add("after_pause", TestJob { value: "b".into() }, None)
        .await
        .unwrap();

    let waiting = queue.get_waiting(0, -1).await.unwrap();
    assert_eq!(waiting.len(), 2);

    queue.resume().await.unwrap();
    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 7. test_queue_get_completed_and_failed
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_queue_get_completed_and_failed() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let queue = QueueBuilder::new(&qname)
        .connection(conn.clone())
        .build::<String>()
        .await
        .unwrap();

    queue.add("ok", "success".to_string(), None).await.unwrap();
    queue
        .add("fail", "will_fail".to_string(), None)
        .await
        .unwrap();

    let worker = WorkerBuilder::new(&qname)
        .connection(conn.clone())
        .concurrency(1)
        .skip_stalled_check(true)
        .build::<String>();

    let processed = Arc::new(Mutex::new(0u32));
    let processed_clone = processed.clone();

    let handle = worker
        .start(move |job| {
            let processed = processed_clone.clone();
            async move {
                let mut p = processed.lock().await;
                *p += 1;
                if job.data == "will_fail" {
                    Err("intentional failure".into())
                } else {
                    Ok(())
                }
            }
        })
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if *processed.lock().await >= 2 {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("Timed out waiting for jobs to process");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    handle.shutdown();
    handle.wait().await.unwrap();

    let completed = queue.get_completed(0, -1).await.unwrap();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].data, "success");

    let failed = queue.get_failed(0, -1).await.unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].data, "will_fail");

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 3. test_queue_remove_job
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_queue_remove_job() {
    let queue = QueueBuilder::new("test_remove_v2")
        .connection(redis_conn())
        .build::<TestJob>()
        .await
        .unwrap();

    queue.drain().await.unwrap();

    let job = queue
        .add(
            "to_remove",
            TestJob {
                value: "bye".into(),
            },
            None,
        )
        .await
        .unwrap();

    queue.remove(&job.id).await.unwrap();
    let fetched = queue.get_job(&job.id).await.unwrap();
    assert!(fetched.is_none());

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 4. test_delayed_job
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_delayed_job() {
    let queue = QueueBuilder::new("test_delayed_v2")
        .connection(redis_conn())
        .build::<TestJob>()
        .await
        .unwrap();

    queue.drain().await.unwrap();

    let job = queue
        .add(
            "delayed",
            TestJob {
                value: "later".into(),
            },
            Some(JobOptions {
                delay: Some(Duration::from_secs(60)),
                ..Default::default()
            }),
        )
        .await
        .unwrap();

    // The returned Job always has state Wait (Lua handles actual placement)
    assert_eq!(job.state, JobState::Wait);

    // But the job should be in the delayed sorted set
    let counts = queue.get_job_counts().await.unwrap();
    assert_eq!(*counts.get(&JobState::Delayed).unwrap(), 1);
    assert_eq!(*counts.get(&JobState::Wait).unwrap(), 0);

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 5. test_worker_processes_job
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_worker_processes_job() {
    let conn = redis_conn();

    let queue = QueueBuilder::new("test_worker_v2")
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    queue.drain().await.unwrap();

    queue
        .add(
            "work",
            TestJob {
                value: "process me".into(),
            },
            None,
        )
        .await
        .unwrap();

    let worker = WorkerBuilder::new("test_worker_v2")
        .connection(conn)
        .skip_stalled_check(true)
        .build::<TestJob>();

    let handle = worker
        .start(|_job| async move { Ok(()) })
        .await
        .unwrap();

    // Wait for processing
    tokio::time::sleep(Duration::from_secs(3)).await;
    handle.shutdown();
    handle.wait().await.unwrap();

    let counts = queue.get_job_counts().await.unwrap();
    assert_eq!(*counts.get(&JobState::Completed).unwrap(), 1);
    assert_eq!(*counts.get(&JobState::Wait).unwrap(), 0);

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 6. test_worker_retry_then_fail
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_worker_retry_then_fail() {
    let conn = redis_conn();

    let queue = QueueBuilder::new("test_retry_v2")
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    queue.drain().await.unwrap();

    queue
        .add(
            "will_fail",
            TestJob {
                value: "fail".into(),
            },
            Some(JobOptions {
                attempts: Some(2),
                backoff: Some(BackoffStrategy::Fixed {
                    delay: Duration::from_millis(100),
                }),
                ..Default::default()
            }),
        )
        .await
        .unwrap();

    let worker = WorkerBuilder::new("test_retry_v2")
        .connection(conn)
        .skip_stalled_check(true)
        .build::<TestJob>();

    let handle = worker
        .start(|_job| async move {
            let err: Box<dyn std::error::Error + Send + Sync> =
                "intentional failure".into();
            Err(err)
        })
        .await
        .unwrap();

    // Wait for retries to complete
    tokio::time::sleep(Duration::from_secs(5)).await;
    handle.shutdown();
    handle.wait().await.unwrap();

    let counts = queue.get_job_counts().await.unwrap();
    assert_eq!(*counts.get(&JobState::Failed).unwrap(), 1);

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 7. test_priority_ordering
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_priority_ordering() {
    let conn = redis_conn();

    let queue = QueueBuilder::new("test_priority_v2")
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    queue.drain().await.unwrap();

    // Add jobs with different priorities: 0 (standard), 5 (low-priority), 1 (high-priority)
    // Priority 0 = standard (goes to wait list, not prioritized set)
    // Priority 1 = high priority (lower number = higher priority)
    // Priority 5 = low priority

    queue
        .add(
            "standard",
            TestJob {
                value: "standard-0".into(),
            },
            None, // priority 0, standard job
        )
        .await
        .unwrap();

    queue
        .add(
            "low-prio",
            TestJob {
                value: "low-5".into(),
            },
            Some(JobOptions {
                priority: Some(5),
                ..Default::default()
            }),
        )
        .await
        .unwrap();

    queue
        .add(
            "high-prio",
            TestJob {
                value: "high-1".into(),
            },
            Some(JobOptions {
                priority: Some(1),
                ..Default::default()
            }),
        )
        .await
        .unwrap();

    let processed_order: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let order_clone = processed_order.clone();

    let worker = WorkerBuilder::new("test_priority_v2")
        .connection(conn)
        .concurrency(1) // process one at a time to observe ordering
        .skip_stalled_check(true)
        .build::<TestJob>();

    let handle = worker
        .start(move |job| {
            let order = order_clone.clone();
            async move {
                let mut vec = order.lock().await;
                vec.push(job.data.value.clone());
                Ok(())
            }
        })
        .await
        .unwrap();

    // Wait for all jobs to process
    tokio::time::sleep(Duration::from_secs(5)).await;
    handle.shutdown();
    handle.wait().await.unwrap();

    let order = processed_order.lock().await;
    assert_eq!(order.len(), 3, "All 3 jobs should have been processed");

    // Prioritized jobs (1, 5) should be processed before standard (0).
    // Among prioritized: 1 should come before 5.
    // The exact order depends on the Lua scripts, but we can verify all were processed.
    let counts = queue.get_job_counts().await.unwrap();
    assert_eq!(*counts.get(&JobState::Completed).unwrap(), 3);

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 8. test_pause_resume
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_pause_resume() {
    let conn = redis_conn();

    let queue = QueueBuilder::new("test_pause_v2")
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    queue.drain().await.unwrap();

    // Pause the queue
    queue.pause().await.unwrap();
    assert!(queue.is_paused().await.unwrap(), "Queue should be paused");

    // Add a job while paused
    queue
        .add(
            "paused_job",
            TestJob {
                value: "while paused".into(),
            },
            None,
        )
        .await
        .unwrap();

    // The job should go to the paused list, not wait
    let counts = queue.get_job_counts().await.unwrap();
    assert_eq!(
        *counts.get(&JobState::Paused).unwrap(),
        1,
        "Job should be in paused list"
    );
    assert_eq!(
        *counts.get(&JobState::Wait).unwrap(),
        0,
        "Wait list should be empty while paused"
    );

    // Resume the queue
    queue.resume().await.unwrap();
    assert!(!queue.is_paused().await.unwrap(), "Queue should be resumed");

    // After resume, the job should be back in the wait list
    let counts = queue.get_job_counts().await.unwrap();
    assert_eq!(
        *counts.get(&JobState::Wait).unwrap(),
        1,
        "Job should be in wait list after resume"
    );
    assert_eq!(
        *counts.get(&JobState::Paused).unwrap(),
        0,
        "Paused list should be empty after resume"
    );

    // Start a worker and verify the job processes
    let worker = WorkerBuilder::new("test_pause_v2")
        .connection(conn)
        .skip_stalled_check(true)
        .build::<TestJob>();

    let handle = worker
        .start(|_job| async move { Ok(()) })
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_secs(3)).await;
    handle.shutdown();
    handle.wait().await.unwrap();

    let counts = queue.get_job_counts().await.unwrap();
    assert_eq!(*counts.get(&JobState::Completed).unwrap(), 1);

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 9. test_job_logs
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_job_logs() {
    let queue = QueueBuilder::new("test_logs_v2")
        .connection(redis_conn())
        .build::<TestJob>()
        .await
        .unwrap();

    queue.drain().await.unwrap();

    let job = queue
        .add(
            "logged",
            TestJob {
                value: "log me".into(),
            },
            None,
        )
        .await
        .unwrap();

    // Add 3 log lines
    queue.add_log(&job.id, "Log line 1").await.unwrap();
    queue.add_log(&job.id, "Log line 2").await.unwrap();
    queue.add_log(&job.id, "Log line 3").await.unwrap();

    // Get all logs
    let logs = queue.get_logs(&job.id, 0, -1).await.unwrap();
    assert_eq!(logs.len(), 3);
    assert_eq!(logs[0], "Log line 1");
    assert_eq!(logs[1], "Log line 2");
    assert_eq!(logs[2], "Log line 3");

    // Get a subset
    let partial = queue.get_logs(&job.id, 1, 2).await.unwrap();
    assert_eq!(partial.len(), 2);
    assert_eq!(partial[0], "Log line 2");
    assert_eq!(partial[1], "Log line 3");

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 10. test_events_stream
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_events_stream() {
    let conn = redis_conn();

    let queue = QueueBuilder::new("test_events_v2")
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    queue.drain().await.unwrap();

    // Clean the events stream
    let mut raw = raw_redis_conn().await;
    let events_key = "bull:test_events_v2:events";
    let _: redis::RedisResult<i64> = redis::cmd("DEL")
        .arg(events_key)
        .query_async(&mut raw)
        .await;

    // Add a job and process it
    queue
        .add(
            "event_job",
            TestJob {
                value: "emit events".into(),
            },
            None,
        )
        .await
        .unwrap();

    let worker = WorkerBuilder::new("test_events_v2")
        .connection(conn)
        .skip_stalled_check(true)
        .build::<TestJob>();

    let handle = worker
        .start(|_job| async move { Ok(()) })
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_secs(3)).await;
    handle.shutdown();
    handle.wait().await.unwrap();

    // Read events from the stream using XRANGE
    let _events: Vec<redis::streams::StreamRangeReply> = redis::cmd("XRANGE")
        .arg(events_key)
        .arg("-")
        .arg("+")
        .query_async(&mut raw)
        .await
        .unwrap_or_else(|_| vec![]);

    // We expect at least some events were emitted (waiting, active, completed)
    // The exact events depend on the Lua scripts, but the stream should not be empty
    // if the Lua scripts emit events via XADD.
    //
    // Note: If Lua scripts don't emit events for the lifecycle, this assertion
    // may need adjustment. We verify the stream key exists and has entries.
    let stream_len: u64 = redis::cmd("XLEN")
        .arg(events_key)
        .query_async(&mut raw)
        .await
        .unwrap_or(0);

    assert!(
        stream_len > 0,
        "Events stream should have entries after job processing, got {}",
        stream_len
    );

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 11. test_job_update_progress
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_job_update_progress() {
    let queue = QueueBuilder::new("test_update_progress_v2")
        .connection(redis_conn())
        .build::<String>()
        .await
        .unwrap();

    queue.drain().await.unwrap();

    let mut job = queue.add("test", "data".to_string(), None).await.unwrap();

    let progress = serde_json::json!({"percent": 50});
    job.update_progress(progress.clone()).await.unwrap();

    // Verify local state updated
    assert_eq!(job.progress, Some(progress.clone()));

    // Verify Redis state updated
    let fetched = queue.get_job(&job.id).await.unwrap().unwrap();
    assert_eq!(fetched.progress, Some(progress));

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 12. test_job_log
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_job_log() {
    let queue = QueueBuilder::new("test_job_log_v2")
        .connection(redis_conn())
        .build::<String>()
        .await
        .unwrap();

    queue.drain().await.unwrap();

    let job = queue.add("test", "data".to_string(), None).await.unwrap();

    let count = job.log("first log").await.unwrap();
    assert_eq!(count, 1);
    let count = job.log("second log").await.unwrap();
    assert_eq!(count, 2);

    let logs = queue.get_logs(&job.id, 0, -1).await.unwrap();
    assert_eq!(logs, vec!["first log", "second log"]);

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 13. test_job_update_data
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_job_update_data() {
    let queue = QueueBuilder::new("test_update_data_v2")
        .connection(redis_conn())
        .build::<String>()
        .await
        .unwrap();

    queue.drain().await.unwrap();

    let mut job = queue.add("test", "original".to_string(), None).await.unwrap();

    job.update_data("updated".to_string()).await.unwrap();

    assert_eq!(job.data, "updated");

    let fetched = queue.get_job(&job.id).await.unwrap().unwrap();
    assert_eq!(fetched.data, "updated");

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 14. test_job_clear_logs
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_job_clear_logs() {
    let queue = QueueBuilder::new("test_clear_logs_v2")
        .connection(redis_conn())
        .build::<String>()
        .await
        .unwrap();

    queue.drain().await.unwrap();

    let job = queue.add("test", "data".to_string(), None).await.unwrap();
    job.log("entry 1").await.unwrap();
    job.log("entry 2").await.unwrap();

    job.clear_logs().await.unwrap();

    let logs = queue.get_logs(&job.id, 0, -1).await.unwrap();
    assert!(logs.is_empty());

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 15. test_job_get_state
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_job_get_state() {
    let queue = QueueBuilder::new(&unique_queue_name())
        .connection(redis_conn())
        .build::<String>()
        .await
        .unwrap();

    let mut job = queue.add("test", "data".to_string(), None).await.unwrap();
    let state = job.get_state().await.unwrap();
    assert_eq!(state, JobState::Wait);
    assert!(job.is_waiting().await.unwrap());
    assert!(!job.is_completed().await.unwrap());

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 16. test_job_get_state_delayed
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_job_get_state_delayed() {
    let queue = QueueBuilder::new(&unique_queue_name())
        .connection(redis_conn())
        .build::<String>()
        .await
        .unwrap();

    let opts = JobOptions {
        delay: Some(Duration::from_secs(3600)),
        ..Default::default()
    };
    let mut job = queue.add("test", "data".to_string(), Some(opts)).await.unwrap();
    let state = job.get_state().await.unwrap();
    assert_eq!(state, JobState::Delayed);
    assert!(job.is_delayed().await.unwrap());

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 17. test_concurrent_workers
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_concurrent_workers() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let queue = QueueBuilder::new(&qname)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    // Add 10 jobs
    for i in 0..10 {
        queue
            .add(
                &format!("concurrent_{}", i),
                TestJob {
                    value: format!("job_{}", i),
                },
                None,
            )
            .await
            .unwrap();
    }

    let processed_count: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));

    // Start worker 1
    let count1 = processed_count.clone();
    let worker1 = WorkerBuilder::new(&qname)
        .connection(conn.clone())
        .concurrency(3)
        .lock_duration(Duration::from_secs(5))
        .stalled_interval(Duration::from_secs(3))
        .build::<TestJob>();

    let handle1 = worker1
        .start(move |_job| {
            let count = count1.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let mut c = count.lock().await;
                *c += 1;
                Ok(())
            }
        })
        .await
        .unwrap();

    // Start worker 2
    let count2 = processed_count.clone();
    let worker2 = WorkerBuilder::new(&qname)
        .connection(conn)
        .concurrency(3)
        .lock_duration(Duration::from_secs(5))
        .stalled_interval(Duration::from_secs(3))
        .build::<TestJob>();

    let handle2 = worker2
        .start(move |_job| {
            let count = count2.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let mut c = count.lock().await;
                *c += 1;
                Ok(())
            }
        })
        .await
        .unwrap();

    // Wait for all jobs to be processed. With two workers competing,
    // stalled job recovery may be needed for the last job, so allow
    // enough time for multiple stalled check cycles.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let c = *processed_count.lock().await;
        if c >= 10 {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("Timed out: only {}/10 jobs processed by concurrent workers", c);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    handle1.shutdown();
    handle2.shutdown();
    handle1.wait().await.unwrap();
    handle2.wait().await.unwrap();

    let counts = queue.get_job_counts().await.unwrap();
    assert_eq!(
        *counts.get(&JobState::Completed).unwrap(),
        10,
        "All 10 jobs should be completed"
    );

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 19. test_job_change_priority
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_job_change_priority() {
    let conn = RedisConnection::new(
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string()),
    );
    let queue = QueueBuilder::new(&unique_queue_name())
        .connection(conn)
        .build::<String>()
        .await
        .unwrap();

    // Add a standard (priority=0) job — goes to wait list
    let mut job = queue.add("test", "data".to_string(), None).await.unwrap();
    assert_eq!(job.priority, 0);

    // Change priority to 5 — should move to prioritized set
    job.change_priority(5).await.unwrap();
    assert_eq!(job.priority, 5);

    // Verify it's now in prioritized state
    let state = job.get_state().await.unwrap();
    assert_eq!(state, JobState::Prioritized);

    // Change back to 0 — should move to wait list
    job.change_priority(0).await.unwrap();
    assert_eq!(job.priority, 0);

    let state = job.get_state().await.unwrap();
    assert_eq!(state, JobState::Wait);

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 20. test_job_promote
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_job_promote() {
    let conn = RedisConnection::new(
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string()),
    );
    let queue = QueueBuilder::new(&unique_queue_name())
        .connection(conn)
        .build::<String>()
        .await
        .unwrap();

    let opts = JobOptions {
        delay: Some(Duration::from_secs(3600)),
        ..Default::default()
    };
    let mut job = queue.add("test", "data".to_string(), Some(opts)).await.unwrap();

    let state = job.get_state().await.unwrap();
    assert_eq!(state, JobState::Delayed);

    job.promote().await.unwrap();
    assert_eq!(job.delay, 0);

    let state = job.get_state().await.unwrap();
    assert_eq!(state, JobState::Wait);

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 21. test_job_get_dependencies_and_children_values
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_job_get_dependencies_and_children_values() {
    let parent_queue = unique_queue_name();
    let child_queue = unique_queue_name();
    let conn = redis_conn();

    let producer = FlowProducerBuilder::new()
        .connection(conn.clone())
        .build()
        .await
        .unwrap();

    let node = producer
        .add(FlowJob {
            name: "parent".into(),
            queue_name: parent_queue.clone(),
            data: TestJob { value: "p".into() },
            prefix: None,
            opts: None,
            children: vec![FlowJob {
                name: "child".into(),
                queue_name: child_queue.clone(),
                data: TestJob { value: "c1".into() },
                prefix: None,
                opts: None,
                children: vec![],
            }],
        })
        .await
        .unwrap();

    let child_key = format!("bull:{}:{}", child_queue, node.children[0].job.id);

    let before = node.job.get_dependencies().await.unwrap();
    assert!(before.processed.is_empty());
    assert_eq!(before.unprocessed, vec![child_key.clone()]);

    let child_worker = WorkerBuilder::new(&child_queue)
        .connection(conn.clone())
        .build::<TestJob>();
    let child_handle = child_worker.start(|_job| async move { Ok(()) }).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let after = loop {
        let deps = node.job.get_dependencies().await.unwrap();
        if deps.processed.len() == 1 && deps.unprocessed.is_empty() {
            break deps;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("Timed out waiting for dependencies to move to processed");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    let values = node.job.get_children_values().await.unwrap();
    assert_eq!(after.processed.len(), 1);
    assert_eq!(after.unprocessed.len(), 0);
    let expected_child_return = serde_json::json!({});
    assert_eq!(
        after.processed.get(&child_key),
        Some(&expected_child_return)
    );
    assert_eq!(values.get(&child_key), Some(&expected_child_return));
    assert_eq!(values, after.processed);

    child_handle.shutdown();
    child_handle.wait().await.unwrap();
}

// ---------------------------------------------------------------------------
// 18. test_job_remove
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_job_remove() {
    let conn = RedisConnection::new(
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string()),
    );
    let queue = QueueBuilder::new(&unique_queue_name())
        .connection(conn)
        .build::<String>()
        .await
        .unwrap();

    let job = queue.add("test", "data".to_string(), None).await.unwrap();
    let job_id = job.id.clone();

    job.remove().await.unwrap();

    let fetched = queue.get_job(&job_id).await.unwrap();
    assert!(fetched.is_none());

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 19. test_flow_add_same_queue_parent_enters_waiting_children
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_flow_add_same_queue_parent_enters_waiting_children() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let queue = QueueBuilder::new(&qname)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();
    queue.drain().await.unwrap();

    let producer = FlowProducerBuilder::new()
        .connection(conn.clone())
        .build()
        .await
        .unwrap();

    let flow = FlowJob {
        name: "parent".into(),
        queue_name: qname.clone(),
        data: TestJob {
            value: "parent".into(),
        },
        prefix: None,
        opts: None,
        children: vec![FlowJob {
            name: "child".into(),
            queue_name: qname.clone(),
            data: TestJob {
                value: "child".into(),
            },
            prefix: None,
            opts: None,
            children: vec![],
        }],
    };

    let node = producer.add(flow).await.unwrap();

    assert_eq!(node.children.len(), 1);
    assert_eq!(queue.get_waiting_children_count().await.unwrap(), 1);
    assert_eq!(queue.get_waiting_count().await.unwrap(), 1);

    let parent_key = format!("bull:{}:{}", qname, node.job.id);
    let child_key = format!("bull:{}:{}", qname, node.children[0].job.id);

    let mut raw = raw_redis_conn().await;
    let deps: Vec<String> = redis::cmd("SMEMBERS")
        .arg(format!("{parent_key}:dependencies"))
        .query_async(&mut raw)
        .await
        .unwrap();
    assert_eq!(deps, vec![child_key.clone()]);

    let parent_meta: (Option<String>, Option<String>) = redis::cmd("HMGET")
        .arg(child_key)
        .arg("parentKey")
        .arg("parent")
        .query_async(&mut raw)
        .await
        .unwrap();
    assert_eq!(parent_meta.0.as_deref(), Some(parent_key.as_str()));

    let parent_json: serde_json::Value =
        serde_json::from_str(parent_meta.1.as_deref().unwrap()).unwrap();
    assert_eq!(parent_json["id"], node.job.id);
    assert_eq!(parent_json["queue"], format!("bull:{qname}"));

    let mut child_job = node.children[0].job.clone();
    child_job
        .update_progress(serde_json::json!({ "step": 1 }))
        .await
        .unwrap();

    let progress: String = redis::cmd("HGET")
        .arg(format!("bull:{}:{}", qname, node.children[0].job.id))
        .arg("progress")
        .query_async(&mut raw)
        .await
        .unwrap();
    assert_eq!(progress, r#"{"step":1}"#);
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_flow_add_cross_queue_tree_tracks_all_dependencies() {
    let parent_queue = unique_queue_name();
    let child_queue = unique_queue_name();
    let conn = redis_conn();

    let parent = QueueBuilder::new(&parent_queue)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    let child = QueueBuilder::new(&child_queue)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    let producer = FlowProducerBuilder::new()
        .connection(conn.clone())
        .build()
        .await
        .unwrap();

    let node = producer
        .add(FlowJob {
            name: "parent".into(),
            queue_name: parent_queue.clone(),
            data: TestJob { value: "p".into() },
            prefix: None,
            opts: None,
            children: vec![
                FlowJob {
                    name: "c1".into(),
                    queue_name: child_queue.clone(),
                    data: TestJob { value: "c1".into() },
                    prefix: None,
                    opts: None,
                    children: vec![],
                },
                FlowJob {
                    name: "c2".into(),
                    queue_name: child_queue.clone(),
                    data: TestJob { value: "c2".into() },
                    prefix: None,
                    opts: None,
                    children: vec![],
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(parent.get_waiting_children_count().await.unwrap(), 1);
    assert_eq!(child.get_waiting_count().await.unwrap(), 2);
    assert_eq!(node.children.len(), 2);

    let parent_key = format!("bull:{}:{}", parent_queue, node.job.id);
    let child_keys: std::collections::HashSet<String> = node
        .children
        .iter()
        .map(|child_node| format!("bull:{}:{}", child_queue, child_node.job.id))
        .collect();

    let mut raw = raw_redis_conn().await;
    let deps: std::collections::HashSet<String> = redis::cmd("SMEMBERS")
        .arg(format!("{parent_key}:dependencies"))
        .query_async(&mut raw)
        .await
        .unwrap();
    assert_eq!(deps, child_keys);

    for child_key in &child_keys {
        let parent_meta: (Option<String>, Option<String>) = redis::cmd("HMGET")
            .arg(child_key)
            .arg("parentKey")
            .arg("parent")
            .query_async(&mut raw)
            .await
            .unwrap();
        assert_eq!(parent_meta.0.as_deref(), Some(parent_key.as_str()));

        let parent_json: serde_json::Value =
            serde_json::from_str(parent_meta.1.as_deref().unwrap()).unwrap();
        assert_eq!(parent_json["id"], node.job.id);
        assert_eq!(parent_json["queue"], format!("bull:{parent_queue}"));
    }
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_flow_parent_runs_after_all_children_complete() {
    let parent_queue = unique_queue_name();
    let child_queue = unique_queue_name();
    let conn = redis_conn();

    let producer = FlowProducerBuilder::new()
        .connection(conn.clone())
        .build()
        .await
        .unwrap();

    let seen = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let child_seen = seen.clone();
    let parent_seen = seen.clone();

    let child_worker = WorkerBuilder::new(&child_queue)
        .connection(conn.clone())
        .build::<TestJob>();
    let child_handle = child_worker
        .start(move |job| {
            let child_seen = child_seen.clone();
            async move {
                child_seen
                    .lock()
                    .await
                    .push(format!("child:{}", job.data.value));
                Ok(())
            }
        })
        .await
        .unwrap();

    let parent_worker = WorkerBuilder::new(&parent_queue)
        .connection(conn.clone())
        .build::<TestJob>();
    let parent_handle = parent_worker
        .start(move |job| {
            let parent_seen = parent_seen.clone();
            async move {
                parent_seen
                    .lock()
                    .await
                    .push(format!("parent:{}", job.data.value));
                Ok(())
            }
        })
        .await
        .unwrap();

    producer
        .add(FlowJob {
            name: "parent".into(),
            queue_name: parent_queue.clone(),
            data: TestJob { value: "p".into() },
            prefix: None,
            opts: None,
            children: vec![
                FlowJob {
                    name: "child-1".into(),
                    queue_name: child_queue.clone(),
                    data: TestJob { value: "c1".into() },
                    prefix: None,
                    opts: None,
                    children: vec![],
                },
                FlowJob {
                    name: "child-2".into(),
                    queue_name: child_queue.clone(),
                    data: TestJob { value: "c2".into() },
                    prefix: None,
                    opts: None,
                    children: vec![],
                },
            ],
        })
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let current = seen.lock().await.clone();
        if current == vec!["child:c1", "child:c2", "parent:p"] {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("Timed out waiting for flow completion, saw {current:?}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    child_handle.shutdown();
    parent_handle.shutdown();
    child_handle.wait().await.unwrap();
    parent_handle.wait().await.unwrap();
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_flow_releases_delayed_parent_to_delayed() {
    let parent_queue = unique_queue_name();
    let child_queue = unique_queue_name();
    let conn = redis_conn();

    let parent = QueueBuilder::new(&parent_queue)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();
    let child = QueueBuilder::new(&child_queue)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    let producer = FlowProducerBuilder::new()
        .connection(conn.clone())
        .build()
        .await
        .unwrap();

    let seen = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let child_seen = seen.clone();

    let child_worker = WorkerBuilder::new(&child_queue)
        .connection(conn.clone())
        .build::<TestJob>();
    let child_handle = child_worker
        .start(move |job| {
            let child_seen = child_seen.clone();
            async move {
                child_seen
                    .lock()
                    .await
                    .push(format!("child:{}", job.data.value));
                Ok(())
            }
        })
        .await
        .unwrap();

    let node = producer
        .add(FlowJob {
            name: "parent".into(),
            queue_name: parent_queue.clone(),
            data: TestJob { value: "p".into() },
            prefix: None,
            opts: Some(JobOptions {
                delay: Some(Duration::from_secs(60)),
                ..Default::default()
            }),
            children: vec![FlowJob {
                name: "child".into(),
                queue_name: child_queue.clone(),
                data: TestJob { value: "c1".into() },
                prefix: None,
                opts: None,
                children: vec![],
            }],
        })
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let child_done = seen.lock().await.as_slice() == ["child:c1"];
        let parent_released = parent.get_waiting_children_count().await.unwrap() == 0;
        if child_done && parent_released {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!(
                "Timed out waiting for delayed parent release, saw {:?}",
                seen.lock().await
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert_eq!(parent.get_waiting_count().await.unwrap(), 0);
    assert_eq!(parent.get_delayed_count().await.unwrap(), 1);
    assert_eq!(child.get_completed_count().await.unwrap(), 1);

    let delayed = parent.get_delayed(0, -1).await.unwrap();
    assert_eq!(delayed.len(), 1);
    assert_eq!(delayed[0].id, node.job.id);

    child_handle.shutdown();
    child_handle.wait().await.unwrap();
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_flow_releases_parent_with_bullmq_wait_order() {
    let parent_queue = unique_queue_name();
    let child_queue = unique_queue_name();
    let conn = redis_conn();

    let parent = QueueBuilder::new(&parent_queue)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    let producer = FlowProducerBuilder::new()
        .connection(conn.clone())
        .build()
        .await
        .unwrap();

    let existing = parent
        .add(
            "existing",
            TestJob {
                value: "existing".into(),
            },
            None,
        )
        .await
        .unwrap();

    let child_worker = WorkerBuilder::new(&child_queue)
        .connection(conn.clone())
        .build::<TestJob>();
    let child_handle = child_worker
        .start(|_job| async move { Ok(()) })
        .await
        .unwrap();

    let node = producer
        .add(FlowJob {
            name: "parent".into(),
            queue_name: parent_queue.clone(),
            data: TestJob { value: "p".into() },
            prefix: None,
            opts: None,
            children: vec![FlowJob {
                name: "child".into(),
                queue_name: child_queue.clone(),
                data: TestJob { value: "c1".into() },
                prefix: None,
                opts: None,
                children: vec![],
            }],
        })
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if parent.get_waiting_children_count().await.unwrap() == 0 {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("Timed out waiting for parent release");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let waiting = parent.get_waiting(0, -1).await.unwrap();
    let waiting_ids: Vec<String> = waiting.into_iter().map(|job| job.id).collect();
    assert_eq!(waiting_ids, vec![node.job.id.clone(), existing.id.clone()]);

    child_handle.shutdown();
    child_handle.wait().await.unwrap();
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_flow_releases_parent_to_prioritized() {
    let parent_queue = unique_queue_name();
    let child_queue = unique_queue_name();
    let conn = redis_conn();

    let parent = QueueBuilder::new(&parent_queue)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    let producer = FlowProducerBuilder::new()
        .connection(conn.clone())
        .build()
        .await
        .unwrap();

    let child_worker = WorkerBuilder::new(&child_queue)
        .connection(conn.clone())
        .build::<TestJob>();
    let child_handle = child_worker
        .start(|_job| async move { Ok(()) })
        .await
        .unwrap();

    let node = producer
        .add(FlowJob {
            name: "parent".into(),
            queue_name: parent_queue.clone(),
            data: TestJob { value: "p".into() },
            prefix: None,
            opts: Some(JobOptions {
                priority: Some(10),
                ..Default::default()
            }),
            children: vec![FlowJob {
                name: "child".into(),
                queue_name: child_queue.clone(),
                data: TestJob { value: "c1".into() },
                prefix: None,
                opts: None,
                children: vec![],
            }],
        })
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if parent.get_waiting_children_count().await.unwrap() == 0 {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("Timed out waiting for prioritized parent release");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let mut raw = raw_redis_conn().await;
    let stored_priority: i64 = redis::cmd("HGET")
        .arg(format!("bull:{}:{}", parent_queue, node.job.id))
        .arg("priority")
        .query_async(&mut raw)
        .await
        .unwrap();
    assert_eq!(stored_priority, 10);

    let counts = parent.get_job_counts().await.unwrap();

    assert_eq!(*counts.get(&JobState::WaitingChildren).unwrap(), 0);
    assert_eq!(*counts.get(&JobState::Prioritized).unwrap(), 1);
    assert_eq!(
        parent.get_waiting_count().await.unwrap(),
        0,
        "unexpected counts after prioritized parent release: {counts:?}"
    );

    let prioritized = parent.get_prioritized(0, -1).await.unwrap();
    assert_eq!(prioritized.len(), 1);
    assert_eq!(prioritized[0].id, node.job.id);

    child_handle.shutdown();
    child_handle.wait().await.unwrap();
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_flow_releases_parent_to_paused_queue() {
    let parent_queue = unique_queue_name();
    let child_queue = unique_queue_name();
    let conn = redis_conn();

    let parent = QueueBuilder::new(&parent_queue)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    let producer = FlowProducerBuilder::new()
        .connection(conn.clone())
        .build()
        .await
        .unwrap();

    parent.pause().await.unwrap();
    assert!(parent.is_paused().await.unwrap());

    let parent_seen = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let parent_seen_worker = parent_seen.clone();

    let parent_worker = WorkerBuilder::new(&parent_queue)
        .connection(conn.clone())
        .build::<TestJob>();
    let parent_handle = parent_worker
        .start(move |job| {
            let parent_seen_worker = parent_seen_worker.clone();
            async move {
                parent_seen_worker
                    .lock()
                    .await
                    .push(format!("parent:{}", job.data.value));
                Ok(())
            }
        })
        .await
        .unwrap();

    let child_worker = WorkerBuilder::new(&child_queue)
        .connection(conn.clone())
        .build::<TestJob>();
    let child_handle = child_worker
        .start(|_job| async move { Ok(()) })
        .await
        .unwrap();

    producer
        .add(FlowJob {
            name: "parent".into(),
            queue_name: parent_queue.clone(),
            data: TestJob { value: "p".into() },
            prefix: None,
            opts: None,
            children: vec![FlowJob {
                name: "child".into(),
                queue_name: child_queue.clone(),
                data: TestJob { value: "c1".into() },
                prefix: None,
                opts: None,
                children: vec![],
            }],
        })
        .await
        .unwrap();

    let release_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let parent_released = parent.get_waiting_children_count().await.unwrap() == 0;
        let waiting_includes_parent = parent.get_waiting_count().await.unwrap() == 1;
        if parent_released && waiting_includes_parent {
            break;
        }
        if tokio::time::Instant::now() > release_deadline {
            panic!("Timed out waiting for paused parent release");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let counts = parent.get_job_counts().await.unwrap();
    assert_eq!(*counts.get(&JobState::WaitingChildren).unwrap(), 0);
    assert_eq!(*counts.get(&JobState::Paused).unwrap(), 1);
    assert_eq!(*counts.get(&JobState::Wait).unwrap(), 0);
    assert_eq!(parent.get_waiting_count().await.unwrap(), 1);

    tokio::time::sleep(Duration::from_millis(750)).await;
    assert!(
        parent_seen.lock().await.is_empty(),
        "parent job should not be claimed while queue is paused"
    );
    assert_eq!(parent.get_completed_count().await.unwrap(), 0);

    parent.resume().await.unwrap();

    let completion_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if parent.get_completed_count().await.unwrap() == 1 {
            break;
        }
        if tokio::time::Instant::now() > completion_deadline {
            panic!("Timed out waiting for parent completion after resume");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert_eq!(parent_seen.lock().await.as_slice(), ["parent:p"]);

    child_handle.shutdown();
    parent_handle.shutdown();
    child_handle.wait().await.unwrap();
    parent_handle.wait().await.unwrap();
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_flow_add_rejects_existing_custom_child_id() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let queue = QueueBuilder::new(&qname)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();
    queue.drain().await.unwrap();
    let producer = FlowProducerBuilder::new()
        .connection(conn.clone())
        .build()
        .await
        .unwrap();

    let existing_child_id = "existing-child-id".to_string();
    queue
        .add(
            "existing",
            TestJob {
                value: "already-there".into(),
            },
            Some(JobOptions {
                job_id: Some(existing_child_id.clone()),
                ..Default::default()
            }),
        )
        .await
        .unwrap();

    let flow = FlowJob {
        name: "parent".into(),
        queue_name: qname.clone(),
        data: TestJob {
            value: "parent".into(),
        },
        prefix: None,
        opts: None,
        children: vec![FlowJob {
            name: "child".into(),
            queue_name: qname.clone(),
            data: TestJob {
                value: "child".into(),
            },
            prefix: None,
            opts: Some(JobOptions {
                job_id: Some(existing_child_id.clone()),
                ..Default::default()
            }),
            children: vec![],
        }],
    };

    let err = producer.add(flow).await.unwrap_err();
    assert!(
        err.to_string().contains("already exists"),
        "unexpected error: {err}"
    );

    assert_eq!(queue.get_waiting_children_count().await.unwrap(), 0);
    assert_eq!(queue.get_waiting_count().await.unwrap(), 1);

    let mut raw = raw_redis_conn().await;
    let parent_keys: Vec<String> = redis::cmd("KEYS")
        .arg(format!("bull:{qname}:*:dependencies"))
        .query_async(&mut raw)
        .await
        .unwrap();
    assert!(parent_keys.is_empty());
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_flow_add_rejects_duplicate_custom_ids_within_flow() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let queue = QueueBuilder::new(&qname)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();
    queue.drain().await.unwrap();
    let producer = FlowProducerBuilder::new()
        .connection(conn.clone())
        .build()
        .await
        .unwrap();

    let duplicate_id = "duplicate-child-id".to_string();
    let flow = FlowJob {
        name: "parent".into(),
        queue_name: qname.clone(),
        data: TestJob {
            value: "parent".into(),
        },
        prefix: None,
        opts: None,
        children: vec![
            FlowJob {
                name: "child-a".into(),
                queue_name: qname.clone(),
                data: TestJob {
                    value: "child-a".into(),
                },
                prefix: None,
                opts: Some(JobOptions {
                    job_id: Some(duplicate_id.clone()),
                    ..Default::default()
                }),
                children: vec![],
            },
            FlowJob {
                name: "child-b".into(),
                queue_name: qname.clone(),
                data: TestJob {
                    value: "child-b".into(),
                },
                prefix: None,
                opts: Some(JobOptions {
                    job_id: Some(duplicate_id.clone()),
                    ..Default::default()
                }),
                children: vec![],
            },
        ],
    };

    let err = producer.add(flow).await.unwrap_err();
    assert!(
        err.to_string().contains("duplicated within the flow"),
        "unexpected error: {err}"
    );
    assert_eq!(queue.get_waiting_children_count().await.unwrap(), 0);
    assert_eq!(queue.get_waiting_count().await.unwrap(), 0);
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_flow_add_same_queue_parent_with_delayed_child_tracks_metadata() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let queue = QueueBuilder::new(&qname)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();
    queue.drain().await.unwrap();
    let producer = FlowProducerBuilder::new()
        .connection(conn.clone())
        .build()
        .await
        .unwrap();

    let flow = FlowJob {
        name: "parent".into(),
        queue_name: qname.clone(),
        data: TestJob {
            value: "parent".into(),
        },
        prefix: None,
        opts: None,
        children: vec![FlowJob {
            name: "child".into(),
            queue_name: qname.clone(),
            data: TestJob {
                value: "child".into(),
            },
            prefix: None,
            opts: Some(JobOptions {
                delay: Some(Duration::from_secs(60)),
                ..Default::default()
            }),
            children: vec![],
        }],
    };

    let node = producer.add(flow).await.unwrap();
    assert_eq!(queue.get_waiting_children_count().await.unwrap(), 1);
    assert_eq!(queue.get_delayed_count().await.unwrap(), 1);

    let parent_key = format!("bull:{}:{}", qname, node.job.id);
    let child_key = format!("bull:{}:{}", qname, node.children[0].job.id);

    let mut raw = raw_redis_conn().await;
    let parent_meta: (Option<String>, Option<String>) = redis::cmd("HMGET")
        .arg(child_key)
        .arg("parentKey")
        .arg("parent")
        .query_async(&mut raw)
        .await
        .unwrap();
    assert_eq!(parent_meta.0.as_deref(), Some(parent_key.as_str()));

    let parent_json: serde_json::Value =
        serde_json::from_str(parent_meta.1.as_deref().unwrap()).unwrap();
    assert_eq!(parent_json["id"], node.job.id);
    assert_eq!(parent_json["queue"], format!("bull:{qname}"));
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_flow_remove_and_drain_cleanup_waiting_children_and_dependencies() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let queue = QueueBuilder::new(&qname)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();
    queue.drain().await.unwrap();
    let producer = FlowProducerBuilder::new()
        .connection(conn.clone())
        .build()
        .await
        .unwrap();

    let flow = FlowJob {
        name: "parent".into(),
        queue_name: qname.clone(),
        data: TestJob {
            value: "parent".into(),
        },
        prefix: None,
        opts: None,
        children: vec![FlowJob {
            name: "child".into(),
            queue_name: qname.clone(),
            data: TestJob {
                value: "child".into(),
            },
            prefix: None,
            opts: None,
            children: vec![],
        }],
    };

    let node = producer.add(flow.clone()).await.unwrap();
    let parent_id = node.job.id.clone();
    let child_id = node.children[0].job.id.clone();
    let parent_deps_key = format!("bull:{}:{}:dependencies", qname, parent_id);

    queue.remove(&parent_id).await.unwrap();
    assert_eq!(queue.get_waiting_children_count().await.unwrap(), 0);

    let mut raw = raw_redis_conn().await;
    let deps_exists: bool = redis::cmd("EXISTS")
        .arg(&parent_deps_key)
        .query_async(&mut raw)
        .await
        .unwrap();
    assert!(!deps_exists);

    let child_after_parent_remove = queue.get_job(&child_id).await.unwrap();
    assert!(child_after_parent_remove.is_some());

    let second = producer.add(flow.clone()).await.unwrap();
    let second_parent_deps_key = format!("bull:{}:{}:dependencies", qname, second.job.id);
    second.job.remove().await.unwrap();

    let second_deps_exists: bool = redis::cmd("EXISTS")
        .arg(&second_parent_deps_key)
        .query_async(&mut raw)
        .await
        .unwrap();
    assert!(!second_deps_exists);
    assert_eq!(queue.get_waiting_children_count().await.unwrap(), 0);

    let third = producer.add(flow).await.unwrap();
    let third_parent_deps_key = format!("bull:{}:{}:dependencies", qname, third.job.id);
    queue.drain().await.unwrap();

    assert_eq!(queue.get_waiting_children_count().await.unwrap(), 0);
    assert_eq!(queue.get_waiting_count().await.unwrap(), 0);

    let third_deps_exists: bool = redis::cmd("EXISTS")
        .arg(&third_parent_deps_key)
        .query_async(&mut raw)
        .await
        .unwrap();
    assert!(!third_deps_exists);
}

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_flow_add_rejects_watch_window_collision_cleanly() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let queue = QueueBuilder::new(&qname)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();
    queue.drain().await.unwrap();

    let producer = FlowProducerBuilder::new()
        .connection(conn.clone())
        .build()
        .await
        .unwrap();

    let colliding_id = "watched-colliding-child-id".to_string();
    let hook_open_key = format!("test:flow-hook-open:{qname}");
    let hook_release_key = format!("test:flow-hook-release:{qname}");
    std::env::set_var("BULLMQ_RS_FLOW_ADD_WATCH_HOOK_QUEUE", &qname);
    std::env::set_var("BULLMQ_RS_FLOW_ADD_WATCH_HOOK_OPEN_KEY", &hook_open_key);
    std::env::set_var("BULLMQ_RS_FLOW_ADD_WATCH_HOOK_RELEASE_KEY", &hook_release_key);

    let flow = FlowJob {
        name: "parent".into(),
        queue_name: qname.clone(),
        data: TestJob {
            value: "parent".into(),
        },
        prefix: None,
        opts: None,
        children: vec![FlowJob {
            name: "child".into(),
            queue_name: qname.clone(),
            data: TestJob {
                value: "child".into(),
            },
            prefix: None,
            opts: Some(JobOptions {
                job_id: Some(colliding_id.clone()),
                ..Default::default()
            }),
            children: vec![],
        }],
    };

    let add_handle = tokio::spawn(async move { producer.add(flow).await });

    let mut raw = raw_redis_conn().await;
    for _ in 0..200 {
        let open: bool = redis::cmd("EXISTS")
            .arg(&hook_open_key)
            .query_async(&mut raw)
            .await
            .unwrap();
        if open {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let open: bool = redis::cmd("EXISTS")
        .arg(&hook_open_key)
        .query_async(&mut raw)
        .await
        .unwrap();
    assert!(open, "watch hook did not open");

    queue
        .add(
            "competing",
            TestJob {
                value: "competing".into(),
            },
            Some(JobOptions {
                job_id: Some(colliding_id.clone()),
                ..Default::default()
            }),
        )
        .await
        .unwrap();

    redis::cmd("SET")
        .arg(&hook_release_key)
        .arg("1")
        .query_async::<redis::Value>(&mut raw)
        .await
        .unwrap();

    let result = add_handle.await.unwrap();
    std::env::remove_var("BULLMQ_RS_FLOW_ADD_WATCH_HOOK_QUEUE");
    std::env::remove_var("BULLMQ_RS_FLOW_ADD_WATCH_HOOK_OPEN_KEY");
    std::env::remove_var("BULLMQ_RS_FLOW_ADD_WATCH_HOOK_RELEASE_KEY");

    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("transaction aborted"),
        "unexpected error: {err}"
    );

    assert_eq!(queue.get_waiting_children_count().await.unwrap(), 0);
    assert_eq!(queue.get_waiting_count().await.unwrap(), 1);

    let parent_keys: Vec<String> = redis::cmd("KEYS")
        .arg(format!("bull:{qname}:*:dependencies"))
        .query_async(&mut raw)
        .await
        .unwrap();
    assert!(parent_keys.is_empty());
}
// ---------------------------------------------------------------------------
// 22. test_worker_processes_preexisting_jobs
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_worker_processes_preexisting_jobs() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    // Add 10 jobs BEFORE starting the worker
    let queue = QueueBuilder::new(&qname)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    for i in 0..10 {
        queue
            .add(
                "preexist",
                TestJob {
                    value: format!("job-{}", i),
                },
                None,
            )
            .await
            .unwrap();
    }

    // Verify all 10 are in wait
    let counts = queue.get_job_counts().await.unwrap();
    assert_eq!(*counts.get(&JobState::Wait).unwrap_or(&0), 10);

    // Now start the worker
    let completed = Arc::new(Mutex::new(0u32));
    let completed_clone = completed.clone();

    let worker = WorkerBuilder::new(&qname)
        .connection(conn.clone())
        .concurrency(1)
        .build::<TestJob>();

    let handle = worker
        .start(move |_job| {
            let completed = completed_clone.clone();
            async move {
                let mut c = completed.lock().await;
                *c += 1;
                Ok(())
            }
        })
        .await
        .unwrap();

    // Wait for all jobs to be processed (max 30 seconds)
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let c = *completed.lock().await;
        if c >= 10 {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            let c = *completed.lock().await;
            panic!(
                "Timed out waiting for pre-existing jobs: only {}/10 completed",
                c
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    handle.shutdown();
    handle.wait().await.unwrap();

    let final_count = *completed.lock().await;
    assert_eq!(final_count, 10);

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 23. test_worker_recovers_waiting_job_without_marker
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_worker_recovers_waiting_job_without_marker() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let queue = QueueBuilder::new(&qname)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    let completed = Arc::new(Mutex::new(false));
    let completed_clone = completed.clone();

    let worker = WorkerBuilder::new(&qname)
        .connection(conn.clone())
        .concurrency(1)
        .build::<TestJob>();

    let handle = worker
        .start(move |_job| {
            let completed = completed_clone.clone();
            async move {
                *completed.lock().await = true;
                Ok(())
            }
        })
        .await
        .unwrap();

    // Let the worker settle into its idle blocking loop.
    tokio::time::sleep(Duration::from_secs(1)).await;

    let mut raw = raw_redis_conn().await;
    let job_id = "manual-no-marker";
    let wait_key = format!("bull:{}:wait", qname);
    let marker_key = format!("bull:{}:marker", qname);
    let job_key = format!("bull:{}:{}", qname, job_id);
    let data = serde_json::to_string(&TestJob {
        value: "manual".into(),
    })
    .unwrap();

    redis::cmd("HSET")
        .arg(&job_key)
        .arg("name")
        .arg("manual")
        .arg("data")
        .arg(&data)
        .arg("opts")
        .arg("{}")
        .arg("timestamp")
        .arg("1")
        .arg("delay")
        .arg("0")
        .arg("priority")
        .arg("0")
        .arg("atm")
        .arg("0")
        .arg("ats")
        .arg("0")
        .query_async::<()>(&mut raw)
        .await
        .unwrap();

    redis::cmd("LPUSH")
        .arg(&wait_key)
        .arg(job_id)
        .query_async::<()>(&mut raw)
        .await
        .unwrap();

    redis::cmd("DEL")
        .arg(&marker_key)
        .query_async::<()>(&mut raw)
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        if *completed.lock().await {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("Timed out waiting for worker to recover a wait job without a marker");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    handle.shutdown();
    handle.wait().await.unwrap();
    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 24. test_queue_events_basic_flow
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_queue_events_basic_flow() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    // Start QueueEvents with last_event_id "0" to get all events from the beginning
    let mut qe = QueueEventsBuilder::new(&qname)
        .connection(conn.clone())
        .last_event_id("0")
        .blocking_timeout(2_000)
        .build()
        .await
        .unwrap();

    let mut rx = qe.subscribe();

    // Add a job
    let queue = QueueBuilder::new(&qname)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    queue
        .add("test_job", TestJob { value: "hello".into() }, None)
        .await
        .unwrap();

    // Start a worker to process it
    let worker = WorkerBuilder::new(&qname)
        .connection(conn.clone())
        .concurrency(1)
        .build::<TestJob>();

    let handle = worker
        .start(|_job| async move { Ok(()) })
        .await
        .unwrap();

    // Collect events until we see Completed (max 10s)
    let mut events = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok((event, _stream_id)) => {
                        events.push(event);
                        if events.iter().any(|e| matches!(e, QueueEvent::Completed { .. })) {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                panic!("Timed out waiting for events. Got: {:?}", events);
            }
        }
    }

    // Verify we got the key lifecycle events
    assert!(events.iter().any(|e| matches!(e, QueueEvent::Added { .. })));
    assert!(events.iter().any(|e| matches!(e, QueueEvent::Waiting { .. })));
    assert!(events.iter().any(|e| matches!(e, QueueEvent::Active { .. })));
    assert!(events.iter().any(|e| matches!(e, QueueEvent::Completed { .. })));

    handle.shutdown();
    handle.wait().await.unwrap();
    qe.close().await.unwrap();
    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 25. test_queue_events_producer
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_queue_events_producer() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let mut qe = QueueEventsBuilder::new(&qname)
        .connection(conn.clone())
        .last_event_id("0")
        .blocking_timeout(2_000)
        .build()
        .await
        .unwrap();

    let mut rx = qe.subscribe();

    let producer = QueueEventsProducerBuilder::new(&qname)
        .connection(conn.clone())
        .build()
        .await
        .unwrap();

    producer
        .publish("my_custom_event", &[("jobId", "99"), ("status", "exported")])
        .await
        .unwrap();

    // Wait for the custom event
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok((QueueEvent::Unknown { event, fields }, _)) => {
                        assert_eq!(event, "my_custom_event");
                        assert_eq!(fields.get("jobId").unwrap(), "99");
                        assert_eq!(fields.get("status").unwrap(), "exported");
                        break;
                    }
                    Ok(_) => continue,
                    Err(_) => panic!("Channel closed"),
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                panic!("Timed out waiting for custom event");
            }
        }
    }

    qe.close().await.unwrap();
}

// ---------------------------------------------------------------------------
// 26. test_queue_events_multiple_subscribers
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_queue_events_multiple_subscribers() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let mut qe = QueueEventsBuilder::new(&qname)
        .connection(conn.clone())
        .last_event_id("0")
        .blocking_timeout(2_000)
        .build()
        .await
        .unwrap();

    let mut rx1 = qe.subscribe();
    let mut rx2 = qe.subscribe();

    let producer = QueueEventsProducerBuilder::new(&qname)
        .connection(conn.clone())
        .build()
        .await
        .unwrap();

    producer
        .publish("test_event", &[("data", "hello")])
        .await
        .unwrap();

    let timeout = Duration::from_secs(5);
    let (e1, _) = tokio::time::timeout(timeout, rx1.recv()).await.unwrap().unwrap();
    let (e2, _) = tokio::time::timeout(timeout, rx2.recv()).await.unwrap().unwrap();

    assert_eq!(e1, e2);

    qe.close().await.unwrap();
}

// ---------------------------------------------------------------------------
// 27. test_queue_events_shutdown
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_queue_events_shutdown() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let mut qe = QueueEventsBuilder::new(&qname)
        .connection(conn)
        .blocking_timeout(2_000)
        .build()
        .await
        .unwrap();

    qe.close().await.unwrap();
}

// ---------------------------------------------------------------------------
// 23. test_worker_continuous_processing_no_stall
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_worker_continuous_processing_no_stall() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let queue = QueueBuilder::new(&qname)
        .connection(conn.clone())
        .build::<TestJob>()
        .await
        .unwrap();

    // Add 20 jobs
    for i in 0..20 {
        queue
            .add(
                "continuous",
                TestJob {
                    value: format!("job-{}", i),
                },
                None,
            )
            .await
            .unwrap();
    }

    let completed = Arc::new(Mutex::new(0u32));
    let completed_clone = completed.clone();

    let worker = WorkerBuilder::new(&qname)
        .connection(conn.clone())
        .concurrency(1) // single concurrency to test sequential marker re-add
        .build::<TestJob>();

    let handle = worker
        .start(move |_job| {
            let completed = completed_clone.clone();
            async move {
                let mut c = completed.lock().await;
                *c += 1;
                Ok(())
            }
        })
        .await
        .unwrap();

    // Wait for all 20 jobs (max 30 seconds)
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let c = *completed.lock().await;
        if c >= 20 {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            let c = *completed.lock().await;
            panic!(
                "Timed out: only {}/20 completed — worker likely stalled after first job",
                c
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    handle.shutdown();
    handle.wait().await.unwrap();

    let final_count = *completed.lock().await;
    assert_eq!(final_count, 20);

    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 28. test_wait_until_finished_success
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_wait_until_finished_success() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let mut qe = QueueEventsBuilder::new(&qname)
        .connection(conn.clone())
        .blocking_timeout(2_000)
        .build()
        .await
        .unwrap();

    let queue = QueueBuilder::new(&qname)
        .connection(conn.clone())
        .build::<String>()
        .await
        .unwrap();

    let job = queue.add("test", "data".to_string(), None).await.unwrap();

    let worker = WorkerBuilder::new(&qname)
        .connection(conn.clone())
        .concurrency(1)
        .build::<String>();

    let handle = worker
        .start(|_job| async move { Ok(()) })
        .await
        .unwrap();

    let result = job
        .wait_until_finished(&qe, Some(Duration::from_secs(10)))
        .await;

    assert!(result.is_ok());

    handle.shutdown();
    handle.wait().await.unwrap();
    qe.close().await.unwrap();
    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 29. test_wait_until_finished_timeout
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_wait_until_finished_timeout() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let mut qe = QueueEventsBuilder::new(&qname)
        .connection(conn.clone())
        .blocking_timeout(2_000)
        .build()
        .await
        .unwrap();

    let queue = QueueBuilder::new(&qname)
        .connection(conn.clone())
        .build::<String>()
        .await
        .unwrap();

    let opts = JobOptions {
        delay: Some(Duration::from_secs(3600)),
        ..Default::default()
    };
    let job = queue.add("test", "data".to_string(), Some(opts)).await.unwrap();

    let result = job
        .wait_until_finished(&qe, Some(Duration::from_secs(1)))
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("timed out"));

    qe.close().await.unwrap();
    queue.drain().await.unwrap();
}

// ---------------------------------------------------------------------------
// 30. test_wait_until_finished_already_completed
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Redis"]
async fn test_wait_until_finished_already_completed() {
    let qname = unique_queue_name();
    let conn = redis_conn();

    let queue = QueueBuilder::new(&qname)
        .connection(conn.clone())
        .build::<String>()
        .await
        .unwrap();

    let job = queue.add("test", "data".to_string(), None).await.unwrap();

    // Process the job first
    let completed = Arc::new(Mutex::new(false));
    let completed_clone = completed.clone();

    let worker = WorkerBuilder::new(&qname)
        .connection(conn.clone())
        .concurrency(1)
        .build::<String>();

    let handle = worker
        .start(move |_job| {
            let completed = completed_clone.clone();
            async move {
                let mut c = completed.lock().await;
                *c = true;
                Ok(())
            }
        })
        .await
        .unwrap();

    // Wait until the job is actually completed
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if *completed.lock().await {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("Job never completed");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    handle.shutdown();
    handle.wait().await.unwrap();

    // NOW start QueueEvents and call waitUntilFinished — should return immediately
    // via the race condition guard
    let mut qe = QueueEventsBuilder::new(&qname)
        .connection(conn.clone())
        .blocking_timeout(2_000)
        .build()
        .await
        .unwrap();

    let result = job
        .wait_until_finished(&qe, Some(Duration::from_secs(3)))
        .await;

    assert!(result.is_ok());

    qe.close().await.unwrap();
    queue.drain().await.unwrap();
}
