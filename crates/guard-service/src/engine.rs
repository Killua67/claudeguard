use guard_core::{config::Profile, domain::*, policy};
use guard_platform::{Clock, Platform};

pub fn scan(
    platform: &dyn Platform,
    profile: &Profile,
    clock: &dyn Clock,
    progress: &mut dyn FnMut(Check),
) -> Report {
    if profile.validate().is_err() {
        let checks: Vec<_> = RuleId::ALL
            .into_iter()
            .map(|id| {
                Check::new(
                    id,
                    Status::Skipped,
                    "代理配置尚未确认或无效，未读取本机或发起外网请求",
                )
            })
            .collect();
        for check in &checks {
            progress(check.clone());
        }
        return Report {
            checks,
            checked_at: clock.now_ms(),
            baseline: None,
        };
    }
    let mut checks = Vec::new();
    let mut initial = None;
    let mut probes = Vec::new();
    let mut baseline = None;
    for id in RuleId::ALL {
        let started = clock.now_ms();
        let mut check = match id {
            RuleId::HttpsTls => {
                let ready = checks.iter().all(|c: &Check| c.status == Status::Pass);
                if !ready {
                    Check::new(id, Status::Skipped, "本地前置未通过，未发外网请求")
                } else {
                    match platform.local(profile).and_then(|local| {
                        initial = Some(local);
                        platform.probe(profile, clock)
                    }) {
                        Ok(p) => {
                            probes.push(p);
                            Check::new(id, Status::Pass, "显式本机代理HTTPS 200/TLS验证通过")
                        }
                        Err(e) => Check::new(id, Status::Fail, e.detail()),
                    }
                }
            }
            RuleId::ExitRegion => probes
                .first()
                .map(|p| policy::region(p, profile))
                .unwrap_or_else(|| Check::new(id, Status::Skipped, "未取得有效出口，不另发请求")),
            RuleId::ExitStability => {
                if probes.len() != 1
                    || profile
                        .expected_country
                        .as_ref()
                        .is_some_and(|c| *c != probes[0].country)
                {
                    Check::new(id, Status::Skipped, "出口尚未符合配置")
                } else {
                    let mut failure = None;
                    for _ in 0..2 {
                        match platform.probe(profile, clock) {
                            Ok(p) => probes.push(p),
                            Err(error) => {
                                failure = Some(error.detail());
                                break;
                            }
                        }
                    }
                    failure.map_or_else(
                        || policy::stability(&probes, profile),
                        |detail| Check::new(id, Status::Fail, detail),
                    )
                }
            }
            RuleId::Clock => match probes.first() {
                Some(p) => match p.server_time {
                    Some(ts) => Check::new(
                        id,
                        if (ts - p.local_midpoint).abs()
                            > 120.0_f64.max(p.duration_ms as f64 / 1000.0 * 3.0)
                        {
                            Status::Warning
                        } else {
                            Status::Info
                        },
                        "HTTPS粗略时间参考；不调整系统时间",
                    ),
                    None => Check::new(id, Status::Skipped, "未获得可用服务器时钟参考；不进行校时"),
                },
                None => Check::new(id, Status::Skipped, "未取得时间参考"),
            },
            RuleId::FinalRecheck => {
                let ready = checks.iter().all(|c| c.status != Status::Fail)
                    && checks
                        .iter()
                        .any(|c| c.id == RuleId::ExitStability && c.status == Status::Pass);
                if !ready {
                    Check::new(id, Status::Skipped, "前置条件未通过，不发最终外网请求")
                } else {
                    match platform.local(profile).and_then(|local| {
                        if initial.as_ref() != Some(&local) {
                            return Err(guard_platform::Fault::Drift);
                        }
                        let p = platform.probe(profile, clock)?;
                        if probes
                            .first()
                            .is_none_or(|first| first.ip != p.ip || first.country != p.country)
                            || platform.local(profile)? != local
                        {
                            return Err(guard_platform::Fault::Drift);
                        }
                        baseline = Some(Baseline { local, exit: p });
                        Ok(())
                    }) {
                        Ok(()) => {
                            Check::new(id, Status::Pass, "已复核配置、进程和出口，基线仅存内存")
                        }
                        Err(e) => Check::new(id, Status::Fail, e.detail()),
                    }
                }
            }
            _ => platform.inspect(id, profile, clock),
        };
        check.elapsed_ms = clock.now_ms().saturating_sub(started);
        progress(check.clone());
        checks.push(check);
    }
    Report {
        checks,
        checked_at: clock.now_ms(),
        baseline,
    }
}
