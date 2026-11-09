//! Exercise the real macOS parser + proxy-preflight gate through Controller, with
//! recorded command output, virtual time, and a harmless submission counter.
#![cfg(target_os = "macos")]
use guard_core::{config::Profile, domain::*};
use guard_platform::{
    command::{CommandRunner, Output},
    demo::DemoPlatform,
    macos::{ApplicationRegistry, MacPlatform},
    Clock, Fault, Platform,
};
use guard_service::controller::Controller;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
#[derive(Default)]
struct VirtualClock(AtomicU64);
impl Clock for VirtualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
    fn wall_seconds(&self) -> f64 {
        1_800_000_000. + self.now_ms() as f64 / 1000.
    }
}
#[derive(Default)]
struct RecordedRunner {
    mode: AtomicUsize,
    calls: Mutex<Vec<String>>,
}
impl CommandRunner for RecordedRunner {
    fn run(&self, exe: &str, args: &[String], _: Duration) -> Result<Output, Fault> {
        self.calls.lock().unwrap().push(exe.into());
        let mode = self.mode.load(Ordering::SeqCst);
        let stdout=match (exe,args.first().map(String::as_str)) {
            ("/usr/sbin/scutil",Some("--proxy")) => format!("HTTPEnable : {}\nHTTPProxy : 127.0.0.1\nHTTPPort : 17890\nHTTPSEnable : 1\nHTTPSProxy : 127.0.0.1\nHTTPSPort : 17890\nProxyAutoConfigEnable : {}\n",if mode==4 {0} else {1},if mode==7 {1} else {0}),
            ("/usr/sbin/lsof",_) => "p123\nn127.0.0.1:17890\n".into(),
            ("/bin/ps",Some("-p")) => if mode==5 {"/fixture/wrong-owner"} else {"/fixture/proxy"}.into(),
            ("/sbin/ifconfig",_) if mode==3 => "".into(),
            ("/sbin/ifconfig",_) => format!("stf0: flags=0<> mtu 1280\nen0: flags=8863<UP,BROADCAST,RUNNING,SIMPLEX,MULTICAST> mtu 1500\n\tinet6 fe80::1%en0 prefixlen 64\n\tstatus: active\n{}",if mode==1 || mode>=4 {"\tinet6 2001:db8::7 prefixlen 64\n"} else {""}),
            ("/usr/sbin/netstat",_) if args.last().map(String::as_str)==Some("inet6") => format!("Routing tables\n\nInternet6:\nDestination Gateway Flags Netif Expire\nfe80::%en0/64 link#1 UCI en0\n{}",if mode==2 {"default fe80::1%en0 UG en0\n"} else {""}),
            ("/usr/sbin/netstat",_) => if mode==6 {""} else {"default 192.168.1.1 UGScg en0"}.into(),
            _ => panic!("simulation forbids any other command: {exe}"),
        };
        Ok(Output { code: 0, stdout })
    }
}
struct NoApplications;
impl ApplicationRegistry for NoApplications {
    fn has_running_bundle(&self, _: &str) -> Result<bool, Fault> {
        Ok(false)
    }
}
struct SimulatedPlatform {
    runner: Arc<RecordedRunner>,
    mac: MacPlatform,
    demo: DemoPlatform,
    probes: AtomicUsize,
    submitted: AtomicUsize,
    preparation_mode: AtomicUsize,
    final_probe_mode: AtomicUsize,
}
impl Platform for SimulatedPlatform {
    fn name(&self) -> &'static str {
        "ipv6-simulation"
    }
    fn clients(&self) -> ClientAvailability {
        self.demo.clients()
    }
    fn inspect(&self, id: RuleId, p: &Profile, c: &dyn Clock) -> Check {
        if id == RuleId::Dns && self.runner.mode.load(Ordering::SeqCst) == 9 {
            Check::new(id, Status::Warning, "DNS fixture unavailable")
        } else if id == RuleId::Ipv6 {
            self.mac.inspect(id, p, c)
        } else {
            self.demo.inspect(id, p, c)
        }
    }
    fn local(&self, p: &Profile) -> Result<LocalSnapshot, Fault> {
        self.mac.local(p)
    }
    fn probe(&self, p: &Profile, c: &dyn Clock) -> Result<Probe, Fault> {
        if self.runner.mode.load(Ordering::SeqCst) == 8 {
            return Err(Fault::Transport(60));
        }
        let count = self.probes.fetch_add(1, Ordering::SeqCst);
        let probe = self.demo.probe(p, c);
        let mode = self.final_probe_mode.load(Ordering::SeqCst);
        if count == 3 && mode > 0 {
            self.runner.mode.store(mode, Ordering::SeqCst);
        }
        probe
    }

    fn launch(
        &self,
        _: ClientTarget,
        _: &Profile,
        _: &Path,
        _: Option<&Path>,
        authorize: &dyn Fn() -> bool,
    ) -> Result<(), Fault> {
        let mode = self.preparation_mode.load(Ordering::SeqCst);
        if mode > 0 {
            self.runner.mode.store(mode, Ordering::SeqCst);
        }
        if !authorize() {
            return Err(Fault::Client);
        }
        self.submitted.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
fn fixture() -> (Controller, Arc<SimulatedPlatform>, Arc<VirtualClock>) {
    let clock = Arc::new(VirtualClock(AtomicU64::new(100_000)));
    let runner = Arc::new(RecordedRunner::default());
    let platform = Arc::new(SimulatedPlatform {
        mac: MacPlatform::with_dependencies(runner.clone(), Arc::new(NoApplications)),
        runner,
        demo: DemoPlatform::default(),
        probes: AtomicUsize::new(0),
        submitted: AtomicUsize::new(0),
        preparation_mode: AtomicUsize::new(0),
        final_probe_mode: AtomicUsize::new(0),
    });
    let profile = Profile {
        schema_version: 1,
        proxy_url: "http://127.0.0.1:17890".into(),
        expected_country: None,
        expected_proxy_executable: "/fixture/proxy".into(),
        non_mainland_timezone: false,
    };
    (
        Controller::new(platform.clone(), clock.clone(), profile, false).unwrap(),
        platform,
        clock,
    )
}
#[test]
fn coverage_unknown_is_advisory_and_both_simulated_clients_require_manual_fresh_preflight() {
    for mode in [1, 2, 3] {
        let (controller, platform, _) = fixture();
        platform.runner.mode.store(mode, Ordering::SeqCst);
        let report = controller.scan().unwrap();
        assert!(report.complete());
        let coverage = report.checks.iter().find(|c| c.id == RuleId::Ipv6).unwrap();
        assert_eq!(coverage.status, Status::Warning);
        assert!(!coverage.blocks_launch());
        assert_eq!(platform.probes.load(Ordering::SeqCst), 4);
        assert_eq!(platform.submitted.load(Ordering::SeqCst), 0);
        let json = serde_json::to_value(controller.snapshot()).unwrap();
        assert_eq!(json["preflightMode"], "proxy_preflight");
        let view = json["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == "ipv6")
            .unwrap();
        assert_eq!(view["blocksLaunch"], false);
        for target in [ClientTarget::Cli, ClientTarget::Desktop] {
            controller
                .launch(target, Path::new("/fixture/checker"), None)
                .unwrap();
        }
        assert_eq!(platform.submitted.load(Ordering::SeqCst), 2);
    }
}
#[test]
fn proxy_anomalies_and_other_warnings_still_block_even_with_ipv6_advisories() {
    // Disabled proxy, wrong owner, missing route, PAC bypass, TLS failure, DNS warning.
    for mode in 4..=9 {
        let (controller, platform, _) = fixture();
        platform.runner.mode.store(mode, Ordering::SeqCst);
        let report = controller.scan().unwrap();
        assert!(!report.complete(), "mode={mode}");
        assert!(report.checks.iter().any(Check::blocks_launch));
        for target in [ClientTarget::Cli, ClientTarget::Desktop] {
            assert!(controller
                .launch(target, Path::new("/fixture/checker"), None)
                .is_err());
        }
        assert_eq!(platform.submitted.load(Ordering::SeqCst), 0);
        assert!(!controller.snapshot().can_launch);
    }
}
#[test]
fn ipv6_changes_do_not_invent_proxy_failure_while_real_proxy_failures_notify_and_lock() {
    for mode in [1, 2, 3] {
        let (controller, platform, clock) = fixture();
        assert!(controller.scan().unwrap().complete());
        platform.runner.mode.store(mode, Ordering::SeqCst);
        for at in (105_000..=135_000).step_by(5_000) {
            clock.0.store(at, Ordering::SeqCst);
            controller.tick();
        }
        assert!(controller.take_notices().is_empty());
        assert!(controller.snapshot().issue.is_none());
        assert!(controller.snapshot().can_launch);
        // A subsequent full scan refreshes the informational IPv6 evidence.
        assert!(controller.scan().unwrap().complete());
        assert!(controller
            .snapshot()
            .checks
            .iter()
            .any(|c| c.id == RuleId::Ipv6 && c.status == Status::Warning));
        platform.runner.mode.store(4, Ordering::SeqCst);
        for at in (140_000..=160_000).step_by(5_000) {
            clock.0.store(at, Ordering::SeqCst);
            controller.tick();
        }
        assert_eq!(controller.take_notices().len(), 1);
        assert!(!controller.snapshot().can_launch);
        assert_eq!(platform.submitted.load(Ordering::SeqCst), 0);
        let text = serde_json::to_string(&controller.snapshot()).unwrap();
        assert!(!text.contains("2001:db8"));
    }
}
#[test]
fn final_submission_rechecks_proxy_state_but_does_not_treat_ipv6_as_an_anomaly() {
    for mode in [1, 4, 5, 6, 7] {
        for target in [ClientTarget::Cli, ClientTarget::Desktop] {
            let (controller, platform, _) = fixture();
            platform.preparation_mode.store(mode, Ordering::SeqCst);
            let result = controller.launch(target, Path::new("/fixture/checker"), None);
            assert_eq!(result.is_ok(), mode == 1);
            assert_eq!(
                platform.submitted.load(Ordering::SeqCst),
                usize::from(mode == 1)
            );
            if mode != 1 {
                assert!(!controller.snapshot().can_launch);
            }
        }
    }
}
#[test]
fn final_probe_keeps_ipv6_advisory_but_rejects_proxy_changes_without_a_baseline() {
    for mode in [1, 4] {
        let (controller, platform, _) = fixture();
        platform.final_probe_mode.store(mode, Ordering::SeqCst);
        let report = controller.scan().unwrap();
        assert_eq!(report.complete(), mode == 1);
        assert_eq!(report.baseline.is_some(), mode == 1);
        assert_eq!(platform.submitted.load(Ordering::SeqCst), 0);
    }
}
