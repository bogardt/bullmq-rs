//! Compat fixture: processes jobs added by BullMQ Node.js and exits.
//!
//! See tests/compat/README.md for the full cross-language flow.

use bullmq_rs::{JobState, QueueBuilder, RedisConnection, WorkerBuilder};
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
        std::env::var("COMPAT_N2R_QUEUE").unwrap_or_else(|_| "compat-node-to-rust".into());

    let conn = RedisConnection::new(&redis_url);

    let queue = QueueBuilder::new(&queue_name)
        .connection(conn.clone())
        .build::<Email>()
        .await?;

    let worker = WorkerBuilder::new(&queue_name)
        .connection(conn)
        .concurrency(1)
        .lock_duration(Duration::from_secs(30))
        .build::<Email>();

    let handle = worker
        .start(|job| async move {
            println!(
                "Processing job {} (name={}): to={}, subject={}",
                job.id, job.name, job.data.to, job.data.subject
            );
            if job.name != "welcome" {
                return Err(format!("unexpected job name '{}'", job.name).into());
            }
            if job.data.to.is_empty() || job.data.subject.is_empty() || job.data.body.is_empty() {
                return Err("job data fields missing after Node->Rust round-trip".into());
            }
            Ok(())
        })
        .await?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let completed;
    loop {
        let counts = queue.get_job_counts().await?;
        let current = *counts.get(&JobState::Completed).unwrap_or(&0);
        let failed = *counts.get(&JobState::Failed).unwrap_or(&0);
        if failed > 0 {
            eprintln!("FAILED: {} job(s) failed during processing", failed);
            handle.shutdown();
            handle.wait().await?;
            std::process::exit(1);
        }
        if current >= 1 {
            completed = current;
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            eprintln!(
                "FAILED: no job completed within 30s (queue '{}')",
                queue_name
            );
            handle.shutdown();
            handle.wait().await?;
            std::process::exit(1);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    handle.shutdown();
    handle.wait().await?;

    println!(
        "SUCCESS: {} Node-produced job(s) processed by bullmq-rs on queue '{}'",
        completed, queue_name
    );
    Ok(())
}
