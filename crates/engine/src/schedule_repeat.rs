//! When a repeating schedule ([`zeron_proto::Repeat`]) next fires, and what a
//! new one is checked against. Pure: nothing here touches the registry.
//!
//! Cron is `croner` (5 fields, no seconds or year), read as wall-clock time
//! in an IANA zone (`chrono-tz`). Its DST policy is the one the tool tells
//! agents: a fixed time that falls in a spring-forward gap fires just after
//! the gap that day, and one that happens twice in a fall-back fires once (at
//! the first). Hour- or minute-wildcard and step patterns (`*`, `*/15`)
//! instead skip occurrences inside a gap and fire on every occurrence in a
//! repeated hour.
//!
//! `every_n_weeks` / `every_n_days` keep every Nth week (Monday to Sunday) or
//! day of the cron's matches, counted from `anchor_date`: the first day the
//! cron matches on or after `start_date` (default: the day it was made),
//! fixed when the schedule is created so the phase never moves.

use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use croner::Cron;
use croner::parser::{CronParser, Seconds, Year};
use serde::Deserialize;
use zeron_proto::Repeat;

/// `every_seconds` is at least this (a minute).
pub const MIN_EVERY_SECONDS: u64 = 60;
/// ... and at most a year.
pub const MAX_EVERY_SECONDS: u64 = 366 * 24 * 3600;
/// The most sends a `max_runs` may ask for.
pub const MAX_RUNS_LIMIT: u32 = 10_000;
/// The largest `every_n_weeks` / `every_n_days`.
pub const MAX_EVERY_N_WEEKS: u32 = 52;
pub const MAX_EVERY_N_DAYS: u32 = 366;
/// Pending repeating schedules one chat may have as a target.
pub const MAX_RECURRING_PER_CHAT: usize = 25;
/// ... and one agent chat may have made.
pub const MAX_RECURRING_PER_AGENT: usize = 10;

/// How a new schedule asks to repeat: the same names as the tool's arguments.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepeatSpec {
    pub cron: Option<String>,
    pub timezone: Option<String>,
    pub every_n_weeks: Option<u32>,
    pub every_n_days: Option<u32>,
    pub start_date: Option<String>,
    pub every_seconds: Option<u64>,
}

/// A new schedule, checked: its repeat rule (with the anchor fixed) and its
/// first fire, strictly after the time it was planned at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub repeat: Option<Repeat>,
    pub first: DateTime<Utc>,
}

fn parse_cron(expr: &str) -> Result<Cron, String> {
    CronParser::builder()
        .seconds(Seconds::Disallowed)
        .year(Year::Disallowed)
        .build()
        .parse(expr)
        .map_err(|e| format!("'cron' {expr:?} isn't a valid 5-field cron expression: {e}"))
}

fn parse_zone(name: &str) -> Result<Tz, String> {
    name.parse::<Tz>()
        .map_err(|_| format!("'timezone' {name:?} isn't an IANA zone name (like America/New_York)"))
}

fn parse_date(text: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
        .map_err(|_| format!("'start_date' {text:?} isn't a local YYYY-MM-DD date"))
}

/// The first instant of local day `date` in `tz` (just after the gap when
/// midnight doesn't exist that day).
fn day_start(tz: Tz, date: NaiveDate) -> DateTime<Utc> {
    let midnight = date.and_hms_opt(0, 0, 0).unwrap_or_default();
    let found = match tz.from_local_datetime(&midnight) {
        LocalResult::Single(t) => Some(t),
        LocalResult::Ambiguous(first, _) => Some(first),
        LocalResult::None => (1..=180).find_map(|m| {
            tz.from_local_datetime(&(midnight + Duration::minutes(m)))
                .earliest()
        }),
    };
    found.map_or_else(
        || Utc.from_utc_datetime(&midnight),
        |t| t.with_timezone(&Utc),
    )
}

fn monday_of(date: NaiveDate) -> NaiveDate {
    date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
}

/// Whether `date` is one of the every-Nth days/weeks counted from `anchor`.
fn qualifies(date: NaiveDate, anchor: NaiveDate, weeks: Option<u32>, days: Option<u32>) -> bool {
    next_qualifying(date, anchor, weeks, days) == date
}

