# Cross-Language Compatibility Tests

These tests verify that bullmq-rs v2 produces Redis data that BullMQ Node.js can read and vice versa.

## Prerequisites

- Redis running on localhost:6379 (or set REDIS_URL)
- Node.js 18+
- `npm install` in this directory

## Running

```bash
# Rust adds job, Node.js verifies it
cargo run --example basic_queue  # add some jobs first
node rust_producer_node_reader.js

# Node.js adds job, Rust reads it
node node_producer.js
cargo run --example basic_worker  # process the job
```
