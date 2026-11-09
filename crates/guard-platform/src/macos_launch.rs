//! Explicit manual launch backend. No inspection or automatic task calls this.
use crate::{macos::MacPlatform, *};
use block2::RcBlock;
use objc2_app_kit::{NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSString, NSURL};
use std::{fs, path::Path, time::Duration};
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
fn final_desktop_authorization(
    platform: &MacPlatform,
    authorize: &dyn Fn() -> bool,
) -> Result<(), Fault> {
    platform.ensure_desktop_not_running()?;
    if !authorize() {
        return Err(Fault::Client);
    }
    Ok(())
}
pub fn cli_script(
    checker: &Path,
    target: &Path,
    directory: &Path,
    profile: &Profile,
) -> Result<String, Fault> {
    let proxy = guard_core::proxy::ClientProxy::from_profile(profile).map_err(|_| Fault::Client)?;
    let frozen = serde_json::to_string(&guard_core::config::LaunchProfile::new(profile.clone()))
        .map_err(|_| Fault::Client)?;
    let mut paths = Vec::new();
    for p in [checker, target, directory] {
        let s = p.to_str().ok_or(Fault::Client)?;
        if !p.is_absolute() || s.chars().any(|c| c.is_control()) {
            return Err(Fault::Client);
        }
        paths.push(quote(s));
    }
    let environment = proxy
        .environment
        .iter()
        .map(|(k, v)| format!("{k}={}", quote(v)))
        .collect::<Vec<_>>()
        .join(" ");
    Ok(format!("#!/bin/zsh -f\nset -eu\ncd -- {}\n/usr/bin/codesign --verify --strict -R {} {} || exit 1\n{} --check-for-launch --profile-json {} || {{ printf '%s\\n' '启动复检失败，未运行客户端。'; exit 1; }}\nunset NODE_OPTIONS NODE_TLS_REJECT_UNAUTHORIZED ELECTRON_RUN_AS_NODE\nexec /usr/bin/env {environment} {} --settings {}\n",paths[2],quote("=anchor apple generic and certificate leaf[subject.OU] = \"Q6L2SF6YDW\""),paths[1],paths[0],quote(&frozen),paths[1],quote(&proxy.cli_settings())))
}

