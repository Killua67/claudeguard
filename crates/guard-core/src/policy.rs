use crate::{
    config::Profile,
    domain::{Check, Probe, RuleId as R, Status as S},
};
use std::{collections::BTreeMap, net::IpAddr};

pub fn dns(raw: &str) -> Check {
    let servers: std::collections::BTreeSet<IpAddr> = raw
        .lines()
        .filter_map(|line| {
            let (key, address) = line.trim().split_once(" : ")?;
            if !key.starts_with("nameserver[") || !key.ends_with(']') {
                return None;
            }
            address.trim().parse().ok()
        })
        .collect();
    if servers.is_empty() {
        Check::new(
            R::Dns,
            S::Warning,
            "未取得可识别的解析器；未进行DNS泄漏测试",
        )
    } else {
        Check::new(
            R::Dns,
            S::Info,
            format!(
                "识别到 {} 个本机解析器；不代表代理端解析路径",
                servers.len()
            ),
        )
    }
}

pub fn proxy_checks(raw: &str, profile: &Profile) -> (Check, Check) {
    let mut values = BTreeMap::new();
    let mut duplicate = false;
    let keys = [
        "HTTPEnable",
        "HTTPProxy",
        "HTTPPort",
        "HTTPSEnable",
        "HTTPSProxy",
        "HTTPSPort",
        "ProxyAutoConfigEnable",
        "ProxyAutoDiscoveryEnable",
        "ExcludeSimpleHostnames",
    ];
    for line in raw.lines() {
        if let Some((k, v)) = line.trim().split_once(" : ") {
            if keys.contains(&k) && values.insert(k, v.trim()).is_some() {
                duplicate = true;
            }
        }
    }
    let port = profile.proxy_port().to_string();
    let expected = [
        ("HTTPEnable", "1"),
        ("HTTPProxy", "127.0.0.1"),
        ("HTTPPort", port.as_str()),
        ("HTTPSEnable", "1"),
        ("HTTPSProxy", "127.0.0.1"),
        ("HTTPSPort", port.as_str()),
    ];
    let valid = !duplicate && expected.iter().all(|(k, v)| values.get(k) == Some(v));
    let proxy = Check::new(
        R::SystemProxy,
        if valid { S::Pass } else { S::Fail },
        if valid {
            "HTTP/HTTPS 指向配置中的本机代理"
        } else {
            "系统代理与配置不一致或无法完整确认"
        },
    );
    let safe = !duplicate
        && !raw.is_empty()
        && [
            "ProxyAutoConfigEnable",
            "ProxyAutoDiscoveryEnable",
            "ExcludeSimpleHostnames",
        ]
        .iter()
        .all(|k| values.get(k).is_none_or(|v| *v == "0"))
        && raw
            .split_once("ExceptionsList")
            .is_none_or(|(_, tail)| allowed_proxy_exceptions(tail));
    (
        proxy,
        Check::new(
            R::ProxyBypass,
            if safe { S::Pass } else { S::Fail },
            if safe {
                "PAC/自动发现关闭，旁路匹配已允许的本机和内网条目"
            } else {
                "代理自动配置、旁路或重复字段无法确认"
            },
        ),
    )
}

fn allowed_proxy_exceptions(tail: &str) -> bool {
    const ALLOWED: &[&str] = &[
        "localhost",
        "127.0.0.1",
        "::1",
        "192.168.0.0/16",
        "10.0.0.0/8",
        "172.16.0.0/12",
        "*.local",
        "<local>",
    ];
    let mut lines = tail.lines().map(str::trim).filter(|line| !line.is_empty());
    if lines.next() != Some(": <array> {") {
        return false;
    }
    let mut index = 0;
    while let Some(line) = lines.next() {
        if line == "}" {
            return !lines.any(|line| line.contains("ExceptionsList"));
        }
        let Some((key, value)) = line.split_once(" : ") else {
            return false;
        };
        if key.parse::<usize>() != Ok(index) || !ALLOWED.contains(&value.trim()) {
            return false;
        }
        index += 1;
    }
    false
}

pub fn parse_probe(
    raw: &str,
    code: i32,
    started: f64,
    ended: f64,
    sampled_at: u64,
) -> Result<Probe, &'static str> {
    if code != 0 {
        return Err("出口请求失败或超时；未回退直连");
    }
    let mut meta = None;
    for line in raw.lines() {
        if line.starts_with("CHECK_META:") && meta.replace(line).is_some() {
            return Err("出口元数据重复");
        }
    }
    let m: Vec<_> = meta.ok_or("缺少出口元数据")?.split(':').collect();
    if m.len() != 5 || m[1] != "200" || m[2] != "127.0.0.1" || m[4] != "0" {
        return Err("代理对端、HTTP或TLS验证未通过");
    }
    let elapsed = m[3].parse::<f64>().map_err(|_| "无效耗时")?;
    if !elapsed.is_finite() || !(0.0..30.0).contains(&elapsed) {
        return Err("无效耗时");
    }
    parse_verified_trace(raw, (elapsed * 1000.0) as u64, started, ended, sampled_at)
}