/// The first day on or after `date` that qualifies.
fn next_qualifying(
    date: NaiveDate,
    anchor: NaiveDate,
    weeks: Option<u32>,
    days: Option<u32>,
) -> NaiveDate {
    if let Some(n) = days.filter(|&n| n >= 2) {
        let rem = (date - anchor).num_days().rem_euclid(i64::from(n));
        return if rem == 0 {
            date
        } else {
            date + Duration::days(i64::from(n) - rem)
        };
    }
    if let Some(n) = weeks.filter(|&n| n >= 2) {
        let week = (monday_of(date) - monday_of(anchor))
            .num_days()
            .div_euclid(7);
        let rem = week.rem_euclid(i64::from(n));
        return if rem == 0 {
            date
        } else {
            monday_of(date) + Duration::days(7 * (i64::from(n) - rem))
        };
    }
    date
}

/// The local date of the first cron match on or after `start`.
fn anchor_from(cron: &Cron, tz: Tz, start: NaiveDate) -> Option<NaiveDate> {
    let from = day_start(tz, start).with_timezone(&tz);
    cron.find_next_occurrence(&from, true)
        .ok()
        .map(|occ| occ.date_naive())
}

/// The next cron occurrence strictly after `after` that is on or after
/// `start_date` and one of the every-Nth days or weeks.
fn cron_next(cron: &Cron, tz: Tz, repeat: &Repeat, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let Repeat::Cron {
        every_n_weeks,
        every_n_days,
        start_date,
        anchor_date,
        ..
    } = repeat
    else {
        return None;
    };
    let anchor = NaiveDate::parse_from_str(anchor_date, "%Y-%m-%d").ok()?;
    let floor = start_date
        .as_deref()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .map(|d| day_start(tz, d));
    let (mut from, mut inclusive) = (after, false);
    if let Some(floor) = floor.filter(|f| *f > after) {
        (from, inclusive) = (floor, true);
    }
    // Skipping to the next qualifying day makes this a few steps; the bound
    // is for a rule that can never match (every 2nd day of a weekly cron).
    for _ in 0..64 {
        let occ = cron
            .find_next_occurrence(&from.with_timezone(&tz), inclusive)
            .ok()?;
        let date = occ.date_naive();
        if qualifies(date, anchor, *every_n_weeks, *every_n_days) {
            return Some(occ.with_timezone(&Utc));
        }
        from = day_start(
            tz,
            next_qualifying(date, anchor, *every_n_weeks, *every_n_days),
        );
        inclusive = true;
    }
    None
}

/// The first fire of `repeat` strictly after `after`, whose last fire (or
/// first) was due at `due`. `None` when it never fires again.
pub fn next_after(
    repeat: &Repeat,
    due: DateTime<Utc>,
    after: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    match repeat {
        Repeat::Every { seconds } => {
            let step = i64::try_from(*seconds).ok().filter(|s| *s > 0)?;
            if after < due {
                return Some(due);
            }
            let k = (after - due).num_seconds().div_euclid(step) + 1;
            due.checked_add_signed(Duration::seconds(k.checked_mul(step)?))
        }
        Repeat::Cron { expr, timezone, .. } => {
            let cron = parse_cron(expr).ok()?;
            cron_next(&cron, parse_zone(timezone).ok()?, repeat, after)
        }
    }
}

/// When a schedule next fires after it fires at `now`: `None` ends it (its
/// last run: `max_runs` reached, or nothing before `until`).
pub fn next_run(schedule: &zeron_proto::Schedule, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let repeat = schedule.repeat.as_ref()?;
    if schedule
        .max_runs
        .is_some_and(|max| schedule.runs + 1 >= max)
    {
        return None;
    }
    next_within(repeat, schedule, now)
}

/// When a repeating schedule runs next after skipping an occurrence as too
/// late at `now` (no run is counted).
pub fn next_skipping(
    schedule: &zeron_proto::Schedule,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    next_within(schedule.repeat.as_ref()?, schedule, now)
}

fn next_within(
    repeat: &Repeat,
    schedule: &zeron_proto::Schedule,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    next_after(repeat, schedule.fire_at, now)
        .filter(|next| schedule.until.is_none_or(|until| *next <= until))
}

