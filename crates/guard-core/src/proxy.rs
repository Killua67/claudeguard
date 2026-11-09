//! One validated proxy configuration for CLI, Desktop and harmless transport
//! simulations. It configures cooperating clients; it is not network isolation.
use crate::config::Profile;
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
pub struct ClientProxy {
    pub environment: BTreeMap<String, String>,
    pub desktop_arguments: Vec<String>,
}
impl ClientProxy {
    pub fn from_profile(profile: &Profile) -> Result<Self, &'static str> {
        profile.validate()?;
        let mut environment = BTreeMap::new();
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            environment.insert(key.into(), profile.proxy_url.clone());
        }
        for key in ["NO_PROXY", "no_proxy"] {
            environment.insert(key.into(), "localhost,127.0.0.1,::1".into());
        }
        Ok(Self {
            environment,
            desktop_arguments: vec![
                format!("--proxy-server={}", profile.proxy_url),
                "--proxy-bypass-list=localhost;127.0.0.1;[::1]".into(),
            ],
        })
    }
    pub fn cli_settings(&self) -> String {
        let mut environment = self.environment.clone();
        environment.insert("NODE_TLS_REJECT_UNAUTHORIZED".into(), "1".into());
        serde_json::json!({"env": environment}).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> Profile {
        Profile {
            schema_version: 1,
            proxy_url: "http://127.0.0.1:17890".into(),
            expected_country: None,
            expected_proxy_executable: "/fixture/proxy".into(),
            non_mainland_timezone: false,
        }
    }
    #[test]
    fn cli_and_desktop_share_proxy_and_only_loopback_bypass_without_direct_fallback() {
        let profile = profile();
        let proxy = ClientProxy::from_profile(&profile).unwrap();
        assert_eq!(proxy.environment.len(), 8);
        for (key, value) in &proxy.environment {
            if key.eq_ignore_ascii_case("no_proxy") {
                assert_eq!(value, "localhost,127.0.0.1,::1");
            } else {
                assert_eq!(value, &profile.proxy_url);
            }
        }
        assert_eq!(
            proxy.desktop_arguments[0],
            "--proxy-server=http://127.0.0.1:17890"
        );
        assert!(!proxy.desktop_arguments.join(" ").contains("direct://"));
        let settings: serde_json::Value = serde_json::from_str(&proxy.cli_settings()).unwrap();
        for (key, value) in &proxy.environment {
            assert_eq!(settings["env"][key], *value);
        }
        assert_eq!(settings["env"]["NODE_TLS_REJECT_UNAUTHORIZED"], "1");
    }
    #[test]
    fn unconfirmed_invalid_or_remote_proxy_cannot_create_launch_configuration() {
        assert!(ClientProxy::from_profile(&Profile::default()).is_err());
        for url in [
            "http://remote.invalid:8080",
            "http://127.0.0.1:8080/path",
            "http://user:secret@127.0.0.1:8080",
        ] {
            let mut p = profile();
            p.proxy_url = url.into();
            assert!(ClientProxy::from_profile(&p).is_err());
        }
    }
}
