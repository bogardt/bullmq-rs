# BullMQ v5 Parity

This document describes the parity status of `main` relative to BullMQ
Node.js v5.x. It is updated as features land; the original v2.0.0 scope and
the follow-up roadmap were tracked in
[#4](https://github.com/bogardt/bullmq-rs/issues/4).

## Goal

BullMQ v5 wire compatibility plus the queue, worker, events, scheduler and
flow behavior needed for real interoperability between Rust and Node:

- Node BullMQ producers can enqueue work that Rust workers can process
- Rust producers can enqueue work that Node BullMQ can inspect and process
- Bull Board and other BullMQ ecosystem tools can operate against Redis data
  created by `bullmq-rs`
- Rust and Node workers can cohabit on the same queues (shared locks,
  rate limits, schedulers)

## Implemented

### Wire-compatible Redis layout

- BullMQ-compatible queue keys and data structures:
  - `wait`, `active`, `paused` as lists
  - `prioritized`, `delayed`, `completed`, `failed`, `waiting-children`,
    `marker`, `repeat` as sorted sets
  - `meta`, `repeat:<id>`, `metrics:<state>` as hashes
  - `events` as stream; `limiter`, `de:<id>` as strings
- BullMQ-compatible job hash fields (`atm`, `ats`, `processedOn`,
  `finishedOn`, `failedReason`, `returnvalue`, `pb`, `opts`, `rjk`, `deid`,
  `nrjid`)

### Lua-script-driven state transitions

- 23 Lua scripts ported from BullMQ v5 (add-job family, moveToActive,
  moveToFinished, moveToDelayed, retryJob, moveStalledJobsToWait,
  extendLock/extendLocks, pause, changePriority, promote, addLog,
  moveJobsToWait, cleanJobsInSet, obliterate, getMetrics,
  addJobScheduler, updateJobScheduler, getJobScheduler, removeJobScheduler)
  plus their `includes/` dependencies
- Script loader with BullMQ-style include resolution
- Marker-based wakeup model for workers

### Queue surface

- `add`, `add_bulk`, `get_job`, `get_job_counts`, `count`
- `get_job_ids`, `get_jobs`, `get_waiting`, `get_active`, `get_delayed`,
  `get_prioritized`, `get_completed`, `get_failed`, `get_waiting_children`
- `remove`, `drain`, `clean`, `obliterate`
- `retry_jobs`, `promote_jobs`
- `pause`, `resume`, `is_paused`
- `add_log`, `get_logs`
- `get_metrics`
- `upsert_job_scheduler`, `get_job_scheduler`, `get_job_schedulers`,
  `remove_job_scheduler`
- `get_next_job`, `extend_job_locks` (manual processing)

### Worker/runtime behavior

- Marker-based blocking loop using `BZPOPMIN`
- Token-based job locks with lock extension and stalled job recovery
- `moveToFinished` fast-path with prefetch-safe dispatch
- Startup and timeout recovery for missed markers / pre-existing backlog
- Rate limiting (`WorkerOptions::limiter`, shared `limiter` key)
- Metrics collection (`WorkerOptions::metrics`)
- Automatic rescheduling of repeatable jobs (next iteration on finish)
- `WorkerHandle`: `pause(do_not_wait_active)`, `resume`, `is_paused`,
  `is_running`, `cancel_job`, graceful `shutdown`/`wait`
- Callbacks: `on_completed`, `on_failed`, `on_active`, `on_error`
- Typed local worker events: `WorkerHandle::subscribe()` returns a
  `tokio::sync::broadcast` receiver of `WorkerEvent` (`Active`, `Completed`,
  `Failed`, `Error`, `Drained`, `Paused`, `Resumed`, `Cancelled`,
  `RateLimited`)

### Job options

- Priorities, delays, attempts with fixed/exponential backoff
- Custom job ids, deduplication (`DeduplicationOptions { id, ttl }`)
- Repeat metadata on scheduler-produced jobs

### Job active-handle methods

- `update_progress`, `log`, `update_data`, `clear_logs`
- `get_state` and state helpers (`is_completed`, `is_failed`, …)
- `change_delay`, `retry`, `remove`, `change_priority`, `promote`
- `wait_until_finished`, `get_dependencies`, `get_children_values`
- `move_to_completed`, `move_to_failed` (manual processing)

### Queue events

- `QueueEvents` stream consumer via `XREAD BLOCK`, typed `QueueEvent` enum
- `QueueEventsProducer` custom event publishing

### Core Flows parity

- `FlowProducer` — same-queue and cross-queue trees
- `waiting-children` lifecycle, parent release on child completion
  (including delayed / prioritized / paused parents)
- `Job::get_dependencies()`, `Job::get_children_values()`

### Conformance

- Automated cross-language harness in CI (`compat` job): Rust producer →
  Node reader and Node producer → Rust worker against `bullmq@5.x` on
  Redis 7

## Known gaps

### Rate limiting

- Dynamic `queue.rateLimit()` / `worker.rateLimit()` and the manual
  rate-limit error are not ported (worker-level `limiter` option only)

### Deduplication

- `replace` / `extend` / `keepLastIfActive` modes are not exposed
- Removing a job does not proactively clear a still-live TTL'd dedup key

### Scheduler

- Repeat strategy is not pluggable (default cron/every strategy only)
- Legacy (pre-Job-Scheduler) repeatable key format is not supported

### Worker

- `cancel_job` aborts the processing future; there is no cooperative
  `AbortSignal` equivalent — the cancelled job is requeued via stalled
  recovery
- Node's stringly-typed worker `EventEmitter` is replaced by the typed
  `WorkerEvent` broadcast (`WorkerHandle::subscribe`); lagging subscribers
  skip events (broadcast semantics, `RecvError::Lagged`) — use
  `QueueEvents` for the cross-process stream

### Advanced Flows surface

- `getFlow(...)`, dependency pagination / cursors
- ignored / failed dependency buckets and failure-policy variants

### Out of scope (design discussion first)

- Sandboxed processors (separate-process handlers)

## Verification

- `cargo test --test unit_tests` — 39 tests
- `cargo test --lib` — 9 tests
- `cargo test --test integration_tests -- --ignored --test-threads=1` —
  82 tests against a live Redis 7
- CI `compat` job — cross-language wire checks in both directions