/// Up to `n` fires from `first`, honouring `until` and `max_runs`.
pub fn upcoming(
    repeat: Option<&Repeat>,
    first: DateTime<Utc>,
    until: Option<DateTime<Utc>>,
    max_runs: Option<u32>,
    n: usize,
) -> Vec<DateTime<Utc>> {
    let mut fires = vec![first];
    let Some(repeat) = repeat else { return fires };
    while fires.len() < n && max_runs.is_none_or(|max| (fires.len() as u64) < u64::from(max)) {
        let last = *fires.last().unwrap_or(&first);
        match next_after(repeat, last, last).filter(|next| until.is_none_or(|u| *next <= u)) {
            Some(next) => fires.push(next),
            None => break,
        }
    }
    fires
}

/// Check a new schedule made at `now` and fix its repeat rule and first fire.
/// `fire_at` is the `at`/`in_seconds` time (a one-shot's due time, or the
/// first fire of `every_seconds`); a cron works its own out.
pub fn plan(
    spec: Option<&RepeatSpec>,
    fire_at: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
    max_runs: Option<u32>,
    now: DateTime<Utc>,
) -> Result<Plan, String> {
    let Some(spec) = spec else {
        if until.is_some() || max_runs.is_some() {
            return Err("'until' and 'max_runs' are for recurring schedules (give 'cron' or 'every_seconds')".into());
        }
        let first = fire_at.ok_or("give one of 'at', 'in_seconds', 'cron' or 'every_seconds'")?;
        if first <= now {
            return Err("that time has already passed".into());
        }
        return Ok(Plan {
            repeat: None,
            first,
        });
    };
    if let Some(max) = max_runs
        && !(1..=MAX_RUNS_LIMIT).contains(&max)
    {
        return Err(format!("'max_runs' is 1 to {MAX_RUNS_LIMIT}"));
    }
    let (repeat, first) = match (&spec.cron, spec.every_seconds) {
        (Some(_), Some(_)) => return Err("give either 'cron' or 'every_seconds', not both".into()),
        (None, None) => return Err("a recurring schedule needs 'cron' or 'every_seconds'".into()),
        (None, Some(seconds)) => {
            if spec.timezone.is_some()
                || spec.every_n_weeks.is_some()
                || spec.every_n_days.is_some()
                || spec.start_date.is_some()
            {
                return Err("'timezone', 'every_n_weeks', 'every_n_days' and 'start_date' go with 'cron', not 'every_seconds'".into());
            }
            if !(MIN_EVERY_SECONDS..=MAX_EVERY_SECONDS).contains(&seconds) {
                return Err(format!(
                    "'every_seconds' is {MIN_EVERY_SECONDS} to {MAX_EVERY_SECONDS}"
                ));
            }
            let first = match fire_at {
                Some(first) if first <= now => {
                    return Err("the first fire ('at'/'in_seconds') must be in the future".into());
                }
                Some(first) => first,
                None => now + Duration::seconds(i64::try_from(seconds).unwrap_or(i64::MAX / 2)),
            };
            (Repeat::Every { seconds }, first)
        }
        (Some(expr), None) => {
            if fire_at.is_some() {
                return Err("a 'cron' works out its own first fire: leave out 'at' and 'in_seconds' (use 'start_date' to begin later)".into());
            }
            let cron = parse_cron(expr)?;
            let timezone = spec
                .timezone
                .as_deref()
                .ok_or("'timezone' is required with 'cron' (an IANA name like America/New_York)")?;
            let tz = parse_zone(timezone)?;
            if spec.every_n_weeks.is_some() && spec.every_n_days.is_some() {
                return Err("give 'every_n_weeks' or 'every_n_days', not both".into());
            }
            if let Some(n) = spec.every_n_weeks
                && !(2..=MAX_EVERY_N_WEEKS).contains(&n)
            {
                return Err(format!(
                    "'every_n_weeks' is 2 to {MAX_EVERY_N_WEEKS} (for every week, leave it out)"
                ));
            }
            if let Some(n) = spec.every_n_days
                && !(2..=MAX_EVERY_N_DAYS).contains(&n)
            {
                return Err(format!(
                    "'every_n_days' is 2 to {MAX_EVERY_N_DAYS} (for every day, leave it out)"
                ));
            }
            let start = match spec.start_date.as_deref() {
                Some(text) => parse_date(text)?,
                None => now.with_timezone(&tz).date_naive(),
            };
            let anchor = anchor_from(&cron, tz, start)
                .ok_or_else(|| format!("'cron' {expr:?} never matches"))?;
            let repeat = Repeat::Cron {
                expr: expr.trim().to_owned(),
                timezone: timezone.to_owned(),
                every_n_weeks: spec.every_n_weeks,
                every_n_days: spec.every_n_days,
                start_date: spec.start_date.as_deref().map(|d| d.trim().to_owned()),
                anchor_date: anchor.format("%Y-%m-%d").to_string(),
            };
            let first = cron_next(&cron, tz, &repeat, now)
                .ok_or("that cron never fires from its start date")?;
            (repeat, first)
        }
    };
    if first <= now {
        return Err("the first fire must be in the future".into());
    }
    if let Some(until) = until
        && until < first
    {
        return Err(format!(
            "'until' is before the first fire ({})",
            first.to_rfc3339()
        ));
    }
    Ok(Plan {
        repeat: Some(repeat),
        first,
    })
}

