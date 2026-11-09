//! Mac uses the real UserNotifications authorization API, never plugin Granted.
#[cfg(any(target_os = "macos", test))]
use guard_core::notifications::delivery::TestPhase;
#[cfg(target_os = "macos")]
use guard_core::notifications::delivery::TestRecord;
use guard_core::notifications::{
    delivery::TestTracker,
    diagnostics::{NotificationDiagnostics, SystemSettings},
    Authorization, NotificationAction,
};
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // Constructed only by Mac notification callbacks.
enum NotificationError {
    Authorization,
    Delivery,
}
impl NotificationError {
    fn message(self) -> &'static str {
        match self {
            Self::Authorization => "系统通知授权请求失败，请检查系统设置",
            Self::Delivery => "上次系统通知发送失败；请查看状态栏",
        }
    }
}
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
type Job = Box<dyn FnOnce() + Send>;
pub type Dispatcher = Arc<dyn Fn(Job) + Send + Sync>;
#[derive(Default)]
struct State {
    authorization: Option<Authorization>,
    error: Option<NotificationError>,
    settings: SystemSettings,
    tests: TestTracker,
}
pub struct Notifications {
    state: Mutex<State>,
    query_revision: AtomicU64,
    notice_revision: AtomicU64,
    center_revision: AtomicU64,
    #[cfg(target_os = "macos")]
    session_tag: u128,
    dispatch: Dispatcher,
    #[cfg(target_os = "macos")]
    show: Arc<dyn Fn() + Send + Sync>,
    demo: bool,
    active: AtomicBool,
}
impl Notifications {
    pub fn new(demo: bool, dispatch: Dispatcher, show: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        #[cfg(not(target_os = "macos"))]
        let _ = show;
        Arc::new(Self {
            state: Mutex::new(State::default()),
            query_revision: AtomicU64::new(0),
            notice_revision: AtomicU64::new(0),
            center_revision: AtomicU64::new(0),
            #[cfg(target_os = "macos")]
            session_tag: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            dispatch,
            #[cfg(target_os = "macos")]
            show,
            demo,
            active: AtomicBool::new(true),
        })
    }
    pub fn status(&self) -> String {
        if self.demo {
            return "离线演示不发送系统通知".into();
        }
        if !self.active.load(Ordering::Acquire) {
            return "系统通知服务已停止".into();
        }
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let auth = state.authorization.unwrap_or(Authorization::Unknown);
        match state.error {
            Some(error) => format!("{}；{}", auth.message(), error.message()),
            None => auth.message().into(),
        }
    }
    pub fn action(&self, enabled: bool) -> NotificationAction {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .authorization
            .unwrap_or(Authorization::Unknown)
            .action(enabled)
    }
    pub fn preference_status(&self, enabled: bool) -> String {
        let text = self.status();
        if self.demo || enabled {
            return text;
        }
        format!("{text}；本工具提醒尚未启用")
    }
    pub fn initialize(self: &Arc<Self>) {
        if self.demo {
            return;
        }
        #[cfg(target_os = "macos")]
        native::install(self.clone());
        self.refresh();
    }
    pub fn refresh(self: &Arc<Self>) {
        #[cfg(target_os = "macos")]
        if !self.demo && self.active.load(Ordering::Acquire) {
            native::refresh(self.clone(), false);
        }
    }
    pub fn enable(self: &Arc<Self>) -> Result<String, String> {
        if self.demo || !self.active.load(Ordering::Acquire) {
            return Err("当前模式不请求系统通知权限".into());
        }
        #[cfg(target_os = "macos")]
        {
            native::refresh(self.clone(), true);
            Ok("正在读取权限；仅未决定时向系统请求授权。".into())
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err("此平台系统通知尚未验收".into())
        }
    }
    /// Explicit local self-test only. Does not sample networks, change runtime
    /// health, request permission, or invoke any client.
    pub fn send_test(self: &Arc<Self>) -> Result<(), String> {
        let sequence = self.begin_test()?;
        #[cfg(target_os = "macos")]
        {
            native::send_test(self.clone(), sequence)
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.complete_test(sequence, false);
            Err("此平台系统通知尚未验收".into())
        }
    }
    fn begin_test(&self) -> Result<u64, String> {
        if self.demo || !self.active.load(Ordering::Acquire) {
            return Err("当前模式不发送测试通知".into());
        }
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .tests
            .begin(unix_ms())
            .map_err(str::to_string)
    }
    #[cfg(any(target_os = "macos", test))]
    fn claim_test(&self, sequence: u64) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        self.active.load(Ordering::Acquire) && state.tests.claim_submission(sequence)
    }
    fn complete_test(&self, sequence: u64, accepted: bool) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        self.active.load(Ordering::Acquire) && state.tests.complete(sequence, accepted)
    }
    #[cfg(any(target_os = "macos", test))]
    fn expire_test(&self, sequence: u64) {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .tests
            .expire(sequence);
    }
    #[cfg(target_os = "macos")]
    fn test_request_id(&self, sequence: u64) -> String {
        format!("notification-self-test-{:x}-{sequence}", self.session_tag)
    }
    #[cfg(target_os = "macos")]
    fn latest_test(&self) -> Option<TestRecord> {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .tests
            .last
            .clone()
    }
    #[cfg(any(target_os = "macos", test))]
    fn center_result(&self, sequence: u64, revision: u64, present: bool, queried_at: u64) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if self.active.load(Ordering::Acquire)
            && self.center_revision.load(Ordering::SeqCst) == revision
        {
            state.tests.center_result(sequence, present, queried_at);
        }
    }
    pub fn confirm_test(&self, sequence: u64, received: bool) -> Result<(), String> {
        if self.demo || !self.active.load(Ordering::Acquire) {
            return Err("当前模式不能确认测试结果".into());
        }
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .tests
            .confirm(sequence, received)
            .map_err(str::to_string)
    }
    pub fn diagnostics(&self, enabled: bool) -> NotificationDiagnostics {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let mut view = NotificationDiagnostics {
            app_enabled: enabled,
            service_active: self.active.load(Ordering::Acquire),
            demo: self.demo,
            application_name: crate::branding::APP_NAME,
            application_id: crate::diagnostics::APP_ID,
            system: state.settings.clone(),
            last_test: state.tests.last.clone(),
            suggestions: vec![],
        };
        view.suggestions = view.suggestions();
        view
    }
    pub fn post(self: &Arc<Self>, notice: guard_core::monitoring::Notice, enabled: bool) {
        if self.demo || !self.active.load(Ordering::Acquire) {
            return;
        }
        if !enabled {
            if matches!(notice, guard_core::monitoring::Notice::Problem(_)) {
                (self.dispatch)(Box::new(guard_platform::background::problem_sound));
            }
            return;
        }
        let revision = self.notice_revision.fetch_add(1, Ordering::SeqCst) + 1;
        #[cfg(target_os = "macos")]
        native::post(self.clone(), notice, revision);
        #[cfg(not(target_os = "macos"))]
        let _ = (notice, revision);
    }
    pub fn stop(&self) {
        if self.demo {
            return;
        }
        self.cancel_pending();
        #[cfg(target_os = "macos")]
        native::remove_delegate();
    }
    fn cancel_pending(&self) {
        self.active.store(false, Ordering::Release);
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .tests
            .cancel_pending();
        self.notice_revision.fetch_add(1, Ordering::SeqCst);
        self.center_revision.fetch_add(1, Ordering::SeqCst);
        self.query_revision.fetch_add(1, Ordering::SeqCst);
    }
    #[cfg(test)]
    fn accept_query(&self, revision: u64, auth: Authorization) -> bool {
        self.accept_settings(
            revision,
            SystemSettings {
                authorization: auth,
                ..Default::default()
            },
        )
    }
    #[cfg(any(target_os = "macos", test))]
    fn accept_settings(&self, revision: u64, settings: SystemSettings) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if !self.active.load(Ordering::Acquire)
            || self.query_revision.load(Ordering::SeqCst) != revision
        {
            return false;
        }
        let auth = settings.authorization;
        state.settings = settings;
        state.authorization = Some(auth);
        if auth.may_present() && state.error == Some(NotificationError::Authorization) {
            state.error = None;
        }
        true
    }
    #[cfg(any(target_os = "macos", test))]
    fn finish_delivery(&self, revision: u64, failed: bool) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if !self.active.load(Ordering::Acquire)
            || self.notice_revision.load(Ordering::SeqCst) != revision
        {
            return false;
        }
        if failed {
            state.error = Some(NotificationError::Delivery);
        } else if state.error == Some(NotificationError::Delivery) {
            state.error = None;
        }
        true
    }
    #[cfg(any(target_os = "macos", test))]
    fn finish_authorization(&self, revision: u64, failed: bool) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if !self.active.load(Ordering::Acquire)
            || self.query_revision.load(Ordering::SeqCst) != revision
        {
            return false;
        }
        if failed {
            state.error = Some(NotificationError::Authorization);
        } else if state.error == Some(NotificationError::Authorization) {
            state.error = None;
        }
        !failed
    }
}
fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
#[cfg(target_os = "macos")]
#[path = "notifications/macos.rs"]
mod native;
#[cfg(test)]
mod tests {
    use super::*;
    fn service() -> Arc<Notifications> {
        Notifications::new(
            false,
            Arc::new(|_| {}),
            Arc::new(|| panic!("no native UI in tests")),
        )
    }
    #[test]
    fn stale_permission_queries_and_stopped_completions_cannot_overwrite_state() {
        let service = service();
        service.query_revision.store(2, Ordering::SeqCst);
        assert!(service.accept_query(2, Authorization::Denied));
        assert!(!service.accept_query(1, Authorization::Authorized));
        assert!(!service.finish_authorization(1, true));
        assert!(service.status().contains("已被拒绝"));
        service.cancel_pending();
        assert!(!service.accept_query(3, Authorization::Authorized));
        assert!(!service.finish_authorization(3, true));
        assert!(service.enable().is_err());
    }
    #[test]
    fn authorization_failure_survives_background_read_until_a_new_successful_request() {
        let service = service();
        service.query_revision.store(1, Ordering::SeqCst);
        assert!(!service.finish_authorization(1, true));
        service.query_revision.store(2, Ordering::SeqCst);
        assert!(service.accept_query(2, Authorization::NotDetermined));
        assert!(service.status().contains("授权请求失败"));
        assert!(service.finish_authorization(2, false));
        assert!(!service.status().contains("请求失败"));
    }
    #[test]
    fn disabled_preference_does_not_submit_native_notification() {
        let jobs = Arc::new(AtomicU64::new(0));
        let observed = jobs.clone();
        let service = Notifications::new(
            false,
            Arc::new(move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
            }),
            Arc::new(|| {}),
        );
        service.post(guard_core::monitoring::Notice::Recovered, false);
        assert_eq!(jobs.load(Ordering::SeqCst), 0);
        service.post(
            guard_core::monitoring::Notice::Problem("fixture".into()),
            false,
        );
        assert_eq!(jobs.load(Ordering::SeqCst), 1); // harmless queued beep, never executed
        assert_eq!(service.notice_revision.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn external_system_grant_clears_only_obsolete_authorization_failure() {
        let service = service();
        service.query_revision.store(1, Ordering::SeqCst);
        assert!(!service.finish_authorization(1, true));
        service.query_revision.store(2, Ordering::SeqCst);
        assert!(service.accept_query(2, Authorization::Authorized));
        assert!(service.status().contains("已授权"));
        assert!(!service.status().contains("失败"));
        // A stale denied callback must not undo the newly granted state.
        assert!(!service.accept_query(1, Authorization::Denied));
        assert!(service.status().contains("已授权"));
    }
    #[test]
    fn delivery_failure_is_preserved_by_permission_refresh_and_cleared_by_real_delivery_success() {
        let service = service();
        service.notice_revision.store(2, Ordering::SeqCst);
        assert!(service.finish_delivery(2, true));
        service.query_revision.store(1, Ordering::SeqCst);
        assert!(service.accept_query(1, Authorization::Authorized));
        assert!(service.status().contains("发送失败"));
        assert!(!service.finish_delivery(1, false));
        assert!(service.status().contains("发送失败"));
        assert!(service.finish_delivery(2, false));
        assert!(!service.status().contains("失败"));
        service.cancel_pending();
        assert!(!service.finish_delivery(3, true));
    }
    #[test]
    fn self_test_rejects_demo_stopped_and_duplicate_requests_without_native_calls() {
        let demo = Notifications::new(
            true,
            Arc::new(|_| panic!("must not dispatch")),
            Arc::new(|| {}),
        );
        assert!(demo.send_test().is_err());
        let service = service();
        service.begin_test().unwrap();
        assert!(service.send_test().is_err());

        service.cancel_pending();
        assert!(service.send_test().is_err());
        assert_eq!(service.notice_revision.load(Ordering::SeqCst), 1); // stop only; no runtime incident
    }
    #[test]
    fn a_queued_test_after_timeout_cannot_submit_and_center_reads_are_ordered() {
        let service = service();
        let old = service.begin_test().unwrap();
        service.expire_test(old);
        assert!(!service.claim_test(old));
        assert!(!service.complete_test(old, true));
        assert!(service.confirm_test(old, true).is_err());
        let id = service.begin_test().unwrap();
        assert!(service.claim_test(id));
        assert!(service.complete_test(id, true));
        service.center_revision.store(2, Ordering::SeqCst);
        service.center_result(id, 2, true, 20);
        service.center_result(id, 1, false, 10);
        let view = service.diagnostics(true);
        let test = view.last_test.unwrap();
        assert_eq!(test.phase, TestPhase::Accepted);
        assert_eq!(test.in_notification_center, Some(true));
        assert_eq!(test.center_queried_at, Some(20));
        assert!(service.confirm_test(old, false).is_err());
    }
}
