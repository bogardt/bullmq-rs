use std::time::Duration;

use chrono::TimeZone;

use crate::error::{BullmqError, BullmqResult};
use crate::types::{JobOptions, RepeatOptions};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Compute the next iteration timestamp for a cron `pattern`, mirroring
/// BullMQ Node's `defaultRepeatStrategy`: the search starts at
/// `max(millis, startDate)`, `immediately` short-circuits to now, and a
/// result past `endDate` yields `None`.
pub(crate) fn next_pattern_millis(
    millis: u64,
    opts: &RepeatOptions,
    pattern: &str,
) -> BullmqResult<Option<u64>> {
    if opts.immediately {
        return Ok(Some(now_ms()));
    }

    let current = millis.max(opts.start_date.unwrap_or(0)) as i64;
    let cron = croner::Cron::new(pattern)
        .with_seconds_optional()
        .parse()
        .map_err(|e| BullmqError::Other(format!("Invalid cron pattern '{}': {}", pattern, e)))?;

    let next = match &opts.tz {
        Some(tz_name) => {
            let tz: chrono_tz::Tz = tz_name
                .parse()
                .map_err(|_| BullmqError::Other(format!("Invalid timezone: {}", tz_name)))?;
            let current_dt = tz
                .timestamp_millis_opt(current)
                .single()
                .ok_or_else(|| BullmqError::Other("Invalid current timestamp".into()))?;
            cron.find_next_occurrence(&current_dt, false)
                .ok()
                .map(|d| d.timestamp_millis() as u64)
        }
        None => {
            let current_dt = chrono::Local
                .timestamp_millis_opt(current)
                .single()
                .ok_or_else(|| BullmqError::Other("Invalid current timestamp".into()))?;
            cron.find_next_occurrence(&current_dt, false)
                .ok()
                .map(|d| d.timestamp_millis() as u64)
        }
    };

    match (next, opts.end_date) {
        (Some(n), Some(end)) if n > end => Ok(None),
        _ => Ok(next),
    }
}

/// Build the merged options stored on a scheduler-produced job, mirroring
/// Node's `JobScheduler.getNextJobOpts`. In 'every' mode (`pattern_next_millis`
/// is `None`) the job id, delay, and prevMillis are computed by the Lua script
/// and therefore left unset here.
pub(crate) fn build_iteration_opts(
    base_opts: &JobOptions,
    repeat_opts: &RepeatOptions,
    scheduler_id: &str,
    iteration_count: u64,
    pattern_next_millis: Option<u64>,
) -> JobOptions {
    let mut merged = base_opts.clone();

    let mut next_repeat = repeat_opts.clone();
    next_repeat.immediately = false;
    next_repeat.count = Some(iteration_count);
    if next_repeat.every.is_none() {
        next_repeat.offset = None;
    }
    merged.repeat = Some(next_repeat);
    merged.repeat_job_key = Some(scheduler_id.to_string());

    let now = now_ms();
    merged.timestamp = Some(now);

    match pattern_next_millis {
        Some(next) => {
            merged.job_id = Some(format!("repeat:{}:{}", scheduler_id, next));
            merged.prev_millis = Some(next);
            merged.delay = Some(Duration::from_millis(next.saturating_sub(now)));
        }
        None => {
            merged.job_id = None;
            merged.prev_millis = None;
            merged.delay = None;
        }
    }

    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_pattern_millis_every_minute() {
        let now = 1_700_000_000_000u64;
        let opts = RepeatOptions::default();
        let next = next_pattern_millis(now, &opts, "* * * * *")
            .unwrap()
            .unwrap();
        assert!(next > now);
        assert_eq!(next % 60_000, 0);
        assert!(next - now <= 60_000);
    }

    #[test]
    fn test_next_pattern_millis_respects_start_date() {
        let now = 1_700_000_000_000u64;
        let start = now + 3_600_000;
        let opts = RepeatOptions {
            start_date: Some(start),
            ..Default::default()
        };
        let next = next_pattern_millis(now, &opts, "* * * * *")
            .unwrap()
            .unwrap();
        assert!(next >= start);
    }

    #[test]
    fn test_next_pattern_millis_respects_end_date() {
        let now = 1_700_000_000_000u64;
        let opts = RepeatOptions {
            end_date: Some(now + 1),
            ..Default::default()
        };
        assert!(next_pattern_millis(now, &opts, "* * * * *")
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_next_pattern_millis_with_tz() {
        let now = 1_700_000_000_000u64;
        let opts = RepeatOptions {
            tz: Some("Europe/Paris".into()),
            ..Default::default()
        };
        let next = next_pattern_millis(now, &opts, "0 12 * * *")
            .unwrap()
            .unwrap();
        assert!(next > now);
    }

    #[test]
    fn test_next_pattern_millis_invalid_pattern() {
        let opts = RepeatOptions::default();
        assert!(next_pattern_millis(0, &opts, "not a cron").is_err());
    }
}
