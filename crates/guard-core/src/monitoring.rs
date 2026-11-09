use crate::{
    config::Profile,
    domain::{Baseline, LocalSnapshot, Probe},
};
use serde::Serialize;
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum Notice {
    Problem(String),
    Recovered,
}
/// Launch latch and ongoing observations are deliberately independent.
#[derive(Default)]
pub struct Monitor {
    pub active: bool,
    pub launch_healthy: bool,
    pub issue: Option<String>,
    pub last_sample: Option<u64>,
    pub revision: u64,
    baseline: Option<Baseline>,
    latest_exit: Option<Probe>,
    last_exit_attempt: Option<u64>,
    incident: bool,
    recovery_started: Option<u64>,
    recovery_candidate: Option<Baseline>,
}
impl Monitor {
    pub fn pause(&mut self) {
        self.active = false;
        self.launch_healthy = false;
        self.baseline = None;
        self.latest_exit = None;
        self.last_sample = None;
        self.last_exit_attempt = None;
        self.recovery_started = None;
        self.recovery_candidate = None;
    }
    pub fn start(
        &mut self,
        baseline: Option<Baseline>,
        reason: Option<&str>,
        now: u64,
    ) -> Option<Notice> {
        self.active = true;
        self.recovery_candidate = None;
        self.recovery_started = None;
        self.launch_healthy = baseline.is_some() && reason.is_none();
        self.last_sample = baseline.as_ref().map(|_| now);
        self.last_exit_attempt = baseline.as_ref().map(|b| b.exit.sampled_at);
        self.latest_exit = baseline.as_ref().map(|b| b.exit.clone());
        self.baseline = baseline;
        if let Some(reason) = reason {
            return self.fail(reason);
        }
        self.issue = None;
        if self.incident {
            self.incident = false;
            self.recovery_started = None;
            self.recovery_candidate = None;
            return Some(Notice::Recovered);
        }
        None
    }
    pub fn invalidate(&mut self, reason: &str) -> Option<Notice> {
        self.revision = self.revision.wrapping_add(1);
        self.fail(reason)
    }
    pub fn fail(&mut self, reason: &str) -> Option<Notice> {
        self.launch_healthy = false;
        self.issue = Some(reason.into());
        self.latest_exit = None;
        self.recovery_started = None;
        self.recovery_candidate = None;
        if self.incident {
            return None;
        }
        self.incident = true;
        Some(Notice::Problem(reason.into()))
    }
    pub fn needs_exit(&self, now: u64) -> bool {
        self.active
            && self
                .last_exit_attempt
                .is_none_or(|t| now < t || now - t >= 30_000)
    }
    pub fn attempted(&mut self, exit: bool, now: u64) {
        if exit {
            self.last_exit_attempt = Some(now)
        }
    }
    pub fn freshness(&mut self, now: u64) -> Option<Notice> {
        if !self.active {
            return None;
        }
        if self
            .last_sample
            .is_some_and(|t| now < t || now - t > 20_000)
            || self
                .latest_exit
                .as_ref()
                .is_some_and(|p| now < p.sampled_at || now - p.sampled_at > 75_000)
        {
            return self.fail("采样已过期或系统刚恢复运行");
        }
        None
    }
    pub fn recovering(&self) -> bool {
        self.incident && self.recovery_candidate.is_some()
    }
    pub fn observe(
        &mut self,
        local: LocalSnapshot,
        exit: Option<Probe>,
        profile: &Profile,
        now: u64,
    ) -> Option<Notice> {
        let stale = self.freshness(now);
        self.last_sample = Some(now);
        if let Some(p) = exit.as_ref() {
            if now < p.sampled_at || now - p.sampled_at > 20_000 {
                return self.fail("出口采样时间无效").or(stale);
            }
            if profile
                .expected_country
                .as_ref()
                .is_some_and(|c| *c != p.country)
            {
                return self.fail("出口地区与配置偏好不同").or(stale);
            }
        }
        if self.incident {
            // Recovery observes a separate candidate. A changed PID/IP can become
            // stably observable without changing any launch report or latch.
            if self
                .recovery_candidate
                .as_ref()
                .is_some_and(|b| b.local != local)
            {
                self.recovery_candidate = None;
                self.recovery_started = None;
                self.latest_exit = None;
            }
            if let Some(p) = exit {
                let same = self.recovery_candidate.as_ref().is_some_and(|b| {
                    b.local == local && b.exit.ip == p.ip && b.exit.country == p.country
                });
                let stable =
                    same && self.recovery_candidate.as_ref().is_some_and(|b| {
                        p.sampled_at >= b.exit.sampled_at
                            && p.sampled_at - b.exit.sampled_at >= 30_000
                    }) && self
                        .recovery_started
                        .is_some_and(|t| now >= t && now - t >= 30_000);
                self.latest_exit = Some(p.clone());
                if stable {
                    self.baseline = Some(Baseline { local, exit: p });
                    self.incident = false;
                    self.recovery_candidate = None;
                    self.recovery_started = None;
                    self.issue = None;
                    return Some(Notice::Recovered);
                }
                if !same {
                    self.recovery_candidate = Some(Baseline { local, exit: p });
                    self.recovery_started = Some(now);
                }
            }
            // Keep the incident visible while the second exit sample is pending.
            return stale;
        }
        if self.baseline.as_ref().is_some_and(|b| b.local != local) {
            return self.fail("代理进程、设置或路由发生变化").or(stale);
        }
        if let Some(p) = exit {
            if self.baseline.as_ref().is_some_and(|b| b.exit.ip != p.ip) {
                return self.fail("出口 IP 发生变化（不展示完整地址）").or(stale);
            }
            if self
                .baseline
                .as_ref()
                .is_some_and(|b| b.exit.country != p.country)
            {
                return self.fail("出口地区发生变化").or(stale);
            }
            if self.baseline.is_none() {
                self.baseline = Some(Baseline {
                    local,
                    exit: p.clone(),
                });
            }
            self.latest_exit = Some(p);
        }
        if self
            .latest_exit
            .as_ref()
            .is_some_and(|p| now >= p.sampled_at && now - p.sampled_at <= 75_000)
        {
            self.issue = None;
        }
        stale
    }
}
