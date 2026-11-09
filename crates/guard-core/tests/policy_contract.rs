#![allow(clippy::field_reassign_with_default)] // Fixtures deliberately mutate one policy input at a time.
use guard_core::{config::*, domain::*, launch::*, monitoring::*, policy::*};
use std::collections::BTreeMap;
fn raw(ip: &str, country: &str) -> String {
    format!("ip={ip}\nloc={country}\nts=1800000000\nCHECK_META:200:127.0.0.1:0.2:0")
}
fn probe(at: u64) -> Probe {
    parse_probe(&raw("203.0.113.1", "US"), 0, 1800000000.0, 1800000001.0, at).unwrap()
}
fn local(pid: u32) -> LocalSnapshot {
    LocalSnapshot {
        proxy_fingerprint: "fixture".into(),
        owner_pid: pid,
        routes: "en0".into(),
    }
}
fn baseline(at: u64) -> Baseline {
    Baseline {
        local: local(1),
        exit: probe(at),
    }
}
fn report(at: u64) -> Report {
    Report {
        checks: RuleId::ALL
            .iter()
            .map(|id| {
                Check::new(
                    *id,
                    if id.core() {
                        Status::Pass
                    } else {
                        Status::Info
                    },
                    "fixture",
                )
            })
            .collect(),
        checked_at: at,
        baseline: Some(baseline(at)),
    }
}
fn proxy() -> &'static str {
    "<dictionary> {\nHTTPEnable : 1\nHTTPProxy : 127.0.0.1\nHTTPPort : 17890\nHTTPSEnable : 1\nHTTPSProxy : 127.0.0.1\nHTTPSPort : 17890\nProxyAutoConfigEnable : 0\n}"
}
#[test]
fn profile_accepts_legacy_without_changing_it() {
    let p = fixture_profile();
    assert!(p.validate().is_ok());
    assert_eq!(p.proxy_port(), 17890);
    assert_eq!(p.expected_country.as_deref(), Some("US"));
    assert!(p.non_mainland_timezone);
}
#[test]
fn profile_rejects_external_urls_credentials_paths_and_controls() {
    for url in [
        "http://example.com:80",
        "http://user:secret@127.0.0.1:1234",
        "http://127.0.0.1:1234/path",
        "http://127.0.0.1:1234/?token=secret",
        "https://127.0.0.1:1234",
        "http://127.0.0.1",
    ] {
        let p = Profile {
            proxy_url: url.into(),
            ..fixture_profile()
        };
        assert!(p.validate().is_err(), "{url}")
    }
    let p = Profile {
        expected_proxy_executable: "/a\n/b".into(),
        ..fixture_profile()
    };
    assert!(p.validate().is_err());
}
#[test]
fn config_unknown_fields_do_not_add_active_hosts() {
    let json = serde_json::json!({"schemaVersion":1,"proxyUrl":"http://127.0.0.1:1234","expectedCountry":null,"expectedProxyExecutable":"/fixture","nonMainlandTimezone":false,"probeUrl":"https://forbidden.invalid"});
    assert!(serde_json::from_value::<Profile>(json).is_err());
}
#[test]
fn probe_arguments_are_fixed_and_tls_strict() {
    let args = fixture_profile().curl_arguments();
    assert_eq!(args[0], "-q");
    assert_eq!(args.last().unwrap(), PROBE_URL);
    assert_eq!(args.iter().filter(|s| s.starts_with("https://")).count(), 1);
    for bad in ["--insecure", "--location", "--proxy-anyauth"] {
        assert!(!args.contains(&bad.into()))
    }
    assert!(args.windows(2).any(|p| p == ["--noproxy", ""]));
}
#[test]
fn proxy_requires_both_protocols_and_unambiguous_values() {
    let (a, b) = proxy_checks(proxy(), &fixture_profile());
    assert_eq!(a.status, Status::Pass);
    assert_eq!(b.status, Status::Pass);
    for text in [
        proxy().replace("HTTPSEnable : 1", "HTTPSEnable : 0"),
        format!("{}\nHTTPPort : 1234", proxy()),
        proxy().replace("17890", "1234"),
    ] {
        assert_eq!(
            proxy_checks(&text, &fixture_profile()).0.status,
            Status::Fail
        )
    }
}
#[test]
fn pac_and_bypass_are_not_silently_allowed() {
    for text in [
        proxy().replace("ProxyAutoConfigEnable : 0", "ProxyAutoConfigEnable : 1"),
        format!("{}\nExcludeSimpleHostnames : 1", proxy()),
        format!(
            "{}\nExceptionsList : <array> {{\n0 : *.example.com\n}}",
            proxy()
        ),
        format!("{}\nExceptionsList : malformed", proxy()),
    ] {
        assert_eq!(
            proxy_checks(&text, &fixture_profile()).1.status,
            Status::Fail
        )
    }
}
#[test]
fn loopback_bypass_is_explicit() {
    let text = format!(
        "{}\nExceptionsList : <array> {{\n0 : localhost\n1 : 127.0.0.1\n2 : ::1\n}}",
        proxy()
    );
    assert_eq!(
        proxy_checks(&text, &fixture_profile()).1.status,
        Status::Pass
    )
}
#[test]
fn accepts_known_local_bypass_without_allowing_public_exceptions() {
    let text = format!(
        "{}\nExceptionsList : <array> {{\n0 : 127.0.0.1\n1 : 192.168.0.0/16\n2 : 10.0.0.0/8\n3 : 172.16.0.0/12\n4 : localhost\n5 : *.local\n6 : <local>\n}}",
        proxy()
    );
    assert_eq!(
        proxy_checks(&text, &fixture_profile()).1.status,
        Status::Pass
    );
    for raw in [
        text.replace("6 : <local>", "6 : <local>\n7 : *.crashlytics.com"),
        text.replace("10.0.0.0/8", "0.0.0.0/0"),
        text.replace("*.local", "*.local.example.com"),
        text.replace("6 : <local>", "5 : <local>"),
        format!("{text} : *.example.com"),
        format!("{text}\nExceptionsList : <array> {{\n0 : *.example.com\n}}"),
    ] {
        assert_eq!(
            proxy_checks(&raw, &fixture_profile()).1.status,
            Status::Fail
        );
    }
}
#[test]
fn probe_rejects_duplicates_empty_values_and_nulls() {
    let good = raw("203.0.113.1", "US");
    for text in [
        format!("{good}\nloc="),
        format!("{good}\nCHECK_META:200:127.0.0.1:0.2:0"),
        raw("203.0.113.1\0BAD", "US"),
        raw("bad", "US"),
        raw("203.0.113.1", "USA"),
    ] {
        assert!(parse_probe(&text, 0, 1.0, 2.0, 100).is_err())
    }
}
#[test]
fn probe_rejects_direct_tls_failures_redirects_nan_and_curl_errors() {
    let good = raw("203.0.113.1", "US");
    for text in [
        good.replace("CHECK_META:200:127.0.0.1", "CHECK_META:200:203.0.113.1"),
        good.replace(":0.2:0", ":0.2:1"),
        good.replace("CHECK_META:200", "CHECK_META:302"),
        good.replace(":0.2:", ":NaN:"),
    ] {
        assert!(parse_probe(&text, 0, 1.0, 2.0, 100).is_err())
    }
    assert!(parse_probe(&good, 28, 1.0, 2.0, 100).is_err());
}
#[test]
fn stability_warns_on_rotating_addresses() {
    let a = probe(100);
    let mut b = a.clone();
    b.ip = "203.0.113.2".parse().unwrap();
    assert_eq!(
        stability(&[a.clone(), a.clone(), a.clone()], &fixture_profile()).status,
        Status::Pass
    );
    assert_eq!(
        stability(&[a.clone(), b, a], &fixture_profile()).status,
        Status::Warning
    )
}
#[test]
fn region_is_a_configured_preference() {
    let mut p = probe(100);
    p.country = "SG".into();
    assert_eq!(region(&p, &fixture_profile()).status, Status::Fail);
    assert_eq!(
        region(
            &p,
            &Profile {
                expected_country: None,
                ..fixture_profile()
            }
        )
        .status,
        Status::Pass
    )
}
#[test]
fn timezone_preserves_personal_preference() {
    assert_eq!(
        timezone(
            Some("Asia/Singapore"),
            "Asia/Singapore",
            true,
            &fixture_profile()
        )
        .status,
        Status::Pass
    );
    assert_eq!(
        timezone(
            Some("Asia/Shanghai"),
            "Asia/Shanghai",
            true,
            &fixture_profile()
        )
        .status,
        Status::Warning
    );
    assert_eq!(
        timezone(
            Some("Asia/Shanghai"),
            "Asia/Shanghai",
            true,
            &Profile {
                non_mainland_timezone: false,
                ..fixture_profile()
            }
        )
        .status,
        Status::Pass
    );
    assert_eq!(
        timezone(Some("Asia/Singapore"), "UTC", true, &fixture_profile()).status,
        Status::Warning
    )
}
#[test]
fn environment_shows_names_not_credentials_or_values() {
    let mut vars = BTreeMap::new();
    vars.insert(
        "ANTHROPIC_BASE_URL".into(),
        "https://private.invalid/secret".into(),
    );
    vars.insert("ANTHROPIC_API_KEY".into(), "private-token".into());
    let check = environment(&vars);
    assert_eq!(check.status, Status::Fail);
    assert!(check.detail.contains("ANTHROPIC_BASE_URL"));
    assert!(!check.detail.contains("private"));
    assert!(!check.detail.contains("API_KEY"));
}
#[test]
fn report_requires_all_stable_ids_once() {
    let mut r = report(100);
    assert!(r.complete());
    r.checks[1].id = r.checks[0].id;
    assert!(!r.complete());
    let mut r = report(100);
    r.checks.pop();
    assert!(!r.complete())
}
#[test]
fn labels_do_not_affect_authorization() {
    let mut r = report(100);
    for c in &mut r.checks {
        c.detail = "Different language".into()
    }
    assert!(r.complete());
}
#[test]
fn warnings_skips_and_bad_core_info_prevent_launch() {
    for status in [Status::Warning, Status::Skipped, Status::Fail, Status::Info] {
        let mut r = report(100);
        r.checks[0].status = status;
        assert!(!r.fresh(100))
    }
}
#[test]
fn serialized_report_does_not_export_authority_or_ip() {
    let text = serde_json::to_string(&ReportView::from(&report(100))).unwrap();
    for secret in [
        "203.0.113",
        "checkedAt",
        "baseline",
        "sampledAt",
        "owner_pid",
        "fixture-route",
    ] {
        assert!(!text.contains(secret))
    }
}
#[test]
fn freshness_uses_both_report_and_actual_exit() {
    assert!(report(100).fresh(19999));
    assert!(!report(100).fresh(20100));
    let mut r = report(10000);
    r.baseline.as_mut().unwrap().exit.sampled_at = 1;
    assert!(!r.fresh(20001));
    assert!(!report(100).fresh(99));
}
#[test]
fn no_scan_can_create_manual_intent() {
    let mut gate = LaunchGate::default();
    assert!(gate.consume(&report(100), 100).is_err());
}
#[test]
fn intent_is_single_use_even_when_recheck_fails() {
    let mut gate = LaunchGate::default();
    gate.request(ClientTarget::Cli).unwrap();
    assert!(gate.request(ClientTarget::Desktop).is_err());
    assert!(gate.consume(&report(100), 20100).is_err());
    assert!(gate.consume(&report(100), 100).is_err());
    gate.request(ClientTarget::Cli).unwrap();
    assert_eq!(gate.consume(&report(100), 100), Ok(ClientTarget::Cli));
    assert!(gate.consume(&report(100), 100).is_err());
}
#[test]
fn async_ticket_rechecks_generation_and_deadline() {
    let ticket = LaunchTicket {
        revision: 7,
        checked_at: 100,
    };
    assert!(ticket.permits(7, true, 110));
    assert!(!ticket.permits(8, true, 110));
    assert!(!ticket.permits(7, false, 110));
    assert!(!ticket.permits(7, true, 20100));
}
#[test]
fn observer_never_unlocks_a_launch_latch() {
    let mut m = Monitor::default();
    m.start(Some(baseline(100000)), None, 100000);
    m.fail("fixture");
    for t in (105000..=160000).step_by(5000) {
        m.observe(local(1), Some(probe(t)), &fixture_profile(), t);
    }
    assert!(!m.launch_healthy);
    assert!(m.active);
}
#[test]
fn failed_scan_still_detects_changes_after_recovery() {
    let mut m = Monitor::default();
    m.start(None, Some("scan failed"), 100000);
    m.observe(local(1), Some(probe(105000)), &fixture_profile(), 105000);
    for t in (110000..=130000).step_by(5000) {
        m.observe(local(1), None, &fixture_profile(), t);
    }
    assert_eq!(
        m.observe(local(1), Some(probe(135000)), &fixture_profile(), 135000),
        Some(Notice::Recovered)
    );
    let mut changed = probe(140000);
    changed.ip = "203.0.113.2".parse().unwrap();
    assert!(m
        .observe(local(1), Some(changed), &fixture_profile(), 140000)
        .is_some());
    assert!(!m.issue.unwrap().contains("203.0.113"));
}
#[test]
fn failed_probe_attempts_are_throttled() {
    let mut m = Monitor::default();
    m.start(None, Some("failed"), 100000);
    assert!(m.needs_exit(100000));
    m.attempted(true, 100000);
    assert!(!m.needs_exit(105000));
    assert!(m.needs_exit(130000));
}
#[test]
fn repeated_failures_notify_once() {
    let mut m = Monitor::default();
    m.start(Some(baseline(100000)), None, 100000);
    assert!(m.fail("one").is_some());
    for _ in 0..20 {
        assert!(m.fail("changed reason").is_none())
    }
    assert!(m.active);
}
#[test]
fn recovery_requires_continuous_local_observations() {
    let mut m = Monitor::default();
    m.start(Some(baseline(100000)), None, 100000);
    m.fail("down");
    m.observe(local(1), Some(probe(130000)), &fixture_profile(), 130000);
    assert!(m
        .observe(local(1), Some(probe(160000)), &fixture_profile(), 160000)
        .is_none());
}
#[test]
fn local_recovery_cannot_replace_fresh_exit() {
    let mut m = Monitor::default();
    m.start(Some(baseline(100000)), None, 100000);
    m.fail("down");
    m.observe(local(1), None, &fixture_profile(), 105000);
    assert!(m.issue.is_some());
}
#[test]
fn local_drift_and_expiry_latch() {
    let mut m = Monitor::default();
    m.start(Some(baseline(100000)), None, 100000);
    m.observe(local(2), None, &fixture_profile(), 105000);
    assert!(!m.launch_healthy);
    let mut m = Monitor::default();
    m.start(Some(baseline(100000)), None, 100000);
    assert!(m.freshness(121000).is_some());
    assert!(!m.launch_healthy);
}
#[test]
fn first_good_sample_during_recovery_keeps_issue_visible() {
    let mut m = Monitor::default();
    m.start(Some(baseline(100000)), None, 100000);
    m.fail("代理监听失效");
    assert!(m
        .observe(local(1), Some(probe(105000)), &fixture_profile(), 105000)
        .is_none());
    assert!(
        m.issue.is_some(),
        "one good sample must not claim stable recovery"
    );
    assert!(!m.launch_healthy);
}
#[test]
fn new_valid_proxy_process_can_recover_observation_without_unlocking_launch() {
    let mut m = Monitor::default();
    m.start(Some(baseline(100000)), None, 100000);
    assert!(matches!(
        m.observe(local(2), None, &fixture_profile(), 105000),
        Some(Notice::Problem(_))
    ));
    assert!(m
        .observe(local(2), Some(probe(110000)), &fixture_profile(), 110000)
        .is_none());
    for now in (115000..140000).step_by(5000) {
        assert!(m.observe(local(2), None, &fixture_profile(), now).is_none());
    }
    assert_eq!(
        m.observe(local(2), Some(probe(140000)), &fixture_profile(), 140000),
        Some(Notice::Recovered)
    );
    assert!(!m.launch_healthy);
}
#[test]
fn slow_post_probe_work_cannot_fake_thirty_seconds_of_exit_observations() {
    let mut m = Monitor::default();
    m.start(Some(baseline(100000)), None, 100000);
    m.fail("fixture");
    m.observe(local(1), Some(probe(140000)), &fixture_profile(), 140000);
    for now in (145000..=165000).step_by(5000) {
        m.observe(local(1), None, &fixture_profile(), now);
    }
    // A ten-second delay after a fresh probe must not extend its observation span.
    assert!(m
        .observe(local(1), Some(probe(160000)), &fixture_profile(), 170000)
        .is_none());
    assert!(m.recovering());
    assert!(m.issue.is_some());
    assert_eq!(
        m.observe(local(1), Some(probe(175000)), &fixture_profile(), 175000),
        Some(Notice::Recovered)
    );
    assert!(!m.launch_healthy);
}

