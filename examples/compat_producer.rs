//! Compat fixture: adds jobs for BullMQ Node.js to read.
//!
//! See tests/compat/README.md for the full cross-language flow.

use bullmq_rs::{JobOptions, JobState, QueueBuilder, RedisConnection};
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
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
    let queue_name =
        std::env::var("COMPAT_R2N_QUEUE").unwrap_or_else(|_| "compat-rust-to-node".into());

    let conn = RedisConnection::new(&redis_url);
    let queue = QueueBuilder::new(&queue_name)
        .connection(conn)
        .build::<Email>()
        .await?;

    let plain = queue
        .add(
            "welcome",
            Email {
                to: "user@example.com".into(),
                subject: "Welcome!".into(),
                body: "Produced by bullmq-rs".into(),
            },
            None,
        )
        .await?;
    println!("Added plain job {}", plain.id);

    let delayed = queue
        .add(
            "reminder",
            Email {
                to: "user@example.com".into(),
                subject: "Delayed".into(),
                body: "Produced by bullmq-rs (delayed)".into(),
            },
            Some(JobOptions {
                delay: Some(Duration::from_secs(3600)),
                ..Default::default()
            }),
        )
        .await?;
    println!("Added delayed job {}", delayed.id);

    let prioritized = queue
        .add(
            "urgent",
            Email {
                to: "vip@example.com".into(),
                subject: "Priority".into(),
                body: "Produced by bullmq-rs (priority 5)".into(),
            },
            Some(JobOptions {
                priority: Some(5u32),
                ..Default::default()
            }),
        )
        .await?;
    println!("Added prioritized job {}", prioritized.id);

    let counts = queue.get_job_counts().await?;
    let wait = *counts.get(&JobState::Wait).unwrap_or(&0);
    let delayed_count = *counts.get(&JobState::Delayed).unwrap_or(&0);
    let prioritized_count = *counts.get(&JobState::Prioritized).unwrap_or(&0);
    println!(
        "Counts: wait={} delayed={} prioritized={}",
        wait, delayed_count, prioritized_count
    );

    if wait < 1 || delayed_count < 1 || prioritized_count < 1 {
        eprintln!("FAILED: expected at least one job in each of wait/delayed/prioritized");
        std::process::exit(1);
    }

    println!("SUCCESS: jobs produced on queue '{}'", queue_name);
    Ok(())
}
