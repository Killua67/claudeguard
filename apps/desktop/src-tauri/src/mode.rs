//! Parse before OS collection. Diagnostics never construct a GUI or launcher.
use guard_core::config::LaunchProfile;
pub enum Mode {
    Live,
    Demo,
    DemoFirstRun,
    ObserveOnly,
    Catalog,
    MonitorCheck,
    Diagnostics { json: bool, launch_check: bool },
    CheckOnly(LaunchProfile),
}
pub fn parse(args: &[String]) -> Result<Mode, &'static str> {
    match args {
        [] => return Ok(Mode::Live),
        [a] if a == "--demo" => return Ok(Mode::Demo),
        [a] if a == "--demo-first-run" => return Ok(Mode::DemoFirstRun),
        [a] if a == "--observe-only" => return Ok(Mode::ObserveOnly),
        [a] if a == "--list-network-dependencies" => return Ok(Mode::Catalog),
        [a] if a == "--monitor-check" => return Ok(Mode::MonitorCheck),
        [a, b, json]
            if a == "--check-for-launch" && b == "--profile-json" && json.len() <= 16_384 =>
        {
            let frozen: LaunchProfile =
                serde_json::from_str(json).map_err(|_| "启动配置无法解析")?;
            frozen.validate()?;
            return Ok(Mode::CheckOnly(frozen));
        }
        _ => {}
    }
    let unique: std::collections::HashSet<_> = args.iter().collect();
    if unique.len() == args.len()
        && !args.is_empty()
        && args
            .iter()
            .all(|a| ["--check", "--json", "--check-for-launch"].contains(&a.as_str()))
    {
        return Ok(Mode::Diagnostics {
            json: args.iter().any(|a| a == "--json"),
            launch_check: args.iter().any(|a| a == "--check-for-launch"),
        });
    }
    Err("启动参数无效；未运行任何检测或客户端")
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
    use guard_core::config::Profile;
    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|v| v.to_string()).collect()
    }
    #[test]
    fn modes_do_not_mix_and_unknown_arguments_fail_before_collection() {
        for a in [
            &["--demo", "--check-for-launch"][..],
            &["--observe-only", "--demo"],
            &["--monitor-check", "--json"],
            &["--profile-json", "{}"],
            &["--check", "--check"],
            &["--unknown"],
        ] {
            assert!(parse(&args(a)).is_err());
        }
        assert!(matches!(parse(&args(&["--demo"])), Ok(Mode::Demo)));
        assert!(matches!(
            parse(&args(&["--observe-only"])),
            Ok(Mode::ObserveOnly)
        ));
    }
    #[test]
    fn legacy_diagnostics_are_all_check_only() {
        for a in [
            &["--check"][..],
            &["--json"],
            &["--check-for-launch"],
            &["--check", "--json"],
            &["--json", "--check-for-launch"],
        ] {
            assert!(matches!(parse(&args(a)), Ok(Mode::Diagnostics { .. })));
        }
        assert!(matches!(
            parse(&args(&["--monitor-check"])),
            Ok(Mode::MonitorCheck)
        ));
    }
    #[test]
    fn frozen_profile_and_policy_are_not_replaced_by_defaults() {
        let profile = Profile {
            proxy_url: "http://127.0.0.1:9999".into(),
            ..fixture_profile()
        };
        let mut frozen = LaunchProfile::new(profile.clone());
        let args = |f: &LaunchProfile| {
            vec![
                "--check-for-launch".into(),
                "--profile-json".into(),
                serde_json::to_string(f).unwrap(),
            ]
        };
        assert!(matches!(parse(&args(&frozen)),Ok(Mode::CheckOnly(f)) if f.profile==profile));
        frozen.policy_version += 1;
        assert!(parse(&args(&frozen)).is_err());
    }
}
