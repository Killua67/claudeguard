//! No GUI, notifications, launch state, or client execution in these entrypoints.
pub use crate::branding::APP_ID;
use crate::{mode::Mode, platform, settings};
use guard_core::{config::Profile, domain::Report};
use guard_platform::{Clock, SystemClock};
pub fn settings_path() -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .map(|p| p.join("Library/Application Support"));
    #[cfg(not(target_os = "macos"))]
    let root = std::env::var_os("APPDATA").map(std::path::PathBuf::from);
    Ok(root
        .ok_or("无法确认本工具设置目录")?
        .join(APP_ID)
        .join("settings.json"))
}
fn profile() -> Result<Profile, String> {
    let saved = settings::load(&settings_path()?)?;
    saved.require_confirmed()?;
    Ok(saved.profile)
}
pub(crate) fn confirmed_frozen(
    frozen: &guard_core::config::LaunchProfile,
    saved: Result<settings::Settings, String>,
) -> Result<Profile, String> {
    saved?.require_confirmed()?;
    frozen.validate().map_err(str::to_string)?;
    Ok(frozen.profile.clone())
}
pub fn run(mode: &Mode) -> Option<i32> {
    if !cfg!(any(target_os = "macos", windows))
        && matches!(
            mode,
            Mode::CheckOnly(_)
                | Mode::Diagnostics {
                    launch_check: true,
                    ..
                }
        )
    {
        eprintln!("此平台仅支持只读预检，尚未支持启动复核");
        return Some(2);
    }
    if matches!(mode, Mode::Catalog) {
        println!(
            "{}",
            include_str!("../../../../resources/network-dependencies.json")
        );
        return Some(0);
    }
    let (profile, json, launch_check) = match mode {
        Mode::CheckOnly(f) => (
            confirmed_frozen(f, settings_path().and_then(|path| settings::load(&path))),
            false,
            true,
        ),
        Mode::Diagnostics { json, launch_check } => (profile(), *json, *launch_check),
        Mode::MonitorCheck => {
            let result = profile().and_then(|p| {
                let platform = platform(false);
                let clock = SystemClock::default();
                let initial = platform.local(&p).map_err(|e| e.detail())?;
                let probe = platform.probe(&p, &clock).map_err(|e| e.detail())?;
                if platform.local(&p).map_err(|e| e.detail())? != initial {
                    return Err("采样期间配置改变".into());
                }
                if p.expected_country
                    .as_ref()
                    .is_some_and(|c| *c != probe.country)
                {
                    return Err("出口地区与偏好不同".into());
                }
                Ok(probe.country)
            });
            return Some(match result {
                Ok(country) => {
                    println!("单次代理出口采样成功：Cloudflare {country}；IPv4/IPv6 客户端覆盖未验证，未执行客户端，不代表完整预检通过。");
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            });
        }
        _ => return None,
    };
    let profile = match profile {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return Some(1);
        }
    };
    let clock = SystemClock::default();
    let report =
        guard_service::engine::scan(platform(false).as_ref(), &profile, &clock, &mut |_| {});
    if json {
        println!("{}", json_report(&report));
    } else {
        for check in &report.checks {
            println!("{:?} {}：{}", check.status, check.id.label(), check.detail);
        }
        println!("代理预检模式；IPv4/IPv6 实际客户端覆盖未验证，不提供防直连保护；未执行客户端。");
    }
    Some(
        if if launch_check {
            report.fresh(clock.now_ms())
        } else {
            report.complete()
        } {
            0
        } else {
            1
        },
    )
}
pub fn json_report(report: &Report) -> String {
    // Preserve the old array/name/status/detail shape and add stable IDs/timing.
    let checks:Vec<_>=report.checks.iter().map(|c|serde_json::json!({"id":c.id,"name":c.id.label(),"status":c.status,"detail":c.detail,"elapsedMs":c.elapsed_ms,"blocksLaunch":c.blocks_launch(),"preflightMode":"proxy_preflight","coverageVerified":false})).collect();
    serde_json::to_string_pretty(&checks).expect("fixed serializable report")
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frozen_checks_require_confirmation_but_never_replace_the_frozen_profile() {
        let frozen = guard_core::config::LaunchProfile::new(fixture_profile());
        assert!(confirmed_frozen(&frozen, Ok(settings::Settings::default())).is_err());
        assert!(confirmed_frozen(&frozen, Err("bad file".into())).is_err());
        let mut saved = settings::Settings::demo();
        saved.profile.proxy_url = "http://127.0.0.1:9123".into();
        assert_eq!(
            confirmed_frozen(&frozen, Ok(saved)).unwrap(),
            frozen.profile
        );
    }
    #[test]
    fn json_preserves_legacy_shape_but_never_exports_baseline_or_launch_authority() {
        let clock = SystemClock::default();
        let report = guard_service::engine::scan(
            &guard_platform::demo::DemoPlatform::default(),
            &fixture_profile(),
            &clock,
            &mut |_| {},
        );
        let text = json_report(&report);
        let rows: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 15);
        assert_eq!(rows[4]["name"], "代理出口");
        assert_eq!(rows[4]["id"], "exit_region");
        for field in ["status", "detail", "elapsedMs"] {
            assert!(rows[4].get(field).is_some());
        }
        for field in [
            "baseline",
            "canLaunch",
            "203.0.113",
            "owner_pid",
            "proxy_fingerprint",
        ] {
            assert!(!text.contains(field));
        }
    }
}

#[cfg(test)]
mod preflight_report_tests {
    use super::*;
    use guard_core::domain::{Check, RuleId, Status};
    #[test]
    fn legacy_json_array_keeps_coverage_scope_and_canonical_blocking_impact() {
        let report = Report {
            checks: vec![
                Check::new(RuleId::Ipv6, Status::Warning, "coverage unknown"),
                Check::new(RuleId::HttpsTls, Status::Fail, "proxy failed"),
            ],
            checked_at: 100,
            baseline: None,
        };
        let json: serde_json::Value = serde_json::from_str(&json_report(&report)).unwrap();
        let array = json.as_array().unwrap();
        assert_eq!(array[0]["preflightMode"], "proxy_preflight");
        assert_eq!(array[0]["coverageVerified"], false);
        assert_eq!(array[0]["blocksLaunch"], false);
        assert_eq!(array[1]["blocksLaunch"], true);
        assert!(array.iter().all(|c| c.get("name").is_some()));
        assert!(!json_report(&report).contains("baseline"));
    }
}
