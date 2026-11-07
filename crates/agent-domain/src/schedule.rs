use crate::task::{Priority, TaskId};
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduleError {
    #[error("Invalid cron expression '{expression}': {reason}")]
    InvalidCronExpression { expression: String, reason: String },
    #[error("Invalid schedule interval: {0}")]
    InvalidInterval(String),
    #[error("Could not compute next run time after {0}")]
    NoNextOccurrence(String),
}

/// Parsed cron schedule supporting standard 5-field (`min hour dom month dow`),
/// extended 6-field (`sec min hour dom month dow`), and `@` descriptors (`@every`, `@hourly`, `@daily`, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSchedule {
    seconds: Vec<u32>,
    minutes: Vec<u32>,
    hours: Vec<u32>,
    days_of_month: Vec<u32>,
    months: Vec<u32>,
    days_of_week: Vec<u32>,
    interval_ms_override: Option<u64>,
}

impl CronSchedule {
    pub fn parse(expression: &str) -> Result<Self, ScheduleError> {
        let trimmed = expression.trim();
        if trimmed.is_empty() {
            return Err(ScheduleError::InvalidCronExpression {
                expression: expression.to_string(),
                reason: "expression cannot be empty".to_string(),
            });
        }
        if let Some(rest) = trimmed.strip_prefix("@every ") {
            let ms = parse_duration_ms(rest.trim()).ok_or_else(|| {
                ScheduleError::InvalidCronExpression {
                    expression: expression.to_string(),
                    reason: "invalid @every duration (expected e.g. 500ms, 5s, 2m, 1h)".to_string(),
                }
            })?;
            if ms == 0 {
                return Err(ScheduleError::InvalidInterval(
                    "@every duration must be > 0".into(),
                ));
            }
            return Ok(Self {
                seconds: vec![0],
                minutes: vec![0],
                hours: vec![0],
                days_of_month: vec![1],
                months: vec![1],
                days_of_week: vec![0],
                interval_ms_override: Some(ms),
            });
        }

        let normalized = match trimmed {
            "@yearly" | "@annually" => "0 0 0 1 1 *",
            "@monthly" => "0 0 0 1 * *",
            "@weekly" => "0 0 0 * * 0",
            "@daily" | "@midnight" => "0 0 0 * * *",
            "@hourly" => "0 0 * * * *",
            "@minutely" => "0 * * * * *",
            "@secondly" => "* * * * * *",
            other => other,
        };

        let fields: Vec<&str> = normalized.split_whitespace().collect();
        let (sec_str, min_str, hour_str, dom_str, month_str, dow_str) = match fields.len() {
            5 => ("0", fields[0], fields[1], fields[2], fields[3], fields[4]),
            6 => (
                fields[0], fields[1], fields[2], fields[3], fields[4], fields[5],
            ),
            _ => {
                return Err(ScheduleError::InvalidCronExpression {
                    expression: expression.to_string(),
                    reason: format!("expected 5 or 6 fields, found {}", fields.len()),
                });
            }
        };

        Ok(Self {
            seconds: parse_cron_field(sec_str, 0, 59, false, expression)?,
            minutes: parse_cron_field(min_str, 0, 59, false, expression)?,
            hours: parse_cron_field(hour_str, 0, 23, false, expression)?,
            days_of_month: parse_cron_field(dom_str, 1, 31, false, expression)?,
            months: parse_cron_field(month_str, 1, 12, false, expression)?,
            days_of_week: parse_cron_field(dow_str, 0, 7, true, expression)?,
            interval_ms_override: None,
        })
    }