pub fn launch(
    platform: &MacPlatform,
    target: ClientTarget,
    profile: &Profile,
    checker: &Path,
    directory: Option<&Path>,
    authorize: &dyn Fn() -> bool,
) -> Result<(), Fault> {
    let path = platform.verify(target)?;
    let proxy = guard_core::proxy::ClientProxy::from_profile(profile).map_err(|_| Fault::Client)?;
    let config = NSWorkspaceOpenConfiguration::new();
    config.setActivates(true);
    config.setAllowsRunningApplicationSubstitution(false);
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let completion = RcBlock::new(
        move |application: *mut NSRunningApplication, error: *mut NSError| {
            let _ = sender.try_send(!application.is_null() && error.is_null());
        },
    );
    if target == ClientTarget::Desktop {
        platform.ensure_desktop_not_running()?;
        let keys: Vec<_> = proxy
            .environment
            .keys()
            .map(|s| NSString::from_str(s))
            .collect();
        let values: Vec<_> = proxy
            .environment
            .values()
            .map(|s| NSString::from_str(s))
            .collect();
        let key_refs: Vec<_> = keys.iter().map(|s| &**s).collect();
        let value_refs: Vec<_> = values.iter().map(|s| &**s).collect();
        config.setEnvironment(&NSDictionary::from_slices(&key_refs, &value_refs));
        let args: Vec<_> = proxy
            .desktop_arguments
            .iter()
            .map(|s| NSString::from_str(s))
            .collect();
        let arg_refs: Vec<_> = args.iter().map(|s| &**s).collect();
        config.setArguments(&NSArray::from_slice(&arg_refs));
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str().ok_or(Fault::Client)?));
        final_desktop_authorization(platform, authorize)?;
        NSWorkspace::sharedWorkspace().openApplicationAtURL_configuration_completionHandler(
            &url,
            &config,
            Some(&completion),
        );
    } else {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let directory = directory
            .filter(|p| p.is_absolute() && p.is_dir())
            .ok_or(Fault::Client)?;
        let home = std::env::var_os("HOME").ok_or(Fault::Client)?;
        let root = std::path::PathBuf::from(home)
            .join("Library/Application Support/ClaudeEnvironmentGuard/sessions");
        fs::create_dir_all(&root).map_err(|_| Fault::Client)?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).map_err(|_| Fault::Client)?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Fault::Client)?
            .as_nanos();
        let file = root.join(format!("start-{}-{nonce}.command", std::process::id()));
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&file)
            .map_err(|_| Fault::Client)?;
        output
            .write_all(cli_script(checker, &path, directory, profile)?.as_bytes())
            .map_err(|_| Fault::Client)?;
        if !authorize() {
            let _ = fs::remove_file(&file);
            return Err(Fault::Client);
        }
        let terminal = NSURL::fileURLWithPath(&NSString::from_str(
            "/System/Applications/Utilities/Terminal.app",
        ));
        let script =
            NSURL::fileURLWithPath(&NSString::from_str(file.to_str().ok_or(Fault::Client)?));
        // Readiness is checked after all preparation, directly before submission.
        if !authorize() {
            let _ = fs::remove_file(&file);
            return Err(Fault::Client);
        }
        NSWorkspace::sharedWorkspace()
            .openURLs_withApplicationAtURL_configuration_completionHandler(
                &NSArray::from_slice(&[&*script]),
                &terminal,
                &config,
                Some(&completion),
            );
    }
    if receiver
        .recv_timeout(Duration::from_secs(15))
        .unwrap_or(false)
    {
        Ok(())
    } else {
        Err(Fault::Client)
    }
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
#[allow(clippy::field_reassign_with_default)] // One-field policy fixtures.
mod tests {
    use super::*;
    #[test]
    fn slow_final_registry_lookup_cannot_outlive_the_launch_ticket() {
        use std::sync::{
            atomic::{AtomicU64, Ordering},
            Arc,
        };
        struct SlowRegistry(Arc<AtomicU64>);
        impl crate::macos::ApplicationRegistry for SlowRegistry {
            fn has_running_bundle(&self, _: &str) -> Result<bool, Fault> {
                self.0.store(25_000, Ordering::SeqCst);
                Ok(false)
            }
        }
        let elapsed = Arc::new(AtomicU64::new(0));
        let platform = MacPlatform::with_dependencies(
            Arc::new(crate::command::SystemRunner),
            Arc::new(SlowRegistry(elapsed.clone())),
        );
        assert!(
            final_desktop_authorization(&platform, &|| elapsed.load(Ordering::SeqCst) < 20_000)
                .is_err()
        );
    }
    #[test]
    fn cli_recheck_and_exec_share_the_nondefault_frozen_profile() {
        let mut profile = fixture_profile();
        profile.proxy_url = "http://127.0.0.1:9999".into();
        let script = cli_script(
            Path::new("/tmp/checker's app"),
            Path::new("/tmp/safe fixture"),
            Path::new("/tmp/$(touch impossible)"),
            &profile,
        )
        .unwrap();
        let frozen =
            serde_json::to_string(&guard_core::config::LaunchProfile::new(profile)).unwrap();
        assert!(script.contains(&format!(
            "--check-for-launch --profile-json {}",
            quote(&frozen)
        )));
        assert!(script.contains("HTTP_PROXY='http://127.0.0.1:9999'"));
        assert!(!script.contains(":17890"));
        assert!(script.contains("'/tmp/checker'\\''s app'"));
        assert!(script.starts_with("#!/bin/zsh -f\nset -eu\n"));
        assert!(script.contains("cd -- '/tmp/$(touch impossible)'"));
        assert!(
            script.find("--check-for-launch").unwrap() < script.find("exec /usr/bin/env").unwrap()
        );
    }
    #[test]
    fn script_rejects_control_characters_relative_paths_and_invalid_profile() {
        for path in ["relative", "/tmp/bad\npath"] {
            assert!(cli_script(
                Path::new(path),
                Path::new("/tmp/fixture"),
                Path::new("/tmp"),
                &fixture_profile()
            )
            .is_err());
        }
        let mut profile = fixture_profile();
        profile.proxy_url = "http://example.org:9999".into();
        assert!(cli_script(
            Path::new("/tmp/checker"),
            Path::new("/tmp/fixture"),
            Path::new("/tmp"),
            &profile
        )
        .is_err());
    }
}
