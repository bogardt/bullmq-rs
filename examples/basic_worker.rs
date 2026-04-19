use bullmq_rs::{QueueBuilder, RedisConnection, WorkerBuilder};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Task {
    action: String,
    payload: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let conn = RedisConnection::new("redis://127.0.0.1:6379");

    // Create a queue and add some jobs
    let queue = QueueBuilder::new("tasks")
        .connection(conn.clone())
        .build::<Task>()
        .await?;

    for i in 0..5 {
        queue
            .add(
                "process",
                Task {
                    action: format!("action_{}", i),
                    payload: format!("data for task {}", i),
                },
                None,
            )
            .await?;
    }
    println!("Added 5 jobs to the queue");

    // Create a worker with concurrency and lock_duration.
    // WorkerBuilder::build() is synchronous — connections are established on start().
    let worker = WorkerBuilder::new("tasks")
        .connection(conn)
        .concurrency(2)
        .lock_duration(Duration::from_secs(30))
        .on_completed(|job| {
            println!("[completed] Job {} done", job.id);
        })
        .on_failed(|job, err| {
            println!("[failed] Job {} failed: {}", job.id, err);
        })
        .build::<Task>();

    // start() is async — it establishes Redis connections and begins processing.
    // The handler returns Result<(), Box<dyn Error + Send + Sync>>.
    let handle = worker
        .start(|job| async move {
            println!(
                "Processing job {}: action={}, payload={}",
                job.id, job.data.action, job.data.payload
            );
            // Simulate work
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok(())
        })
        .await?;

    // Let it run for a few seconds then shut down
    tokio::time::sleep(Duration::from_secs(5)).await;
    println!("Shutting down worker...");
    handle.shutdown();
    handle.wait().await?;
    println!("Worker stopped.");

    // Check final counts
    let queue2 = QueueBuilder::new("tasks")
        .connection(RedisConnection::new("redis://127.0.0.1:6379"))
        .build::<Task>()
        .await?;
    let counts = queue2.get_job_counts().await?;
    println!("Final job counts: {:?}", counts);

    Ok(())
}
