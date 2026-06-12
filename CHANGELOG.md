# Changelog

All notable changes to `bullmq-rs` are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [2.1.0] — 2026-06-12

This release closes the [BullMQ v5 parity roadmap (#4)](https://github.com/bogardt/bullmq-rs/issues/4):
everything listed under "Deliberately not in this release" for 2.0.0 has landed,
except sandboxed processors (out of scope pending a design discussion).

### Added

- **JobScheduler / repeatable jobs** — `Queue::upsert_job_scheduler`,
  `get_job_scheduler`, `get_job_schedulers`, `remove_job_scheduler`, with
  cron `pattern` (via `croner`, IANA `tz` support), fixed `every` intervals,
  `limit`, `start_date` / `end_date`, `immediately` and `offset`. Workers
  automatically schedule the next iteration when a scheduler job finishes
  (`addJobScheduler-11` / `updateJobScheduler-12` Lua ports, `repeat` zset +
  `repeat:<id>` scheduler hashes, wire-identical to Node).
- **Queue retention** — `Queue::clean(grace, limit, state)` and
  `Queue::obliterate(force)` (`cleanJobsInSet-3` / `obliterate-2` Lua ports).
- **Bulk operations** — `Queue::add_bulk`, `Queue::retry_jobs`,
  `Queue::promote_jobs` (`moveJobsToWait-8` Lua port).
- **Rate limiting** — `WorkerOptions::limiter` / `WorkerBuilder::limiter`
  with `RateLimiterOptions { max, duration }`; limiter logic restored in
  `moveToActive-11` / `moveToFinished-14` (shared `limiter` key, Node-compatible).
  Dynamic rate limiting: `Queue::rate_limit(duration)`,
  `Queue::remove_rate_limit_key()`, `WorkerHandle::rate_limit(duration)`, and
  `BullmqError::RateLimited` — a handler returning it puts the job back in
  wait without consuming an attempt (`moveJobFromActiveToWait-9` Lua port).
- **Deduplication** — `JobOptions::deduplication` with
  `DeduplicationOptions { id, ttl }`; restored in the add-job scripts and
  `storeJob`; dedup key cleared on finalization; deduplicated adds return
  the existing job's id, mirroring Node.
- **Metrics** — `WorkerOptions::metrics` / `WorkerBuilder::metrics`
  (`MetricsOptions { max_data_points }`) and `Queue::get_metrics(state, start, end)`
  (`getMetrics-2` Lua port, per-minute data points).
- **Worker control surface** — `WorkerHandle::pause(do_not_wait_active)`,
  `resume`, `is_paused`, `is_running`; `WorkerHandle::cancel_job` with
  cooperative cancellation (`Worker::start_with_signal` hands the handler a
  `CancellationToken`; the future is force-aborted after a 5 s grace period);
  `WorkerBuilder::on_active` / `on_error` callbacks.
- **Atomic job removal** — `Queue::remove` / `Job::remove` now use the
  `removeJob-2` Lua port: the deduplication key is purged with the job and a
  `removed` event is emitted.
- **Manual processing** — `Queue::get_next_job(token)`,
  `Job::move_to_completed` / `move_to_failed`,
  `Queue::extend_job_locks(job_ids, tokens, duration)` (`extendLocks-1` Lua port).
- **Automated cross-language conformance in CI** — the `compat` job runs the
  `tests/compat/` harness against `bullmq@5.x` on a Redis 7 service container
  in both directions (Rust producer → Node reader, Node producer → Rust worker).

### Known limitations

- Deduplication `replace` / `extend` / `keepLastIfActive` modes are not exposed.
- The repeat strategy is not pluggable (default cron/every strategy only).
- `Queue::remove` does not remove child jobs (`remove_children=false`,
  whereas Node defaults to `true`).

## [2.0.0] — BullMQ v5 wire compatibility

This is a **breaking release** that brings `bullmq-rs` in line with the
[BullMQ Node.js v5](https://docs.bullmq.io/) Redis wire format and core runtime
behavior. Rust and Node can now share queues, jobs, events, and core
parent/child flow state, and Bull Board / other BullMQ ecosystem tooling can
operate against queues produced by `bullmq-rs`.

See [`MIGRATION.md`](MIGRATION.md) for a step-by-step v1 → v2 migration guide.

### Added

- **BullMQ-compatible Redis data layout**
  - `wait`, `active`, `paused` as Redis Lists
  - `prioritized`, `delayed`, `completed`, `failed`, `waiting-children`, `marker` as Sorted Sets
  - `meta` as Hash, `events` as Stream
- **Lua scripts ported from BullMQ** for atomic state transitions:
  `addStandardJob`, `addPrioritizedJob`, `addDelayedJob`, `addParentJob`,
  `moveToActive`, `moveToFinished`, `moveToDelayed`, `moveStalledJobsToWait`,
  `retryJob`, `changePriority`, `promote`, `pause`, `extendLock`, `addLog`,
  plus their `includes/` dependencies.
- **Marker-based worker loop** using `BZPOPMIN` (no more polling).
- **Token-based job locks** with `PX` TTL, lock extension, stalled-job recovery,
  and a `moveToFinished` fast-path.
- **Startup and timeout recovery** for missed markers / pre-existing backlog.
- **Queue API** parity with BullMQ:
  - `get_job_ids`, `get_jobs`
  - `get_waiting`, `get_active`, `get_delayed`, `get_prioritized`
  - `get_completed`, `get_failed`, `get_waiting_children`
  - `drain`, `pause`, `resume`, `is_paused`
  - `add_log`, `get_logs`
- **`Job<T>` active-handle methods**:
  `update_progress`, `log`, `update_data`, `clear_logs`,
  `get_state` and state helpers (`is_completed`, `is_failed`, `is_delayed`, …),
  `change_delay`, `retry`, `remove`, `change_priority`, `promote`,
  `wait_until_finished`, `get_dependencies`, `get_children_values`.
- **`QueueEvents`** — Redis-stream consumer (`XREAD BLOCK`) with a typed
  `QueueEvent` enum and `tokio::broadcast` delivery.
- **`QueueEventsProducer`** for publishing custom events via `XADD`.
- **`FlowProducer` / `FlowProducerBuilder`** — same-queue and cross-queue
  flow tree creation, `waiting-children` lifecycle, parent-release on child
  completion, delayed/paused/prioritized parent-release behavior.
- **`THIRD_PARTY_NOTICES.md`** documenting the Lua scripts ported from BullMQ.
- **Cross-language compatibility harness** under `tests/compat/`
  (Rust producer ↔ Node consumer, and vice versa).
- **`BULLMQ_V5_PARITY.md`** tracking what is in scope and what is deliberately
  excluded from this release.

### Changed (breaking)

| Area | v1 | v2 |
|------|----|-----|
| `JobState::Waiting` variant | `JobState::Waiting` | `JobState::Wait` |
| `Job.timestamp` | `chrono::DateTime<Utc>` | `u64` (epoch milliseconds) |
| `Job.progress` | `Option<u32>` | `Option<serde_json::Value>` |
| `Job.priority` | `i32` | `u32` |
| `WorkerOptions.poll_interval` | `Duration` | **Removed** (marker-based loop) |
| Handler return type | `Result<(), BullmqError>` | `Result<(), Box<dyn Error + Send + Sync>>` |
| Redis keys | `waiting` (zset), `active` (set) | BullMQ v5 layout (see above) |
| `Job::wait_until_finished` | stub | real implementation; now requires `&QueueEvents` |
| `WorkerBuilder::build()` | `async` | synchronous (connections established on `start().await`) |

### Removed

- `WorkerOptions::poll_interval` and `WorkerBuilder::poll_interval()` —
  the worker is now marker-driven and no longer polls.
- The v1 `waiting` (Sorted Set) and `active` (Set) Redis keys. Queues created
  by v1 are **not** readable by v2; see migration notes.

### Deliberately not in this release

The following are tracked in [`BULLMQ_V5_PARITY.md`](BULLMQ_V5_PARITY.md) and
will land in subsequent `2.x` releases:

- `JobScheduler` / repeatable jobs
- `addBulk`, `clean`, `obliterate`, `retryJobs`, `promoteJobs`
- Worker control surface: `pause` / `resume`, `isPaused` / `isRunning`,
  `close`, `cancelJob`, listener/event-emitter API
- Rate limiting, metrics, deduplication / debounce
- Automated Node.js conformance harness
- Advanced Flows APIs: `getFlow`, dependency pagination/cursors,
  ignored-dependency-on-failure / failure-policy variants

### Verification

- `cargo test --test unit_tests` — 39 tests pass
- `cargo test --test integration_tests -- --ignored --test-threads=1 test_flow_` — 12 tests pass
- `cargo test --test integration_tests -- --ignored --test-threads=1 test_job_get_dependencies_and_children_values` — 1 test passes
- Cross-language wire compatibility validated:
  Rust producer → Node reader, Node producer → Rust worker,
  Node reads queue state after Rust processing,
  `QueueEvents` lifecycle delivery, `waitUntilFinished` result / timeout.

### Acknowledgments

The v2 wire-compat work was contributed by [@enricodeleo](https://github.com/enricodeleo)
in [#3](https://github.com/bogardt/bullmq-rs/pull/3), resolving
[#2](https://github.com/bogardt/bullmq-rs/issues/2).

The Lua scripts under `lua/` are ports of [BullMQ](https://github.com/taskforcesh/bullmq)
(MIT-licensed). See [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).

## [1.1.0] — 2026-03-05

### Changed

- Use `redis::aio::ConnectionManager` to handle reconnection and a wider
  range of failure cases. ([ed6db1d](https://github.com/bogardt/bullmq-rs/commit/ed6db1d))

## [0.2.2] — 2025-02-13

### Fixed

- Worker reliability fixes and connection consolidation onto a single Redis
  connection.

## [0.2.1] — 2025-02-13

- README and metadata updates.

## [0.2.0] — 2025-02-13

### Added

- Initial worker / queue trigger service split.
- Basic test suite.
- Docker / docker-compose setup for local Redis.

## [0.1.0] — 2025-02-11

- First public release.

[Unreleased]: https://github.com/bogardt/bullmq-rs/compare/v2.1.0...HEAD
[2.1.0]: https://github.com/bogardt/bullmq-rs/compare/v2.0.0...v2.1.0
[2.0.0]: https://github.com/bogardt/bullmq-rs/compare/v1.1.0...v2.0.0
[1.1.0]: https://github.com/bogardt/bullmq-rs/compare/v0.2.2...v1.1.0
[0.2.2]: https://github.com/bogardt/bullmq-rs/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/bogardt/bullmq-rs/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/bogardt/bullmq-rs/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/bogardt/bullmq-rs/releases/tag/v0.1.0
