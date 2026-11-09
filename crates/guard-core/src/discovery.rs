//! Read-only discovery evidence is a suggestion, never launch authorization.
use crate::config::Profile;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyCandidate {
    pub profile: Profile,
    pub source: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovery {
    pub supported: bool,
    pub candidates: Vec<ProxyCandidate>,
    pub message: String,
}
impl Discovery {
    pub fn unavailable(message: &str) -> Self {
        Self {
            supported: true,
            candidates: vec![],
            message: message.into(),
        }
    }
    pub fn unsupported() -> Self {
        Self {
            supported: false,
            candidates: vec![],
            message: "此平台尚未实现代理自动识别；不能判定通过".into(),
        }
    }
}
/// No PAC fetch, DNS resolution, environment scan, port scan or raw-value output.
pub fn system_proxy_url(raw: &str) -> Result<String, &'static str> {
    let keys = [
        "HTTPEnable",
        "HTTPProxy",
        "HTTPPort",
        "HTTPSEnable",
        "HTTPSProxy",
        "HTTPSPort",
        "ProxyAutoConfigEnable",
        "ProxyAutoDiscoveryEnable",
    ];
    let mut fields = BTreeMap::new();
    for line in raw.lines() {
        if let Some((key, value)) = line.trim().split_once(" : ") {
            if keys.contains(&key) && fields.insert(key, value.trim()).is_some() {
                return Err("系统代理存在重复配置，无法自动确认；请在设置中人工填写");
            }
        }
    }
    for key in ["ProxyAutoConfigEnable", "ProxyAutoDiscoveryEnable"] {
        if fields.get(key).is_some_and(|v| *v != "0") {
            return Err(
                "检测到 PAC 或自动代理发现；此版本不执行 PAC，请人工配置明确的本机 HTTP 代理",
            );
        }
    }
    if fields.get("HTTPEnable") != Some(&"1") || fields.get("HTTPSEnable") != Some(&"1") {
        return Err("未发现同时开启的系统 HTTP/HTTPS 代理；请开启代理软件后重试，或人工填写");
    }
    if ["HTTPProxy", "HTTPSProxy"]
        .iter()
        .any(|k| fields.get(k) != Some(&"127.0.0.1"))
    {
        return Err("系统代理不是受支持的 127.0.0.1 HTTP 代理；不解析或连接远程地址");
    }
    let port = fields
        .get("HTTPPort")
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|p| *p > 0)
        .ok_or("代理端口无效")?;
    if fields.get("HTTPSPort").and_then(|v| v.parse::<u16>().ok()) != Some(port) {
        return Err("HTTP 与 HTTPS 指向不同端口；此版本需要明确的同一代理");
    }
    Ok(format!("http://127.0.0.1:{port}"))
}

pub fn listener_pid(raw: &str, port: u16) -> Result<u32, &'static str> {
    let mut current = None;
    let mut matches = Vec::new();
    for line in raw.lines() {
        if let Some(pid) = line.strip_prefix('p') {
            current = pid.parse::<u32>().ok().filter(|p| *p > 0);
        }
        if line == format!("n127.0.0.1:{port}") {
            matches.push(current.ok_or("监听进程无法确认")?);
        }
    }
    matches.sort();
    matches.dedup();
    if matches.len() != 1 {
        return Err("本机代理端口未监听或进程归属不唯一");
    }
    Ok(matches[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    fn raw(port: u16) -> String {
        format!("HTTPEnable : 1\nHTTPProxy : 127.0.0.1\nHTTPPort : {port}\nHTTPSEnable : 1\nHTTPSProxy : 127.0.0.1\nHTTPSPort : {port}\n")
    }
    #[test]
    fn discovers_actual_nonpersonal_port_and_rejects_ambiguous_or_remote_configuration() {
        assert_eq!(
            system_proxy_url(&raw(8899)).unwrap(),
            "http://127.0.0.1:8899"
        );
        for text in [
            raw(0),
            raw(7890) + "HTTPPort : 7890\n",
            raw(7890) + "ProxyAutoConfigEnable : 1\n",
            raw(7890) + "ProxyAutoDiscoveryEnable : 1\n",
            raw(7890).replace("127.0.0.1", "remote.invalid"),
            raw(7890).replace("HTTPSPort : 7890", "HTTPSPort : 8080"),
            String::new(),
        ] {
            assert!(system_proxy_url(&text).is_err());
        }
    }
    #[test]
    fn listener_requires_one_explicit_loopback_owner() {
        assert_eq!(listener_pid("p42\nn127.0.0.1:8899\n", 8899), Ok(42));
        for raw in [
            "p42\nn*:8899",
            "p42\nn127.0.0.1:8899\np43\nn127.0.0.1:8899",
            "n127.0.0.1:8899",
            "p0\nn127.0.0.1:8899",
        ] {
            assert!(listener_pid(raw, 8899).is_err());
        }
    }
}