#[test]
fn new_install_has_no_personal_proxy_or_preferences() {
    let profile = Profile::default();
    assert!(profile.proxy_url.is_empty());
    assert!(profile.expected_proxy_executable.is_empty());
    assert_eq!(profile.expected_country, None);
    assert!(!profile.non_mainland_timezone);
    assert!(profile.validate().is_err());
}

fn fixture_profile() -> Profile {
    Profile {
        schema_version: 1,
        proxy_url: "http://127.0.0.1:17890".into(),
        expected_country: Some("US".into()),
        expected_proxy_executable: "/fixture/proxy".into(),
        non_mainland_timezone: true,
    }
}

#[test]
fn proxy_exit_observations_accept_both_ipv4_and_ipv6_without_requiring_host_ipv6_off() {
    for ip in ["203.0.113.7", "2001:db8::7"] {
        let p = parse_probe(&raw(ip, "US"), 0, 1800000000.0, 1800000001.0, 100_000).unwrap();
        assert_eq!(p.ip.to_string(), ip);
        assert_eq!(region(&p, &fixture_profile()).status, Status::Pass);
        let probes = (0..3).map(|_| p.clone()).collect::<Vec<_>>();
        assert_eq!(stability(&probes, &fixture_profile()).status, Status::Pass);
    }
}
