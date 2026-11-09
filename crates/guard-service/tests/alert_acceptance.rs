//! Complete runtime pipeline, virtual time and a recording OS-notification sink.
//! No processes, sockets, OS alerts, Claude or external requests exist here.
use guard_core::{config::Profile, domain::*, monitoring::Notice};
use guard_platform::{demo::DemoPlatform, Clock, Fault, Platform};
use guard_service::{
    alerts::{deliver_pending, NotificationSink},
    controller::Controller,
};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
#[derive(Default)]
struct FakeClock(AtomicU64);
impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
    fn wall_seconds(&self) -> f64 {
        1_800_000_000.0 + self.now_ms() as f64 / 1000.0
    }
}
#[derive(Default)]
struct Scenario {
    local_failure: Option<Fault>,
    exit_failure: Option<Fault>,
    new_ip: bool,
    new_country: bool,
    new_pid: bool,
    new_route: bool,
}
struct Scripted {
    state: Mutex<Scenario>,
    local_calls: AtomicUsize,
    probe_calls: AtomicUsize,
    demo: DemoPlatform,
}
impl Platform for Scripted {
    fn name(&self) -> &'static str {
        "recorded-simulation"
    }
    fn clients(&self) -> ClientAvailability {
        self.demo.clients()
    }
    fn inspect(&self, id: RuleId, p: &Profile, c: &dyn Clock) -> Check {
        self.demo.inspect(id, p, c)
    }
    fn local(&self, p: &Profile) -> Result<LocalSnapshot, Fault> {
        self.local_calls.fetch_add(1, Ordering::SeqCst);
        let s = self.state.lock().unwrap();
        if let Some(e) = s.local_failure {
            return Err(e);
        }
        let mut local = self.demo.local(p)?;
        if s.new_pid {
            local.owner_pid += 1;
        }
        if s.new_route {
            local.routes = "fixture-second-route".into();
        }
        Ok(local)
    }
    fn probe(&self, p: &Profile, c: &dyn Clock) -> Result<Probe, Fault> {
        self.probe_calls.fetch_add(1, Ordering::SeqCst);
        let s = self.state.lock().unwrap();
        if let Some(e) = s.exit_failure {
            return Err(e);
        }
        let mut probe = self.demo.probe(p, c)?;
        if s.new_ip {
            probe.ip = "203.0.113.2".parse().unwrap();
        }
        if s.new_country {
            probe.country = "CA".into();
        }
        Ok(probe)
    }
    fn launch(
        &self,
        _: ClientTarget,
        _: &Profile,
        _: &Path,
        _: Option<&Path>,
        _: &dyn Fn() -> bool,
    ) -> Result<(), Fault> {
        panic!("simulation must never invoke a client")
    }
}
struct Sink {
    clock: Arc<FakeClock>,
    records: Mutex<Vec<(u64, Notice)>>,
}
impl NotificationSink for Sink {
    fn notify(&self, notice: Notice) {
        self.records
            .lock()
            .unwrap()
            .push((self.clock.now_ms(), notice));
    }
}
struct Harness {
    clock: Arc<FakeClock>,
    platform: Arc<Scripted>,
    controller: Controller,
    sink: Sink,
}
impl Harness {
    fn new() -> Self {
        let clock = Arc::new(FakeClock(AtomicU64::new(100_000)));
        let platform = Arc::new(Scripted {
            state: Mutex::new(Scenario::default()),
            local_calls: AtomicUsize::new(0),
            probe_calls: AtomicUsize::new(0),
            demo: DemoPlatform::default(),
        });
        let controller =
            Controller::new(platform.clone(), clock.clone(), fixture_profile(), false).unwrap();
        assert!(controller.scan().unwrap().complete());
        let sink = Sink {
            clock: clock.clone(),
            records: Mutex::new(vec![]),
        };
        assert_eq!(deliver_pending(&controller, &sink), 0);
        Self {
            clock,
            platform,
            controller,
            sink,
        }
    }
    fn tick(&self, at: u64) {
        self.clock.0.store(at, Ordering::SeqCst);
        self.controller.tick();
        deliver_pending(&self.controller, &self.sink);
        assert_eq!(deliver_pending(&self.controller, &self.sink), 0);
    }
    fn through(&self, from: u64, to: u64) {
        for now in (from..=to).step_by(5000) {
            self.tick(now);
        }
    }
    fn counts(&self) -> (usize, usize) {
        let r = self.sink.records.lock().unwrap();
        (
            r.iter()
                .filter(|(_, n)| matches!(n, Notice::Problem(_)))
                .count(),
            r.iter()
                .filter(|(_, n)| matches!(n, Notice::Recovered))
                .count(),
        )
    }
    fn assert_locked_and_private(&self) {
        let view = self.controller.snapshot();
        assert!(!view.can_launch);
        assert!(view.requires_recheck);
        assert!(view.events.len() <= 8);
        let text = serde_json::to_string(&view).unwrap();
        for secret in [
            "203.0.113",
            "fixture-second-route",
            "owner_pid",
            "proxy_fingerprint",
        ] {
            assert!(!text.contains(secret));
        }
    }
}
#[test]
fn local_failures_notify_once_continue_sampling_skip_exit_and_confirm_stable_recovery() {
    for failure in [
        Fault::ProxySettings,
        Fault::Listener,
        Fault::ProcessIdentity,
        Fault::Routes,
    ] {
        let h = Harness::new();
        let before = h.platform.local_calls.load(Ordering::SeqCst);
        h.platform.state.lock().unwrap().local_failure = Some(failure);
        h.through(105000, 130000);
        assert_eq!(h.counts(), (1, 0));
        assert!(h.platform.local_calls.load(Ordering::SeqCst) > before);
        assert_eq!(h.platform.probe_calls.load(Ordering::SeqCst), 4);
        assert_eq!(h.sink.records.lock().unwrap()[0].0, 105000);
        h.assert_locked_and_private();
        h.platform.state.lock().unwrap().local_failure = None;
        h.through(135000, 155000);
        assert_eq!(h.counts(), (1, 0));
        h.tick(160000);
        assert!(h.controller.snapshot().recovering);
        assert!(h.controller.snapshot().issue.is_some());
        h.through(165000, 185000);
        assert_eq!(h.counts(), (1, 0));
        h.tick(190000);
        assert_eq!(h.counts(), (1, 1));
        h.assert_locked_and_private();
        h.platform.state.lock().unwrap().local_failure = Some(failure);
        h.tick(195000);
        assert_eq!(h.counts(), (2, 1));
    }
}
#[test]
fn exit_changes_and_timeouts_are_throttled_and_never_expose_ip() {
    for case in 0..3 {
        let h = Harness::new();
        {
            let mut s = h.platform.state.lock().unwrap();
            match case {
                0 => s.new_ip = true,
                1 => s.new_country = true,
                _ => {
                    s.exit_failure = Some(Fault::TransportTimed {
                        code: 28,
                        elapsed_ms: 5030,
                    })
                }
            }
        }
        h.through(105000, 125000);
        assert_eq!(h.counts(), (0, 0));
        assert_eq!(h.platform.probe_calls.load(Ordering::SeqCst), 4);
        h.tick(130000);
        assert_eq!(h.counts(), (1, 0));
        assert_eq!(h.platform.probe_calls.load(Ordering::SeqCst), 5);
        h.assert_locked_and_private();
        h.through(135000, 155000);
        assert_eq!(h.counts(), (1, 0));
        assert_eq!(h.platform.probe_calls.load(Ordering::SeqCst), 5);
        *h.platform.state.lock().unwrap() = Scenario::default();
        h.tick(160000);
        assert!(h.controller.snapshot().recovering);
        h.through(165000, 185000);
        assert_eq!(h.counts(), (1, 0));
        h.tick(190000);
        assert_eq!(h.counts(), (1, 1));
        h.assert_locked_and_private();
    }
}
#[test]
fn new_valid_process_route_or_ip_can_recover_observation_but_never_launch_permission() {
    for case in 0..3 {
        let h = Harness::new();
        {
            let mut s = h.platform.state.lock().unwrap();
            match case {
                0 => s.new_pid = true,
                1 => s.new_route = true,
                _ => s.new_ip = true,
            }
        }
        h.through(105000, 155000);
        assert_eq!(h.counts(), (1, 0));
        h.tick(160000);
        assert_eq!(h.counts(), if case == 2 { (1, 0) } else { (1, 1) });
        h.through(165000, 185000);
        if case == 2 {
            h.tick(190000);
        }
        assert_eq!(h.counts(), (1, 1));
        assert!(h.controller.snapshot().issue.is_none());
        h.assert_locked_and_private();
    }
}
#[test]
fn native_path_sleep_and_wake_signals_invalidate_immediately_and_deduplicate() {
    for reason in [
        "网络路径发生变化，旧检测已失效",
        "系统即将休眠，旧检测已失效",
        "系统已唤醒，旧检测已失效",
    ] {
        let h = Harness::new();
        h.controller.invalidate(reason);
        assert_eq!(deliver_pending(&h.controller, &h.sink), 1);
        h.controller.invalidate(reason);
        assert_eq!(deliver_pending(&h.controller, &h.sink), 0);
        h.assert_locked_and_private();
        h.through(105000, 155000);
        assert_eq!(h.counts(), (1, 0));
        h.tick(160000);
        assert_eq!(h.counts(), (1, 1));
        h.assert_locked_and_private();
    }
}
#[test]
fn flapping_and_sampling_gaps_reset_recovery_confirmation() {
    let h = Harness::new();
    h.controller.invalidate("网络路径变化");
    deliver_pending(&h.controller, &h.sink);
    h.through(105000, 130000);
    assert!(h.controller.snapshot().recovering);
    h.platform.state.lock().unwrap().local_failure = Some(Fault::Listener);
    h.tick(135000);
    assert!(!h.controller.snapshot().recovering);
    h.platform.state.lock().unwrap().local_failure = None;
    h.through(140000, 160000);
    assert!(h.controller.snapshot().recovering);
    assert_eq!(h.counts(), (1, 0));
    // Missing observations cannot count toward the 30 seconds of stable recovery.
    h.tick(185000);
    assert_eq!(h.counts(), (1, 0));
    h.through(190000, 215000);
    assert_eq!(h.counts(), (1, 0));
    h.tick(220000);
    assert_eq!(h.counts(), (1, 1));
    h.assert_locked_and_private();
}
#[test]
fn repeated_incidents_keep_a_bounded_event_history_and_no_notice_replay() {
    let h = Harness::new();
    let mut now = 100000;
    for _ in 0..6 {
        h.controller.invalidate("网络路径变化");
        deliver_pending(&h.controller, &h.sink);
        h.through(now + 5000, now + 65000);
        now += 65000;
    }
    assert_eq!(h.counts(), (6, 6));
    assert_eq!(h.controller.snapshot().events.len(), 8);
    h.assert_locked_and_private();
}

#[cfg(test)]
fn fixture_profile() -> guard_core::config::Profile {
    guard_core::config::Profile {
        schema_version: 1,
        proxy_url: "http://127.0.0.1:17890".into(),
        expected_country: Some("US".into()),
        expected_proxy_executable: "/fixture/proxy".into(),
        non_mainland_timezone: true,
    }
}
