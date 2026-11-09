use crate::{
    engine,
    state::{Snapshot, State},
};
use guard_core::{config::Profile, domain::*, launch::LaunchTicket, monitoring::Notice};
use guard_platform::{Clock, Fault, Platform};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
static NEXT_SESSION: AtomicU64 = AtomicU64::new(0);
/// The only owner of report freshness, launch intent and observation state.
/// OS/network work runs outside the state lock; generations discard stale work.
pub struct Controller {
    state: Mutex<State>,
    platform: Arc<dyn Platform>,
    clock: Arc<dyn Clock>,
    demo: bool,
    launch_allowed: bool,
    session_id: String,
    sequence: AtomicU64,
}
impl Controller {
    pub fn new(
        platform: Arc<dyn Platform>,
        clock: Arc<dyn Clock>,
        profile: Profile,
        demo: bool,
    ) -> Result<Self, &'static str> {
        profile.validate()?;
        Ok(Self::create(platform, clock, profile, demo))
    }
    /// Empty first-run state: no scan or launch can execute until configured.
    pub fn unconfigured(
        platform: Arc<dyn Platform>,
        clock: Arc<dyn Clock>,
        demo: bool,
        launch_allowed: bool,
    ) -> Self {
        let mut controller = Self::create(platform, clock, Profile::default(), demo);
        controller.launch_allowed = launch_allowed && !demo;
        controller
    }
    fn create(
        platform: Arc<dyn Platform>,
        clock: Arc<dyn Clock>,
        profile: Profile,
        demo: bool,
    ) -> Self {
        Self {
            state: Mutex::new(State::new(profile)),
            platform,
            clock,
            demo,
            launch_allowed: !demo,
            session_id: format!(
                "{:x}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                NEXT_SESSION.fetch_add(1, Ordering::Relaxed)
            ),
            sequence: AtomicU64::new(0),
        }
    }
    /// Real observations with an immutable backend execution ban for acceptance.
    pub fn observer(
        platform: Arc<dyn Platform>,
        clock: Arc<dyn Clock>,
        profile: Profile,
    ) -> Result<Self, &'static str> {
        let mut controller = Self::new(platform, clock, profile, false)?;
        controller.launch_allowed = false;
        Ok(controller)
    }
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub fn snapshot(&self) -> Snapshot {
        let mut view = self.state().view(
            self.platform.name(),
            self.demo,
            self.clock.now_ms(),
            &self.session_id,
            self.sequence.fetch_add(1, Ordering::Relaxed),
        );
        view.launch_allowed = self.launch_allowed;
        view.can_launch &= self.launch_allowed;
        view
    }
    pub fn configure(&self, profile: Profile) -> Result<(), &'static str> {
        self.configure_with(profile, || Ok(()))
            .map_err(|_| "配置无效、保存失败或检查正在进行")
    }
    /// Persist before publishing the new profile while holding the same state
    /// lock as scans. A failed write never leaves in-memory and saved profiles split.
    pub fn configure_with(
        &self,
        profile: Profile,
        persist: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        profile.validate().map_err(str::to_string)?;
        let mut s = self.state();
        if s.busy {
            return Err("检查或启动期间不能修改配置".into());
        };
        persist()?;
        s.profile = profile;
        s.generation = s.generation.wrapping_add(1);
        s.report = None;
        s.monitor.pause();
        s.gate.cancel();
        s.launch_message = None;
        Ok(())
    }
    pub fn invalidate(&self, reason: &str) {
        let mut s = self.state();
        if s.profile.validate().is_err() {
            return;
        }
        s.generation = s.generation.wrapping_add(1);
        s.gate.cancel();
        let notice = s.monitor.invalidate(reason);
        s.notice(notice, self.clock.now_ms());
    }
    pub fn take_notices(&self) -> Vec<Notice> {
        std::mem::take(&mut self.state().notices)
    }
    pub fn scan(&self) -> Result<Report, Fault> {
        self.scan_for(None, &|_| {})
    }
    /// Publish real state transitions without polling or holding the state lock
    /// across the UI callback. The observer cannot create a launch intent.
    pub fn scan_with_progress(&self, progress: &dyn Fn(Snapshot)) -> Result<Report, Fault> {
        self.scan_for(None, progress)
    }
    fn scan_for(
        &self,
        target: Option<ClientTarget>,
        progress: &dyn Fn(Snapshot),
    ) -> Result<Report, Fault> {
        let (profile, generation) = {
            let mut s = self.state();
            if s.busy {
                return Err(Fault::Busy);
            }
            s.profile.validate().map_err(|_| Fault::Local)?;
            if let Some(t) = target {
                s.gate.request(t).map_err(|_| Fault::Busy)?
            } else {
                s.gate.cancel()
            };
            s.busy = true;
            s.scanning = true;
            s.launching = target.is_some();
            s.generation = s.generation.wrapping_add(1);
            s.checks.clear();
            s.monitor.pause();
            s.launch_message = None;
            (s.profile.clone(), s.generation)
        };
        progress(self.snapshot());
        let report = engine::scan(
            self.platform.as_ref(),
            &profile,
            self.clock.as_ref(),
            &mut |check| {
                let mut s = self.state();
                if s.generation == generation {
                    s.checks.push(check)
                }
                drop(s);
                progress(self.snapshot());
            },
        );
        let clients = self.platform.clients();
        let now = self.clock.now_ms();
        let mut s = self.state();
        s.scanning = false;
        if s.generation != generation {
            s.busy = false;
            s.launching = false;
            s.gate.cancel();
            let n = s.monitor.start(None, Some("检测期间环境发生变化"), now);
            s.notice(n, now);
            return Err(Fault::Drift);
        }
        let good = report.fresh(now);
        let baseline = good.then(|| report.baseline.clone()).flatten();
        let n = s.monitor.start(
            baseline,
            if good {
                None
            } else {
                Some("完整检查未全部通过，后台仍继续采样")
            },
            now,
        );
        s.notice(n, now);
        s.report = Some(report.clone());
        s.clients = clients;
        if target.is_none() {
            s.busy = false;
            s.launching = false;
        }
        drop(s);
        progress(self.snapshot());
        Ok(report)
    }
    pub fn launch(
        &self,
        target: ClientTarget,
        checker: &Path,
        directory: Option<&Path>,
    ) -> Result<(), Fault> {
        if !self.launch_allowed {
            return Err(Fault::Unsupported);
        }
        let report = self.scan_for(Some(target), &|_| {})?;
        let (profile, ticket) = {
            let mut s = self.state();
            let selected = s.gate.consume(&report, self.clock.now_ms());
            let available = match target {
                ClientTarget::Cli => s.clients.cli,
                ClientTarget::Desktop => s.clients.desktop,
            };
            if selected != Ok(target) || !available {
                s.busy = false;
                s.launching = false;
                s.launch_message = Some("复检未通过，未启动客户端".into());
                return Err(Fault::Client);
            }
            let checked_at = report
                .baseline
                .as_ref()
                .map(|b| b.exit.sampled_at.min(report.checked_at))
                .unwrap_or(0);
            (
                s.profile.clone(),
                LaunchTicket {
                    revision: s.monitor.revision,
                    checked_at,
                },
            )
        };
        let result = self
            .platform
            .launch(target, &profile, checker, directory, &|| {
                // Preparation (including static signing) may take time while
                // the sampler is busy. Re-read local evidence at submission;
                // a previous green scan cannot authorize a changed proxy path.
                let current = self.platform.local(&profile);
                let mut s = self.state();
                let unchanged = current
                    .as_ref()
                    .is_ok_and(|local| report.baseline.as_ref().is_some_and(|b| b.local == *local));
                if !unchanged {
                    let reason = current.err().unwrap_or(Fault::Drift).message();
                    let now = self.clock.now_ms();
                    let notice = s.monitor.invalidate(reason);
                    s.notice(notice, now);
                }
                ticket.permits(
                    s.monitor.revision,
                    s.monitor.launch_healthy,
                    self.clock.now_ms(),
                )
            });
        let mut s = self.state();
        s.busy = false;
        s.launching = false;
        s.launch_message = Some(
            if result.is_ok() {
                "已按手动请求提交启动；实际客户端流量未验证"
            } else {
                "启动准备失败或环境已变化；未确认启动"
            }
            .into(),
        );
        result
    }
    /// Deadline maintenance must never wait for a probe worker to finish.
    pub fn heartbeat(&self) {
        let now = self.clock.now_ms();
        let wall = self.clock.wall_seconds();
        let mut s = self.state();
        if s.profile.validate().is_ok()
            && s.last_wall > 0.0
            && (wall - s.last_wall > 20.0 || wall < s.last_wall || now < s.last_tick)
        {
            s.generation = s.generation.wrapping_add(1);
            let n = s.monitor.invalidate("监测中断、休眠恢复或时钟变化");
            s.notice(n, now)
        }
        s.last_tick = now;
        s.last_wall = wall;
        let n = s.monitor.freshness(now);
        s.notice(n, now);
    }
    pub fn tick(&self) {
        self.heartbeat();
        let now = self.clock.now_ms();
        let (profile, generation, include_exit) = {
            let mut s = self.state();
            if !s.monitor.active || s.busy || now.saturating_sub(s.last_attempt) < 5_000 {
                return;
            }
            s.busy = true;
            s.last_attempt = now;
            let include_exit = s.monitor.needs_exit(now);
            s.monitor.attempted(include_exit, now);
            (s.profile.clone(), s.generation, include_exit)
        };
        let result = self.platform.local(&profile).and_then(|local| {
            let probe = if include_exit {
                let p = self.platform.probe(&profile, self.clock.as_ref())?;
                if self.platform.local(&profile)? != local {
                    return Err(Fault::Drift);
                }
                Some(p)
            } else {
                None
            };
            Ok((local, probe))
        });
        let mut s = self.state();
        s.busy = false;
        if generation != s.generation || !s.monitor.active {
            return;
        }
        let now = self.clock.now_ms();
        let n = match result {
            Ok((local, exit)) => s.monitor.observe(local, exit, &profile, now),
            Err(e) => s.monitor.fail(&e.detail()),
        };
        s.notice(n, now);
    }
}
