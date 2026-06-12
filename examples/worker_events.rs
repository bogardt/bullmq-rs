//! Subscribe to typed local worker events (WorkerEvent) while jobs process.

use bullmq_rs::{QueueBuilder, RedisConnection, WorkerBuilder, WorkerEvent};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Task {
    payload: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
    let conn = RedisConnection::new(&redis_url);

    let queue = QueueBuilder::new("events-demo")
        .connection(conn.clone())
        .build::<Task>()
        .await?;

    for i in 0..3 {
        queue
            .add(
                "demo",
                Task {
                    payload: format!("task {}", i),
                },
                None,
            )
            .await?;
    }

    let worker = WorkerBuilder::new("events-demo")
        .connection(conn)
        .build::<Task>();

    let handle = worker
        .start(|job| async move {
            if job.data.payload.ends_with('2') {
                return Err("task 2 always fails".into());
            }
            Ok(())
        })
        .await?;

    let mut events = handle.subscribe();
    let printer = tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            match event {
                WorkerEvent::Active { job } => println!("[active]    {}", job.id),
                WorkerEvent::Completed { job } => println!("[completed] {}", job.id),
                WorkerEvent::Failed { job, reason } => {
                    println!("[failed]    {}: {}", job.id, reason)
                }
                WorkerEvent::Drained => println!("[drained]   no more jobs"),
                other => println!("[event]     {:?}", other),
            }
        }
    });

    tokio::time::sleep(Duration::from_secs(5)).await;
    handle.shutdown();
    handle.wait().await?;
    printer.abort();

    Ok(())
}
