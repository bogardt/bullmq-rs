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
    let conn = RedisConnection::new("redis://127.0.0.1:6379");

    let queue = QueueBuilder::new("emails")
        .connection(conn)
        .build::<Email>()
        .await?;

    // Add a simple job
    let job = queue
        .add(
            "welcome",
            Email {
                to: "user@example.com".into(),
                subject: "Welcome!".into(),
                body: "Hello and welcome to our platform.".into(),
            },
            None,
        )
        .await?;
    println!(
        "Added job: {} (id: {}, state: {})",
        job.name, job.id, job.state
    );

    // Add a delayed job (30 second delay)
    let delayed_job = queue
        .add(
            "reminder",
            Email {
                to: "user@example.com".into(),
                subject: "Don't forget!".into(),
                body: "You have pending tasks.".into(),
            },
            Some(JobOptions {
                delay: Some(Duration::from_secs(30)),
                ..Default::default()
            }),
        )
        .await?;
    println!(
        "Added delayed job: {} (id: {}, state: {})",
        delayed_job.name, delayed_job.id, delayed_job.state
    );

    // Add a prioritized job (priority 5)
    let priority_job = queue
        .add(
            "urgent",
            Email {
                to: "vip@example.com".into(),
                subject: "Priority mail".into(),
                body: "This is a high-priority message.".into(),
            },
            Some(JobOptions {
                priority: Some(5u32),
                ..Default::default()
            }),
        )
        .await?;
    println!(
        "Added priority job: {} (id: {}, priority: {})",
        priority_job.name, priority_job.id, priority_job.priority
    );

    // Add a job with retries and exponential backoff
    let retry_job = queue
        .add(
            "notification",
            Email {
                to: "admin@example.com".into(),
                subject: "Alert".into(),
                body: "System alert notification.".into(),
            },
            Some(JobOptions {
                attempts: Some(3),
                backoff: Some(bullmq_rs::BackoffStrategy::Exponential {
                    base: Duration::from_secs(1),
                    max: Duration::from_secs(30),
                }),
                ..Default::default()
            }),
        )
        .await?;
    println!(
        "Added retry job: {} (id: {}, max_attempts: {:?})",
        retry_job.name, retry_job.id, retry_job.opts.attempts
    );

    // Get job counts
    let counts = queue.get_job_counts().await?;
    println!("\nJob counts:");
    println!(
        "  Wait:        {}",
        counts.get(&JobState::Wait).unwrap_or(&0)
    );
    println!(
        "  Prioritized: {}",
        counts.get(&JobState::Prioritized).unwrap_or(&0)
    );
    println!(
        "  Delayed:     {}",
        counts.get(&JobState::Delayed).unwrap_or(&0)
    );
    println!(
        "  Active:      {}",
        counts.get(&JobState::Active).unwrap_or(&0)
    );
    println!(
        "  Completed:   {}",
        counts.get(&JobState::Completed).unwrap_or(&0)
    );
    println!(
        "  Failed:      {}",
        counts.get(&JobState::Failed).unwrap_or(&0)
    );

    Ok(())
}
