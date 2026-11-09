#![allow(clippy::field_reassign_with_default)] // Fixtures deliberately mutate one policy input at a time.
use guard_core::{config::Profile, domain::*};
use guard_platform::{demo::DemoPlatform, Clock, Fault, Platform};
use guard_service::{controller::Controller, engine};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
};

#[test]
fn heartbeat_expires_and_notifies_while_sampler_is_still_blocked() {
    use std::sync::{atomic::AtomicBool, mpsc, Mutex};
    struct Blocked {
        demo: DemoPlatform,
        block: AtomicBool,
        started: Mutex<mpsc::Sender<()>>,
        release: Mutex<mpsc::Receiver<()>>,
    }
    impl Platform for Blocked {
        fn name(&self) -> &'static str {
            "blocked-fixture"
        }
        fn clients(&self) -> ClientAvailability {
            self.demo.clients()
        }
        fn inspect(&self, id: RuleId, p: &Profile, c: &dyn Clock) -> Check {
            self.demo.inspect(id, p, c)
        }
        fn local(&self, p: &Profile) -> Result<LocalSnapshot, Fault> {
            if self.block.swap(false, Ordering::SeqCst) {
                self.started.lock().unwrap().send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
            }
            self.demo.local(p)
        }
        fn probe(&self, p: &Profile, c: &dyn Clock) -> Result<Probe, Fault> {
            self.demo.probe(p, c)
        }
    }
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let clock = Arc::new(TestClock::at(100_000));
    let fixture = Arc::new(Blocked {
        demo: DemoPlatform::default(),
        block: AtomicBool::new(false),
        started: Mutex::new(started_tx),
        release: Mutex::new(release_rx),
    });
    let controller = Arc::new(
        Controller::new(fixture.clone(), clock.clone(), fixture_profile(), false).unwrap(),
    );
    controller.scan().unwrap();
    controller.heartbeat();
    fixture.block.store(true, Ordering::SeqCst);
    clock.set(105_000);
    let worker = {
        let controller = controller.clone();
        std::thread::spawn(move || controller.tick())
    };
    started_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    clock.set(125_001);
    controller.heartbeat();
    let during = controller.snapshot();
    let notices = controller.take_notices();
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    assert!(during.issue.is_some());
    assert!(!during.can_launch);
    assert!(during.requires_recheck);
    assert_eq!(notices.len(), 1);
    assert!(!controller.snapshot().can_launch);
}
#[derive(Default)]
struct TestClock(AtomicU64);
impl TestClock {
    fn at(t: u64) -> Self {
        Self(AtomicU64::new(t))
    }
    fn set(&self, t: u64) {
        self.0.store(t, Ordering::SeqCst)
    }
}
impl Clock for TestClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
    fn wall_seconds(&self) -> f64 {
        1800000000.0 + self.now_ms() as f64 / 1000.0
    }
}
struct Fixture {
    demo: DemoPlatform,
    bad: Option<RuleId>,
    probes: AtomicUsize,
    launches: AtomicUsize,
    clock: Arc<TestClock>,
    late: bool,
    timeout: bool,
    rotate: bool,
    unavailable: bool,
    fail_probe: Option<(usize, i32)>,
}
impl Fixture {
    fn new(clock: Arc<TestClock>) -> Self {
        Self {
            demo: DemoPlatform::default(),
            bad: None,
            probes: AtomicUsize::new(0),
            launches: AtomicUsize::new(0),
            clock,
            late: false,
            timeout: false,
            rotate: false,
            unavailable: false,
            fail_probe: None,
        }
    }
}
impl Platform for Fixture {
    fn name(&self) -> &'static str {
        "fixture"
    }
    fn clients(&self) -> ClientAvailability {
        if self.unavailable {
            ClientAvailability::default()
        } else {
            self.demo.clients()
        }
    }
    fn inspect(&self, id: RuleId, p: &Profile, c: &dyn Clock) -> Check {
        if self.bad == Some(id) {
            Check::new(
                id,
                if id.core() {
                    Status::Fail
                } else {
                    Status::Warning
                },
                "fixture blocked",
            )
        } else {
            self.demo.inspect(id, p, c)
        }
    }
    fn local(&self, p: &Profile) -> Result<LocalSnapshot, Fault> {
        if self.bad == Some(RuleId::SystemProxy) {
            Err(Fault::Local)
        } else {
            self.demo.local(p)
        }
    }
    fn probe(&self, p: &Profile, c: &dyn Clock) -> Result<Probe, Fault> {
        let n = self.probes.fetch_add(1, Ordering::SeqCst);
        if let Some((at, code)) = self.fail_probe {
            if n == at {
                return Err(Fault::TransportTimed {
                    code,
                    elapsed_ms: 5031,
                });
            }
        }
        if self.timeout {
            return Err(Fault::Transport(28));
        }
        let mut probe = self.demo.probe(p, c)?;
        if self.rotate && n > 0 {
            probe.ip = "203.0.113.2".parse().unwrap()
        }
        Ok(probe)
    }
    fn launch(
        &self,
        _: ClientTarget,
        _: &Profile,
        _: &Path,
        _: Option<&Path>,
        authorize: &dyn Fn() -> bool,
    ) -> Result<(), Fault> {
        if self.late {
            self.clock.set(self.clock.now_ms() + 25000)
        }
        if !authorize() {
            return Err(Fault::Client);
        }
        self.launches.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
#[test]
fn backend_refuses_unavailable_target_even_with_a_complete_scan() {
    let clock = Arc::new(TestClock::at(100000));
    let mut fixture = Fixture::new(clock.clone());
    fixture.unavailable = true;
    let fixture = Arc::new(fixture);
    let controller = Controller::new(fixture.clone(), clock, fixture_profile(), false).unwrap();
    assert!(controller
        .launch(ClientTarget::Cli, Path::new("/tmp/checker"), None)
        .is_err());
    assert_eq!(fixture.launches.load(Ordering::SeqCst), 0);
}
#[test]
fn failed_persistence_does_not_publish_a_different_profile() {
    let controller = Controller::new(
        Arc::new(DemoPlatform::default()),
        Arc::new(TestClock::at(100000)),
        fixture_profile(),
        true,
    )
    .unwrap();
    let mut next = fixture_profile();
    next.proxy_url = "http://127.0.0.1:9999".into();
    assert!(controller
        .configure_with(next, || Err("fixture disk failure".into()))
        .is_err());
    assert_eq!(controller.snapshot().profile, fixture_profile());
}
#[test]
fn ordinary_scan_never_launches_and_uses_exact_fifteen_ids() {
    let clock = Arc::new(TestClock::at(100000));
    let fixture = Arc::new(Fixture::new(clock.clone()));
    let controller = Controller::new(fixture.clone(), clock, fixture_profile(), false).unwrap();
    let report = controller.scan().unwrap();
    assert!(report.complete());
    assert_eq!(fixture.probes.load(Ordering::SeqCst), 4);
    assert_eq!(fixture.launches.load(Ordering::SeqCst), 0);
    assert_eq!(controller.snapshot().checks.len(), 15);
}
#[test]
fn progress_publishes_start_each_result_and_completion_without_launching() {
    let clock = Arc::new(TestClock::at(100000));
    let fixture = Arc::new(Fixture::new(clock.clone()));
    let controller = Controller::new(fixture.clone(), clock, fixture_profile(), true).unwrap();
    let updates = std::cell::RefCell::new(Vec::new());
    controller
        .scan_with_progress(&|view| {
            // Re-entering state access proves the observer is outside its mutex.
            assert_eq!(view.session_id, controller.snapshot().session_id);
            updates.borrow_mut().push(view);
        })
        .unwrap();
    let updates = updates.into_inner();
    assert_eq!(updates.len(), 17);
    assert!(updates[0].scanning);
    assert!(updates[0].checks.is_empty());
    for (index, view) in updates[1..16].iter().enumerate() {
        assert!(view.scanning);
        assert_eq!(view.checks.len(), index + 1);
        assert!(!view.can_launch);
    }
    assert!(!updates.last().unwrap().scanning);
    assert_eq!(fixture.launches.load(Ordering::SeqCst), 0);
}
#[test]
fn local_failure_never_probes_or_launches() {
    let clock = Arc::new(TestClock::at(100000));
    let mut f = Fixture::new(clock.clone());
    f.bad = Some(RuleId::SystemProxy);
    let fixture = Arc::new(f);
    let controller = Controller::new(fixture.clone(), clock, fixture_profile(), false).unwrap();
    assert!(!controller.scan().unwrap().complete());
    assert_eq!(fixture.probes.load(Ordering::SeqCst), 0);
    assert!(controller
        .launch(ClientTarget::Cli, Path::new("/fixture/checker"), None)
        .is_err());
    assert_eq!(fixture.launches.load(Ordering::SeqCst), 0);
}
#[test]
fn warning_blocks_a_manual_launch() {
    let clock = Arc::new(TestClock::at(100000));
    let mut f = Fixture::new(clock.clone());
    f.bad = Some(RuleId::Dns);
    let fixture = Arc::new(f);
    let controller = Controller::new(fixture.clone(), clock, fixture_profile(), false).unwrap();
    assert!(controller
        .launch(ClientTarget::Cli, Path::new("/fixture/checker"), None)
        .is_err());
    assert_eq!(fixture.launches.load(Ordering::SeqCst), 0);
}
#[test]
fn manual_intent_rechecks_before_one_harmless_stub_submission() {
    let clock = Arc::new(TestClock::at(100000));
    let fixture = Arc::new(Fixture::new(clock.clone()));
    let controller = Controller::new(fixture.clone(), clock, fixture_profile(), false).unwrap();
    controller.scan().unwrap();
    controller
        .launch(ClientTarget::Cli, Path::new("/fixture/checker"), None)
        .unwrap();
    assert_eq!(fixture.probes.load(Ordering::SeqCst), 8);
    assert_eq!(fixture.launches.load(Ordering::SeqCst), 1);
}
#[test]
fn late_signature_preparation_cannot_submit() {
    let clock = Arc::new(TestClock::at(100000));
    let mut f = Fixture::new(clock.clone());
    f.late = true;
    let fixture = Arc::new(f);
    let controller = Controller::new(fixture.clone(), clock, fixture_profile(), false).unwrap();
    assert!(controller
        .launch(ClientTarget::Desktop, Path::new("/fixture/checker"), None)
        .is_err());
    assert_eq!(fixture.launches.load(Ordering::SeqCst), 0);
}
#[test]
fn demo_cannot_execute_a_client_even_if_invoke_is_forged() {
    let clock = Arc::new(TestClock::at(100000));
    let fixture = Arc::new(Fixture::new(clock.clone()));
    let controller = Controller::new(fixture.clone(), clock, fixture_profile(), true).unwrap();
    controller.scan().unwrap();
    assert!(!controller.snapshot().can_launch);
    assert!(controller
        .launch(ClientTarget::Cli, Path::new("/fixture/checker"), None)
        .is_err());
    assert_eq!(fixture.launches.load(Ordering::SeqCst), 0);
}
#[test]
fn controller_keeps_sampling_after_an_incident_without_unlocking() {
    let clock = Arc::new(TestClock::at(100000));
    let fixture = Arc::new(Fixture::new(clock.clone()));
    let controller =
        Controller::new(fixture.clone(), clock.clone(), fixture_profile(), false).unwrap();
    controller.scan().unwrap();
    controller.invalidate("fixture path change");
    let before = fixture.probes.load(Ordering::SeqCst);
    for now in (105000..=170000).step_by(5000) {
        clock.set(now);
        controller.tick();
    }
    assert!(fixture.probes.load(Ordering::SeqCst) > before);
    assert!(controller.snapshot().monitor_active);
    assert!(controller.snapshot().requires_recheck);
    assert!(!controller.snapshot().can_launch);
    assert_eq!(fixture.launches.load(Ordering::SeqCst), 0);
}
#[test]
fn failed_exit_is_throttled_while_monitor_remains_active() {
    let clock = Arc::new(TestClock::at(100000));
    let mut f = Fixture::new(clock.clone());
    f.timeout = true;
    let fixture = Arc::new(f);
    let controller =
        Controller::new(fixture.clone(), clock.clone(), fixture_profile(), false).unwrap();
    controller.scan().unwrap();
    for now in (105000..=130000).step_by(5000) {
        clock.set(now);
        controller.tick();
    }
    assert_eq!(fixture.probes.load(Ordering::SeqCst), 2);
    assert!(controller.snapshot().monitor_active);
}
#[test]
fn exit_rotation_blocks_final_recheck() {
    let clock = Arc::new(TestClock::at(100000));
    let mut f = Fixture::new(clock.clone());
    f.rotate = true;
    let fixture = Arc::new(f);
    let controller = Controller::new(fixture.clone(), clock, fixture_profile(), false).unwrap();
    let report = controller.scan().unwrap();
    assert!(!report.complete());
    assert_eq!(fixture.probes.load(Ordering::SeqCst), 3);
}
#[test]
#[cfg(not(windows))]
fn windows_adapter_remains_unsupported_outside_windows() {
    let clock = TestClock::at(100000);
    let report = engine::scan(
        &guard_platform::windows::WindowsPlatform,
        &fixture_profile(),
        &clock,
        &mut |_| {},
    );
    assert!(!report.complete());
    assert_eq!(report.checks.len(), 15);
    assert!(report.baseline.is_none());
}
#[test]
fn ipc_view_is_redacted_and_cannot_carry_a_baseline() {
    let clock = Arc::new(TestClock::at(100000));
    let controller = Controller::new(
        Arc::new(Fixture::new(clock.clone())),
        clock,
        fixture_profile(),
        false,
    )
    .unwrap();
    controller.scan().unwrap();
    let text = serde_json::to_string(&controller.snapshot()).unwrap();
    for forbidden in ["203.0.113", "proxy_fingerprint", "owner_pid", "baseline"] {
        assert!(!text.contains(forbidden))
    }
}
#[test]
fn observer_remains_read_only_after_passing_and_reconfiguration() {
    let clock = Arc::new(TestClock::at(100000));
    let fixture = Arc::new(Fixture::new(clock.clone()));
    let controller = Controller::observer(fixture.clone(), clock, fixture_profile()).unwrap();
    assert!(controller.scan().unwrap().complete());
    let probes = fixture.probes.load(Ordering::SeqCst);
    for target in [ClientTarget::Cli, ClientTarget::Desktop] {
        assert!(controller
            .launch(target, Path::new("/fixture/checker"), None)
            .is_err());
    }
    assert_eq!(fixture.probes.load(Ordering::SeqCst), probes);
    controller
        .configure_with(fixture_profile(), || Ok(()))
        .unwrap();
    assert!(!controller.snapshot().launch_allowed);
    assert!(!controller.snapshot().can_launch);
    assert_eq!(fixture.launches.load(Ordering::SeqCst), 0);
}
#[test]
fn path_and_sleep_events_reject_late_inflight_scan_results() {
    let clock = Arc::new(TestClock::at(100000));
    let fixture = Arc::new(Fixture::new(clock.clone()));
    let controller = Controller::new(fixture.clone(), clock, fixture_profile(), false).unwrap();
    for reason in [
        "网络路径发生变化，旧检测已失效",
        "系统即将休眠，旧检测已失效",
        "系统已唤醒，旧检测已失效",
    ] {
        let result = controller.scan_with_progress(&|view| {
            if view.checks.len() == 4 {
                controller.invalidate(reason);
            }
        });
        // A late completed report cannot re-authorize after any native event.
        assert!(result.is_err());
        assert!(controller.snapshot().requires_recheck);
        assert!(!controller.snapshot().can_launch);
    }
    assert_eq!(fixture.launches.load(Ordering::SeqCst), 0);
}

#[test]
fn subsequent_exit_failures_preserve_safe_error_and_stop_requests() {
    for (at, code) in [(1, 28), (2, 60)] {
        let clock = Arc::new(TestClock::at(100000));
        let mut fixture = Fixture::new(clock.clone());
        fixture.fail_probe = Some((at, code));
        let report = engine::scan(&fixture, &fixture_profile(), clock.as_ref(), &mut |_| {});
        let failure = report
            .checks
            .iter()
            .find(|c| c.id == RuleId::ExitStability)
            .unwrap();
        assert_eq!(failure.status, Status::Fail);
        assert!(failure.detail.contains(&format!("curl {code}")));
        assert!(failure.detail.contains("5.03"));
        assert!(!failure.detail.contains("203.0.113"));
        assert_eq!(fixture.probes.load(Ordering::SeqCst), at + 1);
        assert!(!report.complete());
    }
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

#[test]
fn unconfigured_controller_cannot_scan_probe_or_launch_until_confirmed() {
    let fixture = Arc::new(DemoPlatform::default());
    let clock = Arc::new(TestClock::at(100_000));
    let controller = Controller::unconfigured(fixture.clone(), clock.clone(), false, false);
    assert!(controller.snapshot().setup_required);
    assert!(controller.scan().is_err());
    assert!(controller
        .launch(ClientTarget::Cli, Path::new("/fixture/checker"), None)
        .is_err());
    controller.tick();
    controller.invalidate("simulated path change before confirmation");
    assert!(controller.take_notices().is_empty());
    assert_eq!(fixture.probe_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.launch_calls.load(Ordering::SeqCst), 0);
    let report = engine::scan(
        fixture.as_ref(),
        &Profile::default(),
        clock.as_ref(),
        &mut |_| {},
    );
    assert!(!report.complete());
    assert_eq!(fixture.probe_calls.load(Ordering::SeqCst), 0);
    controller
        .configure_with(fixture_profile(), || Ok(()))
        .unwrap();
    assert!(!controller.snapshot().setup_required);
    assert!(controller.scan().unwrap().complete());
    assert!(!controller.snapshot().can_launch); // observer remains immutable
}