    /// Computes the strictly next execution timestamp after `after`.
    pub fn next_after(&self, after: DateTime<Utc>) -> Result<DateTime<Utc>, ScheduleError> {
        if let Some(ms) = self.interval_ms_override {
            return add_milliseconds(after, ms.max(1));
        }

        let base = add_duration(after, Duration::seconds(1))?;
        let mut cur = make_utc(
            base.year(),
            base.month(),
            base.day(),
            base.hour(),
            base.minute(),
            base.second(),
            &after,
        )?;
        let max_year = cur.year() + 5;

        while cur.year() <= max_year {
            if !self.months.contains(&cur.month()) {
                let (ny, nm) = if cur.month() == 12 {
                    (cur.year() + 1, 1)
                } else {
                    (cur.year(), cur.month() + 1)
                };
                cur = make_utc(ny, nm, 1, 0, 0, 0, &after)?;
                continue;
            }
            let dow = cur.weekday().num_days_from_sunday();
            if !self.days_of_month.contains(&cur.day()) || !self.days_of_week.contains(&dow) {
                let nd = add_duration(cur, Duration::days(1))?;
                cur = make_utc(nd.year(), nd.month(), nd.day(), 0, 0, 0, &after)?;
                continue;
            }
            if !self.hours.contains(&cur.hour()) {
                let nh = add_duration(cur, Duration::hours(1))?;
                cur = make_utc(nh.year(), nh.month(), nh.day(), nh.hour(), 0, 0, &after)?;
                continue;
            }
            if !self.minutes.contains(&cur.minute()) {
                let nmin = add_duration(cur, Duration::minutes(1))?;
                cur = make_utc(
                    nmin.year(),
                    nmin.month(),
                    nmin.day(),
                    nmin.hour(),
                    nmin.minute(),
                    0,
                    &after,
                )?;
                continue;
            }
            if !self.seconds.contains(&cur.second()) {
                if let Some(&next_sec) = self.seconds.iter().find(|&&s| s > cur.second()) {
                    cur = make_utc(
                        cur.year(),
                        cur.month(),
                        cur.day(),
                        cur.hour(),
                        cur.minute(),
                        next_sec,
                        &after,
                    )?;
                } else {
                    let nmin = add_duration(cur, Duration::minutes(1))?;
                    cur = make_utc(
                        nmin.year(),
                        nmin.month(),
                        nmin.day(),
                        nmin.hour(),
                        nmin.minute(),
                        0,
                        &after,
                    )?;
                }
                continue;
            }
            return Ok(cur);
        }
        Err(ScheduleError::NoNextOccurrence(after.to_rfc3339()))
    }
}

fn add_duration(reference: DateTime<Utc>, delta: Duration) -> Result<DateTime<Utc>, ScheduleError> {
    reference
        .checked_add_signed(delta)
        .ok_or_else(|| ScheduleError::NoNextOccurrence(reference.to_rfc3339()))
}

fn add_milliseconds(
    reference: DateTime<Utc>,
    milliseconds: u64,
) -> Result<DateTime<Utc>, ScheduleError> {
    let milliseconds = i64::try_from(milliseconds)
        .map_err(|_| ScheduleError::NoNextOccurrence(reference.to_rfc3339()))?;
    add_duration(reference, Duration::milliseconds(milliseconds))
}

fn make_utc(
    y: i32,
    m: u32,
    d: u32,
    h: u32,
    min: u32,
    s: u32,
    r: &DateTime<Utc>,
) -> Result<DateTime<Utc>, ScheduleError> {
    Utc.with_ymd_and_hms(y, m, d, h, min, s)
        .single()
        .ok_or_else(|| ScheduleError::NoNextOccurrence(r.to_rfc3339()))
}

fn parse_duration_ms(raw: &str) -> Option<u64> {
    if let Some(v) = raw.strip_suffix("ms") {
        v.trim().parse::<u64>().ok()
    } else if let Some(v) = raw.strip_suffix('s') {
        v.trim()
            .parse::<u64>()
            .ok()
            .map(|s| s.saturating_mul(1_000))
    } else if let Some(v) = raw.strip_suffix('m') {
        v.trim()
            .parse::<u64>()
            .ok()
            .map(|m| m.saturating_mul(60_000))
    } else if let Some(v) = raw.strip_suffix('h') {
        v.trim()
            .parse::<u64>()
            .ok()
            .map(|h| h.saturating_mul(3_600_000))
    } else {
        None
    }
}

