use std::collections::HashMap;

use redis::aio::ConnectionManager;

use crate::error::{BullmqError, BullmqResult};

pub mod commands;

/// Resolve `--@include "name"` directives by inlining the named include content.
pub(crate) fn resolve_includes(source: &str, includes: &HashMap<String, String>) -> String {
    let mut result = String::with_capacity(source.len() * 2);
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("--@include") {
            if let Some(name) = trimmed
                .strip_prefix("--@include")
                .and_then(|s| s.trim().strip_prefix('"'))
                .and_then(|s| s.strip_suffix('"'))
            {
                if let Some(content) = includes.get(name) {
                    result.push_str(content);
                    result.push('\n');
                }
            }
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}

/// Loads, caches, and invokes Lua scripts via EVALSHA/EVAL.
pub(crate) struct ScriptLoader {
    scripts: HashMap<&'static str, redis::Script>,
}

impl ScriptLoader {
    pub fn new() -> Self {
        let includes = Self::load_includes();
        let mut scripts = HashMap::new();

        macro_rules! register {
            ($name:literal, $file:literal) => {
                let source = include_str!(concat!("../../lua/", $file));
                let resolved = resolve_includes(source, &includes);
                scripts.insert($name, redis::Script::new(&resolved));
            };
        }

        register!("addStandardJob", "addStandardJob-9.lua");
        register!("addPrioritizedJob", "addPrioritizedJob-9.lua");
        register!("addDelayedJob", "addDelayedJob-6.lua");
        register!("addParentJob", "addParentJob-6.lua");
        register!("moveToActive", "moveToActive-11.lua");
        register!("moveToFinished", "moveToFinished-14.lua");
        register!("moveToDelayed", "moveToDelayed-8.lua");
        register!("retryJob", "retryJob-11.lua");
        register!("moveStalledJobsToWait", "moveStalledJobsToWait-8.lua");
        register!("extendLock", "extendLock-2.lua");
        register!("pause", "pause-7.lua");
        register!("addLog", "addLog-2.lua");
        register!("changePriority", "changePriority-7.lua");
        register!("promote", "promote-9.lua");

        Self { scripts }
    }

    fn load_includes() -> HashMap<String, String> {
        let mut m = HashMap::new();
        macro_rules! inc {
            ($name:literal, $file:literal) => {
                m.insert(
                    $name.to_string(),
                    include_str!(concat!("../../lua/includes/", $file)).to_string(),
                );
            };
        }
        inc!("storeJob", "storeJob.lua");
        inc!("addJobInTargetList", "addJobInTargetList.lua");
        inc!("addJobWithPriority", "addJobWithPriority.lua");
        inc!("addBaseMarkerIfNeeded", "addBaseMarkerIfNeeded.lua");
        inc!("addDelayMarkerIfNeeded", "addDelayMarkerIfNeeded.lua");
        inc!("getDelayedScore", "getDelayedScore.lua");
        inc!("getTargetQueueList", "getTargetQueueList.lua");
        inc!("getPriorityScore", "getPriorityScore.lua");
        inc!("moveJobToWait", "moveJobToWait.lua");
        inc!("moveParentToWait", "moveParentToWait.lua");
        inc!("moveParentToWaitIfNeeded", "moveParentToWaitIfNeeded.lua");
        inc!(
            "moveParentToWaitIfNoPendingDependencies",
            "moveParentToWaitIfNoPendingDependencies.lua"
        );
        inc!("promoteDelayedJobs", "promoteDelayedJobs.lua");
        inc!("removeLock", "removeLock.lua");
        inc!("updateParentDepsIfNeeded", "updateParentDepsIfNeeded.lua");
        m
    }

    pub async fn invoke(
        &self,
        name: &str,
        conn: &mut ConnectionManager,
        keys: &[String],
        args: &[Vec<u8>],
    ) -> BullmqResult<redis::Value> {
        let script = self
            .scripts
            .get(name)
            .ok_or_else(|| BullmqError::ScriptError(format!("Unknown script: {}", name)))?;
        let mut invocation = script.prepare_invoke();
        for key in keys {
            invocation.key(key);
        }
        for arg in args {
            invocation.arg(arg.as_slice());
        }
        let result: redis::Value = invocation.invoke_async(conn).await?;
        Ok(result)
    }

    pub async fn load(&self, name: &str, conn: &mut ConnectionManager) -> BullmqResult<()> {
        let script = self
            .scripts
            .get(name)
            .ok_or_else(|| BullmqError::ScriptError(format!("Unknown script: {}", name)))?;
        script.prepare_invoke().load_async(conn).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_includes() {
        let main = "local foo = 1\n--@include \"helper\"\nlocal bar = 2";
        let includes = HashMap::from([("helper".to_string(), "local helper_val = 42".to_string())]);
        let resolved = resolve_includes(main, &includes);
        assert!(resolved.contains("local helper_val = 42"));
        assert!(resolved.contains("local foo = 1"));
        assert!(resolved.contains("local bar = 2"));
        assert!(!resolved.contains("--@include"));
    }

    #[test]
    fn test_resolve_includes_multiple() {
        let main = "--@include \"a\"\n--@include \"b\"";
        let includes = HashMap::from([
            ("a".to_string(), "local a = 1".to_string()),
            ("b".to_string(), "local b = 2".to_string()),
        ]);
        let resolved = resolve_includes(main, &includes);
        assert!(resolved.contains("local a = 1"));
        assert!(resolved.contains("local b = 2"));
    }

    #[test]
    fn test_resolve_includes_missing() {
        let main = "--@include \"nonexistent\"\nlocal x = 1";
        let includes = HashMap::new();
        let resolved = resolve_includes(main, &includes);
        assert!(resolved.contains("local x = 1"));
        assert!(!resolved.contains("nonexistent"));
    }

    #[test]
    fn test_script_loader_creates_all_scripts() {
        let loader = ScriptLoader::new();
        assert!(loader.scripts.contains_key("addStandardJob"));
        assert!(loader.scripts.contains_key("addPrioritizedJob"));
        assert!(loader.scripts.contains_key("addDelayedJob"));
        assert!(loader.scripts.contains_key("addParentJob"));
        assert!(loader.scripts.contains_key("moveToActive"));
        assert!(loader.scripts.contains_key("moveToFinished"));
        assert!(loader.scripts.contains_key("moveToDelayed"));
        assert!(loader.scripts.contains_key("retryJob"));
        assert!(loader.scripts.contains_key("moveStalledJobsToWait"));
        assert!(loader.scripts.contains_key("extendLock"));
        assert!(loader.scripts.contains_key("pause"));
        assert!(loader.scripts.contains_key("addLog"));
        assert!(loader.scripts.contains_key("changePriority"));
        assert!(loader.scripts.contains_key("promote"));
        assert_eq!(loader.scripts.len(), 14);
    }
}
