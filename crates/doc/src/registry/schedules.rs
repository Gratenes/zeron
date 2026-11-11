use super::*;
use zeron_proto::Schedule;

impl RegistryDoc {
    /// A new schedule (a `schedules` row). Each outcome stamp is its own
    /// field, so a cancel on one device and the host's fire both land.
    ///
    /// A repeating schedule's due time goes in `nextFireAt` and a one-shot's
    /// in `fireAt`: an engine from before repeats requires `fireAt`, so it
    /// ignores a repeating row instead of sending it once and stamping it done.
    pub fn create_schedule(&mut self, schedule: &Schedule) -> Result<(), DocError> {
        let due_field = if schedule.repeat.is_some() {
            "nextFireAt"
        } else {
            "fireAt"
        };
        let mut set = fields([
            ("id", json!(schedule.id)),
            ("chatId", json!(schedule.chat_id)),
            ("text", json!(schedule.text)),
            (due_field, json!(schedule.fire_at.timestamp_millis())),
            ("createdBy", opt_str(schedule.created_by.as_deref())),
            ("createdAt", json!(schedule.created_at.timestamp_millis())),
        ]);
        if let Some(repeat) = &schedule.repeat {
            set.insert(
                "repeat".into(),
                serde_json::to_value(repeat).unwrap_or(Value::Null),
            );
            set.insert("runs".into(), json!(0));
            set.insert("until".into(), opt_ms(schedule.until));
            set.insert(
                "maxRuns".into(),
                schedule.max_runs.map_or(Value::Null, |n| json!(n)),
            );
        }
        self.write(KIND_SCHEDULES, &schedule.id.clone(), OpKind::Upsert, set);
        Ok(())
    }

    pub fn schedule(&self, id: &str) -> Option<Schedule> {
        self.overlay_row(KIND_SCHEDULES, id).and_then(|row| {
            serde_json::from_value::<crate::workspace::RawSchedule>(Value::Object(
                row.fields.into_iter().collect(),
            ))
            .ok()
            .and_then(crate::workspace::RawSchedule::into_schedule)
        })
    }

    /// Every schedule, soonest due first.
    pub fn read_schedules(&self) -> Vec<Schedule> {
        let mut all: Vec<Schedule> = self
            .read_kind::<crate::workspace::RawSchedule>(KIND_SCHEDULES)
            .into_iter()
            .filter_map(crate::workspace::RawSchedule::into_schedule)
            .collect();
        all.sort_by(|a, b| a.fire_at.cmp(&b.fire_at).then(a.id.cmp(&b.id)));
        all
    }

    fn pending_schedule(&self, id: &str) -> Option<Schedule> {
        self.schedule(id).filter(|s| s.is_pending())
    }

    /// Cancel a pending schedule. `false` when there is none, or it already
    /// fired, was missed or was cancelled.
    pub fn cancel_schedule(&mut self, id: &str, at: DateTime<Utc>) -> Result<bool, DocError> {
        if self.pending_schedule(id).is_none() {
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

    /// The host takes a pending one-shot to send it: stamped here, before
    /// the send, so a send is never repeated. `false` when it isn't pending
    /// (or repeats: [`Self::advance_schedule`]).
    pub fn fire_schedule(&mut self, id: &str, at: DateTime<Utc>) -> Result<bool, DocError> {
        if !self
            .pending_schedule(id)
            .is_some_and(|s| s.repeat.is_none())
        {
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

    /// The host takes a pending repeating schedule to send it, as ONE write:
    /// the fire is stamped (`lastFiredAt`, `runs` + 1) and the due time moves
    /// to `next`, so a send is never repeated and the next one is already
    /// recorded. `next: None` ends it (its last run): `firedAt` is stamped.
    /// `false` when it isn't a pending repeating schedule.
    pub fn advance_schedule(
        &mut self,
        id: &str,
        at: DateTime<Utc>,
        next: Option<DateTime<Utc>>,
    ) -> Result<bool, DocError> {
        let Some(schedule) = self.pending_schedule(id).filter(|s| s.repeat.is_some()) else {
            return Ok(false);
        };
        let mut set = fields([
            ("lastFiredAt", json!(at.timestamp_millis())),
            ("runs", json!(schedule.runs + 1)),
        ]);
        match next {
            Some(next) => set.insert("nextFireAt".into(), json!(next.timestamp_millis())),
            None => set.insert("firedAt".into(), json!(at.timestamp_millis())),
        };
        self.write(KIND_SCHEDULES, id, OpKind::Update, set);
        Ok(true)
    }

    /// The host found a pending one-shot too late to send. `false` when it
    /// isn't pending (or repeats: [`Self::skip_schedule`]).
    pub fn miss_schedule(&mut self, id: &str, at: DateTime<Utc>) -> Result<bool, DocError> {
        if !self
            .pending_schedule(id)
            .is_some_and(|s| s.repeat.is_none())
        {
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

    /// The host found a repeating schedule's occurrence too late and skips
    /// it, with no burst of catch-up: the due time moves to `next`. With no
    /// `next` (nothing left before `until`), it ends as missed. `false` when
    /// it isn't a pending repeating schedule.
    pub fn skip_schedule(
        &mut self,
        id: &str,
        at: DateTime<Utc>,
        next: Option<DateTime<Utc>>,
    ) -> Result<bool, DocError> {
        if self
            .pending_schedule(id)
            .filter(|s| s.repeat.is_some())
            .is_none()
        {
            return Ok(false);
        }
        let mut set = fields([("lastMissedAt", json!(at.timestamp_millis()))]);
        match next {
            Some(next) => set.insert("nextFireAt".into(), json!(next.timestamp_millis())),
            None => set.insert("missedAt".into(), json!(at.timestamp_millis())),
        };
        self.write(KIND_SCHEDULES, id, OpKind::Update, set);
        Ok(true)
    }
}