fn parse_cron_field(
    field: &str,
    min: u32,
    max: u32,
    norm_sun: bool,
    expr: &str,
) -> Result<Vec<u32>, ScheduleError> {
    let mut values = Vec::new();
    for part in field.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(ScheduleError::InvalidCronExpression {
                expression: expr.into(),
                reason: "empty segment".into(),
            });
        }
        let (range_part, step) = if let Some((r, s)) = part.split_once('/') {
            let st = s
                .parse::<u32>()
                .map_err(|_| ScheduleError::InvalidCronExpression {
                    expression: expr.into(),
                    reason: format!("invalid step '{s}'"),
                })?;
            if st == 0 {
                return Err(ScheduleError::InvalidCronExpression {
                    expression: expr.into(),
                    reason: "step cannot be 0".into(),
                });
            }
            (r, st)
        } else {
            (part, 1)
        };

        let (start, end) = if range_part == "*" {
            (min, max)
        } else if let Some((a, b)) = range_part.split_once('-') {
            let s = a
                .parse::<u32>()
                .map_err(|_| ScheduleError::InvalidCronExpression {
                    expression: expr.into(),
                    reason: format!("invalid start '{a}'"),
                })?;
            let e = b
                .parse::<u32>()
                .map_err(|_| ScheduleError::InvalidCronExpression {
                    expression: expr.into(),
                    reason: format!("invalid end '{b}'"),
                })?;
            if s < min || e > max || s > e {
                return Err(ScheduleError::InvalidCronExpression {
                    expression: expr.into(),
                    reason: format!("range {s}-{e} out of [{min},{max}]"),
                });
            }
            (s, e)
        } else {
            let val =
                range_part
                    .parse::<u32>()
                    .map_err(|_| ScheduleError::InvalidCronExpression {
                        expression: expr.into(),
                        reason: format!("invalid value '{range_part}'"),
                    })?;
            if val < min || val > max {
                return Err(ScheduleError::InvalidCronExpression {
                    expression: expr.into(),
                    reason: format!("value {val} out of [{min},{max}]"),
                });
            }
            if part.contains('/') {
                (val, max)
            } else {
                (val, val)
            }
        };

        let mut v = start;
        while v <= end {
            let n = if norm_sun && v == 7 { 0 } else { v };
            if !values.contains(&n) {
                values.push(n);
            }
            v = v.saturating_add(step);
        }
    }
    values.sort_unstable();
    Ok(values)
}

/// Unified schedule specification for Delayed Task Queue and Recurring Cron / Interval tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduleSpec {
    OnceAt {
        run_after: DateTime<Utc>,
    },
    Delayed {
        delay_ms: u64,
        run_after: DateTime<Utc>,
    },
    Interval {
        interval_ms: u64,
    },
    Cron {
        expression: String,
    },
}

impl ScheduleSpec {
    pub fn once_at(run_after: DateTime<Utc>) -> Self {
        Self::OnceAt { run_after }
    }

    pub fn after_delay(delay: std::time::Duration) -> Self {
        Self::after_delay_from(Utc::now(), delay)
    }

    pub fn after_delay_from(now: DateTime<Utc>, delay: std::time::Duration) -> Self {
        let delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
        let delta = Duration::milliseconds(i64::try_from(delay_ms).unwrap_or(i64::MAX));
        let run_after = now
            .checked_add_signed(delta)
            .unwrap_or(DateTime::<Utc>::MAX_UTC);
        Self::Delayed {
            delay_ms,
            run_after,
        }
    }

    pub fn every(interval: std::time::Duration) -> Self {
        let ms = u64::try_from(interval.as_millis())
            .unwrap_or(u64::MAX)
            .max(1);
        Self::Interval { interval_ms: ms }
    }

    pub fn every_seconds(secs: u64) -> Self {
        Self::Interval {
            interval_ms: secs.max(1).saturating_mul(1_000),
        }
    }

    pub fn cron(expression: impl Into<String>) -> Result<Self, ScheduleError> {
        let expr = expression.into();
        let _ = CronSchedule::parse(&expr)?;
        Ok(Self::Cron { expression: expr })
    }

    pub fn is_recurring(&self) -> bool {
        matches!(self, Self::Interval { .. } | Self::Cron { .. })
    }

