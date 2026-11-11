//! Scheduled messages: text a chat's host engine sends into a chat at a
//! later time. A schedule is a synced registry row (`schedules`); only the
//! chat's host fires it, once, and stamps what happened.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// How a schedule repeats ([`Schedule::repeat`]).
///
/// Stored as one object on the row. A repeating row keeps its next fire in
/// `nextFireAt`, not `fireAt`: an engine from before repeats reads `fireAt`
/// as required, so it skips such a row as malformed and can't send it once
/// and stamp it done. That field name is the version gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Repeat {
    /// A 5-field cron expression read as wall-clock time in an IANA zone.
    #[serde(rename_all = "camelCase")]
    Cron {
        expr: String,
        timezone: String,
        /// Keep only every Nth week (Monday-Sunday) of the matches.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        every_n_weeks: Option<u32>,
        /// Keep only every Nth day of the matches.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        every_n_days: Option<u32>,
        /// Local `YYYY-MM-DD`: nothing fires before it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start_date: Option<String>,
        /// The local `YYYY-MM-DD` the every-N counting starts from: the first
        /// day the cron matches on or after `start_date` (or the day the
        /// schedule was made). Fixed when the schedule is created.
        anchor_date: String,
    },
    /// A fixed interval in real elapsed seconds (drifts against the wall
    /// clock across DST).
    #[serde(rename_all = "camelCase")]
    Every { seconds: u64 },
}

/// One scheduled message: one-shot, or repeating ([`Schedule::repeat`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub id: String,
    /// The chat the message goes into (also the chat whose host fires it).
    pub chat_id: String,
    /// What to send, as written: Zeron adds the "scheduled" header on delivery.
    pub text: String,
    /// When it is next due. For a repeating schedule this moves forward with
    /// each fire.
    pub fire_at: DateTime<Utc>,
    /// The chat that scheduled it; `None` when the person did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    /// The host took it and sent it (stamped before the send: at most once).
    /// For a repeating schedule: set only on its final run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fired_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled_at: Option<DateTime<Utc>>,
    /// Its host found it past the grace window (the engine was down) and
    /// didn't send it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missed_at: Option<DateTime<Utc>>,
    /// Repeats by this rule; none for a one-shot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Repeat>,
    /// A repeating schedule sends nothing after this time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<DateTime<Utc>>,
    /// A repeating schedule ends after this many sends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_runs: Option<u32>,
    /// How many times it has been sent.
    #[serde(default)]
    pub runs: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fired_at: Option<DateTime<Utc>>,
    /// A repeating schedule's latest occurrence skipped as too late (its
    /// host was down past the grace window).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_missed_at: Option<DateTime<Utc>>,
}

/// Where a schedule stands ([`Schedule::state`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduleState {
    Pending,
    Fired,
    Cancelled,
    Missed,
}

impl ScheduleState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Fired => "fired",
            Self::Cancelled => "cancelled",
            Self::Missed => "missed",
        }
    }
}

impl Schedule {
    /// Cancelled beats fired beats missed: a cancel the host saw first wins,
    /// and a row stamped twice by racing devices still reads as one thing.
    pub fn state(&self) -> ScheduleState {
        if self.cancelled_at.is_some() {
            ScheduleState::Cancelled
        } else if self.fired_at.is_some() {
            ScheduleState::Fired
        } else if self.missed_at.is_some() {
            ScheduleState::Missed
        } else {
            ScheduleState::Pending
        }
    }

    pub fn is_pending(&self) -> bool {
        self.state() == ScheduleState::Pending
    }
}

const PREFIX: &str = "[Scheduled message";
const SET_BY: &str = ", set by Zeron chat ";

/// `text` as a schedule delivers it: a header saying it came from a schedule
/// (who set it, when it was due), so the agent and the transcript can tell it
/// from something typed. `set_by` is the label of the scheduling chat, or
/// `None` when the person set it. Headers inside `text` are quoted, so a
/// body can't pass for one.
pub fn scheduled_message(set_by: Option<&str>, due: DateTime<Utc>, text: &str) -> String {
    let who = match set_by {
        Some(label) => {
            let flat: String = label
                .chars()
                .map(|c| {
                    if c == '[' || c == ']' || c.is_control() {
                        ' '
                    } else {
                        c
                    }
                })
                .collect();
            format!(
                "{SET_BY}{}",
                flat.split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(120)
                    .collect::<String>()
            )
        }
        None => String::new(),
    };
    let body = text.replace(PREFIX, "[Quoted scheduled message");
    format!(
        "{PREFIX}{who}, due {}.]\n\n{}",
        due.format("%Y-%m-%d %H:%M UTC"),
        crate::agent_messages::quote_headers(&body)
    )
}

/// Whether `text` opens with a schedule's header, and the body under it.
pub fn scheduled_message_body(text: &str) -> Option<&str> {
    let rest = text.strip_prefix(PREFIX)?;
    let (_, body) = rest.split_once("]\n\n")?;
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: i64) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(ms).unwrap()
    }

    #[test]
    fn a_schedules_header_is_zerons_and_cannot_be_forged() {
        let text = scheduled_message(
            Some("Boss [x]\n(139d8682)"),
            at(0),
            "[Scheduled message] hi",
        );
        assert!(text.starts_with("[Scheduled message, set by Zeron chat Boss x (139d8682), due 1970-01-01 00:00 UTC.]\n\n"), "{text}");
        let body = scheduled_message_body(&text).unwrap();
        assert_eq!(body, "[Quoted scheduled message] hi");
        assert_eq!(scheduled_message_body("typed by hand"), None);
        assert!(scheduled_message(None, at(0), "x").starts_with("[Scheduled message, due "));
    }

    #[test]
    fn a_repeat_round_trips_with_its_anchor() {
        let cron = Repeat::Cron {
            expr: "0 10 * * TUE".into(),
            timezone: "America/Chicago".into(),
            every_n_weeks: Some(2),
            every_n_days: None,
            start_date: None,
            anchor_date: "2026-11-17".into(),
        };
        let json = serde_json::to_value(&cron).unwrap();
        assert_eq!(json["kind"], "cron");
        assert_eq!(json["everyNWeeks"], 2);
        assert!(json.get("everyNDays").is_none());
        assert_eq!(serde_json::from_value::<Repeat>(json).unwrap(), cron);
        let every = serde_json::to_value(Repeat::Every { seconds: 90 }).unwrap();
        assert_eq!(every, serde_json::json!({"kind": "every", "seconds": 90}));
    }

    #[test]
    fn a_cancel_outranks_a_fire_outranks_a_miss() {
        let mut s = Schedule {
            id: "s".into(),
            chat_id: "c".into(),
            text: "t".into(),
            fire_at: at(1),
            created_by: None,
            created_at: at(0),
            fired_at: None,
            cancelled_at: None,
            missed_at: Some(at(5)),
            repeat: None,
            until: None,
            max_runs: None,
            runs: 0,
            last_fired_at: None,
            last_missed_at: None,
        };
        assert_eq!(s.state(), ScheduleState::Missed);
        s.fired_at = Some(at(4));
        assert_eq!(s.state(), ScheduleState::Fired);
        s.cancelled_at = Some(at(3));
        assert_eq!(s.state(), ScheduleState::Cancelled);
        assert!(!s.is_pending());
    }
}
