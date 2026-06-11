# Cross-Language Compatibility Tests

These tests verify that bullmq-rs v2 produces Redis data that BullMQ Node.js can read and vice versa. They run in CI (see the `compat` job in `.github/workflows/ci.yml`) and can also be run locally.

## Prerequisites

- Redis running on localhost:6379 (or set `REDIS_URL`)
- Node.js 18+
- `npm ci` in this directory

## Running

```bash
# Rust adds jobs (plain, delayed, prioritized), Node.js verifies them
cargo run --example compat_producer
node rust_producer_node_reader.js

# Node.js adds a job, Rust processes it, Node.js verifies completion
node node_producer.js
cargo run --example compat_worker
node node_verify_completed.js
```

## Queues

Each direction uses its own queue so runs do not interfere:

| Direction   | Default queue        | Override env var   |
| ----------- | -------------------- | ------------------ |
| Rust → Node | `compat-rust-to-node` | `COMPAT_R2N_QUEUE` |
| Node → Rust | `compat-node-to-rust` | `COMPAT_N2R_QUEUE` |

When re-running locally against a non-fresh Redis, flush the compat queues first (e.g. `redis-cli --scan --pattern 'bull:compat-*' | xargs redis-cli del`).
