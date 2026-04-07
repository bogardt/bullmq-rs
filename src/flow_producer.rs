use std::sync::Arc;

use crate::connection::RedisConnection;
use crate::error::{BullmqError, BullmqResult};
use crate::job::Job;
use crate::scripts::commands::add_delayed_job::add_delayed_job_with_parent;
use crate::scripts::commands::add_parent_job::add_parent_job;
use crate::scripts::commands::add_prioritized_job::add_prioritized_job_with_parent;
use crate::scripts::commands::add_standard_job::add_standard_job_with_parent;
use crate::scripts::commands::key;
use crate::scripts::ScriptLoader;
use crate::types::{JobOptions, JobState, DEFAULT_MAX_EVENTS};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::json;

/// Producer for BullMQ flows.
pub struct FlowProducer {
    connection: RedisConnection,
    prefix: String,
    scripts: Arc<ScriptLoader>,
}

/// Builder for creating a [`FlowProducer`].
pub struct FlowProducerBuilder {
    connection: RedisConnection,
    prefix: String,
}

/// A flow job definition.
#[derive(Debug, Clone)]
pub struct FlowJob<T = serde_json::Value> {
    pub name: String,
    pub queue_name: String,
    pub data: T,
    pub prefix: Option<String>,
    pub opts: Option<JobOptions>,
    pub children: Vec<FlowJob<T>>,
}

/// A node in a flow tree.
#[derive(Debug, Clone)]
pub struct FlowNode<T = serde_json::Value> {
    pub job: Job<T>,
    pub children: Vec<FlowNode<T>>,
}

impl FlowProducerBuilder {
    /// Create a new flow producer builder.
    pub fn new() -> Self {
        Self {
            connection: RedisConnection::default(),
            prefix: "bull".to_string(),
        }
    }

    /// Set the Redis connection configuration.
    pub fn connection(mut self, conn: RedisConnection) -> Self {
        self.connection = conn;
        self
    }

    /// Set a custom key prefix (default: "bull").
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Build the flow producer.
    pub async fn build(self) -> BullmqResult<FlowProducer> {
        let _conn = self.connection.get_manager().await?;
        Ok(FlowProducer {
            connection: self.connection,
            prefix: self.prefix,
            scripts: Arc::new(ScriptLoader::new()),
        })
    }
}

