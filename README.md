# bullmq-rs
[![crates.io](https://img.shields.io/crates/v/bullmq-rs.svg)](https://crates.io/crates/bullmq-rs)
[![docs.rs](https://img.shields.io/docsrs/bullmq-rs)](https://docs.rs/bullmq-rs)
[![CI](https://github.com/bogardt/bullmq-rs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/bogardt/bullmq-rs/actions/workflows/ci.yml)
[![License](https://img.shields.io/crates/l/bullmq-rs.svg)](#license)

A Rust implementation of [BullMQ](https://bullmq.io/) — a Redis-based
distributed job queue with typed payloads, priorities, delays, retries
with backoff, concurrency control, and worker management.

**Wire-compatible with BullMQ Node.js v5.x** — Rust workers and Node.js
workers can share the same queues, and Bull Board / other BullMQ
ecosystem tooling works out of the box against queues produced by
`bullmq-rs`.

> Upgrading from 1.x? Read the [migration guide](MIGRATION.md). The
> Redis data layout has changed.

## Features

- **BullMQ v5 wire compatibility** — same Redis key layout, same Lua
  scripts, same event stream format. Interop with BullMQ Node and Bull
  Board.
- **Typed jobs** — generic `Job<T>` over any `Serialize + Deserialize` payload.
- **Atomic state transitions** via Lua scripts executed server-side
  (`addStandardJob`, `moveToActive`, `moveToFinished`, `retryJob`, …).
- **Marker-based worker loop** with `BZPOPMIN` — no polling, no missed jobs.
- **Token-based job locks** with TTL, lock extension, and stalled-job recovery.
- **Priorities** (sorted-set backed), **delays**, **retries** with fixed or
  exponential backoff.
- **Concurrency** with prefetch-safe job dispatch.
- **Repeatable jobs / JobScheduler** — cron patterns (with IANA timezones)
  or fixed intervals, `limit`, start/end dates; workers reschedule the next
  iteration automatically.
- **Rate limiting** — `max` jobs per `duration` window, shared across all
  workers of a queue (including Node.js ones).
- **Deduplication** — `JobOptions::deduplication { id, ttl }`; duplicate adds
  return the existing job.
- **Queue retention** — `clean(grace, limit, state)` and `obliterate(force)`.
- **Bulk operations** — `add_bulk`, `retry_jobs`, `promote_jobs`.
- **Metrics** — per-minute finished-job counts (`get_metrics`), collected by
  workers opted in via `WorkerBuilder::metrics`.
- **Worker controls** — `pause`/`resume`/`is_paused`/`is_running` on the
  handle, `cancel_job`, manual fetch with `get_next_job` + `extend_job_locks`.
- **Queue events** via Redis Streams — typed `QueueEvent` enum delivered
  through `tokio::broadcast`.
- **Local worker events** — typed `WorkerEvent` enum (`Active`, `Completed`,
  `Failed`, `Drained`, …) via `handle.subscribe()`, the Rust answer to
  Node's worker-level `EventEmitter`.
- **Flows** — `FlowProducer` for parent/child job trees, same-queue and
  cross-queue, with parent-release on child completion.
- **Job active-handle API** — `update_progress`, `log`, `retry`,
  `change_priority`, `promote`, `change_delay`, `remove`,
  `wait_until_finished`, `get_state`, `get_dependencies`,
  `get_children_values`.
- **Graceful shutdown** — workers finish in-flight jobs before stopping.

For the precise list of what is and isn't covered relative to BullMQ
Node v5, see [BULLMQ_V5_PARITY.md](BULLMQ_V5_PARITY.md).

## Installation

```toml
[dependencies]
bullmq-rs = "2.2"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
```

## Quick start

### 1. Add jobs to a queue

```rust
use bullmq_rs::{BackoffStrategy, JobOptions, QueueBuilder, RedisConnection};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Email {
    to: String,
    subject: String,
    body: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conn = RedisConnection::new("redis://127.0.0.1:6379");

    let queue = QueueBuilder::new("emails")
        .connection(conn)
        .build::<Email>()
        .await?;

    // Simple job
    queue.add("welcome", Email {
        to: "user@example.com".into(),
        subject: "Welcome!".into(),
        body: "Hello!".into(),
    }, None).await?;

    // Delayed + retried job
    queue.add("reminder", Email {
        to: "user@example.com".into(),
        subject: "Reminder".into(),
        body: "Don't forget!".into(),
    }, Some(JobOptions {
        delay: Some(Duration::from_secs(60)),
        attempts: Some(3),
        backoff: Some(BackoffStrategy::Exponential {
            base: Duration::from_secs(1),
            max: Duration::from_secs(30),
        }),
        ..Default::default()
    })).await?;

    Ok(())
}
```

### 2. Process jobs with a worker

```rust
use bullmq_rs::{RedisConnection, WorkerBuilder};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Email {
    to: String,
    subject: String,
    body: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conn = RedisConnection::new("redis://127.0.0.1:6379");

    // `build()` is synchronous; connections open on `start().await`.
    let worker = WorkerBuilder::new("emails")
        .connection(conn)
        .concurrency(5)
        .lock_duration(Duration::from_secs(30))
        .on_completed(|job| println!("Job {} completed", job.id))
        .on_failed(|job, err| println!("Job {} failed: {}", job.id, err))
        .build::<Email>();

    // Handler returns Result<(), Box<dyn Error + Send + Sync>>.
    let handle = worker.start(|job| async move {
        println!("Sending email to {}", job.data.to);
        Ok(())
    }).await?;

    // Graceful shutdown
    handle.shutdown();
    handle.wait().await?;
    Ok(())
}
```

### 3. Subscribe to queue events

```rust
use bullmq_rs::{QueueEvent, QueueEventsBuilder, RedisConnection};

let events = QueueEventsBuilder::new("emails")
    .connection(RedisConnection::new("redis://127.0.0.1:6379"))
    .build()
    .await?;

let mut rx = events.subscribe();
while let Ok((event, _id)) = rx.recv().await {
    match event {
        QueueEvent::Completed { job_id, .. } => println!("✅ {job_id} done"),
        QueueEvent::Failed { job_id, reason }   => println!("❌ {job_id}: {reason}"),
        _ => {}
    }
}
```

### 4. Parent / child flows

```rust
use bullmq_rs::{FlowJob, FlowProducerBuilder, RedisConnection};
use serde_json::json;

let producer = FlowProducerBuilder::new()
    .connection(RedisConnection::new("redis://127.0.0.1:6379"))
    .build()
    .await?;

let tree = FlowJob {
    name: "rollup".into(),
    queue_name: "reports".into(),
    data: json!({ "period": "2026-06" }),
    prefix: None,
    opts: None,
    children: vec![
        FlowJob {
            name: "collect".into(),
            queue_name: "reports".into(),
            data: json!({ "source": "stripe" }),
            prefix: None,
            opts: None,
            children: vec![],
        },
        FlowJob {
            name: "collect".into(),
            queue_name: "reports".into(),
            data: json!({ "source": "paypal" }),
            prefix: None,
            opts: None,
            children: vec![],
        },
    ],
};

let root = producer.add(tree).await?;
println!("Parent {} will run once both children complete", root.job.id);
```

### 5. Repeatable jobs

```rust
use bullmq_rs::{JobSchedulerTemplate, RepeatOptions};
use std::time::Duration;

// Every 5 minutes, forever
queue.upsert_job_scheduler(
    "cleanup",
    RepeatOptions { every: Some(Duration::from_secs(300)), ..Default::default() },
    JobSchedulerTemplate { name: Some("cleanup".into()), data: None, opts: None },
).await?;

// Weekdays at 08:00 Paris time, 10 runs max
queue.upsert_job_scheduler(
    "daily-report",
    RepeatOptions {
        pattern: Some("0 8 * * 1-5".into()),
        tz: Some("Europe/Paris".into()),
        limit: Some(10),
        ..Default::default()
    },
    JobSchedulerTemplate::default(),
).await?;

// Inspect / remove
let schedulers = queue.get_job_schedulers(0, -1).await?;
queue.remove_job_scheduler("cleanup").await?;
```

Workers process scheduler jobs like any other job and automatically
enqueue the next iteration on completion.

### 6. Local worker events

```rust
use bullmq_rs::WorkerEvent;

let handle = worker.start(|_job| async move { Ok(()) }).await?;

let mut events = handle.subscribe();
tokio::spawn(async move {
    while let Ok(event) = events.recv().await {
        match event {
            WorkerEvent::Active { job }            => println!("▶ {}", job.id),
            WorkerEvent::Completed { job }         => println!("✅ {}", job.id),
            WorkerEvent::Failed { job, reason }    => println!("❌ {}: {reason}", job.id),
            WorkerEvent::Drained                   => println!("queue drained"),
            _ => {}
        }
    }
});
```

Process-local equivalent of Node's worker-level `EventEmitter` — for
cross-process events use `QueueEvents` (section 3).

## More recipes

### Deduplication

```rust
use bullmq_rs::{DeduplicationOptions, JobOptions};

// While the dedup key lives (TTL or until the job finishes),
// adds with the same id return the EXISTING job instead of a new one.
let job = queue.add("sync", data, Some(JobOptions {
    deduplication: Some(DeduplicationOptions {
        id: "sync-user-42".into(),
        ttl: Some(Duration::from_secs(60)),
    }),
    ..Default::default()
})).await?;
```

### Rate limiting

```rust
use bullmq_rs::{BullmqError, RateLimiterOptions};

// Max 10 jobs per second, shared across ALL workers of the queue
// (including Node.js ones).
let worker = WorkerBuilder::new("emails")
    .connection(conn)
    .limiter(RateLimiterOptions { max: 10, duration: Duration::from_secs(1) })
    .build::<Email>();

// Manual rate limiting from inside a handler: the job goes back to
// wait WITHOUT consuming an attempt.
let handle = worker.start(|job| async move {
    if upstream_api_throttled() {
        return Err(Box::new(BullmqError::RateLimited));
    }
    Ok(())
}).await?;

// Dynamic, queue-wide: block fetching for 30s.
queue.rate_limit(Duration::from_secs(30)).await?;
queue.remove_rate_limit_key().await?;
```

### Worker pause / cooperative cancellation

```rust
// Pause this worker (jobs keep accumulating; other workers unaffected).
handle.pause(false).await;   // waits for in-flight jobs
handle.resume();

// Cooperative cancellation: the handler receives a CancellationToken.
let handle = worker.start_with_signal(|job, token| async move {
    tokio::select! {
        _ = token.cancelled() => Err("cancelled".into()),
        _ = do_work(&job) => Ok(()),
    }
}).await?;
handle.cancel_job(&job_id);  // triggers the token, force-aborts after 5s
```

### Metrics

```rust
use bullmq_rs::{JobState, MetricsOptions};

let worker = WorkerBuilder::new("emails")
    .connection(conn)
    .metrics(MetricsOptions { max_data_points: 60 * 24 })  // 1 day of minutes
    .build::<Email>();

// Later, from anywhere:
let metrics = queue.get_metrics(JobState::Completed, 0, -1).await?;
println!("{} completed, {} data points", metrics.meta.count, metrics.count);
```

### Retention & bulk operations

```rust
// Remove completed jobs older than 1h (up to 1000).
queue.clean(Duration::from_secs(3600), 1000, JobState::Completed).await?;

// Re-enqueue all failed jobs.
queue.retry_jobs(1000, JobState::Failed, None).await?;

// Nuke the queue entirely (must have no active jobs unless force).
queue.obliterate(false).await?;
```

## Interop with BullMQ Node

Because the Redis wire format matches BullMQ v5, you can mix Rust and
Node freely:

```js
// Node producer
import { Queue } from "bullmq";
const q = new Queue("emails", { connection: { host: "127.0.0.1", port: 6379 } });
await q.add("welcome", { to: "user@example.com", subject: "Hi", body: "..." });
```

```rust
// Rust worker reads it
let worker = WorkerBuilder::new("emails")
    .connection(RedisConnection::new("redis://127.0.0.1:6379"))
    .build::<serde_json::Value>();
let handle = worker.start(|job| async move {
    println!("got {}: {}", job.name, job.data);
    Ok(())
}).await?;
```

The reverse — Rust producer, Node worker — works the same way. There's
a small harness under `tests/compat/` that exercises both directions.

## Redis key schema (BullMQ v5)

All keys use `{prefix}:{queue_name}:{suffix}` (default prefix: `bull`):

| Key | Type | Purpose |
|---|---|---|
| `bull:<q>:wait` | List | Pending jobs (FIFO) |
| `bull:<q>:active` | List | Currently running jobs |
| `bull:<q>:paused` | List | Jobs accumulated while the queue is paused |
| `bull:<q>:prioritized` | Sorted Set | Pending jobs with `priority` > 0 |
| `bull:<q>:delayed` | Sorted Set | Jobs scheduled for later (`delay`) |
| `bull:<q>:completed` | Sorted Set | Successfully completed jobs |
| `bull:<q>:failed` | Sorted Set | Permanently failed jobs |
| `bull:<q>:waiting-children` | Sorted Set | Flow parents waiting on children |
| `bull:<q>:marker` | Sorted Set | Worker wake-up marker (`BZPOPMIN`) |
| `bull:<q>:meta` | Hash | Queue metadata (paused flag, etc.) |
| `bull:<q>:events` | Stream | `XADD` event stream consumed by `QueueEvents` |
| `bull:<q>:<job_id>` | Hash | Individual job data |
| `bull:<q>:id` | String | Auto-incrementing job ID counter |
| `bull:<q>:<job_id>:logs` | List | Per-job logs (`job.log(...)`) |
| `bull:<q>:<job_id>:lock` | String | Worker lock token, PX TTL |
| `bull:<q>:<job_id>:dependencies` | Sorted Set | Flow children of this job |
| `bull:<q>:repeat` | Sorted Set | Job schedulers (score = next run, ms) |
| `bull:<q>:repeat:<id>` | Hash | Scheduler template + iteration count |
| `bull:<q>:limiter` | String | Rate-limiter window counter (PX TTL) |
| `bull:<q>:de:<dedup_id>` | String | Deduplication key (optional TTL) |
| `bull:<q>:metrics:<state>` | Hash | Metrics meta (count, prevTS, prevCount) |
| `bull:<q>:metrics:<state>:data` | List | Per-minute metrics data points |

## Requirements

- Rust **1.75+**
- Redis **6.2+** (Streams, `BZPOPMIN`, sorted-set operations used in the Lua scripts)

## Docker

```sh
docker compose up -d        # starts Redis on :6379
```

## Running tests

```sh
# Unit tests (no Redis required)
cargo test --test unit_tests

# Integration tests (requires Redis running)
cargo test --test integration_tests -- --ignored --test-threads=1

# Cross-language compat harness (requires Node 18+)
# See tests/compat/README.md for individual scripts.
cd tests/compat && npm install
```

## Examples

```sh
cargo run --example basic_queue       # add jobs (simple, delayed, prioritized, retried)
cargo run --example basic_worker      # process jobs with concurrency
cargo run --example repeatable_jobs   # JobScheduler: every-interval, inspect, remove
cargo run --example worker_events     # subscribe to typed WorkerEvent stream
```

## Documentation

- [Migration guide v1 → v2](MIGRATION.md)
- [Changelog](CHANGELOG.md)
- [BullMQ v5 parity](BULLMQ_V5_PARITY.md) — what is in scope and what isn't
- [API docs on docs.rs](https://docs.rs/bullmq-rs)

## Acknowledgments

The BullMQ wire-compatibility work was contributed by
[@enricodeleo](https://github.com/enricodeleo) in
[#3](https://github.com/bogardt/bullmq-rs/pull/3).

The Lua scripts under `lua/` are ported from
[BullMQ](https://github.com/taskforcesh/bullmq) (MIT licensed).
See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## License

Licensed under either of `MIT` or `Apache-2.0` at your option.