    pub fn initial_run_at(&self, now: DateTime<Utc>) -> Result<DateTime<Utc>, ScheduleError> {
        match self {
            Self::OnceAt { run_after } | Self::Delayed { run_after, .. } => Ok(*run_after),
            Self::Interval { interval_ms } => {
                if *interval_ms == 0 {
                    return Err(ScheduleError::InvalidInterval(
                        "interval_ms must be > 0".into(),
                    ));
                }
                add_milliseconds(now, *interval_ms)
            }
            Self::Cron { expression } => CronSchedule::parse(expression)?.next_after(now),
        }
    }

    pub fn next_run_after(
        &self,
        reference: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>, ScheduleError> {
        match self {
            Self::OnceAt { run_after } | Self::Delayed { run_after, .. } => {
                Ok((reference < *run_after).then_some(*run_after))
            }
            Self::Interval { interval_ms } => {
                if *interval_ms == 0 {
                    return Err(ScheduleError::InvalidInterval(
                        "interval_ms must be > 0".into(),
                    ));
                }
                Ok(Some(add_milliseconds(reference, *interval_ms)?))
            }
            Self::Cron { expression } => Ok(Some(
                CronSchedule::parse(expression)?.next_after(reference)?,
            )),
        }
    }
}

/// Persistent schedule entry stored in `task_store`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskScheduleRecord {
    pub schedule_id: String,
    pub task_id: Option<TaskId>,
    pub title: String,
    pub goal_description: String,
    pub priority: Priority,
    pub spec: ScheduleSpec,
    pub next_run_at: DateTime<Utc>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_spawned_task_id: Option<TaskId>,
    pub run_count: u64,
    pub max_runs: Option<u64>,
    pub enabled: bool,
    pub engine_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TaskScheduleRecord {
    pub fn new(
        title: &str,
        goal: &str,
        priority: Priority,
        spec: ScheduleSpec,
        now: DateTime<Utc>,
    ) -> Result<Self, ScheduleError> {
        let next_run_at = spec.initial_run_at(now)?;
        Ok(Self {
            schedule_id: format!("SCHED-{}", Uuid::new_v4().simple()),
            task_id: None,
            title: title.to_string(),
            goal_description: goal.to_string(),
            priority,
            spec,
            next_run_at,
            last_run_at: None,
            last_spawned_task_id: None,
            run_count: 0,
            max_runs: None,
            enabled: true,
            engine_name: None,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn with_task_id(mut self, task_id: TaskId) -> Self {
        self.schedule_id = task_id.0.clone();
        self.task_id = Some(task_id);
        self
    }

    pub fn with_max_runs(mut self, max_runs: u64) -> Self {
        self.max_runs = Some(max_runs.max(1));
        self
    }

    pub fn with_engine_name(mut self, engine_name: impl Into<String>) -> Self {
        self.engine_name = Some(engine_name.into());
        self
    }

    pub fn is_due(&self, now: DateTime<Utc>) -> bool {
        self.enabled && self.next_run_at <= now
    }

    pub fn advance_after_trigger(
        &mut self,
        triggered_at: DateTime<Utc>,
        spawned_task_id: TaskId,
    ) -> Result<Option<DateTime<Utc>>, ScheduleError> {
        self.run_count = self.run_count.saturating_add(1);
        self.last_run_at = Some(triggered_at);
        self.last_spawned_task_id = Some(spawned_task_id);
        self.updated_at = triggered_at;
        let reached_limit = self.max_runs.is_some_and(|max| self.run_count >= max);
        if reached_limit || !self.spec.is_recurring() {
            self.enabled = false;
            return Ok(None);
        }
        match self.spec.next_run_after(triggered_at)? {
            Some(next) => {
                self.next_run_at = next;
                Ok(Some(next))
            }
            None => {
                self.enabled = false;
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32, sec: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, sec)
            .single()
            .expect("valid schedule test fixture")
    }

    fn record(spec: ScheduleSpec, now: DateTime<Utc>) -> TaskScheduleRecord {
        TaskScheduleRecord::new("title", "goal", Priority::Normal, spec, now)
            .expect("valid schedule test fixture")
    }

    #[test]
    fn cron_parse_accepts_five_field_expression() {
        assert!(CronSchedule::parse("*/5 * * * *").is_ok());
        assert!(CronSchedule::parse("0 */2 * * * *").is_ok());
        assert!(CronSchedule::parse("@daily").is_ok());
        assert!(CronSchedule::parse("@every 5s").is_ok());

        for invalid in [
            "",
            "* * *",
            "* * * * * * *",
            "*/0 * * * *",
            "60 * * * *",
            "0 24 * * *",
            "0 0 32 * *",
            "0 0 0 0 *",
            "0 0 0 * 13",
            "0 0 0 * * 8",
            "@every 5fortnights",
        ] {
            assert!(
                CronSchedule::parse(invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
        assert!(matches!(
            CronSchedule::parse("@every 0s"),
            Err(ScheduleError::InvalidInterval(_))
        ));
        assert!(matches!(
            CronSchedule::parse("@every 0ms"),
            Err(ScheduleError::InvalidInterval(_))
        ));
    }

    #[test]
    fn cron_next_after_is_strict_and_handles_every_and_exhaustion() {
        let start = at(2026, 11, 7, 9, 30, 0);
        let cron = CronSchedule::parse("0 30 9 * * *").expect("valid schedule test fixture");
        assert_eq!(
            cron.next_after(start).expect("valid schedule test fixture"),
            at(2026, 11, 8, 9, 30, 0)
        );

        let every = CronSchedule::parse("@every 5s").expect("valid schedule test fixture");
        assert_eq!(
            every
                .next_after(start)
                .expect("valid schedule test fixture"),
            at(2026, 11, 7, 9, 30, 5)
        );

        let secondly = CronSchedule::parse("* * * * * *").expect("valid schedule test fixture");
        assert_eq!(
            secondly
                .next_after(start + Duration::milliseconds(500))
                .expect("valid schedule test fixture"),
            start + Duration::seconds(1)
        );

        let impossible = CronSchedule::parse("0 0 0 31 2 *").expect("valid schedule test fixture");
        assert!(matches!(
            impossible.next_after(start),
            Err(ScheduleError::NoNextOccurrence(_))
        ));
    }

    #[test]
    fn once_at_keeps_the_requested_timestamp() {
        let run_after = at(2026, 12, 1, 12, 0, 0);
        assert_eq!(
            ScheduleSpec::once_at(run_after),
            ScheduleSpec::OnceAt { run_after }
        );
    }

    #[test]
    fn after_delay_sets_run_time_near_construction_time() {
        let before = Utc::now();
        let delay = std::time::Duration::from_millis(2_500);
        let spec = ScheduleSpec::after_delay(delay);
        let after = Utc::now();
        assert!(matches!(
            spec,
            ScheduleSpec::Delayed {
                delay_ms: 2_500,
                run_after,
            } if run_after >= before + Duration::milliseconds(2_500)
                && run_after <= after + Duration::milliseconds(2_500)
        ));
    }

    #[test]
    fn after_delay_saturates_unrepresentable_delay() {
        assert_eq!(
            ScheduleSpec::after_delay(std::time::Duration::MAX),
            ScheduleSpec::Delayed {
                delay_ms: u64::MAX,
                run_after: DateTime::<Utc>::MAX_UTC,
            }
        );
    }

    #[test]
    fn after_delay_from_uses_fixed_time_and_millisecond_precision() {
        let now = at(2026, 11, 7, 9, 30, 0);
        assert_eq!(
            ScheduleSpec::after_delay_from(now, std::time::Duration::from_millis(1_234)),
            ScheduleSpec::Delayed {
                delay_ms: 1_234,
                run_after: now + Duration::milliseconds(1_234),
            }
        );

        assert_eq!(
            ScheduleSpec::after_delay_from(now, std::time::Duration::from_secs(u64::MAX)),
            ScheduleSpec::Delayed {
                delay_ms: u64::MAX,
                run_after: DateTime::<Utc>::MAX_UTC,
            }
        );

        assert_eq!(
            ScheduleSpec::after_delay_from(now, std::time::Duration::ZERO),
            ScheduleSpec::Delayed {
                delay_ms: 0,
                run_after: now,
            }
        );
    }

    #[test]
    fn every_seconds_converts_to_milliseconds_and_clamps_zero() {
        assert_eq!(
            ScheduleSpec::every_seconds(3),
            ScheduleSpec::Interval { interval_ms: 3_000 }
        );
        assert_eq!(
            ScheduleSpec::every_seconds(0),
            ScheduleSpec::Interval { interval_ms: 1_000 }
        );
        assert_eq!(
            ScheduleSpec::every_seconds(u64::MAX),
            ScheduleSpec::Interval {
                interval_ms: u64::MAX
            }
        );
    }

    #[test]
    fn is_recurring_matches_schedule_kind() {
        let now = at(2026, 11, 7, 9, 30, 0);
        assert!(!ScheduleSpec::once_at(now).is_recurring());
        assert!(!ScheduleSpec::after_delay_from(now, std::time::Duration::ZERO).is_recurring());
        assert!(ScheduleSpec::every_seconds(1).is_recurring());
        assert!(ScheduleSpec::cron("@daily")
            .expect("valid schedule test fixture")
            .is_recurring());
    }

    #[test]
    fn initial_run_at_uses_one_time_and_recurring_rules() {
        let now = at(2026, 11, 7, 9, 30, 0);
        let future = now + Duration::hours(1);
        assert_eq!(
            ScheduleSpec::once_at(future)
                .initial_run_at(now)
                .expect("valid schedule test fixture"),
            future
        );
        assert_eq!(
            ScheduleSpec::after_delay_from(now, std::time::Duration::from_secs(5))
                .initial_run_at(now)
                .expect("valid schedule test fixture"),
            now + Duration::seconds(5)
        );
        assert_eq!(
            ScheduleSpec::every_seconds(10)
                .initial_run_at(now)
                .expect("valid schedule test fixture"),
            now + Duration::seconds(10)
        );
        assert_eq!(
            ScheduleSpec::cron("0 0 10 * * *")
                .expect("valid schedule test fixture")
                .initial_run_at(now)
                .expect("valid schedule test fixture"),
            at(2026, 11, 7, 10, 0, 0)
        );
        assert!(matches!(
            ScheduleSpec::Interval { interval_ms: 0 }.initial_run_at(now),
            Err(ScheduleError::InvalidInterval(_))
        ));
    }

    #[test]
    fn next_run_after_observes_boundaries_and_rejects_zero_interval() {
        let now = at(2026, 11, 7, 9, 30, 0);
        let future = now + Duration::seconds(1);
        let once = ScheduleSpec::once_at(future);
        assert_eq!(
            once.next_run_after(now)
                .expect("valid schedule test fixture"),
            Some(future)
        );
        assert_eq!(
            once.next_run_after(future)
                .expect("valid schedule test fixture"),
            None
        );
        assert_eq!(
            once.next_run_after(future + Duration::seconds(1))
                .expect("valid schedule test fixture"),
            None
        );
        assert_eq!(
            ScheduleSpec::after_delay_from(now, std::time::Duration::from_secs(2))
                .next_run_after(now)
                .expect("valid schedule test fixture"),
            Some(now + Duration::seconds(2))
        );
        assert_eq!(
            ScheduleSpec::after_delay_from(now, std::time::Duration::ZERO)
                .next_run_after(now)
                .expect("valid schedule test fixture"),
            None
        );
        assert_eq!(
            ScheduleSpec::Interval { interval_ms: 500 }
                .next_run_after(now)
                .expect("valid schedule test fixture"),
            Some(now + Duration::milliseconds(500))
        );
        assert_eq!(
            ScheduleSpec::cron("0 0 10 * * *")
                .expect("valid schedule test fixture")
                .next_run_after(now)
                .expect("valid schedule test fixture"),
            Some(at(2026, 11, 7, 10, 0, 0))
        );
        assert!(matches!(
            ScheduleSpec::Interval { interval_ms: 0 }.next_run_after(now),
            Err(ScheduleError::InvalidInterval(_))
        ));
    }

    #[test]
    fn next_run_after_reports_timestamp_overflow_for_interval() {
        let schedule = ScheduleSpec::every_seconds(1);

        assert!(matches!(
            schedule.next_run_after(DateTime::<Utc>::MAX_UTC),
            Err(ScheduleError::NoNextOccurrence(_))
        ));
    }

    #[test]
    fn initial_run_at_reports_timestamp_overflow_for_interval() {
        let schedule = ScheduleSpec::every_seconds(1);

        assert!(matches!(
            schedule.initial_run_at(DateTime::<Utc>::MAX_UTC),
            Err(ScheduleError::NoNextOccurrence(_))
        ));
    }

    #[test]
    fn cron_every_reports_timestamp_overflow() {
        let cron = CronSchedule::parse("@every 1s").expect("valid schedule test fixture");

        assert!(matches!(
            cron.next_after(DateTime::<Utc>::MAX_UTC),
            Err(ScheduleError::NoNextOccurrence(_))
        ));
    }

    #[test]
    fn record_builders_set_task_id_limits_and_engine_name() {
        let now = at(2026, 11, 7, 9, 30, 0);
        let task_id = TaskId("TASK-fixed".into());
        let built = record(ScheduleSpec::every_seconds(60), now)
            .with_task_id(task_id.clone())
            .with_max_runs(0)
            .with_engine_name("engine-x");
        assert_eq!(built.task_id, Some(task_id.clone()));
        assert_eq!(built.schedule_id, task_id.0);
        assert_eq!(built.max_runs, Some(1));
        assert_eq!(built.engine_name.as_deref(), Some("engine-x"));
        assert_eq!(
            record(ScheduleSpec::once_at(now), now)
                .with_max_runs(4)
                .max_runs,
            Some(4)
        );
    }

    #[test]
    fn is_due_requires_enabled_and_next_run_not_after_now() {
        let now = at(2026, 11, 7, 9, 30, 0);
        let mut entry = record(ScheduleSpec::once_at(now), now);
        assert!(!entry.is_due(now - Duration::nanoseconds(1)));
        assert!(entry.is_due(now));
        assert!(entry.is_due(now + Duration::nanoseconds(1)));
        entry.enabled = false;
        assert!(!entry.is_due(now + Duration::days(1)));
    }

    #[test]
    fn advance_after_trigger_updates_run_state_and_stops_when_required() {
        let now = at(2026, 11, 7, 9, 30, 0);
        let triggered_at = now + Duration::seconds(30);
        let spawned_task_id = TaskId("TASK-spawned".into());

        let mut recurring = TaskScheduleRecord::new(
            "title",
            "goal",
            Priority::Normal,
            ScheduleSpec::every_seconds(60),
            now,
        )
        .expect("valid schedule test fixture")
        .with_max_runs(2);
        assert_eq!(
            recurring
                .advance_after_trigger(triggered_at, spawned_task_id.clone())
                .expect("valid schedule test fixture"),
            Some(triggered_at + Duration::seconds(60))
        );
        assert_eq!(recurring.run_count, 1);
        assert_eq!(recurring.last_run_at, Some(triggered_at));
        assert_eq!(
            recurring.last_spawned_task_id,
            Some(spawned_task_id.clone())
        );
        assert_eq!(recurring.updated_at, triggered_at);
        assert_eq!(recurring.next_run_at, triggered_at + Duration::seconds(60));
        assert!(recurring.enabled);

        assert_eq!(
            recurring
                .advance_after_trigger(
                    triggered_at + Duration::seconds(60),
                    spawned_task_id.clone()
                )
                .expect("valid schedule test fixture"),
            None
        );
        assert_eq!(recurring.run_count, 2);
        assert!(!recurring.enabled);

        let mut one_shot = TaskScheduleRecord::new(
            "title",
            "goal",
            Priority::Normal,
            ScheduleSpec::once_at(now),
            now,
        )
        .expect("valid schedule test fixture");
        assert_eq!(
            one_shot
                .advance_after_trigger(triggered_at, spawned_task_id)
                .expect("valid schedule test fixture"),
            None
        );
        assert_eq!(one_shot.run_count, 1);
        assert!(!one_shot.enabled);
    }
}
