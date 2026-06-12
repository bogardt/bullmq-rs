//! Repeatable jobs with a JobScheduler: fixed interval, inspection, removal.

use bullmq_rs::{
    JobSchedulerTemplate, QueueBuilder, RateLimiterOptions, RedisConnection, RepeatOptions,
    WorkerBuilder,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Report {
    kind: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
    let conn = RedisConnection::new(&redis_url);

    let queue = QueueBuilder::new("reports")
        .connection(conn.clone())
        .build::<Report>()
        .await?;

    // Every 2 seconds, at most 3 iterations.
    queue
        .upsert_job_scheduler(
            "demo-report",
            RepeatOptions {
                every: Some(Duration::from_secs(2)),
                limit: Some(3),
                ..Default::default()
            },
            JobSchedulerTemplate {
                name: Some("generate".into()),
                data: Some(Report {
                    kind: "demo".into(),
                }),
                opts: None,
            },
        )
        .await?;

    for scheduler in queue.get_job_schedulers(0, -1).await? {
        println!(
            "scheduler '{}': every={:?} next={:?} iterations={:?}",
            scheduler.id, scheduler.every, scheduler.next, scheduler.iteration_count
        );
    }

    let worker = WorkerBuilder::new("reports")
        .connection(conn)
        .limiter(RateLimiterOptions {
            max: 10,
            duration: Duration::from_secs(1),
        })
        .build::<Report>();

    let handle = worker
        .start(|job| async move {
            println!("generated {} ({})", job.id, job.data.kind);
            Ok(())
        })
        .await?;

    // Let the 3 iterations run (~6s), then clean up.
    tokio::time::sleep(Duration::from_secs(8)).await;
    handle.shutdown();
    handle.wait().await?;

    queue.remove_job_scheduler("demo-report").await?;
    println!("scheduler removed");

    Ok(())
}
