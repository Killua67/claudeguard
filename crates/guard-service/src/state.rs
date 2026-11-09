use guard_core::{
    config::{Profile, POLICY_VERSION},
    domain::*,
    launch::LaunchGate,
    monitoring::{Monitor, Notice},
};
use serde::Serialize;
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventView {
    pub at_ms: u64,
    pub notice: Notice,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckGuidance {
    pub id: RuleId,
    pub advice: guard_core::guidance::Guidance,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub preflight_mode: &'static str,
    pub launch_allowed: bool,
    pub setup_required: bool,
    pub notification_status: Option<String>,
    pub notification_action: Option<guard_core::notifications::NotificationAction>,
    pub notification_diagnostics:
        Option<guard_core::notifications::diagnostics::NotificationDiagnostics>,
    pub recovering: bool,
    pub issue_guidance: Option<guard_core::guidance::Guidance>,
    pub session_id: String,
    pub sequence: u64,
    pub policy_version: u32,
    pub platform: String,
    pub demo: bool,
    pub scanning: bool,
    pub launching: bool,
    pub checks: Vec<Check>,
    pub check_guidance: Vec<CheckGuidance>,
    pub profile: Profile,
    pub clients: ClientAvailability,
    pub monitor_active: bool,
    pub issue: Option<String>,
    pub last_sample: Option<u64>,
    pub requires_recheck: bool,
    pub can_launch: bool,
    pub events: Vec<EventView>,
    pub launch_message: Option<String>,
    pub now_ms: u64,
}
pub(crate) struct State {
    pub profile: Profile,
    pub report: Option<Report>,
    pub monitor: Monitor,
    pub gate: LaunchGate,
    pub checks: Vec<Check>,
    pub clients: ClientAvailability,
    pub busy: bool,
    pub scanning: bool,
    pub launching: bool,
    pub generation: u64,
    pub last_tick: u64,
    pub last_wall: f64,
    pub last_attempt: u64,
    pub events: Vec<EventView>,
    pub notices: Vec<Notice>,
    pub launch_message: Option<String>,
}
impl State {
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            report: None,
            monitor: Monitor::default(),
            gate: LaunchGate::default(),
            checks: Vec::new(),
            clients: ClientAvailability::default(),
            busy: false,
            scanning: false,
            launching: false,
            generation: 0,
            last_tick: 0,
            last_wall: 0.0,
            last_attempt: 0,
            events: Vec::new(),
            notices: Vec::new(),
            launch_message: None,
        }
    }
    pub fn notice(&mut self, n: Option<Notice>, now: u64) {
        if let Some(notice) = n {
            self.events.insert(
                0,
                EventView {
                    at_ms: now,
                    notice: notice.clone(),
                },
            );
            self.events.truncate(8);
            self.notices.push(notice);
        }
    }
    pub fn view(
        &self,
        platform: &str,
        demo: bool,
        now: u64,
        session_id: &str,
        sequence: u64,
    ) -> Snapshot {
        Snapshot {
            preflight_mode: "proxy_preflight",
            launch_allowed: !demo,
            setup_required: self.profile.validate().is_err(),
            notification_status: None,
            notification_action: None,
            notification_diagnostics: None,
            recovering: self.monitor.recovering(),
            issue_guidance: self
                .monitor
                .issue
                .as_deref()
                .map(guard_core::guidance::issue),
            session_id: session_id.into(),
            sequence,
            policy_version: POLICY_VERSION,
            platform: platform.into(),
            demo,
            scanning: self.scanning,
            launching: self.launching,
            checks: self.checks.clone(),
            check_guidance: self
                .checks
                .iter()
                .filter_map(|c| {
                    guard_core::guidance::check(c.id, c.status)
                        .map(|advice| CheckGuidance { id: c.id, advice })
                })
                .collect(),
            profile: self.profile.clone(),
            clients: self.clients.clone(),
            monitor_active: self.monitor.active,
            issue: self.monitor.issue.clone(),
            last_sample: self.monitor.last_sample,
            requires_recheck: !self.monitor.launch_healthy,
            can_launch: !demo
                && !self.busy
                && self.report.as_ref().is_some_and(Report::complete)
                && self.monitor.launch_healthy,
            events: self.events.clone(),
            launch_message: self.launch_message.clone(),
            now_ms: now,
        }
    }
}