/// `Thu 2026-11-12 09:00 EST (14:00 UTC)` in `zone` (UTC when none).
pub fn format_fire(at: DateTime<Utc>, zone: Option<&str>) -> String {
    match zone.and_then(|z| z.parse::<Tz>().ok()) {
        Some(tz) => {
            let local = at.with_timezone(&tz);
            let utc = if local.date_naive() == at.date_naive() {
                at.format("%H:%M").to_string()
            } else {
                at.format("%Y-%m-%d %H:%M").to_string()
            };
            format!(
                "{} {} ({utc} UTC)",
                local.format("%a %Y-%m-%d %H:%M"),
                local.format("%Z")
            )
        }
        None => format!("{} UTC", at.format("%a %Y-%m-%d %H:%M")),
    }
}

/// The zone a repeat's times are shown in (none for a fixed interval).
pub fn zone_of(repeat: Option<&Repeat>) -> Option<&str> {
    match repeat {
        Some(Repeat::Cron { timezone, .. }) => Some(timezone),
        _ => None,
    }
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn duration_words(seconds: u64) -> String {
    let parts = [
        (86_400, "day"),
        (3_600, "hour"),
        (60, "minute"),
        (1, "second"),
    ];
    let mut left = seconds;
    let mut out = Vec::new();
    for (unit, name) in parts {
        if left >= unit {
            let n = left / unit;
            left %= unit;
            out.push(format!("{n} {name}{}", if n == 1 { "" } else { "s" }));
        }
    }
    out.join(" ")
}

/// A repeat in plain words, for the tool's answer.
pub fn describe(
    repeat: Option<&Repeat>,
    until: Option<DateTime<Utc>>,
    max_runs: Option<u32>,
) -> String {
    let Some(repeat) = repeat else {
        return "once".into();
    };
    let mut words = match repeat {
        Repeat::Every { seconds } => format!(
            "every {} of real elapsed time (drifts against the wall clock across DST)",
            duration_words(*seconds)
        ),
        Repeat::Cron {
            expr,
            timezone,
            every_n_weeks,
            every_n_days,
            start_date,
            anchor_date,
        } => {
            let what = parse_cron(expr).map_or_else(
                |_| expr.clone(),
                |c| c.describe().trim_end_matches('.').to_owned(),
            );
            let mut words = format!("{what} ({expr}), {timezone} time");
            if let Some(n) = every_n_weeks {
                words += &format!(
                    ", only every {} week (Monday-Sunday weeks, counted from the week of {anchor_date})",
                    ordinal(*n)
                );
            }
            if let Some(n) = every_n_days {
                words += &format!(
                    ", only every {} day (counted from {anchor_date})",
                    ordinal(*n)
                );
            }
            if let Some(start) = start_date {
                words += &format!(", not before {start}");
            }
            words
        }
    };
    if let Some(until) = until {
        words += &format!(", until {}", format_fire(until, zone_of(Some(repeat))));
    }
    if let Some(max) = max_runs {
        words += &format!(", at most {max} time{}", if max == 1 { "" } else { "s" });
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn cron(expr: &str, zone: &str) -> RepeatSpec {
        RepeatSpec {
            cron: Some(expr.into()),
            timezone: Some(zone.into()),
            ..Default::default()
        }
    }

    fn planned(spec: &RepeatSpec, now: &str) -> Plan {
        plan(Some(spec), None, None, None, utc(now)).unwrap()
    }

    /// The next `n` fires of a planned schedule, as RFC 3339 UTC.
    fn fires(spec: &RepeatSpec, now: &str, n: usize) -> Vec<String> {
        let p = planned(spec, now);
        upcoming(p.repeat.as_ref(), p.first, None, None, n)
            .into_iter()
            .map(|t| t.format("%Y-%m-%dT%H:%MZ").to_string())
            .collect()
    }

    fn refused(
        spec: Option<&RepeatSpec>,
        fire_at: Option<&str>,
        until: Option<&str>,
        max: Option<u32>,
    ) -> String {
        plan(
            spec,
            fire_at.map(utc),
            until.map(utc),
            max,
            utc("2026-11-11T12:00:00Z"),
        )
        .unwrap_err()
    }

    #[test]
    fn a_time_in_a_spring_forward_gap_fires_just_after_it_that_day() {
        // New York, 2026-03-08: 02:00 EST jumps to 03:00 EDT, so 02:30 isn't a time.
        let got = fires(
            &cron("30 2 * * *", "America/New_York"),
            "2026-03-07T00:00:00Z",
            3,
        );
        assert_eq!(
            got,
            [
                "2026-03-07T07:30Z",
                "2026-03-08T07:00Z",
                "2026-03-09T06:30Z"
            ]
        );
    }

    #[test]
    fn a_time_that_happens_twice_in_a_fall_back_fires_once_at_the_first() {
        // New York, 2026-11-01: 02:00 EDT falls back to 01:00 EST, so 01:30 happens twice.
        let got = fires(
            &cron("30 1 * * *", "America/New_York"),
            "2026-10-31T12:00:00Z",
            3,
        );
        assert_eq!(
            got,
            [
                "2026-11-01T05:30Z",
                "2026-11-02T06:30Z",
                "2026-11-03T06:30Z"
            ]
        );
    }

    #[test]
    fn hourly_and_step_patterns_follow_each_hour_not_the_fixed_time_rule() {
        // In the repeated hour every occurrence fires: 01:00 EDT and 01:00 EST.
        let got = fires(
            &cron("0 * * * *", "America/New_York"),
            "2026-11-01T03:30:00Z",
            4,
        );
        assert_eq!(
            got,
            [
                "2026-11-01T04:00Z",
                "2026-11-01T05:00Z",
                "2026-11-01T06:00Z",
                "2026-11-01T07:00Z"
            ]
        );
        // In the gap, the missing hour is skipped.
        let got = fires(
            &cron("0 * * * *", "America/New_York"),
            "2026-03-08T05:30:00Z",
            3,
        );
        assert_eq!(
            got,
            [
                "2026-03-08T06:00Z",
                "2026-03-08T07:00Z",
                "2026-03-08T08:00Z"
            ]
        );
    }

    #[test]
    fn the_last_day_of_the_month_and_nth_weekdays() {
        let spec = cron("0 9 L * *", "UTC");
        assert_eq!(
            fires(&spec, "2026-01-15T00:00:00Z", 4),
            [
                "2026-01-31T09:00Z",
                "2026-02-28T09:00Z",
                "2026-03-31T09:00Z",
                "2026-04-30T09:00Z"
            ]
        );
        assert_eq!(
            fires(&spec, "2028-02-01T00:00:00Z", 1),
            ["2028-02-29T09:00Z"],
            "leap year"
        );
        assert_eq!(
            fires(&cron("0 9 * * FRI#L", "UTC"), "2026-11-01T00:00:00Z", 2),
            ["2026-11-27T09:00Z", "2026-12-25T09:00Z"]
        );
        assert_eq!(
            fires(&cron("0 9 * * FRI#2", "UTC"), "2026-11-01T00:00:00Z", 2),
            ["2026-11-13T09:00Z", "2026-12-11T09:00Z"]
        );
    }

    #[test]
    fn day_of_month_and_day_of_week_are_either_unless_plus_makes_them_both() {
        // Either: the 13th, or any Friday.
        let either = fires(&cron("0 9 13 * FRI", "UTC"), "2026-02-01T00:00:00Z", 3);
        assert_eq!(
            either,
            [
                "2026-02-06T09:00Z",
                "2026-02-13T09:00Z",
                "2026-02-20T09:00Z"
            ]
        );
        // Both: Friday the 13th (Feb and Mar 2026; the next after is Nov).
        let both = fires(&cron("0 9 13 * +FRI", "UTC"), "2026-01-01T00:00:00Z", 3);
        assert_eq!(
            both,
            [
                "2026-02-13T09:00Z",
                "2026-03-13T09:00Z",
                "2026-11-13T09:00Z"
            ]
        );
    }

    #[test]
    fn steps_restart_each_period() {
        // */2 in day-of-month is the odd days, so the 31st is followed by the 1st.
        let got = fires(&cron("0 9 */2 * *", "UTC"), "2026-01-30T00:00:00Z", 3);
        assert_eq!(
            got,
            [
                "2026-01-31T09:00Z",
                "2026-02-01T09:00Z",
                "2026-02-03T09:00Z"
            ]
        );
    }

    #[test]
    fn every_other_week_keeps_its_phase_across_a_dst_change() {
        // Chicago, Tuesdays 10:00, from Tue 2026-03-03; US DST begins Sun 2026-03-08.
        let mut spec = cron("0 10 * * TUE", "America/Chicago");
        spec.every_n_weeks = Some(2);
        spec.start_date = Some("2026-03-03".into());
        let p = planned(&spec, "2026-03-01T00:00:00Z");
        let Some(Repeat::Cron { anchor_date, .. }) = &p.repeat else {
            panic!()
        };
        assert_eq!(anchor_date, "2026-03-03");
        assert_eq!(
            fires(&spec, "2026-03-01T00:00:00Z", 4),
            [
                "2026-03-03T16:00Z",
                "2026-03-17T15:00Z",
                "2026-03-31T15:00Z",
                "2026-04-14T15:00Z"
            ]
        );
        // Made later, the phase is the same: it never moves with `now`.
        assert_eq!(
            fires(&spec, "2026-03-18T00:00:00Z", 2),
            ["2026-03-31T15:00Z", "2026-04-14T15:00Z"]
        );
    }

    #[test]
    fn every_n_weeks_counts_monday_to_sunday_weeks_and_defaults_to_today() {
        // Made on Wed 2026-11-11: the first Mon/Fri match is Fri 11-13, week 0.
        let mut spec = cron("0 9 * * MON,FRI", "UTC");
        spec.every_n_weeks = Some(2);
        let got = fires(&spec, "2026-11-11T12:00:00Z", 5);
        assert_eq!(
            got,
            [
                "2026-11-13T09:00Z",
                "2026-11-23T09:00Z",
                "2026-11-27T09:00Z",
                "2026-12-07T09:00Z",
                "2026-12-11T09:00Z"
            ]
        );
        let Some(Repeat::Cron { anchor_date, .. }) = planned(&spec, "2026-11-11T12:00:00Z").repeat
        else {
            panic!()
        };
        assert_eq!(anchor_date, "2026-11-13");
    }

    #[test]
    fn every_n_days_counts_from_the_first_match() {
        let mut spec = cron("0 8 * * *", "Europe/Berlin");
        spec.every_n_days = Some(3);
        spec.start_date = Some("2026-12-30".into());
        // Across the new year, and a start date in the future holds everything back.
        assert_eq!(
            fires(&spec, "2026-11-11T00:00:00Z", 3),
            [
                "2026-12-30T07:00Z",
                "2027-01-02T07:00Z",
                "2027-01-05T07:00Z"
            ]
        );
    }

    #[test]
    fn a_start_date_holds_back_a_plain_cron() {
        let mut spec = cron("0 9 * * *", "UTC");
        spec.start_date = Some("2026-12-01".into());
        assert_eq!(
            fires(&spec, "2026-11-11T00:00:00Z", 2),
            ["2026-12-01T09:00Z", "2026-12-02T09:00Z"]
        );
    }

    #[test]
    fn the_first_fire_is_strictly_after_now() {
        let spec = cron("0 9 * * *", "UTC");
        assert_eq!(
            planned(&spec, "2026-11-11T09:00:00Z").first,
            utc("2026-11-12T09:00:00Z")
        );
        assert_eq!(
            planned(&spec, "2026-11-11T08:59:59Z").first,
            utc("2026-11-11T09:00:00Z")
        );
        // every_seconds: an 'at' at or before now is refused, and none means one interval.
        let every = RepeatSpec {
            every_seconds: Some(3600),
            ..Default::default()
        };
        let now = utc("2026-11-11T12:00:00Z");
        assert!(plan(Some(&every), Some(now), None, None, now).is_err());
        assert_eq!(
            plan(Some(&every), None, None, None, now).unwrap().first,
            utc("2026-11-11T13:00:00Z")
        );
        assert_eq!(
            plan(
                Some(&every),
                Some(utc("2026-11-11T12:00:01Z")),
                None,
                None,
                now
            )
            .unwrap()
            .first,
            utc("2026-11-11T12:00:01Z")
        );
    }

    #[test]
    fn every_seconds_never_drifts_and_skips_what_is_past() {
        let every = Repeat::Every { seconds: 3600 };
        let due = utc("2026-11-11T12:00:00Z");
        assert_eq!(
            next_after(&every, due, utc("2026-11-11T12:00:05Z")),
            Some(utc("2026-11-11T13:00:00Z")),
            "from the due time, not from when it fired"
        );
        assert_eq!(
            next_after(&every, due, utc("2026-11-11T17:30:00Z")),
            Some(utc("2026-11-11T18:00:00Z")),
            "no burst after a long gap"
        );
        assert_eq!(
            next_after(&every, due, utc("2026-11-11T13:00:00Z")),
            Some(utc("2026-11-11T14:00:00Z")),
            "strictly after"
        );
    }

    #[test]
    fn until_and_max_runs_end_a_schedule() {
        let every = Repeat::Every { seconds: 3600 };
        let first = utc("2026-11-11T13:00:00Z");
        let until = utc("2026-11-11T15:00:00Z");
        assert_eq!(
            upcoming(Some(&every), first, Some(until), None, 10).len(),
            3,
            "13, 14 and 15 (until is inclusive)"
        );
        assert_eq!(upcoming(Some(&every), first, None, Some(2), 10).len(), 2);
        assert_eq!(
            upcoming(None, first, None, None, 3),
            vec![first],
            "a one-shot"
        );

        let mut s = zeron_proto::Schedule {
            id: "s".into(),
            chat_id: "c".into(),
            text: "t".into(),
            fire_at: first,
            created_by: None,
            created_at: first,
            fired_at: None,
            cancelled_at: None,
            missed_at: None,
            repeat: Some(every),
            until: Some(until),
            max_runs: Some(3),
            runs: 0,
            last_fired_at: None,
            last_missed_at: None,
        };
        assert_eq!(next_run(&s, first), Some(utc("2026-11-11T14:00:00Z")));
        s.runs = 2; // this is the third send
        assert_eq!(next_run(&s, first), None, "max_runs reached");
        s.runs = 0;
        s.fire_at = utc("2026-11-11T15:00:00Z");
        assert_eq!(next_run(&s, s.fire_at), None, "nothing left before until");
        assert_eq!(next_skipping(&s, utc("2026-11-11T15:30:00Z")), None);
    }

    #[test]
    fn a_new_schedule_is_checked_loudly() {
        let ok = cron("0 9 * * *", "America/New_York");
        let with = |f: &dyn Fn(&mut RepeatSpec)| {
            let mut s = ok.clone();
            f(&mut s);
            s
        };
        for (spec, want) in [
            (with(&|s| s.timezone = None), "'timezone' is required"),
            (
                with(&|s| s.timezone = Some("Mars/Base".into())),
                "isn't an IANA zone",
            ),
            (
                with(&|s| s.cron = Some("0 0 9 * * *".into())),
                "valid 5-field",
            ),
            (
                with(&|s| s.cron = Some("61 9 * * *".into())),
                "valid 5-field",
            ),
            (
                with(&|s| s.cron = Some("0 9 30 2 *".into())),
                "never matches",
            ),
            (with(&|s| s.every_seconds = Some(3600)), "not both"),
            (
                with(&|s| {
                    s.every_n_weeks = Some(2);
                    s.every_n_days = Some(2);
                }),
                "not both",
            ),
            (with(&|s| s.every_n_weeks = Some(1)), "2 to 52"),
            (with(&|s| s.every_n_days = Some(400)), "2 to 366"),
            (
                with(&|s| s.start_date = Some("next friday".into())),
                "YYYY-MM-DD",
            ),
            (
                RepeatSpec {
                    every_seconds: Some(30),
                    ..Default::default()
                },
                "60 to",
            ),
            (
                RepeatSpec {
                    every_seconds: Some(3600),
                    timezone: Some("UTC".into()),
                    ..Default::default()
                },
                "go with 'cron'",
            ),
            (RepeatSpec::default(), "needs 'cron' or 'every_seconds'"),
        ] {
            let err = refused(Some(&spec), None, None, None);
            assert!(err.contains(want), "{spec:?}: {err}");
        }
        // A rule that rarely lines up still finds its next fire (a weekly cron
        // every 366 days is a Monday 366 weeks on) without searching forever.
        let mut odd = cron("0 9 * * MON", "UTC");
        odd.every_n_days = Some(366);
        let p = planned(&odd, "2026-11-11T12:00:00Z");
        assert_eq!(
            next_after(p.repeat.as_ref().unwrap(), p.first, p.first),
            Some(utc("2033-11-21T09:00:00Z"))
        );
        assert!(
            refused(Some(&ok), Some("2026-11-12T00:00:00Z"), None, None).contains("own first fire")
        );
        assert!(
            refused(Some(&ok), None, Some("2026-11-11T12:30:00Z"), None)
                .contains("before the first fire")
        );
        assert!(refused(Some(&ok), None, None, Some(0)).contains("'max_runs' is 1 to"));
        assert!(
            refused(
                None,
                Some("2026-11-12T00:00:00Z"),
                Some("2026-12-01T00:00:00Z"),
                None
            )
            .contains("for recurring")
        );
        assert!(refused(None, None, None, None).contains("give one of"));
        assert!(refused(None, Some("2020-01-01T00:00:00Z"), None, None).contains("already passed"));
    }

    /// The only floor is a minute: `* * * * *` and `every_seconds: 60` work.
    #[test]
    fn the_only_floor_is_one_minute() {
        let now = utc("2026-11-11T12:00:00Z");
        let every = |s| RepeatSpec {
            every_seconds: Some(s),
            ..Default::default()
        };
        assert!(plan(Some(&every(59)), None, None, None, now).is_err());
        assert_eq!(
            plan(Some(&every(60)), None, None, None, now).unwrap().first,
            utc("2026-11-11T12:01:00Z")
        );
        let each_minute = fires(&cron("* * * * *", "UTC"), "2026-11-11T12:00:30Z", 3);
        assert_eq!(
            each_minute,
            [
                "2026-11-11T12:01Z",
                "2026-11-11T12:02Z",
                "2026-11-11T12:03Z"
            ]
        );
        plan(
            Some(&cron("*/2 * * * *", "America/New_York")),
            None,
            None,
            None,
            now,
        )
        .unwrap();
    }

    #[test]
    fn fires_and_repeats_read_in_plain_words() {
        assert_eq!(
            format_fire(utc("2026-11-12T14:00:00Z"), Some("America/New_York")),
            "Thu 2026-11-12 09:00 EST (14:00 UTC)"
        );
        assert_eq!(
            format_fire(utc("2026-07-12T14:00:00Z"), Some("America/New_York")),
            "Sun 2026-07-12 10:00 EDT (14:00 UTC)"
        );
        assert_eq!(
            format_fire(utc("2026-11-13T03:00:00Z"), Some("America/New_York")),
            "Thu 2026-11-12 22:00 EST (2026-11-13 03:00 UTC)"
        );
        assert_eq!(
            format_fire(utc("2026-11-12T14:00:00Z"), None),
            "Thu 2026-11-12 14:00 UTC"
        );

        let mut spec = cron("0 10 * * TUE", "America/Chicago");
        spec.every_n_weeks = Some(2);
        let p = planned(&spec, "2026-11-11T12:00:00Z");
        let words = describe(
            p.repeat.as_ref(),
            Some(utc("2027-01-01T00:00:00Z")),
            Some(5),
        );
        assert!(
            words.contains("(0 10 * * TUE), America/Chicago time"),
            "{words}"
        );
        assert!(words.contains("only every 2nd week"), "{words}");
        assert!(
            words.contains("until Thu 2026-12-31 18:00 CST (2027-01-01 00:00 UTC)"),
            "{words}"
        );
        assert!(words.ends_with("at most 5 times"), "{words}");
        assert_eq!(
            describe(Some(&Repeat::Every { seconds: 90_000 }), None, None),
            "every 1 day 1 hour of real elapsed time (drifts against the wall clock across DST)"
        );
        assert_eq!(describe(None, None, None), "once");
        assert_eq!(
            ordinal(2) + &ordinal(3) + &ordinal(11) + &ordinal(22),
            "2nd3rd11th22nd"
        );
    }
}
