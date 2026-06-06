# Migrating from `bullmq-rs` 1.x to 2.0

`bullmq-rs` 2.0 is a **breaking release** that aligns the crate with the
[BullMQ Node.js v5](https://docs.bullmq.io/) Redis wire format. Queues are
now interoperable with BullMQ Node and ecosystem tooling like Bull Board,
but the v1 in-Rust data layout, types, and worker loop have changed.

This guide walks through every breaking change with a concrete v1 → v2
diff. If anything is missing, please open an issue.

> **Redis data is not backward-compatible.** v2 reads and writes a
> different set of Redis keys (BullMQ v5 layout). Existing v1 queues
> cannot be read by a v2 worker. See [§ Redis data migration](#redis-data-migration)
> at the end of this guide.

## Quick checklist

- [ ] Bump `bullmq-rs` to `2.0` in `Cargo.toml`.
- [ ] Rename `JobState::Waiting` → `JobState::Wait`.
- [ ] Update fields you read off `Job<T>`: `timestamp` is now `u64` (ms),
  `priority` is `u32`, `progress` is `Option<serde_json::Value>`.
- [ ] Remove all `WorkerBuilder::poll_interval(...)` calls.
- [ ] Change handler return type to `Result<(), Box<dyn Error + Send + Sync>>`.
- [ ] Drop the `.await` after `WorkerBuilder::build()`; keep it on `start(...)`.
- [ ] If you used `Job::wait_until_finished()`, pass a `&QueueEvents`.
- [ ] Drain or wipe any pre-existing Redis queues before starting v2 workers.

## Breaking changes

### 1. `JobState::Waiting` → `JobState::Wait`

BullMQ uses the key `wait` (not `waiting`) for the main pending list, so
the Rust variant was renamed to match.

```rust
// v1
if job.state == JobState::Waiting { ... }

// v2
if job.state == JobState::Wait { ... }
```

A new variant `JobState::WaitingChildren` was also added for flow parents.

### 2. `Job.timestamp` — `DateTime<Utc>` → `u64`

The job timestamp is now stored as **epoch milliseconds** to match BullMQ's
`timestamp` hash field exactly. This avoids round-trip drift with Node and
removes the implicit `chrono` requirement.

```rust
// v1
let created_at: chrono::DateTime<chrono::Utc> = job.timestamp;

// v2
let created_at_ms: u64 = job.timestamp;
// Reconstruct a chrono value if you need one:
let created_at = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(
    job.timestamp as i64,
).unwrap();
```

### 3. `Job.progress` — `Option<u32>` → `Option<serde_json::Value>`

BullMQ allows progress to be **either** a number **or** an arbitrary JSON
object (e.g. `{ "step": "extract", "pct": 42 }`). v2 mirrors this.

```rust
// v1
let pct: Option<u32> = job.progress;

// v2 — numeric progress
let pct = job.progress.as_ref().and_then(|v| v.as_u64());

// v2 — structured progress, set from a handler:
job.update_progress(serde_json::json!({ "step": "extract", "pct": 42 })).await?;
```

### 4. `Job.priority` — `i32` → `u32`

BullMQ priorities are non-negative integers (`0..=2_097_152`). The signed
type was a v1 modeling bug; v2 corrects it.

```rust
// v1
let p: i32 = job.priority;

// v2
let p: u32 = job.priority;
```

If you stored negative values in v1 (you shouldn't have — BullMQ would have
rejected them on the Node side), they will fail to deserialize in v2.

### 5. `WorkerOptions.poll_interval` — **removed**

v1 polled Redis on a fixed interval. v2 uses a **marker-based** worker
loop driven by `BZPOPMIN` on the `marker` sorted set, so the worker
blocks until there is work or a delayed job becomes due — no polling
overhead, no missed jobs near the poll boundary.

```rust
// v1
let worker = WorkerBuilder::new("tasks")
    .connection(conn)
    .concurrency(2)
    .poll_interval(Duration::from_millis(500)) // ❌ removed in v2
    .build::<Task>()
    .await?;

// v2
let worker = WorkerBuilder::new("tasks")
    .connection(conn)
    .concurrency(2)
    .lock_duration(Duration::from_secs(30)) // ✅ new: token-lock TTL
    .build::<Task>(); // note: no .await — see § 7
```

If you need to tune responsiveness, set `lock_duration` instead — it
controls the job-lock PX TTL and how often the worker extends its lock
on in-flight jobs.

### 6. Handler return type

Handlers used to return `Result<(), BullmqError>`. v2 widens this to
`Result<(), Box<dyn std::error::Error + Send + Sync>>` so you can `?`
errors from any library inside the handler without wrapping.

```rust
// v1
worker.start(|job| async move {
    do_work(&job).await
        .map_err(|e| BullmqError::Other(e.to_string()))?;
    Ok::<(), BullmqError>(())
}).await?;

// v2
worker.start(|job| async move {
    do_work(&job).await?; // any error type that is Send + Sync + 'static
    Ok(())
}).await?;
```

The error message and stack are surfaced via `Job.failedReason` exactly
as BullMQ Node does.

### 7. `WorkerBuilder::build()` is now synchronous

v1 opened Redis connections eagerly inside `build()`. v2 defers that to
`start().await`, which makes `build()` infallible and `Send`-free.

```rust
// v1
let worker = WorkerBuilder::new("tasks")
    .connection(conn)
    .build::<Task>()
    .await?; // ❌

// v2
let worker = WorkerBuilder::new("tasks")
    .connection(conn)
    .build::<Task>();      // ✅ sync, infallible

let handle = worker
    .start(|job| async move { /* ... */ Ok(()) })
    .await?;               // ✅ async, can fail
```

### 8. `Job::wait_until_finished` now requires `&QueueEvents`

In v1 this was a stub. In v2 it is a real implementation that subscribes
to the queue's Redis event stream and resolves when the matching
`completed` or `failed` event arrives. That requires a live
`QueueEvents` listener.

```rust
// v1
let result = job.wait_until_finished(Duration::from_secs(30)).await?;

// v2
let events = QueueEventsBuilder::new("tasks")
    .connection(conn.clone())
    .build()
    .await?;

let result = job
    .wait_until_finished(&events, Some(Duration::from_secs(30)))
    .await?;

// Pass `None` to wait indefinitely.
events.shutdown();
```

Tip: in real apps, create **one** `QueueEvents` per queue and reuse it
for all `wait_until_finished` calls — it is cheap to share and avoids
spinning up an `XREAD BLOCK` connection per call.

### 9. Redis keys — full BullMQ v5 layout

| Purpose | v1 | v2 (BullMQ v5) |
|---|---|---|
| Pending jobs | `bull:<queue>:waiting` (Sorted Set) | `bull:<queue>:wait` (**List**) |
| Currently running | `bull:<queue>:active` (Set) | `bull:<queue>:active` (**List**) |
| Paused queue | — | `bull:<queue>:paused` (List) |
| Prioritized | (mixed into `waiting`) | `bull:<queue>:prioritized` (Sorted Set) |
| Delayed | `bull:<queue>:delayed` (Sorted Set) | `bull:<queue>:delayed` (Sorted Set) |
| Completed | `bull:<queue>:completed` (Sorted Set) | `bull:<queue>:completed` (Sorted Set) |
| Failed | `bull:<queue>:failed` (Sorted Set) | `bull:<queue>:failed` (Sorted Set) |
| Waiting on children | — | `bull:<queue>:waiting-children` (Sorted Set) |
| Worker wake-up marker | — | `bull:<queue>:marker` (Sorted Set) |
| Queue metadata | — | `bull:<queue>:meta` (Hash) |
| Event stream | — | `bull:<queue>:events` (Stream) |

The actual breakage between v1 and v2 is concentrated on the two keys in
bold: `waiting` (sorted set → renamed to `wait` and switched to a List)
and `active` (set → List). `completed` / `failed` / `delayed` were
already Sorted Sets in v1, so jobs in those terminal states survive the
upgrade — only `wait` / `active` and the new BullMQ keys diverge.

This is why a v1 worker cannot read a v2 queue and vice versa — the same
queue name maps to different key types for the in-flight states.

## Redis data migration

Because the key layout is incompatible, in-flight v1 jobs cannot be
auto-migrated. Pick the strategy that matches your operational tolerance:

### Option A — drain then upgrade (recommended)

1. Stop producing new jobs into the v1 queue.
2. Let the v1 workers process the remaining jobs to completion.
3. Once `bull:<queue>:waiting` and `bull:<queue>:active` are empty, deploy v2.
4. Resume producing.

### Option B — wipe and start fresh (only if losing pending jobs is acceptable)

```sh
redis-cli --scan --pattern 'bull:<queue>:*' | xargs -r redis-cli del
```

Then deploy v2.

### Option C — change the prefix

Run v2 under a different Redis key prefix (e.g. `bullv2:`) so v1 and v2
data coexist on the same Redis instance during a cutover. Configure the
prefix on the v2 `QueueBuilder` / `WorkerBuilder`:

```rust
let queue = QueueBuilder::new("tasks")
    .connection(conn)
    .prefix("bullv2") // default is "bull"
    .build::<Task>()
    .await?;
```

## After the upgrade

You also get new APIs you didn't have in v1 — see the
[v2.0.0 entry in `CHANGELOG.md`](CHANGELOG.md) for the full list. The
highlights worth looking at first:

- **`FlowProducer`** for parent/child job trees.
- **`QueueEvents`** for typed event streaming via Redis Streams.
- **`Job::get_state()`** and helpers like `is_completed()`, `is_delayed()`.
- **`Job::retry()`**, **`Job::change_priority()`**, **`Job::promote()`**,
  **`Job::change_delay()`**, **`Job::remove()`** as active-handle methods.
- **Bull Board** now works against `bullmq-rs` queues out of the box.

## Need help?

If your migration uncovers something not covered here, please open an
issue at <https://github.com/bogardt/bullmq-rs/issues> with a minimal
v1 snippet so we can document the v2 equivalent.