/// Parses the fixed Cloudflare response only after the platform has verified
/// the proxy tunnel, HTTPS peer and HTTP 200. Does not perform transport checks.
pub fn parse_verified_trace(
    raw: &str,
    duration_ms: u64,
    started: f64,
    ended: f64,
    sampled_at: u64,
) -> Result<Probe, &'static str> {
    let mut fields = BTreeMap::new();
    for line in raw.lines().filter(|line| !line.starts_with("CHECK_META:")) {
        if let Some((k, v)) = line.split_once('=') {
            if fields.insert(k, v).is_some() {
                return Err("出口字段重复");
            }
        }
    }
    let country = fields
        .get("loc")
        .filter(|v| v.len() == 2 && v.bytes().all(|b| b.is_ascii_uppercase()))
        .ok_or("无效地区")?
        .to_string();
    let ip: IpAddr = fields
        .get("ip")
        .ok_or("缺少出口")?
        .parse()
        .map_err(|_| "无效出口地址")?;
    let server_time = fields
        .get("ts")
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && (978307200.0..4102444800.0).contains(v));
    Ok(Probe {
        country,
        ip,
        duration_ms,
        server_time,
        local_midpoint: (started + ended) / 2.0,
        sampled_at,
    })
}
pub fn region(probe: &Probe, profile: &Profile) -> Check {
    let valid = profile
        .expected_country
        .as_ref()
        .is_none_or(|v| *v == probe.country);
    Check::new(
        R::ExitRegion,
        if valid { S::Pass } else { S::Fail },
        format!(
            "Cloudflare 返回 {}，当前预期 {}；不展示或保存完整出口 IP",
            probe.country,
            profile.expected_country.as_deref().unwrap_or("不固定")
        ),
    )
}
pub fn stability(probes: &[Probe], profile: &Profile) -> Check {
    if probes.len() != 3 {
        return Check::new(R::ExitStability, S::Fail, "未完成三次有效采样");
    }
    if probes.iter().any(|p| {
        profile
            .expected_country
            .as_ref()
            .is_some_and(|c| *c != p.country)
    }) {
        return Check::new(R::ExitStability, S::Fail, "地区与配置偏好不同");
    }
    Check::new(
        R::ExitStability,
        if probes.iter().all(|p| p.ip == probes[0].ip) {
            S::Pass
        } else {
            S::Warning
        },
        "完成三次短时出口比较；仅代表采样窗口",
    )
}
pub fn environment(vars: &BTreeMap<String, String>) -> Check {
    let mut conflicts = Vec::new();
    for (k, v) in vars {
        let bad = match k.as_str() {
            "ANTHROPIC_BASE_URL" => {
                !v.is_empty()
                    && v != "https://api.anthropic.com"
                    && v != "https://api.anthropic.com/"
            }
            "CLAUDE_CODE_USE_BEDROCK"
            | "CLAUDE_CODE_USE_VERTEX"
            | "CLAUDE_CODE_USE_FOUNDRY"
            | "CLAUDE_CODE_PROCESS_WRAPPER" => {
                !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
            }
            "NODE_TLS_REJECT_UNAUTHORIZED" => v == "0",
            "NODE_OPTIONS" | "NODE_EXTRA_CA_CERTS" | "SSL_CERT_FILE" | "SSL_CERT_DIR" => {
                !v.is_empty()
            }
            _ => false,
        };
        if bad {
            conflicts.push(k.as_str())
        }
    }
    Check::new(
        R::Environment,
        if conflicts.is_empty() {
            S::Pass
        } else {
            S::Fail
        },
        if conflicts.is_empty() {
            "白名单变量未见冲突；未读取账号或客户端配置".into()
        } else {
            format!("需核实变量名称：{}", conflicts.join("、"))
        },
    )
}
pub fn timezone(system: Option<&str>, process: &str, valid: bool, profile: &Profile) -> Check {
    let mainland = profile.non_mainland_timezone
        && [
            "Asia/Shanghai",
            "Asia/Urumqi",
            "PRC",
            "Asia/Chongqing",
            "Asia/Chungking",
            "Asia/Harbin",
            "Asia/Kashgar",
        ]
        .contains(&process);
    let status = if !valid || system != Some(process) || mainland {
        S::Warning
    } else {
        S::Pass
    };
    Check::new(
        R::Timezone,
        status,
        "核对系统与进程时区及用户偏好；不据此判断账号或地区资格",
    )
}
