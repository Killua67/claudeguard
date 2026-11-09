//! Portable policy fixtures; no OS calls, sockets or clients.
use guard_core::{domain::*, ipv6::*, launch::LaunchGate};
fn observation(
    state: LinkState,
    usable: usize,
    unavailable: usize,
    routes: &[&str],
) -> Observation {
    Observation {
        interfaces: vec![Interface {
            name: "fixture0".into(),
            state,
            usable_public_addresses: usable,
            unavailable_public_addresses: unavailable,
        }],
        external_route_interfaces: routes.iter().map(|s| (*s).into()).collect(),
    }
}
#[test]
fn unknown_empty_coverage_stays_warning_without_claiming_pass() {
    assert_eq!(Observation::default().check().status, Status::Warning);
    assert_eq!(unavailable().status, Status::Warning);
}
#[test]
fn usable_or_unknown_link_public_addresses_remain_explicit_coverage_advisories() {
    for state in [LinkState::Active, LinkState::Unknown] {
        let check = observation(state, 2, 0, &[]).check();
        assert_eq!(check.status, Status::Warning);
        assert!(check.detail.contains("代理覆盖：尚未验证"));
        assert!(check.detail.contains("非阻断提醒"));
        assert!(!check.detail.contains("已发生泄漏"));
    }
}
#[test]
fn inactive_and_unavailable_addresses_are_explained_but_routes_still_warn() {
    for (state, usable, unavailable) in [(LinkState::Inactive, 2, 0), (LinkState::Active, 0, 2)] {
        assert_eq!(
            observation(state, usable, unavailable, &[]).check().status,
            Status::Info
        );
        assert_eq!(
            observation(state, usable, unavailable, &["fixture0"])
                .check()
                .status,
            Status::Warning
        );
    }
}
#[test]
fn virtual_interfaces_and_route_only_paths_never_prove_coverage() {
    let mut o = observation(LinkState::Unknown, 0, 0, &["utun7"]);
    o.interfaces[0].name = "utun7".into();
    assert_eq!(o.check().status, Status::Warning);
    assert!(o
        .check()
        .detail
        .contains("1 个接口存在公网范围目标的候选路由"));
}
#[test]
fn absence_is_an_observation_rather_than_coverage_pass() {
    let check = observation(LinkState::Active, 0, 0, &[]).check();
    assert_eq!(check.status, Status::Info);
    assert!(check.detail.contains("未观察到"));
    assert!(check.detail.contains("不保证未来"));
}
#[test]
fn proxy_preflight_allows_coverage_advisory_only_after_a_fresh_complete_scan() {
    let o = observation(LinkState::Active, 1, 0, &[]);
    let mut report = Report {
        checks: RuleId::ALL
            .into_iter()
            .map(|id| {
                if id == RuleId::Ipv6 {
                    o.check()
                } else {
                    Check::new(
                        id,
                        if id.core() {
                            Status::Pass
                        } else {
                            Status::Info
                        },
                        "fixture",
                    )
                }
            })
            .collect(),
        checked_at: 100_000,
        baseline: Some(Baseline {
            local: LocalSnapshot {
                proxy_fingerprint: "fixture".into(),
                owner_pid: 1,
                routes: "fixture".into(),
            },
            exit: Probe {
                country: "US".into(),
                ip: "203.0.113.1".parse().unwrap(),
                duration_ms: 1,
                server_time: None,
                local_midpoint: 0.,
                sampled_at: 100_000,
            },
        }),
    };
    for check in [o.check(), unavailable()] {
        *report
            .checks
            .iter_mut()
            .find(|c| c.id == RuleId::Ipv6)
            .unwrap() = check;
        assert!(report.complete());
        assert!(report.fresh(100_000));
        let mut gate = LaunchGate::default();
        gate.request(ClientTarget::Cli).unwrap();
        assert!(gate.consume(&report, 100_000).is_ok());
        gate.request(ClientTarget::Desktop).unwrap();
        assert!(gate.consume(&report, 100_000).is_ok());
    }
}
#[test]
fn normalized_evidence_detects_capability_changes_without_address_or_order_noise() {
    let a = observation(LinkState::Inactive, 1, 0, &[]);
    let b = observation(LinkState::Active, 1, 0, &[]);
    assert_ne!(a.fingerprint(), b.fingerprint());
    let mut c = b.clone();
    c.interfaces.push(Interface {
        name: "en9".into(),
        state: LinkState::Inactive,
        usable_public_addresses: 0,
        unavailable_public_addresses: 0,
    });
    let mut d = c.clone();
    d.interfaces.reverse();
    assert_eq!(c.fingerprint(), d.fingerprint());
}

#[test]
fn ipv6_exception_never_accepts_other_warnings_failures_skips_or_missing_baselines() {
    for id in RuleId::ALL {
        for status in [Status::Warning, Status::Fail, Status::Skipped] {
            let c = Check::new(id, status, "fixture");
            assert_eq!(
                c.blocks_launch(),
                !(id == RuleId::Ipv6 && status == Status::Warning)
            );
        }
    }
    for status in [Status::Pass, Status::Info, Status::Warning] {
        let c = Check::new(RuleId::Ipv6, status, "fixture");
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json["blocksLaunch"], false);
    }
    let forged = serde_json::json!({"id":"https_tls","status":"fail","detail":"fixture","elapsedMs":0,"blocksLaunch":false});
    let c: Check = serde_json::from_value(forged).unwrap();
    assert!(c.blocks_launch());
    assert_eq!(serde_json::to_value(c).unwrap()["blocksLaunch"], true);
    let r = Report {
        checks: RuleId::ALL
            .into_iter()
            .map(|id| {
                Check::new(
                    id,
                    if id == RuleId::Ipv6 {
                        Status::Warning
                    } else if id.core() {
                        Status::Pass
                    } else {
                        Status::Info
                    },
                    "fixture",
                )
            })
            .collect(),
        checked_at: 100_000,
        baseline: None,
    };
    assert!(r.complete());
    assert!(!r.fresh(100_000));
    let mut gate = LaunchGate::default();
    gate.request(ClientTarget::Cli).unwrap();
    assert!(gate.consume(&r, 100_000).is_err());
}
