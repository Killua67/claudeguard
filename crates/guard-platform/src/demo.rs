//! Deterministic, no-network/no-process demo adapter for UI and contract tests.
use crate::*;
use std::sync::atomic::{AtomicUsize, Ordering};
#[derive(Default)]
pub struct DemoPlatform {
    pub probe_calls: AtomicUsize,
    pub launch_calls: AtomicUsize,
    inspection_delay: std::time::Duration,
}
impl DemoPlatform {
    /// Only the interactive demo is paced so people can see its simulated steps.
    /// Unit fixtures stay instantaneous; production adapters have no fake delays.
    pub fn paced() -> Self {
        Self {
            inspection_delay: std::time::Duration::from_millis(120),
            ..Self::default()
        }
    }
    pub fn local_fixture() -> LocalSnapshot {
        LocalSnapshot {
            proxy_fingerprint: "fixture-proxy".into(),
            owner_pid: 123,
            routes: "fixture-route".into(),
        }
    }
}
impl Platform for DemoPlatform {
    fn name(&self) -> &'static str {
        "demo-offline"
    }
    fn discover_proxy(&self) -> guard_core::discovery::Discovery {
        guard_core::discovery::Discovery {
            supported: true,
            candidates: vec![guard_core::discovery::ProxyCandidate {
                profile: profile(),
                source: "纯离线示例，未读取本机".into(),
            }],
            message: "离线示例，不代表本机设置".into(),
        }
    }
    fn clients(&self) -> ClientAvailability {
        ClientAvailability {
            cli: true,
            desktop: true,
        }
    }
    fn inspect(&self, id: RuleId, _: &Profile, _: &dyn Clock) -> Check {
        if !self.inspection_delay.is_zero() {
            std::thread::sleep(self.inspection_delay);
        }
        Check::new(
            id,
            if id.core() || matches!(id, RuleId::Timezone | RuleId::ClientSignature) {
                Status::Pass
            } else {
                Status::Info
            },
            "离线模拟证据；未读取本机、未联网、未执行客户端",
        )
    }
    fn local(&self, _: &Profile) -> Result<LocalSnapshot, Fault> {
        Ok(Self::local_fixture())
    }
    fn probe(&self, _: &Profile, clock: &dyn Clock) -> Result<Probe, Fault> {
        self.probe_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Probe {
            country: "US".into(),
            ip: "203.0.113.1".parse().unwrap(),
            duration_ms: 20,
            server_time: Some(clock.wall_seconds()),
            local_midpoint: clock.wall_seconds(),
            sampled_at: clock.now_ms(),
        })
    }
    fn launch(
        &self,
        _: ClientTarget,
        _: &Profile,
        _: &std::path::Path,
        _: Option<&std::path::Path>,
        _: &dyn Fn() -> bool,
    ) -> Result<(), Fault> {
        self.launch_calls.fetch_add(1, Ordering::SeqCst);
        Err(Fault::Unsupported)
    }
}

/// Explicit offline example, never a production default or discovery fallback.
pub fn profile() -> Profile {
    Profile {
        schema_version: 1,
        proxy_url: "http://127.0.0.1:7890".into(),
        expected_country: None,
        expected_proxy_executable: "/demo/local-proxy".into(),
        non_mainland_timezone: false,
    }
}
