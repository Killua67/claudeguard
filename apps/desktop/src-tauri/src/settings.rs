//! Only this app's settings; no Claude, shell, account or credential files.
use guard_core::config::Profile;
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub profile: Profile,
    pub notifications_enabled: bool,
    pub setup_confirmed: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 2,
            profile: Profile::default(),
            notifications_enabled: false,
            setup_confirmed: false,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 2 {
            return Err("设置版本不受支持，原文件已保留".into());
        }
        if !self.setup_confirmed && self.profile == Profile::default() {
            return Ok(());
        }
        self.profile.validate().map_err(str::to_string)
    }
    pub fn require_confirmed(&self) -> Result<(), String> {
        if !self.setup_confirmed {
            return Err("请先确认本机代理；尚未发起外网检测，也不能启动客户端".into());
        }
        self.profile.validate().map_err(str::to_string)
    }
    pub fn demo() -> Self {
        Self {
            profile: guard_platform::demo::profile(),
            setup_confirmed: true,
            ..Self::default()
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacySettings {
    version: u32,
    profile: Profile,
    notifications_enabled: bool,
}

pub fn load(path: &Path) -> Result<Settings, String> {
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings::default())
        }
        Err(_) => return Err("无法读取本工具的设置".into()),
    };
    if !metadata.is_file() || metadata.len() > 16_384 {
        return Err("设置文件类型或大小异常，原文件已保留".into());
    }
    let bytes = fs::read(path).map_err(|_| "无法读取本工具的设置")?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "设置格式异常，原文件已保留")?;
    let settings = if value.get("version").and_then(|v| v.as_u64()) == Some(1) {
        let old: LegacySettings =
            serde_json::from_value(value).map_err(|_| "旧版设置格式异常，原文件已保留")?;
        if old.version != 1 {
            return Err("旧版设置版本异常".into());
        }
        old.profile.validate().map_err(str::to_string)?;
        Settings {
            version: 2,
            profile: old.profile,
            notifications_enabled: old.notifications_enabled,
            setup_confirmed: true,
        }
    } else {
        serde_json::from_value(value).map_err(|_| "设置格式异常，原文件已保留")?
    };
    settings.validate()?;
    // Migration is in memory; loading never rewrites the user's existing file.
    Ok(settings)
}
pub fn save(path: &Path, settings: &Settings) -> Result<(), String> {
    settings.validate()?;
    let parent = path.parent().ok_or("设置目录无效")?;
    fs::create_dir_all(parent).map_err(|_| "无法建立设置目录")?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "系统时钟无效")?
        .as_nanos();
    let temp = parent.join(format!("settings-{}-{nonce}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temp).map_err(|_| "无法写入设置")?;
        let bytes = serde_json::to_vec_pretty(settings).map_err(|_| "设置编码失败")?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "无法保存设置")?;
        fs::rename(&temp, path).map_err(|_| "无法替换设置文件")
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result.map_err(str::to_string)
}
#[cfg(test)]
#[allow(clippy::field_reassign_with_default)] // One-field policy fixtures.
mod tests {
    use super::*;
    #[test]
    fn first_run_is_unconfirmed_and_reading_never_creates_a_file() {
        let dir = root();
        let path = dir.join("settings.json");
        let settings = load(&path).unwrap();
        assert!(!settings.setup_confirmed);
        assert!(settings.require_confirmed().is_err());
        assert_eq!(settings.profile, Profile::default());
        assert!(!path.exists());
        assert!(!dir.exists());
    }
    #[test]
    fn legacy_settings_are_preserved_field_for_field_and_not_rewritten() {
        let dir = root();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let profile = Profile {
            proxy_url: "http://127.0.0.1:9123".into(),
            expected_proxy_executable: "/Applications/Personal Proxy.app/Contents/MacOS/proxy"
                .into(),
            expected_country: Some("SG".into()),
            non_mainland_timezone: true,
            schema_version: 1,
        };
        let bytes = serde_json::to_vec(
            &serde_json::json!({"version":1,"profile":profile,"notificationsEnabled":true}),
        )
        .unwrap();
        fs::write(&path, &bytes).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.profile, profile);
        assert!(loaded.notifications_enabled);
        assert!(loaded.setup_confirmed);
        assert_eq!(loaded.version, 2);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn v2_missing_confirmation_and_bad_legacy_never_become_configured() {
        let dir = root();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        for bytes in [
            br#"{"version":2,"profile":{},"notificationsEnabled":true}"#.as_slice(),
            br#"{"version":1,"profile":{},"notificationsEnabled":true}"#,
        ] {
            fs::write(&path, bytes).unwrap();
            assert!(load(&path).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::remove_dir_all(dir).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn broken_settings_symlink_is_not_silently_replaced_with_defaults() {
        let dir = root();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::os::unix::fs::symlink(dir.join("missing-target"), &path).unwrap();
        assert!(load(&path).is_err());
        assert!(path.symlink_metadata().unwrap().file_type().is_symlink());
        fs::remove_dir_all(dir).unwrap();
    }
    fn root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "guard-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    #[test]
    fn roundtrip_and_bad_version_is_preserved() {
        let dir = root();
        let path = dir.join("settings.json");
        let mut s = Settings::default();
        s.profile = guard_platform::demo::profile();
        s.setup_confirmed = true;
        s.profile.proxy_url = "http://127.0.0.1:9999".into();
        save(&path, &s).unwrap();
        assert_eq!(load(&path).unwrap().profile, s.profile);
        let bad = b"{\"version\":999}";
        fs::write(&path, bad).unwrap();
        assert!(load(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), bad);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn invalid_profile_never_replaces_a_saved_file() {
        let dir = root();
        let path = dir.join("settings.json");
        let mut s = Settings::default();
        save(&path, &s).unwrap();
        let before = fs::read(&path).unwrap();
        s.profile.proxy_url = "http://example.org:8888".into();
        assert!(save(&path, &s).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(dir).unwrap();
    }
}
