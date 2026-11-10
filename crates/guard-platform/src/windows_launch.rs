use super::{client, WindowsPlatform};
use crate::*;
use guard_core::{config::LaunchProfile, proxy::ClientProxy};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io::{BufRead, BufReader, Read, Write},
    mem::size_of,
    os::windows::{ffi::OsStrExt, process::CommandExt},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::{
        SystemInformation::GetTickCount64,
        Threading::{
            CreateProcessW, CREATE_NEW_CONSOLE, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT,
            PROCESS_INFORMATION, STARTUPINFOW,
        },
    },
};

fn wide(value: &OsStr) -> Result<Vec<u16>, Fault> {
    let mut units: Vec<_> = value.encode_wide().collect();
    if units.contains(&0) {
        return Err(Fault::Client);
    }
    units.push(0);
    Ok(units)
}

fn quote(value: &OsStr) -> Result<Vec<u16>, Fault> {
    let units = wide(value)?;
    let mut quoted = vec![b'"' as u16];
    let mut slashes = 0;
    for unit in &units[..units.len() - 1] {
        if *unit == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        if *unit == b'"' as u16 {
            quoted.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
        } else {
            quoted.extend(std::iter::repeat_n(b'\\' as u16, slashes));
        }
        slashes = 0;
        quoted.push(*unit);
    }
    quoted.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    quoted.push(b'"' as u16);
    Ok(quoted)
}

fn environment(
    inherited: impl IntoIterator<Item = (OsString, OsString)>,
    proxy: &ClientProxy,
) -> Result<Vec<u16>, Fault> {
    let mut values = BTreeMap::new();
    for (key, value) in inherited {
        let normalized = key.to_string_lossy().to_uppercase();
        if [
            "NODE_OPTIONS",
            "NODE_TLS_REJECT_UNAUTHORIZED",
            "ELECTRON_RUN_AS_NODE",
        ]
        .contains(&normalized.as_str())
        {
            continue;
        }
        values.insert(normalized, (key, value));
    }
    for (key, value) in &proxy.environment {
        values.insert(
            key.to_uppercase(),
            (OsString::from(key.to_uppercase()), OsString::from(value)),
        );
    }
    let mut block = Vec::new();
    for (_, (key, value)) in values {
        let mut key = wide(&key)?;
        key.pop();
        block.extend(key);
        block.push(b'=' as u16);
        block.extend(wide(&value)?);
    }
    block.push(0);
    Ok(block)
}

struct Process(windows_sys::Win32::Foundation::HANDLE);
impl Drop for Process {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn create_process(
    entry: &Path,
    arguments: &[String],
    directory: &Path,
    proxy: &ClientProxy,
    console: bool,
    authorize: &dyn Fn() -> Result<(), Fault>,
) -> Result<Process, Fault> {
    let application = wide(entry.as_os_str())?;
    let directory = wide(directory.as_os_str())?;
    let mut command = quote(entry.as_os_str())?;
    for argument in arguments {
        command.push(b' ' as u16);
        command.extend(quote(OsStr::new(argument))?);
    }
    command.push(0);
    let environment = environment(std::env::vars_os(), proxy)?;
    let startup = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    let flags = CREATE_UNICODE_ENVIRONMENT | if console { CREATE_NEW_CONSOLE } else { 0 };
    authorize()?;
    if unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            flags,
            environment.as_ptr().cast(),
            directory.as_ptr(),
            &startup,
            &mut process,
        )
    } == 0
    {
        return Err(Fault::Client);
    }
    unsafe {
        CloseHandle(process.hThread);
    }
    Ok(Process(process.hProcess))
}

fn bounded_line(input: &mut impl Read) -> Result<String, Fault> {
    let mut line = Vec::new();
    let mut reader = BufReader::new(input.take(32));
    reader
        .read_until(b'\n', &mut line)
        .map_err(|_| Fault::Client)?;
    if line.last() != Some(&b'\n') {
        return Err(Fault::Client);
    }
    String::from_utf8(line).map_err(|_| Fault::Client)
}

