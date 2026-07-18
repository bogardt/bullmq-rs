//! Regression test for the claim-before-semaphore / lock-renewal race in the
//! worker fetch loop.
//!
//! THE BUG (present up to and including v2.2.0):
//!
//! The fetch loop called `move_to_active` (job becomes ACTIVE + locked in Redis,
//! lock token assigned) and ONLY THEN blocked on `semaphore.acquire_owned()`.
//! The job id was inserted into `active_jobs` — the set the lock-extender renews
//! every `lock_duration/2` — only AFTER the permit was granted, inside the
//! spawned task. So a job claimed while the worker was at its concurrency limit
//! was never renewed while it waited for a permit. If that wait exceeded
//! `lock_duration`, its Redis lock expired; the stalled checker re-queued it;
//! another worker (or the stalled path) picked it up and ran it, and the
//! original worker *also* ran its now-stale in-memory copy once a permit freed
//! up -> double execution.
//!
//! THE FIX (mirroring the official client): reserve capacity FIRST, then claim.
//! The fetch loop now acquires a semaphore permit before fetching, so a claimed
//! job always starts (and starts being renewed) immediately. Jobs prefetched by
//! `moveToFinished(fetchNext)` are registered in `active_jobs` at prefetch time,
//! so their lock keeps being renewed while they wait for a permit.
//!
//! This test is `#[ignore]` because it needs a live Redis. Run it with a Redis
//! server available:
//!
//!     redis-server &                                   # or: docker run --rm -p 6379:6379 redis
//!     RUST_LOG=debug cargo test --test lock_renewal_race -- --ignored --nocapture
//!
//! On the buggy ordering the assertion FAILS — `victim` is processed twice
//! (once by worker2 after the stall re-queue, once by worker1's stale copy).
//! On fixed code it PASSES: the saturated worker1 never claims the victim, so
//! the free worker2 processes it exactly once.

use bullmq_rs::{QueueBuilder, RedisConnection, WorkerBuilder};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct TestJob {
    tag: String,
}

#[tokio::test]
#[ignore = "needs Redis; regression test for the lock-renewal race (see module docs)"]
async fn claim_before_semaphore_can_double_execute_a_job() {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    // Force the race window to be small and observable.
    let lock_duration = Duration::from_secs(2);
    let stalled_interval = Duration::from_secs(1);

    // Shared, append-only record of (job_id, started_at) for every handler run.
    let processed: Arc<Mutex<Vec<(String, Instant)>>> = Arc::new(Mutex::new(Vec::new()));

    let queue_name = format!("race_repro_{}", uuid::Uuid::new_v4());

    // worker1: claims the blocker, then claims the victim and stalls on its permit.
    let w1 = WorkerBuilder::new(&queue_name)
        .connection(RedisConnection::new(&url))
        .concurrency(1)
        .lock_duration(lock_duration)
        .stalled_interval(stalled_interval)
        .build::<TestJob>();

    let processed_w1 = processed.clone();
    let h1 = w1
        .start(move |job| {
            let processed = processed_w1.clone();
            async move {
                match job.data.tag.as_str() {
                    "blocker" => {
                        // Hold worker1's single permit well past lock_duration.
                        tokio::time::sleep(Duration::from_secs(6)).await;
                    }
                    "victim" => {
                        let mut log = processed.lock().await;
                        println!(
                            "[handler] worker1 victim {} started at {:?}",
                            job.id,
                            Instant::now()
                        );
                        log.push((job.id.clone(), Instant::now()));
                    }
                    _ => {}
                }
                Ok(())
            }
        })
        .await
        .expect("worker1 start");

    let queue = QueueBuilder::new(&queue_name)
        .connection(RedisConnection::new(&url))
        .build::<TestJob>()
        .await
        .expect("queue build");
    queue.drain().await.ok();

    // 1) worker1 (the ONLY worker so far) claims the blocker and occupies its permit.
    queue
        .add(
            "a",
            TestJob {
                tag: "blocker".into(),
            },
            None,
        )
        .await
        .expect("enqueue blocker");
    tokio::time::sleep(Duration::from_millis(400)).await;

    // 2) worker1 is the sole worker and is at capacity. On the buggy ordering it
    //    claims the victim via moveToActive and then blocks on the semaphore
    //    (permit held by the blocker), leaving the victim's lock ticking down
    //    with NO renewal. On fixed code it does not claim the victim at all
    //    until it has a free permit.
    queue
        .add(
            "b",
            TestJob {
                tag: "victim".into(),
            },
            None,
        )
        .await
        .expect("enqueue victim");
    tokio::time::sleep(Duration::from_millis(400)).await;

    // 3) Bring worker2 online. On the buggy ordering: once the stalled checker
    //    re-queues the victim (lock expired at ~2s, stalled_interval 1s), worker2
    //    claims and runs it -> handler run #1; later, when the blocker finishes at
    //    ~6s, worker1's stale in-memory victim also runs -> handler run #2. On
    //    fixed code: the victim is still unclaimed in wait, so worker2 simply
    //    processes it exactly once.
    let w2 = WorkerBuilder::new(&queue_name)
        .connection(RedisConnection::new(&url))
        .concurrency(1)
        .lock_duration(lock_duration)
        .stalled_interval(stalled_interval)
        .build::<TestJob>();
    let processed_w2 = processed.clone();
    let h2 = w2
        .start(move |job| {
            let processed = processed_w2.clone();
            async move {
                if job.data.tag == "victim" {
                    let mut log = processed.lock().await;
                    println!(
                        "[handler] worker2 victim {} started at {:?}",
                        job.id,
                        Instant::now()
                    );
                    log.push((job.id.clone(), Instant::now()));
                }
                Ok(())
            }
        })
        .await
        .expect("worker2 start");

    // Let the full timeline play out: stall + worker2 re-run, then worker1 stale run.
    tokio::time::sleep(Duration::from_secs(10)).await;

    h1.shutdown();
    h2.shutdown();
    let _ = h1.wait().await;
    let _ = h2.wait().await;
    queue.drain().await.ok();

    let log = processed.lock().await;
    let victim_runs: Vec<&(String, Instant)> = log.iter().collect();
    println!("[result] victim handler runs: {}", victim_runs.len());

    // Correct behavior: the victim is processed exactly once.
    // On the buggy code this is 2 (double execution after lock loss).
    assert_eq!(
        victim_runs.len(),
        1,
        "victim was processed {} times — expected 1. \
         If this is 2, the claim-before-semaphore race reproduced.",
        victim_runs.len()
    );
}
