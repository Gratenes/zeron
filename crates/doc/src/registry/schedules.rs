use super::*;
use zeron_proto::Schedule;

impl RegistryDoc {
    /// A new schedule (a `schedules` row). Each outcome stamp is its own
    /// field, so a cancel on one device and the host's fire both land.
    pub fn create_schedule(&mut self, schedule: &Schedule) -> Result<(), DocError> {
        let set = fields([
            ("id", json!(schedule.id)),
            ("chatId", json!(schedule.chat_id)),
            ("text", json!(schedule.text)),
            ("fireAt", json!(schedule.fire_at.timestamp_millis())),
            ("createdBy", opt_str(schedule.created_by.as_deref())),
            ("createdAt", json!(schedule.created_at.timestamp_millis())),
        ]);
        self.write(KIND_SCHEDULES, &schedule.id.clone(), OpKind::Upsert, set);
        Ok(())
    }

    pub fn schedule(&self, id: &str) -> Option<Schedule> {
        self.overlay_row(KIND_SCHEDULES, id)
            .and_then(|row| {
                serde_json::from_value::<crate::workspace::RawSchedule>(Value::Object(
                    row.fields.into_iter().collect(),
                ))
                .ok()
            })
            .map(Schedule::from)
    }

    /// Every schedule, soonest due first.
    pub fn read_schedules(&self) -> Vec<Schedule> {
        let mut all: Vec<Schedule> = self
            .read_kind::<crate::workspace::RawSchedule>(KIND_SCHEDULES)
            .into_iter()
            .map(Schedule::from)
            .collect();
        all.sort_by(|a, b| a.fire_at.cmp(&b.fire_at).then(a.id.cmp(&b.id)));
        all
    }

    /// Cancel a pending schedule. `false` when there is none, or it already
    /// fired, was missed or was cancelled.
    pub fn cancel_schedule(&mut self, id: &str, at: DateTime<Utc>) -> Result<bool, DocError> {
        if !self.schedule(id).is_some_and(|s| s.is_pending()) {
            return Ok(false);
        }
        self.write(
            KIND_SCHEDULES,
            id,
            OpKind::Update,
            fields([("cancelledAt", json!(at.timestamp_millis()))]),
        );
        Ok(true)
    }

    /// The host takes a pending schedule to send it: stamped here, before
    /// the send, so a send is never repeated. `false` when it isn't pending.
    pub fn fire_schedule(&mut self, id: &str, at: DateTime<Utc>) -> Result<bool, DocError> {
        if !self.schedule(id).is_some_and(|s| s.is_pending()) {
            return Ok(false);
        }
        self.write(
            KIND_SCHEDULES,
            id,
            OpKind::Update,
            fields([("firedAt", json!(at.timestamp_millis()))]),
        );
        Ok(true)
    }

    /// The host found a pending schedule too late to send. `false` when it
    /// isn't pending.
    pub fn miss_schedule(&mut self, id: &str, at: DateTime<Utc>) -> Result<bool, DocError> {
        if !self.schedule(id).is_some_and(|s| s.is_pending()) {
            return Ok(false);
        }
        self.write(
            KIND_SCHEDULES,
            id,
            OpKind::Update,
            fields([("missedAt", json!(at.timestamp_millis()))]),
        );
        Ok(true)
    }
}