struct SessionChild(Child);
impl Drop for SessionChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn submission_tick() -> u64 {
    unsafe { GetTickCount64() }
}

fn valid_deadline(deadline: u64, now: u64) -> bool {
    now < deadline && deadline - now <= 5_000
}

fn session(
    checker: &Path,
    directory: &Path,
    profile: &Profile,
    authorize: &dyn Fn() -> bool,
) -> Result<(), Fault> {
    let child = Command::new(checker)
        .arg("--windows-cli-session")
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|_| Fault::Client)?;
    let mut child = SessionChild(child);
    let mut input = child.0.stdin.take().ok_or(Fault::Client)?;
    let mut output = child.0.stdout.take().ok_or(Fault::Client)?;
    let frozen =
        serde_json::to_string(&LaunchProfile::new(profile.clone())).map_err(|_| Fault::Client)?;
    writeln!(input, "{frozen}").map_err(|_| Fault::Client)?;
    input.flush().map_err(|_| Fault::Client)?;
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        for _ in 0..2 {
            if send.send(bounded_line(&mut output)).is_err() {
                break;
            }
        }
    });
    let ready = receive
        .recv_timeout(Duration::from_secs(15))
        .map_err(|_| Fault::Client)??;
    if ready != "READY\n" || !authorize() {
        return Err(Fault::Client);
    }
    writeln!(input, "GO {}", submission_tick() + 5_000).map_err(|_| Fault::Client)?;
    input.flush().map_err(|_| Fault::Client)?;
    let submitted = receive
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| Fault::Client)??;
    if submitted != "STARTED\n" || !child.0.wait().map_err(|_| Fault::Client)?.success() {
        return Err(Fault::Client);
    }
    Ok(())
}

pub(super) fn launch(
    target: ClientTarget,
    profile: &Profile,
    checker: &Path,
    directory: Option<&Path>,
    authorize: &dyn Fn() -> bool,
) -> Result<(), Fault> {
    let proxy = ClientProxy::from_profile(profile).map_err(|_| Fault::Client)?;
    if target == ClientTarget::Cli {
        let directory = directory
            .filter(|p| p.is_absolute() && p.is_dir())
            .ok_or(Fault::Client)?;
        if !checker.is_absolute() || !authorize() {
            return Err(Fault::Client);
        }
        return session(checker, directory, profile, authorize);
    }
    let executable = client::verify(target)?;
    let directory = executable.image.path.parent().ok_or(Fault::Client)?;
    create_process(
        &executable.entry,
        &proxy.desktop_arguments,
        directory,
        &proxy,
        false,
        &|| {
            client::ensure_desktop_not_running(&executable)?;
            if authorize() {
                Ok(())
            } else {
                Err(Fault::Client)
            }
        },
    )
    .map(|_| ())
}

fn final_session_authorization(
    platform: &dyn Platform,
    profile: &Profile,
    report: &Report,
    clock: &dyn Clock,
    authorize: &dyn Fn() -> Option<u64>,
    tick: &dyn Fn() -> u64,
    parent_alive: &dyn Fn() -> bool,
) -> Result<(), Fault> {
    if !report.fresh(clock.now_ms()) {
        return Err(Fault::Client);
    }
    let deadline = authorize().ok_or(Fault::Client)?;
    if !valid_deadline(deadline, tick()) {
        return Err(Fault::Client);
    }
    let current = platform.local(profile)?;
    if !report.baseline.as_ref().is_some_and(|b| b.local == current)
        || !parent_alive()
        || !report.fresh(clock.now_ms())
        || !valid_deadline(deadline, tick())
    {
        return Err(Fault::Drift);
    }
    Ok(())
}

