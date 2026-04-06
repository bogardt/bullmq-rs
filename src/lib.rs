//! # bullmq-rs
//!
//! A Rust implementation of [BullMQ](https://bullmq.io/) — a Redis-based
//! distributed job queue with support for priorities, delays, retries with
//! backoff, concurrency control, and typed job payloads.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use bullmq_rs::{RedisConnection, JobOptions};
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
//!     // Queue, Worker, and Job APIs are being rewritten for v2 wire compatibility.
//!     Ok(())
//! }
//! ```

pub mod connection;
pub mod error;
pub mod types;

pub mod job;
pub(crate) mod scripts;

pub mod queue;

// Worker module is temporarily disabled during the v2 rewrite.
#[cfg(any())]
pub mod worker;

pub use connection::RedisConnection;
pub use error::{BullmqError, BullmqResult};
pub use job::Job;
pub use queue::{Queue, QueueBuilder};
pub use types::{BackoffStrategy, JobOptions, JobState, WorkerOptions};
