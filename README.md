# bullmq-rs

A Rust implementation of [BullMQ](https://bullmq.io/) — a Redis-based distributed job queue with typed payloads, priorities, delays, retries with backoff, concurrency control, and worker management.

## Features

- **Typed jobs** — Generic `Job<T>` with any `Serialize + Deserialize` payload
- **Queue** — Add, get, remove, drain jobs with builder pattern
- **Worker** — Process jobs with async handlers and concurrency control
- **Priority** — Lower values = higher priority
- **Delays** — Schedule jobs for later processing
- **Retries** — Automatic retry with fixed or exponential backoff
- **Callbacks** — `on_completed` and `on_failed` event hooks
- **Graceful shutdown** — Worker stops cleanly after current jobs finish

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
bullmq-rs = "0.3"
tokio = { version = "1", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
```

## Quick Start

### 1. Add jobs to a queue

```rust
use bullmq_rs::{QueueBuilder, RedisConnection, JobOptions, BackoffStrategy};
use serde::{Serialize, Deserialize};
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

    // Job with delay and retries
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
use bullmq_rs::{WorkerBuilder, RedisConnection};
use serde::{Serialize, Deserialize};
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

    let worker = WorkerBuilder::new("emails")
        .connection(conn)
        .concurrency(5)
        .poll_interval(Duration::from_millis(500))
        .on_completed(|job| println!("Job {} completed", job.id))
        .on_failed(|job, err| println!("Job {} failed: {}", job.id, err))
        .build::<Email>()
        .await?;

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

## Redis Key Schema

All keys use the pattern `{prefix}:{queue_name}:{suffix}` (default prefix: `bull`):

| Key | Type | Description |
|-----|------|-------------|
| `bull:myqueue:waiting` | Sorted Set | Jobs waiting to be processed |
| `bull:myqueue:delayed` | Sorted Set | Jobs scheduled for later |
| `bull:myqueue:active` | Set | Jobs currently being processed |
| `bull:myqueue:completed` | Sorted Set | Successfully completed jobs |
| `bull:myqueue:failed` | Sorted Set | Permanently failed jobs |
| `bull:myqueue:{job_id}` | Hash | Individual job data |
| `bull:myqueue:id` | String | Auto-incrementing ID counter |

## Requirements

- Rust 1.75+
- Redis 6.0+

## Docker

```sh
docker compose up -d
```

## Running Tests

```sh
# Unit tests
cargo test

# Integration tests (requires running Redis)
cargo test -- --ignored
```

## Examples

```sh
cargo run --example basic_queue
cargo run --example basic_worker
```

## License

This project is licensed under `MIT OR Apache-2.0`.

Parts of the BullMQ wire-compatibility layer are ported or adapted from
[BullMQ](https://github.com/taskforcesh/bullmq), which is licensed under MIT.
See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
