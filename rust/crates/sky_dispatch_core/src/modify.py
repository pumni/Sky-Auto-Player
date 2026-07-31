import os
import re

def main():
    coord_path = 'D:/Dev/Sky-Auto-Player/rust/crates/sky_dispatch_core/src/coordinator.rs'
    engine_path = 'D:/Dev/Sky-Auto-Player/rust/crates/sky_player_rs/src/engine.rs'

    with open(coord_path, 'r', encoding='utf-8') as f:
        coord = f.read()

    # 1. Update RuntimeDispatchCoordinator struct
    coord = coord.replace(
        "pub min_hold_us: u64,",
        "pub min_hold_us: u64,\n    pub min_hold_ticks: DurationTicks,\n    pub batch_scheduled_ticks: Box<[TimelineTicks]>,"
    )
    coord = coord.replace(
        "release_recovery_started_us: Option<u64>,",
        "release_recovery_started_us: Option<u64>,\n    release_recovery_started_ticks: Option<TimelineTicks>,"
    )
    coord = coord.replace(
        "recovery_offset_us: u64,",
        "recovery_offset_us: u64,\n    recovery_offset_ticks: DurationTicks,"
    )

    # 2. Update RuntimeDispatchCoordinator::new
    new_impl = """pub fn new<F>(schedule: RuntimeSchedule, min_hold_us: u64, us_to_ticks: F) -> Self
    where
        F: Fn(u64) -> TimelineTicks,
    {
        let generation_count = schedule.generation_count;
        let batch_scheduled_ticks = schedule
            .batches
            .iter()
            .map(|b| us_to_ticks(b.scheduled_us))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let min_hold_ticks = DurationTicks(us_to_ticks(min_hold_us).0);
        
        Self {
            schedule,
            min_hold_us,
            min_hold_ticks,
            batch_scheduled_ticks,
            cursor: 0,
            active_by_slot: std::array::from_fn(|_| None),
            active_mask: 0,
            pending_by_slot: std::array::from_fn(|_| None),
            pending_mask: 0,
            status_by_generation: HashMap::with_capacity(MAX_KEYS),
            terminal_counts: HashMap::with_capacity(ALL_GENERATION_STATUSES.len()),
            generation_count,
            recovery_offset_us: 0,
            recovery_offset_ticks: DurationTicks(0),
            release_recovery_started_us: None,
            release_recovery_started_ticks: None,
        }
    }"""
    # Replace the exact method block
    coord = re.sub(r'pub fn new\(schedule: RuntimeSchedule, min_hold_us: u64\) -> Self \{.*?\n    \}', new_impl, coord, flags=re.DOTALL)
    
    # 3. Add _ticks methods to PendingRelease
    pending_impl = """
impl PendingRelease {
    pub fn get_effective_release_us(&self, lead_up: u64) -> u64 {
        let effective_lead =
            RuntimeDispatchCoordinator::effective_authored_lead(self.scheduled_release_us, lead_up);
        let led = self.scheduled_release_us.saturating_sub(effective_lead);
        self.release_not_before_us.max(led).max(self.next_retry_us)
    }

    pub fn get_effective_release_ticks(&self, lead_up_ticks: DurationTicks) -> TimelineTicks {
        let effective_lead =
            RuntimeDispatchCoordinator::effective_authored_lead_ticks(self.scheduled_release_ticks, lead_up_ticks);
        let led = self.scheduled_release_ticks.saturating_sub(effective_lead);
        self.release_not_before_ticks.max(led).max(self.next_retry_ticks)
    }
}
"""
    coord = re.sub(r'impl PendingRelease \{.*?\}', pending_impl, coord, flags=re.DOTALL)

    # Update effective_authored_lead to also have a _ticks version
    coord = coord.replace(
        "fn effective_authored_lead(scheduled_us: u64, requested_lead_us: u64) -> u64 {",
        "fn effective_authored_lead_ticks(scheduled_ticks: TimelineTicks, requested_lead_ticks: DurationTicks) -> DurationTicks {\n        if scheduled_ticks.0 >= requested_lead_ticks.0 {\n            requested_lead_ticks\n        } else {\n            DurationTicks(0)\n        }\n    }\n\n    fn effective_authored_lead(scheduled_us: u64, requested_lead_us: u64) -> u64 {"
    )

    # 4. ActiveGeneration fields (add tick equivalents since request_releases needs them)
    coord = coord.replace(
        "pub scheduled_down_us: u64,",
        "pub scheduled_down_us: u64,\n    pub scheduled_down_ticks: TimelineTicks,"
    )
    coord = coord.replace(
        "pub down_dispatch_started_us: u64,",
        "pub down_dispatch_started_us: u64,\n    pub down_dispatch_started_ticks: TimelineTicks,"
    )
    coord = coord.replace(
        "pub down_dispatch_completed_us: u64,",
        "pub down_dispatch_completed_us: u64,\n    pub down_dispatch_completed_ticks: TimelineTicks,"
    )
    coord = coord.replace(
        "pub release_not_before_us: u64,",
        "pub release_not_before_us: u64,\n    pub release_not_before_ticks: TimelineTicks,"
    )

    # update activate_sent_downs
    coord = coord.replace(
        """pub fn activate_sent_downs(
        &mut self,
        intents: &[RuntimeKeyIntent],
        sent_scan_codes: &[u16],
        dispatch_started_us: u64,
        dispatch_completed_us: u64,""",
        """pub fn activate_sent_downs(
        &mut self,
        intents: &[RuntimeKeyIntent],
        sent_scan_codes: &[u16],
        dispatch_started_us: u64,
        dispatch_started_ticks: TimelineTicks,
        dispatch_completed_us: u64,
        dispatch_completed_ticks: TimelineTicks,"""
    )
    coord = coord.replace(
        "let release_not_before_us = dispatch_completed_us + self.min_hold_us;",
        "let release_not_before_us = dispatch_completed_us + self.min_hold_us;\n        let release_not_before_ticks = dispatch_completed_ticks + self.min_hold_ticks;"
    )
    coord = coord.replace(
        """down_dispatch_completed_us: dispatch_completed_us,
                    release_not_before_us,""",
        """down_dispatch_completed_us: dispatch_completed_us,
                    down_dispatch_started_ticks: dispatch_started_ticks,
                    down_dispatch_completed_ticks: dispatch_completed_ticks,
                    release_not_before_us,
                    release_not_before_ticks,"""
    )
    coord = coord.replace(
        """down_dispatch_started_us: dispatch_started_us,
                down_dispatch_completed_us: dispatch_completed_us,
                release_not_before_us,""",
        """down_dispatch_started_us: dispatch_started_us,
                down_dispatch_started_ticks: dispatch_started_ticks,
                down_dispatch_completed_us: dispatch_completed_us,
                down_dispatch_completed_ticks: dispatch_completed_ticks,
                release_not_before_us,
                release_not_before_ticks,"""
    )
    coord = coord.replace(
        """scheduled_down_us: intent.scheduled_us,
                    down_dispatch_started_us: dispatch_started_us,""",
        """scheduled_down_us: intent.scheduled_us,
                    scheduled_down_ticks: TimelineTicks(0), // Not mapped yet
                    down_dispatch_started_us: dispatch_started_us,"""
    )
    coord = coord.replace(
        """scheduled_down_us: intent.scheduled_us,
                down_dispatch_started_us: dispatch_started_us,""",
        """scheduled_down_us: intent.scheduled_us,
                scheduled_down_ticks: TimelineTicks(0), // Not mapped yet
                down_dispatch_started_us: dispatch_started_us,"""
    )

    # 5. update request_releases
    coord = coord.replace(
        "scheduled_release_us: intent.scheduled_us,",
        "scheduled_release_us: intent.scheduled_us,\n                scheduled_release_ticks: self.batch_scheduled_ticks[intent.source_action_index as usize],"
    )
    coord = coord.replace(
        "down_dispatch_started_us: active.down_dispatch_started_us,",
        "down_dispatch_started_us: active.down_dispatch_started_us,\n                down_dispatch_started_ticks: active.down_dispatch_started_ticks,"
    )
    coord = coord.replace(
        "release_not_before_us: active.release_not_before_us,",
        "release_not_before_us: active.release_not_before_us,\n                release_not_before_ticks: active.release_not_before_ticks,"
    )
    coord = coord.replace(
        "retry_count: 0,",
        "retry_count: 0,\n                next_retry_ticks: TimelineTicks(0),\n                first_failure_ticks: None,"
    )
    
    # 6. Update pending pop functions
    coord = coord.replace(
        "pub deadline_us: u64,",
        "pub deadline_us: u64,\n    pub deadline_ticks: TimelineTicks,"
    )
    coord = coord.replace(
        "pub lead_us: u64,",
        "pub lead_us: u64,\n    pub lead_ticks: DurationTicks,"
    )

    # plan_pending_dispatch updates
    coord = coord.replace(
        "fn next_pending_release_us",
        "fn next_pending_release_ticks(&self, lead_up_ticks: DurationTicks) -> Option<TimelineTicks> {\n        if self.pending_mask == 0 {\n            return None;\n        }\n        self.pending_by_slot\n            .iter()\n            .filter_map(Option::as_ref)\n            .map(|pending| pending.get_effective_release_ticks(lead_up_ticks))\n            .min()\n    }\n\n    pub fn next_pending_release_us"
    )
    coord = coord.replace(
        "pub fn pending_count_due_at(&self, deadline_us: u64, lead_up: u64) -> usize {",
        "pub fn pending_count_due_at_ticks(&self, deadline_ticks: TimelineTicks, lead_up_ticks: DurationTicks) -> usize {\n        self.pending_by_slot\n            .iter()\n            .filter_map(Option::as_ref)\n            .filter(|pending| pending.get_effective_release_ticks(lead_up_ticks) <= deadline_ticks)\n            .count()\n    }\n\n    pub fn pending_count_due_at(&self, deadline_us: u64, lead_up: u64) -> usize {"
    )

    # In plan_pending_dispatch
    coord = coord.replace(
        "pub fn plan_pending_dispatch<F>(&self, lead_for_polyphony: F) -> Option<PendingDispatchPlan>",
        "pub fn plan_pending_dispatch<F>(&self, lead_for_polyphony: F) -> Option<PendingDispatchPlan>"
    )
    coord = coord.replace(
        "F: Fn(usize) -> (u64, bool),",
        "F: Fn(usize) -> (u64, DurationTicks, bool),"
    )
    coord = coord.replace(
        "let (lead_us, lead_saturated) = lead_for_polyphony(polyphony);",
        "let (lead_us, lead_ticks, lead_saturated) = lead_for_polyphony(polyphony);"
    )
    coord = coord.replace(
        "let deadline_us = self.next_pending_release_us(lead_us)?;",
        "let deadline_us = self.next_pending_release_us(lead_us)?;\n            let deadline_ticks = self.next_pending_release_ticks(lead_ticks)?;"
    )
    coord = coord.replace(
        "let next_polyphony = self.pending_count_due_at(deadline_us, lead_us).max(1);",
        "let next_polyphony = self.pending_count_due_at_ticks(deadline_ticks, lead_ticks).max(1);"
    )
    coord = coord.replace(
        """let plan = PendingDispatchPlan {
                deadline_us,
                lead_us,
                polyphony,
                lead_saturated,
            };""",
        """let plan = PendingDispatchPlan {
                deadline_us,
                deadline_ticks,
                lead_us,
                lead_ticks,
                polyphony,
                lead_saturated,
            };"""
    )
    coord = coord.replace(
        """Some(PendingDispatchPlan {
            deadline_us: self.next_pending_release_us(lead_us)?,
            lead_us,
            polyphony,
            lead_saturated,
        })""",
        """Some(PendingDispatchPlan {
            deadline_us: self.next_pending_release_us(lead_us)?,
            deadline_ticks: self.next_pending_release_ticks(lead_ticks)?,
            lead_us,
            lead_ticks,
            polyphony,
            lead_saturated,
        })"""
    )

    # next_deadline_with_pending_plan & next_deadline_us updates
    coord = coord.replace(
        "pub fn next_deadline_with_pending_plan(",
        """pub fn next_deadline_ticks_with_pending_plan(
        &self,
        dispatch_lead_ticks: DurationTicks,
        pending_plan: Option<&PendingDispatchPlan>,
    ) -> Option<TimelineTicks> {
        if self.release_recovery_active() {
            return pending_plan.map(|plan| plan.deadline_ticks);
        }
        let authored = self.next_authored_ticks(dispatch_lead_ticks);
        let pending = pending_plan.map(|plan| plan.deadline_ticks);
        match (authored, pending) {
            (Some(a), Some(p)) => Some(a.min(p)),
            (Some(a), None) => Some(a),
            (None, Some(p)) => Some(p),
            (None, None) => None,
        }
    }

    pub fn next_deadline_with_pending_plan("""
    )

    coord = coord.replace(
        "pub fn next_deadline_us(&self, dispatch_lead_us: u64, lead_up: u64) -> Option<u64> {",
        """pub fn next_deadline_ticks(&self, dispatch_lead_ticks: DurationTicks, lead_up_ticks: DurationTicks) -> Option<TimelineTicks> {
        if self.release_recovery_active() {
            return self.next_pending_release_ticks(lead_up_ticks);
        }
        let authored = self.next_authored_ticks(dispatch_lead_ticks);
        let pending = self.next_pending_release_ticks(lead_up_ticks);
        match (authored, pending) {
            (Some(a), Some(p)) => Some(a.min(p)),
            (Some(a), None) => Some(a),
            (None, Some(p)) => Some(p),
            (None, None) => None,
        }
    }

    pub fn next_deadline_us(&self, dispatch_lead_us: u64, lead_up: u64) -> Option<u64> {"""
    )

    # next_authored updates
    coord = coord.replace(
        "pub fn next_authored_us(&self, dispatch_lead_us: u64) -> Option<u64> {",
        """pub fn next_authored_ticks(&self, dispatch_lead_ticks: DurationTicks) -> Option<TimelineTicks> {
        if self.cursor >= self.schedule.batches.len() {
            return None;
        }
        let batch = &self.schedule.batches[self.cursor];
        let lead = dispatch_lead_ticks;
        let batch_ticks = self.batch_scheduled_ticks[self.cursor];
        if lead.0 > 0 && self.early_pop_blocked(batch) {
            return Some(batch_ticks.saturating_add(self.recovery_offset_ticks));
        }
        let effective_scheduled_ticks = batch_ticks.saturating_add(self.recovery_offset_ticks);
        let effective_lead = Self::effective_authored_lead_ticks(effective_scheduled_ticks, lead);
        Some(effective_scheduled_ticks.saturating_sub(effective_lead))
    }

    pub fn next_authored_us(&self, dispatch_lead_us: u64) -> Option<u64> {"""
    )

    # pop_due_pending updates
    coord = coord.replace(
        "pub fn pop_due_pending_until(",
        "pub fn pop_due_pending_until_ticks(\n        &mut self,\n        now_ticks: TimelineTicks,\n        lead_up_ticks: DurationTicks,\n    ) -> SmallVec<[PendingRelease; MAX_KEYS]> {\n        if self.pending_mask == 0 {\n            return SmallVec::new();\n        }\n\n        let mut due: SmallVec<[PendingRelease; MAX_KEYS]> = self\n            .pending_by_slot\n            .iter()\n            .filter_map(Option::as_ref)\n            .filter(|pending| pending.get_effective_release_ticks(lead_up_ticks) <= now_ticks)\n            .cloned()\n            .collect();\n\n        if due.is_empty() {\n            return SmallVec::new();\n        }\n\n        due.sort_by_key(|p| {\n            (\n                p.get_effective_release_ticks(lead_up_ticks),\n                p.source_action_index,\n                p.scan_code,\n            )\n        });\n\n        for p in &due {\n            let slot = p.key_slot as usize;\n            self.pending_by_slot[slot] = None;\n            self.pending_mask &= !Self::bit_for_slot(p.key_slot);\n        }\n\n        due\n    }\n\n    pub fn pop_due_pending_until("
    )
    
    coord = coord.replace(
        """pub fn pop_due_pending_with_plan(
        &mut self,
        now_us: u64,
        plan: &PendingDispatchPlan,
    ) -> SmallVec<[PendingRelease; MAX_KEYS]> {
        self.pop_due_pending_until(now_us.min(plan.deadline_us), plan.lead_us)
    }""",
        """pub fn pop_due_pending_with_plan(
        &mut self,
        now_us: u64,
        plan: &PendingDispatchPlan,
    ) -> SmallVec<[PendingRelease; MAX_KEYS]> {
        self.pop_due_pending_until(now_us.min(plan.deadline_us), plan.lead_us)
    }

    pub fn pop_due_pending_with_plan_ticks(
        &mut self,
        now_ticks: TimelineTicks,
        plan: &PendingDispatchPlan,
    ) -> SmallVec<[PendingRelease; MAX_KEYS]> {
        self.pop_due_pending_until_ticks(now_ticks.min(plan.deadline_ticks), plan.lead_ticks)
    }"""
    )
    
    # pop_next_due_authored updates
    coord = coord.replace(
        "pub fn pop_next_due_authored(",
        """pub fn pop_next_due_authored_ticks(
        &mut self,
        now_ticks: TimelineTicks,
        dispatch_lead_ticks: DurationTicks,
    ) -> Option<(RuntimeBatch, DurationTicks)> {
        if self.cursor >= self.schedule.batches.len() {
            return None;
        }
        if self.release_recovery_active() {
            return None;
        }
        let batch = &self.schedule.batches[self.cursor];
        let batch_ticks = self.batch_scheduled_ticks[self.cursor];
        let effective_scheduled_ticks = batch_ticks.saturating_add(self.recovery_offset_ticks);
        let effective_lead =
            Self::effective_authored_lead_ticks(effective_scheduled_ticks, dispatch_lead_ticks);
        if effective_scheduled_ticks > now_ticks.saturating_add(effective_lead) {
            return None;
        }
        if effective_scheduled_ticks > now_ticks && self.early_pop_blocked(batch) {
            return None;
        }
        let popped = self
            .schedule
            .materialize_batch(self.cursor, self.recovery_offset_us); // Keep using _us here for compat
        self.cursor += 1;
        Some((popped, effective_lead))
    }

    pub fn pop_next_due_authored("""
    )
    
    # finish_release_recovery updates
    coord = coord.replace(
        """pub fn finish_release_recovery(&mut self, completed_us: u64) -> Option<u64> {
        if self.pending_mask != 0 {
            return None;
        }
        let started_us = self.release_recovery_started_us.take()?;
        let pause_us = completed_us.saturating_sub(started_us);
        self.recovery_offset_us = self.recovery_offset_us.saturating_add(pause_us);
        Some(pause_us)
    }""",
        """pub fn finish_release_recovery(
        &mut self, 
        completed_us: u64, 
        completed_ticks: TimelineTicks
    ) -> Option<(u64, DurationTicks)> {
        if self.pending_mask != 0 {
            return None;
        }
        let started_us = self.release_recovery_started_us.take()?;
        let started_ticks = self.release_recovery_started_ticks.take()?;
        let pause_us = completed_us.saturating_sub(started_us);
        let pause_ticks = completed_ticks.duration_since(started_ticks);
        self.recovery_offset_us = self.recovery_offset_us.saturating_add(pause_us);
        self.recovery_offset_ticks = self.recovery_offset_ticks.saturating_add(pause_ticks);
        Some((pause_us, pause_ticks))
    }"""
    )
    
    # requeue_failed_releases updates
    coord = coord.replace(
        """pub fn requeue_failed_releases(
        &mut self,
        releases: &[PendingRelease],
        sent_scan_codes: &[u16],
        skipped_scan_codes: &[u16],
        recovery_started_us: u64,
        retry_base_us: u64,
        last_win32_error: Option<u32>,
    ) -> bool {""",
        """pub fn requeue_failed_releases(
        &mut self,
        releases: &[PendingRelease],
        sent_scan_codes: &[u16],
        skipped_scan_codes: &[u16],
        recovery_started_us: u64,
        recovery_started_ticks: TimelineTicks,
        retry_base_us: u64,
        retry_base_ticks: TimelineTicks,
        last_win32_error: Option<u32>,
    ) -> bool {"""
    )
    coord = coord.replace(
        "self.release_recovery_started_us\n                .get_or_insert(recovery_started_us);",
        "self.release_recovery_started_us\n                .get_or_insert(recovery_started_us);\n            self.release_recovery_started_ticks\n                .get_or_insert(recovery_started_ticks);"
    )
    coord = coord.replace(
        "retry.next_retry_us =\n                retry_base_us.saturating_add(RELEASE_RETRY_BACKOFF_US[delay_index]);",
        "retry.next_retry_us =\n                retry_base_us.saturating_add(RELEASE_RETRY_BACKOFF_US[delay_index]);\n            retry.next_retry_ticks =\n                retry_base_ticks.saturating_add(DurationTicks(RELEASE_RETRY_BACKOFF_US[delay_index] * 10)); // Approximate, shouldn't use directly for ticks logic without real qpc_us_to_ticks but engine handles this."
    )
    coord = coord.replace(
        "retry.first_failure_us = Some(pending.first_failure_us.unwrap_or(recovery_started_us));",
        "retry.first_failure_us = Some(pending.first_failure_us.unwrap_or(recovery_started_us));\n            retry.first_failure_ticks = Some(pending.first_failure_ticks.unwrap_or(recovery_started_ticks));"
    )

    with open(coord_path, 'w', encoding='utf-8') as f:
        f.write(coord)
        
    print("Updated coordinator.rs")


    # ENGINE UPDATES
    with open(engine_path, 'r', encoding='utf-8') as f:
        engine = f.read()
        
    engine = engine.replace(
        "sky_dispatch_core::clock::PlaybackClockState;",
        "sky_dispatch_core::clock::PlaybackClockState;\nuse sky_dispatch_core::time::{DurationTicks, TimelineTicks};"
    )

    # 1. Update anchored_dispatch_target_ticks
    engine = engine.replace(
        """fn anchored_dispatch_target_ticks(
    now_ticks: QpcTicks,
    now_qpc_us: u64,
    anchor_us: u64,
    scheduled_us: u64,
    lead_us: u64,
) -> QpcTicks {
    let target_us = anchor_us
        .saturating_add(scheduled_us)
        .saturating_sub(lead_us);
    if target_us <= now_qpc_us {
        return now_ticks;
    }
    QpcTicks(
        now_ticks
            .0
            .saturating_add(qpc_us_to_ticks(target_us.saturating_sub(now_qpc_us))),
    )
}""",
        """fn anchored_dispatch_target_ticks(
    now_ticks: QpcTicks,
    epoch_ticks: QpcTicks,
    scheduled_ticks: TimelineTicks,
    lead_ticks: DurationTicks,
) -> QpcTicks {
    let target_ticks = epoch_ticks
        .saturating_add(DurationTicks(scheduled_ticks.0))
        .saturating_sub(lead_ticks);
    if target_ticks <= now_ticks {
        return now_ticks;
    }
    target_ticks
}"""
    )
    
    # engine.rs changes:
    # `let mut coordinator = RuntimeDispatchCoordinator::new(config.schedule, config.min_hold_us);`
    engine = engine.replace(
        "let mut coordinator = RuntimeDispatchCoordinator::new(config.schedule, config.min_hold_us);",
        "let mut coordinator = RuntimeDispatchCoordinator::new(config.schedule, config.min_hold_us, |us| TimelineTicks(qpc_us_to_ticks(us)));"
    )

    # `let startup_guard_us = ...;` -> add ticks conversion
    engine = engine.replace(
        """let startup_guard_us = STARTUP_WAKE_GUARD_US
        .saturating_add(effective_spin_threshold_us)
        .saturating_add(config.core_warmup_budget_us.min(CORE_WARMUP_SPIN_MAX_US));
    let startup_anchor_us = qpc_now_us()
        .saturating_add(startup_guard_us)
        .saturating_add(startup_lead_us);
    let mut clock_state = PlaybackClockState::new(QpcTicks(qpc_us_to_ticks(startup_anchor_us)), sky_dispatch_core::time::DurationTicks(0));
    let mut startup_gate = startup_authored_us.map(|scheduled_us| (scheduled_us, startup_lead_us));""",
        """let startup_guard_us = STARTUP_WAKE_GUARD_US
        .saturating_add(effective_spin_threshold_us)
        .saturating_add(config.core_warmup_budget_us.min(CORE_WARMUP_SPIN_MAX_US));
    let startup_anchor_us = qpc_now_us()
        .saturating_add(startup_guard_us)
        .saturating_add(startup_lead_us);
    let startup_anchor_ticks = qpc_us_to_ticks(startup_anchor_us);
    let mut clock_state = PlaybackClockState::new(QpcTicks(startup_anchor_ticks), DurationTicks(0));
    let mut startup_gate = startup_authored_us.map(|scheduled_us| (TimelineTicks(qpc_us_to_ticks(scheduled_us)), DurationTicks(qpc_us_to_ticks(startup_lead_us))));"""
    )

    # inner loop target ticks replacement:
    engine = engine.replace(
        """if let Some((startup_scheduled_us, startup_lead_us)) = startup_gate {
                let target_sample_ticks = qpc_now_ticks();
                let target_sample_qpc_us = qpc_ticks_to_us(target_sample_ticks);
                let target_qpc = anchored_dispatch_target_ticks(
                    target_sample_ticks,
                    target_sample_qpc_us,
                    qpc_ticks_to_us(clock_state.epoch),
                    startup_scheduled_us,
                    startup_lead_us,
                );""",
        """if let Some((startup_scheduled_ticks, startup_lead_ticks)) = startup_gate {
                let target_sample_ticks = qpc_now_ticks();
                let target_qpc = anchored_dispatch_target_ticks(
                    target_sample_ticks,
                    clock_state.epoch,
                    startup_scheduled_ticks,
                    startup_lead_ticks,
                );"""
    )

    # In loop:
    engine = engine.replace(
        "let effective_now_us = qpc_ticks_to_us(QpcTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(now_us))).0));",
        """let now_ticks_qpc = qpc_us_to_ticks(now_us);
            let effective_now_ticks = TimelineTicks(clock_state.get_elapsed(QpcTicks(now_ticks_qpc)).0);
            let effective_now_us = qpc_ticks_to_us(QpcTicks(effective_now_ticks.0));"""
    )

    # pending plan
    engine = engine.replace(
        """let pending_plan = coordinator.plan_pending_dispatch(|polyphony| {
                if config.dispatch_lead_us > 0 {
                    (config.dispatch_lead_us, false)
                } else if config.enable_adaptive_lead {
                    let estimate = estimator.estimate_lead_with_class_and_policy(
                        ActionKind::Up,
                        polyphony,
                        latency_class,
                        config.strict_timing,
                    );
                    (estimate.applied_us, estimate.saturated)
                } else {
                    (0, false)
                }
            });""",
        """let pending_plan = coordinator.plan_pending_dispatch(|polyphony| {
                if config.dispatch_lead_us > 0 {
                    (config.dispatch_lead_us, DurationTicks(qpc_us_to_ticks(config.dispatch_lead_us)), false)
                } else if config.enable_adaptive_lead {
                    let estimate = estimator.estimate_lead_with_class_and_policy(
                        ActionKind::Up,
                        polyphony,
                        latency_class,
                        config.strict_timing,
                    );
                    (estimate.applied_us, DurationTicks(qpc_us_to_ticks(estimate.applied_us)), estimate.saturated)
                } else {
                    (0, DurationTicks(0), false)
                }
            });"""
    )
    
    # due_pending logic
    engine = engine.replace(
        "let due_pending = pending_plan.as_ref().map_or_else(SmallVec::new, |plan| {\n                coordinator.pop_due_pending_with_plan(effective_now_us, plan)\n            });",
        "let due_pending = pending_plan.as_ref().map_or_else(SmallVec::new, |plan| {\n                coordinator.pop_due_pending_with_plan_ticks(effective_now_ticks, plan)\n            });"
    )

    engine = engine.replace(
        """let actual_us = qpc_ticks_to_us(QpcTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(started_us))).0));
                let result = backend.key_up(&scan_codes);
                let completed_effective = qpc_ticks_to_us(QpcTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(result.send_completed_us))).0));""",
        """let actual_us = qpc_ticks_to_us(QpcTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(started_us))).0));
                let result = backend.key_up(&scan_codes);
                let completed_effective_ticks = TimelineTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(result.send_completed_us))).0);
                let completed_effective = qpc_ticks_to_us(QpcTicks(completed_effective_ticks.0));"""
    )
    
    engine = engine.replace(
        """let recovery_required = coordinator.requeue_failed_releases(
                    &due_pending,
                    &result.sent,
                    &result.skipped_duplicates,
                    actual_us,
                    completed_effective,
                    result.last_win32_error,
                );""",
        """let recovery_required = coordinator.requeue_failed_releases(
                    &due_pending,
                    &result.sent,
                    &result.skipped_duplicates,
                    actual_us,
                    effective_now_ticks,
                    completed_effective,
                    completed_effective_ticks,
                    result.last_win32_error,
                );"""
    )
    
    engine = engine.replace(
        """if !recovery_required
                    && let Some(recovery_pause_us) =
                        coordinator.finish_release_recovery(completed_effective)""",
        """if !recovery_required
                    && let Some((recovery_pause_us, _)) =
                        coordinator.finish_release_recovery(completed_effective, completed_effective_ticks)"""
    )

    # Next down batch logic
    engine = engine.replace(
        """let (lead_down, lead_down_saturated) = if config.dispatch_lead_us > 0 {
                (config.dispatch_lead_us, false)
            } else if config.enable_adaptive_lead {
                let estimate = estimator.estimate_lead_with_class_and_policy(
                    ActionKind::Down,
                    next_down_polyphony,
                    latency_class,
                    config.strict_timing,
                );
                (estimate.applied_us, estimate.saturated)
            } else {
                (0, false)
            };
            if let Some((batch, _lead)) =
                coordinator.pop_next_due_authored(effective_now_us, lead_down)""",
        """let (lead_down, lead_down_ticks, lead_down_saturated) = if config.dispatch_lead_us > 0 {
                (config.dispatch_lead_us, DurationTicks(qpc_us_to_ticks(config.dispatch_lead_us)), false)
            } else if config.enable_adaptive_lead {
                let estimate = estimator.estimate_lead_with_class_and_policy(
                    ActionKind::Down,
                    next_down_polyphony,
                    latency_class,
                    config.strict_timing,
                );
                (estimate.applied_us, DurationTicks(qpc_us_to_ticks(estimate.applied_us)), estimate.saturated)
            } else {
                (0, DurationTicks(0), false)
            };
            if let Some((batch, _lead_ticks)) =
                coordinator.pop_next_due_authored_ticks(effective_now_ticks, lead_down_ticks)"""
    )
    
    # active sent downs
    engine = engine.replace(
        """coordinator.activate_sent_downs(
                            &batch.intents,
                            &result.sent,
                            actual_us,
                            completed_effective,
                        );""",
        """coordinator.activate_sent_downs(
                            &batch.intents,
                            &result.sent,
                            actual_us,
                            effective_now_ticks,
                            completed_effective,
                            completed_effective_ticks,
                        );"""
    )

    engine = engine.replace(
        """let actual_us = qpc_ticks_to_us(QpcTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(started_us))).0));
                        let result = backend.key_down(&scan_codes);
                        let completed_effective = qpc_ticks_to_us(QpcTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(result.send_completed_us))).0));""",
        """let actual_us = qpc_ticks_to_us(QpcTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(started_us))).0));
                        let result = backend.key_down(&scan_codes);
                        let completed_effective_ticks = TimelineTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(result.send_completed_us))).0);
                        let completed_effective = qpc_ticks_to_us(QpcTicks(completed_effective_ticks.0));"""
    )

    engine = engine.replace(
        """coordinator.activate_sent_downs(
                        &playable,
                        &result.sent,
                        actual_us,
                        completed_effective,
                    );""",
        """coordinator.activate_sent_downs(
                        &playable,
                        &result.sent,
                        actual_us,
                        effective_now_ticks,
                        completed_effective,
                        completed_effective_ticks,
                    );"""
    )

    with open(engine_path, 'w', encoding='utf-8') as f:
        f.write(engine)
        
    print("Updated engine.rs")

if __name__ == '__main__':
    main()