impl FlowProducer {
    /// Add a flow to Redis.
    pub async fn add<T>(&self, job: FlowJob<T>) -> BullmqResult<FlowNode<T>>
    where
        T: Serialize + DeserializeOwned + Send + Sync + 'static,
    {
        let root_prefix = job.prefix.clone().unwrap_or_else(|| self.prefix.clone());
        let root_queue = job.queue_name.clone();
        validate_same_queue(&job, &root_prefix, &root_queue, &self.prefix)?;

        let auto_id_count = count_auto_ids(&job);
        let mut conn = self.connection.get_manager().await?;

        self.scripts.load("addStandardJob", &mut conn).await?;
        self.scripts.load("addPrioritizedJob", &mut conn).await?;
        self.scripts.load("addDelayedJob", &mut conn).await?;
        self.scripts.load("addParentJob", &mut conn).await?;

        let mut next_auto_id = if auto_id_count == 0 {
            0
        } else {
            let last_id: i64 = redis::cmd("INCRBY")
                .arg(key(&root_prefix, &root_queue, "id"))
                .arg(auto_id_count)
                .query_async(&mut conn)
                .await?;
            last_id - auto_id_count as i64 + 1
        };

        let prepared = prepare_node(job, &root_prefix, &root_queue, &self.prefix, None, &mut next_auto_id)?;

        redis::cmd("MULTI")
            .query_async::<redis::Value>(&mut conn)
            .await?;
        queue_insert(&self.scripts, &mut conn, &prepared).await?;
        let exec_result: redis::Value = redis::cmd("EXEC").query_async(&mut conn).await?;
        if matches!(exec_result, redis::Value::Nil) {
            return Err(BullmqError::Other(
                "Redis transaction aborted while creating flow".into(),
            ));
        }

        Ok(build_flow_node(prepared, self.scripts.clone()))
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn effective_prefix(explicit_prefix: &Option<String>, default_prefix: &str) -> String {
    explicit_prefix
        .clone()
        .unwrap_or_else(|| default_prefix.to_string())
}

fn validate_same_queue<T>(
    node: &FlowJob<T>,
    root_prefix: &str,
    root_queue: &str,
    default_prefix: &str,
) -> BullmqResult<()> {
    let prefix = effective_prefix(&node.prefix, default_prefix);
    if prefix != root_prefix || node.queue_name != root_queue {
        return Err(BullmqError::NotImplemented(
            "FlowProducer::add only supports same-queue flows for now".into(),
        ));
    }

    for child in &node.children {
        validate_same_queue(child, root_prefix, root_queue, default_prefix)?;
    }

    Ok(())
}

fn count_auto_ids<T>(node: &FlowJob<T>) -> usize {
    let own = usize::from(
        node.opts
            .as_ref()
            .and_then(|opts| opts.job_id.as_ref())
            .is_none(),
    );
    own + node.children.iter().map(count_auto_ids).sum::<usize>()
}

struct PreparedNode<T> {
    id: String,
    name: String,
    queue_name: String,
    prefix: String,
    data: T,
    opts: JobOptions,
    timestamp: u64,
    job_key: String,
    parent_key: Option<String>,
    parent_data: Option<String>,
    children: Vec<PreparedNode<T>>,
}

fn prepare_node<T>(
    job: FlowJob<T>,
    root_prefix: &str,
    root_queue: &str,
    default_prefix: &str,
    parent: Option<(String, String)>,
    next_auto_id: &mut i64,
) -> BullmqResult<PreparedNode<T>>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    let prefix = effective_prefix(&job.prefix, default_prefix);
    if prefix != root_prefix || job.queue_name != root_queue {
        return Err(BullmqError::NotImplemented(
            "FlowProducer::add only supports same-queue flows for now".into(),
        ));
    }

    let opts = job.opts.unwrap_or_default();
    let id = match opts.job_id.clone() {
        Some(job_id) => job_id,
        None => {
            let job_id = next_auto_id.to_string();
            *next_auto_id += 1;
            job_id
        }
    };
    let timestamp = now_ms();
    let job_key = key(&prefix, &job.queue_name, &id);
    let parent_descriptor = serde_json::to_string(&json!({
        "id": id,
        "queue": format!("{}:{}", prefix, job.queue_name),
    }))?;

    let children = job
        .children
        .into_iter()
        .map(|child| {
            prepare_node(
                child,
                root_prefix,
                root_queue,
                default_prefix,
                Some((job_key.clone(), parent_descriptor.clone())),
                next_auto_id,
            )
        })
        .collect::<BullmqResult<Vec<_>>>()?;

    let (parent_key, parent_data) = match parent {
        Some((parent_key, parent_data)) => (Some(parent_key), Some(parent_data)),
        None => (None, None),
    };

    Ok(PreparedNode {
        id,
        name: job.name,
        queue_name: job.queue_name,
        prefix,
        data: job.data,
        opts,
        timestamp,
        job_key,
        parent_key,
        parent_data,
        children,
    })
}

fn queue_insert<'a, T>(
    scripts: &'a ScriptLoader,
    conn: &'a mut redis::aio::ConnectionManager,
    node: &'a PreparedNode<T>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = BullmqResult<()>> + 'a>>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    Box::pin(async move {
        let data_json = serde_json::to_string(&node.data)?;
        let opts_json = serde_json::to_string(&node.opts)?;

        if node.children.is_empty() {
            let parent = node
                .parent_key
                .as_deref()
                .zip(node.parent_data.as_deref());
            let delay = node.opts.delay.map(|d| d.as_millis() as u64).unwrap_or(0);
            let priority = node.opts.priority.unwrap_or(0);

            if delay > 0 {
                add_delayed_job_with_parent(
                    scripts,
                    conn,
                    &node.prefix,
                    &node.queue_name,
                    &node.id,
                    &node.name,
                    &data_json,
                    node.timestamp,
                    &opts_json,
                    DEFAULT_MAX_EVENTS,
                    node.timestamp + delay,
                    parent,
                )
                .await?;
            } else if priority > 0 {
                add_prioritized_job_with_parent(
                    scripts,
                    conn,
                    &node.prefix,
                    &node.queue_name,
                    &node.id,
                    &node.name,
                    &data_json,
                    node.timestamp,
                    &opts_json,
                    DEFAULT_MAX_EVENTS,
                    parent,
                )
                .await?;
            } else {
                add_standard_job_with_parent(
                    scripts,
                    conn,
                    &node.prefix,
                    &node.queue_name,
                    &node.id,
                    &node.name,
                    &data_json,
                    node.timestamp,
                    &opts_json,
                    DEFAULT_MAX_EVENTS,
                    parent,
                )
                .await?;
            }

            return Ok(());
        }

        let parent = node
            .parent_key
            .as_deref()
            .zip(node.parent_data.as_deref())
            .map(|(parent_key, parent_data)| {
                (
                    format!("{parent_key}:dependencies"),
                    parent_key,
                    parent_data,
                )
            });

        let parent_ref = parent
            .as_ref()
            .map(|(dependencies_key, parent_key, parent_data)| {
                (
                    dependencies_key.as_str(),
                    *parent_key,
                    *parent_data,
                )
            });

        add_parent_job(
            scripts,
            conn,
            &node.prefix,
            &node.queue_name,
            &node.id,
            &node.name,
            &data_json,
            node.timestamp,
            &opts_json,
            DEFAULT_MAX_EVENTS,
            parent_ref,
        )
        .await?;

        let dependencies_key = format!("{}:dependencies", node.job_key);
        for child in &node.children {
            redis::cmd("SADD")
                .arg(&dependencies_key)
                .arg(&child.job_key)
                .query_async::<redis::Value>(conn)
                .await?;
        }

        for child in &node.children {
            queue_insert(scripts, conn, child).await?;
        }

        Ok(())
    })
}

fn build_flow_node<T>(node: PreparedNode<T>, scripts: Arc<ScriptLoader>) -> FlowNode<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    let mut job = Job::new(
        node.id.clone(),
        node.name,
        node.data,
        Some(node.opts),
    );
    job.timestamp = node.timestamp;
    job.state = if !node.children.is_empty() {
        JobState::WaitingChildren
    } else if job.delay > 0 {
        JobState::Delayed
    } else if job.priority > 0 {
        JobState::Prioritized
    } else {
        JobState::Wait
    };

    let children = node
        .children
        .into_iter()
        .map(|child| build_flow_node(child, scripts.clone()))
        .collect();

    FlowNode { job, children }
}