pub fn cli_session(
    profile: &Profile,
    clock: &dyn Clock,
    scan: &dyn Fn() -> Report,
    authorize: &dyn Fn() -> Option<u64>,
) -> Result<(), Fault> {
    let parent = client::guard_parent()?;
    let proxy = ClientProxy::from_profile(profile).map_err(|_| Fault::Client)?;
    let directory = std::env::current_dir().map_err(|_| Fault::Client)?;
    let executable = client::verify(ClientTarget::Cli)?;
    let report = scan();
    create_process(
        &executable.entry,
        &["--settings".into(), proxy.cli_settings()],
        &directory,
        &proxy,
        true,
        &|| {
            final_session_authorization(
                &WindowsPlatform,
                profile,
                &report,
                clock,
                authorize,
                &submission_tick,
                &|| parent.require_alive().is_ok(),
            )
        },
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        sync::atomic::{AtomicU64, Ordering},
    };
    fn profile() -> Profile {
        Profile {
            schema_version: 1,
            proxy_url: "http://127.0.0.1:9999".into(),
            expected_country: None,
            expected_proxy_executable: r"C:\Fixture\proxy.exe".into(),
            non_mainland_timezone: false,
        }
    }
    #[test]
    fn quoting_preserves_quotes_backslashes_unicode_and_shell_metacharacters() {
        for (input, expected) in [
            ("a b", "\"a b\""),
            ("x\\", "\"x\\\\\""),
            ("a\"b", "\"a\\\"b\""),
            ("目录 & %PATH%", "\"目录 & %PATH%\""),
        ] {
            let units = quote(OsStr::new(input)).unwrap();
            assert_eq!(String::from_utf16(&units).unwrap(), expected);
        }
        assert!(quote(OsStr::new("bad\0path")).is_err());
    }
    #[test]
    fn proxy_environment_collapses_windows_case_and_removes_unsafe_overrides() {
        let proxy = ClientProxy::from_profile(&profile()).unwrap();
        let inherited = [
            ("Http_Proxy", "old"),
            ("NO_PROXY", "*"),
            ("Node_Options", "unsafe"),
            ("NODE_TLS_REJECT_UNAUTHORIZED", "0"),
            ("electron_run_as_node", "1"),
            ("PATH", "fixture"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let block = environment(inherited, &proxy).unwrap();
        assert!(block.ends_with(&[0, 0]));
        let text = String::from_utf16(&block).unwrap();
        assert_eq!(text.matches("HTTP_PROXY=").count(), 1);
        assert!(text.contains("HTTP_PROXY=http://127.0.0.1:9999\0"));
        assert!(text.contains("NO_PROXY=localhost,127.0.0.1,::1\0"));
        for key in [
            "NODE_OPTIONS",
            "NODE_TLS_REJECT_UNAUTHORIZED",
            "ELECTRON_RUN_AS_NODE",
            "old",
            "unsafe",
        ] {
            assert!(!text.contains(key));
        }
        assert!(text.contains("PATH=fixture\0"));
    }
    #[test]
    fn protocol_accepts_only_bounded_complete_lines() {
        assert_eq!(bounded_line(&mut &b"READY\n"[..]).unwrap(), "READY\n");
        assert!(bounded_line(&mut &b"READY"[..]).is_err());
        assert!(bounded_line(&mut &[b'x'; 40][..]).is_err());
    }
    struct TestClock(AtomicU64);
    impl Clock for TestClock {
        fn now_ms(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
        fn wall_seconds(&self) -> f64 {
            1800000000.0
        }
    }
    #[test]
    fn delayed_session_authorization_and_rejected_handshake_cannot_submit() {
        let platform = crate::demo::DemoPlatform::default();
        let clock = TestClock(AtomicU64::new(100));
        let profile = profile();
        let report = Report {
            checks: RuleId::ALL
                .iter()
                .map(|id| Check::new(*id, Status::Pass, "fixture"))
                .collect(),
            checked_at: 100,
            baseline: Some(Baseline {
                local: platform.local(&profile).unwrap(),
                exit: platform.probe(&profile, &clock).unwrap(),
            }),
        };
        let tick = Cell::new(0);
        let called = Cell::new(false);
        assert!(final_session_authorization(
            &platform,
            &profile,
            &report,
            &clock,
            &|| {
                called.set(true);
                None
            },
            &|| tick.get(),
            &|| true
        )
        .is_err());
        assert!(called.get());
        assert!(final_session_authorization(
            &platform,
            &profile,
            &report,
            &clock,
            &|| Some(5_000),
            &|| tick.get(),
            &|| true
        )
        .is_ok());
        assert!(final_session_authorization(
            &platform,
            &profile,
            &report,
            &clock,
            &|| Some(5_000),
            &|| tick.get(),
            &|| false
        )
        .is_err());
        assert!(final_session_authorization(
            &platform,
            &profile,
            &report,
            &clock,
            &|| Some(5_000),
            &|| tick.get(),
            &|| {
                tick.set(5_000);
                true
            }
        )
        .is_err());
        tick.set(0);
        assert!(final_session_authorization(
            &platform,
            &profile,
            &report,
            &clock,
            &|| {
                clock.0.store(25_000, Ordering::SeqCst);
                Some(5_000)
            },
            &|| tick.get(),
            &|| true
        )
        .is_err());
        clock.0.store(100, Ordering::SeqCst);
        let mut incomplete = report;
        incomplete.checks.pop();
        assert!(final_session_authorization(
            &platform,
            &profile,
            &incomplete,
            &clock,
            &|| panic!("blocked report cannot request authorization"),
            &|| tick.get(),
            &|| true
        )
        .is_err());
    }
    #[test]
    fn shared_deadline_rejects_delayed_go_and_extended_leases() {
        assert!(valid_deadline(5_000, 0));
        assert!(!valid_deadline(5_000, 5_000));
        assert!(!valid_deadline(5_000, 6_000));
        assert!(!valid_deadline(10_000, 0));
        assert!(!valid_deadline(0, 0));
    }
    #[test]
    fn harmless_process_fixture() {
        let Some(output) = std::env::var_os("CLAUDEGUARD_FIXTURE_OUTPUT") else {
            return;
        };
        let value = serde_json::json!({"proxy": std::env::var("HTTP_PROXY").unwrap(), "bypass": std::env::var("NO_PROXY").unwrap(), "directory": std::env::current_dir().unwrap()});
        std::fs::write(output, value.to_string()).unwrap();
    }
    #[test]
    fn native_submission_uses_only_the_fixture_with_explicit_environment_and_directory() {
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("guard-spawn-目录 & {}-{nonce}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let output = root.join("result.json");
        let mut proxy = ClientProxy::from_profile(&profile()).unwrap();
        proxy.environment.insert(
            "CLAUDEGUARD_FIXTURE_OUTPUT".into(),
            output.to_str().unwrap().into(),
        );
        let executable = std::env::current_exe().unwrap();
        let args = [
            "--exact".into(),
            "windows::native::launch::tests::harmless_process_fixture".into(),
        ];
        assert!(
            create_process(&executable, &args, &root, &proxy, false, &|| Err(
                Fault::Client
            ))
            .is_err()
        );
        assert!(!output.exists());
        let process = create_process(&executable, &args, &root, &proxy, false, &|| Ok(())).unwrap();
        assert_eq!(unsafe { WaitForSingleObject(process.0, 5_000) }, 0);
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(value["proxy"], "http://127.0.0.1:9999");
        assert_eq!(value["bypass"], "localhost,127.0.0.1,::1");
        assert_eq!(Path::new(value["directory"].as_str().unwrap()), root);
        drop(process);
        std::fs::remove_file(output).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn sleeping_process_fixture() {
        if std::env::var_os("CLAUDEGUARD_SLEEP_FIXTURE").is_some() {
            std::thread::sleep(Duration::from_secs(30));
        }
    }
    #[test]
    fn aborting_an_owned_session_terminates_and_reaps_the_fixture() {
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "windows::native::launch::tests::sleeping_process_fixture",
            ])
            .env("CLAUDEGUARD_SLEEP_FIXTURE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .unwrap();
        let start = std::time::Instant::now();
        drop(SessionChild(child));
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
