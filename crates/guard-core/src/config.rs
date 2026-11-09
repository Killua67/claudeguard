use serde::{Deserialize, Serialize};
use url::Url;

pub const POLICY_VERSION: u32 = 3;
pub const PROBE_URL: &str = "https://www.cloudflare.com/cdn-cgi/trace";

/// A local drive path, never a share, device path, relative path or stream.
pub fn windows_proxy_path(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() <= 4096
        && b.len() > 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && b[2] == b'\\'
        && path[3..].split('\\').all(|part| {
            !part.is_empty()
                && !part.ends_with([' ', '.'])
                && !part.chars().any(|c| {
                    c.is_control() || matches!(c, '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                })
        })
}

#[cfg(test)]
mod windows_path_tests {
    use super::windows_proxy_path;

    #[test]
    fn process_identity_requires_a_plain_local_drive_path() {
        assert!(windows_proxy_path(r"C:\Program Files\Proxy\代理.exe"));
        for path in [
            r"C:proxy.exe",
            r"C:/proxy.exe",
            r"\proxy.exe",
            r"\\host\share\proxy.exe",
            r"\\?\C:\proxy.exe",
            r"C:\app\..\proxy.exe",
            r"C:\proxy.exe:stream",
        ] {
            assert!(!windows_proxy_path(path), "{path}");
        }
    }
}
/// Frozen launch policy passed to the check-only child. No config file or
/// environment fallback may replace this with a different proxy profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchProfile {
    pub policy_version: u32,
    pub profile: Profile,
}
impl LaunchProfile {
    pub fn new(profile: Profile) -> Self {
        Self {
            policy_version: POLICY_VERSION,
            profile,
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.policy_version != POLICY_VERSION {
            return Err("启动规则版本不同，请重新启动检测器");
        }
        self.profile.validate()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub schema_version: u32,
    pub proxy_url: String,
    pub expected_country: Option<String>,
    pub expected_proxy_executable: String,
    pub non_mainland_timezone: bool,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            proxy_url: String::new(),
            expected_country: None,
            expected_proxy_executable: String::new(),
            non_mainland_timezone: false,
        }
    }
}
impl Profile {
    pub fn validate(&self) -> Result<(), &'static str> {
        let url = Url::parse(&self.proxy_url).map_err(|_| "代理地址无效")?;
        if self.schema_version != 1
            || url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || url.port().is_none_or(|port| port == 0)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("此版本仅支持无认证、明确端口的本机 HTTP 代理");
        }
        if self
            .expected_country
            .as_ref()
            .is_some_and(|s| s.len() != 2 || !s.bytes().all(|c| c.is_ascii_uppercase()))
        {
            return Err("地区应为两位大写代码，或不固定地区");
        }
        if self.expected_proxy_executable.len() > 4096
            || !(self.expected_proxy_executable.starts_with('/')
                || (cfg!(windows) && windows_proxy_path(&self.expected_proxy_executable)))
            || self
                .expected_proxy_executable
                .chars()
                .any(|c| c.is_control())
        {
            return Err("代理进程路径须为明确的绝对路径");
        }
        Ok(())
    }
    pub fn proxy_port(&self) -> u16 {
        Url::parse(&self.proxy_url)
            .ok()
            .and_then(|u| u.port())
            .unwrap_or(0)
    }
    pub fn curl_arguments(&self) -> Vec<String> {
        [
            "-q",
            "--proxy",
            &self.proxy_url,
            "--noproxy",
            "",
            "--proto",
            "=https",
            "--silent",
            "--show-error",
            "--fail",
            "--max-filesize",
            "65536",
            "--connect-timeout",
            "5",
            "--max-time",
            "8",
            "--write-out",
            "\nCHECK_META:%{http_code}:%{remote_ip}:%{time_total}:%{ssl_verify_result}",
            PROBE_URL,
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }
}
