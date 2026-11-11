//! Scheduled messages: text a chat's host engine sends into a chat at a
//! later time. A schedule is a synced registry row (`schedules`); only the
//! chat's host fires it, once, and stamps what happened.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One scheduled message. One-shot in v1; a `repeat` field can join later
/// without changing the rest (rows decode leniently, unknown fields ignored).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub id: String,
    /// The chat the message goes into (also the chat whose host fires it).
    pub chat_id: String,
    /// What to send, as written: Zeron adds the "scheduled" header on delivery.
    pub text: String,
    pub fire_at: DateTime<Utc>,
    /// The chat that scheduled it; `None` when the person did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    /// The host took it and sent it (stamped before the send: at most once).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fired_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled_at: Option<DateTime<Utc>>,
    /// Its host found it past the grace window (the engine was down) and
    /// didn't send it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missed_at: Option<DateTime<Utc>>,
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
        };
        assert_eq!(s.state(), ScheduleState::Missed);
        s.fired_at = Some(at(4));
        assert_eq!(s.state(), ScheduleState::Fired);
        s.cancelled_at = Some(at(3));
        assert_eq!(s.state(), ScheduleState::Cancelled);
        assert!(!s.is_pending());
    }
}
