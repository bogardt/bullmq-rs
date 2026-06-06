# BullMQ v5 Parity

This document describes the parity status of the current branch,
`worktree-v2-bullmq-wire-compat`, relative to BullMQ Node.js v5.x.

It does **not** describe the state of `main`.

## Goal

The goal of this branch is BullMQ v5 wire compatibility plus the core queue,
worker, events, and flow behavior needed for real interoperability between
Rust and Node.

In practice, that means:

- Node BullMQ producers can enqueue work that Rust workers can process
- Rust producers can enqueue work that Node BullMQ can inspect and process
- Bull Board and other BullMQ ecosystem tools can operate against Redis data
  created by `bullmq-rs`
- Core parent/child flow state uses BullMQ-compatible Redis structures

## Implemented on This Branch

### Wire-compatible Redis layout

- BullMQ-compatible queue keys and data structures:
  - `wait`, `active`, `paused` as lists
  - `prioritized`, `delayed`, `completed`, `failed`, `waiting-children`,
    `marker` as sorted sets
  - `meta` as hash
  - `events` as stream
- BullMQ-compatible job hash fields such as:
  - `atm`, `ats`, `processedOn`, `finishedOn`, `failedReason`,
    `returnvalue`, `pb`, `opts`

### Lua-script-driven state transitions

- BullMQ-style atomic Lua script ports for core queue and worker transitions
- Script loader with BullMQ-style include resolution
- Marker-based wakeup model for workers

### Queue surface

- `add`
- `get_job`
- `get_job_counts`
- `count`
- `get_job_ids`
- `get_jobs`
- `get_waiting`
- `get_active`
- `get_delayed`
- `get_prioritized`
- `get_completed`
- `get_failed`
- `get_waiting_children`
- `remove`
- `drain`
- `pause`
- `resume`
- `is_paused`
- `add_log`
- `get_logs`

### Worker/runtime behavior

- Marker-based blocking loop using `BZPOPMIN`
- Token-based job locks with lock extension
- Stalled job recovery
- `moveToFinished` fast-path
- Startup recovery for pre-existing backlog
- Timeout recovery for missed marker wakeups
- Prefetch-channel fixes so prefetched jobs are not orphaned in `active`

### Job active-handle methods

- `update_progress`
- `log`
- `update_data`
- `clear_logs`
- `get_state`
- state helpers (`is_completed`, `is_failed`, etc.)
- `change_delay`
- `retry`
- `remove`
- `change_priority`
- `promote`
- `wait_until_finished`

### Queue events

- `QueueEvents` stream consumer via `XREAD BLOCK`
- typed `QueueEvent` enum
- `QueueEventsProducer` custom event publishing

### Core Flows parity

- `FlowProducer`
- same-queue flow creation
- cross-queue flow creation
- `waiting-children` lifecycle
- parent release on child completion
- delayed parent release
- prioritized parent release
- paused parent release
- `Job::get_dependencies()`
- `Job::get_children_values()`

## Deliberately Excluded From This Branch

This branch does **not** try to implement every BullMQ v5 subsystem.
The following areas are intentionally left out of scope for this pass:

- `JobScheduler` / repeatable jobs
- advanced Flows APIs and policies such as:
  - `getFlow(...)`
  - dependency pagination / cursors
  - ignored-dependency-on-failure and related failure-policy variants
- rate limiting
- metrics / Prometheus-style reporting
- deduplication
- debounce
- automated Node.js conformance harness

## Still Missing for Broader BullMQ v5 Parity

These are the main gaps that remain if the goal is broader BullMQ API parity,
not just wire compatibility and core runtime behavior:

### Scheduler / repeatable jobs

- `JobScheduler`
- repeatable job support
- cron / interval scheduling semantics

### Remaining Queue API surface

- `addBulk`
- `clean`
- `obliterate`
- `retryJobs`
- `promoteJobs`

### Remaining Worker API/control surface

- Worker pause / resume controls
- Worker status helpers such as `isPaused` / `isRunning`
- richer close / cancellation / rate-limit controls
- BullMQ-style listener / emitter surface beyond current Rust APIs

### Advanced Flows surface

- `getFlow(...)`
- richer dependency introspection
- ignored / failed dependency buckets
- failure-policy variants for parent/child flows

### Conformance / confidence work

- broader automated Node.js conformance testing
- more end-to-end compatibility coverage beyond the current targeted checks

## What This Branch Should Be Considered

This branch should be considered:

- BullMQ v5 wire-compatible for the implemented queue, worker, events, and
  core flow behavior
- suitable for Rust <-> Node interoperability on the implemented surface
- not yet full BullMQ v5 API parity

## Verification Snapshot

Latest branch-local verification for the flow slice:

- `cargo test --test unit_tests`
- `cargo test --test integration_tests -- --ignored --test-threads=1 test_flow_`
- `cargo test --test integration_tests -- --ignored --test-threads=1 test_job_get_dependencies_and_children_values`
