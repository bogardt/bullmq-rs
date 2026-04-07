//! # bullmq-rs
//!
//! A Rust implementation of [BullMQ](https://bullmq.io/) — a Redis-based
//! distributed job queue with support for priorities, delays, retries with
//! backoff, concurrency control, and typed job payloads.
//!
//! Wire-compatible with BullMQ Node.js v5.x — jobs created by this crate
//! can be consumed by Node.js workers and vice versa.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use bullmq_rs::{RedisConnection, QueueBuilder, WorkerBuilder, JobOptions};
//! use serde::{Serialize, Deserialize};
//! use std::time::Duration;
//!
//! #[derive(Serialize, Deserialize, Debug, Clone)]
//! struct Email {
//!     to: String,
//!     subject: String,
//!     body: String,
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let conn = RedisConnection::new("redis://127.0.0.1:6379");
//!
//!     // Create a queue
//!     let queue = QueueBuilder::new("emails")
//!         .connection(conn.clone())
//!         .build::<Email>()
//!         .await?;
//!
//!     // Add a job
//!     queue.add("welcome", Email {
//!         to: "user@example.com".into(),
//!         subject: "Welcome!".into(),
//!         body: "Thanks for signing up.".into(),
//!     }, None).await?;
//!
//!     // Create and start a worker
//!     let worker = WorkerBuilder::new("emails")
//!         .connection(conn)
//!         .concurrency(5)
//!         .build::<Email>();
//!
//!     let handle = worker.start(|job| async move {
//!         println!("Sending email to {}", job.data.to);
//!         Ok(())
//!     }).await?;
//!
//!     // Graceful shutdown
//!     handle.shutdown();
//!     handle.wait().await?;
//!     Ok(())
//! }
//! ```

pub mod connection;
pub mod error;
pub mod types;

pub mod job;
pub(crate) mod scripts;

pub mod queue;
pub mod queue_events;

pub mod worker;

pub use connection::RedisConnection;
pub use error::{BullmqError, BullmqResult};
pub use job::Job;
pub use queue::{Queue, QueueBuilder};
pub use queue_events::{QueueEvent, QueueEvents, QueueEventsBuilder};
pub use types::{BackoffStrategy, JobOptions, JobState, WorkerOptions};
pub use worker::{Worker, WorkerBuilder, WorkerHandle};
