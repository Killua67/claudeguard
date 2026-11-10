use crate::{diagnostics, settings};
use guard_core::config::LaunchProfile;
use std::{
    io::{BufRead, BufReader, Read, Write},
    sync::{mpsc, Mutex},
    time::Duration,
};

fn read_profile(input: &mut impl BufRead) -> Result<LaunchProfile, String> {
    let mut bytes = Vec::new();
    input
        .take(16_386)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| "启动配置未收到")?;
    if bytes.len() > 16_385 || bytes.last() != Some(&b'\n') {
        return Err("启动配置无效".into());
    }
    let profile: LaunchProfile = serde_json::from_slice(&bytes).map_err(|_| "启动配置无法解析")?;
    profile.validate().map_err(str::to_string)?;
    Ok(profile)
}

fn read_permission(input: &mut impl BufRead) -> Option<u64> {
    let mut bytes = Vec::new();
    input.take(32).read_until(b'\n', &mut bytes).ok()?;
    let number = bytes.strip_prefix(b"GO ")?.strip_suffix(b"\n")?;
    if number.is_empty() || !number.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(number).ok()?.parse().ok()
}

fn request_permission(input: &Mutex<Option<BufReader<std::io::Stdin>>>) -> Option<u64> {
    let mut input = input.lock().ok()?.take()?;
    writeln!(std::io::stdout(), "READY")
        .and_then(|_| std::io::stdout().flush())
        .ok()?;
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(read_permission(&mut input));
    });
    receive.recv_timeout(Duration::from_secs(5)).ok().flatten()
}

pub fn run() -> i32 {
    let result = (|| -> Result<(), String> {
        let parent = guard_platform::windows::guard_parent().map_err(|e| e.detail())?;
        let mut input = BufReader::new(std::io::stdin());
        let frozen = read_profile(&mut input)?;
        let profile = diagnostics::confirmed_frozen(
            &frozen,
            diagnostics::settings_path().and_then(|path| settings::load(&path)),
        )?;
        let input = Mutex::new(Some(input));
        let clock = guard_platform::SystemClock::default();
        guard_platform::windows::cli_session(
            &profile,
            &clock,
            &|| {
                guard_service::engine::scan(
                    crate::platform(false).as_ref(),
                    &profile,
                    &clock,
                    &mut |_| {},
                )
            },
            &|| {
                parent
                    .require_alive()
                    .ok()
                    .and_then(|()| request_permission(&input))
            },
        )
        .map_err(|e| e.detail())?;
        writeln!(std::io::stdout(), "STARTED")
            .and_then(|_| std::io::stdout().flush())
            .map_err(|_| "启动结果未收到")?;
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}；未确认启动");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_requires_bounded_valid_frozen_configuration() {
        for bytes in [
            &b""[..],
            &b"{}\n"[..],
            &b"not-json\n"[..],
            &[b'x'; 16_386][..],
        ] {
            assert!(read_profile(&mut std::io::Cursor::new(bytes)).is_err());
        }
        let profile = guard_core::config::Profile {
            schema_version: 1,
            proxy_url: "http://127.0.0.1:9999".into(),
            expected_country: None,
            expected_proxy_executable: r"C:\Fixture\proxy.exe".into(),
            non_mainland_timezone: false,
        };
        let frozen = LaunchProfile::new(profile);
        let json = format!("{}\n", serde_json::to_string(&frozen).unwrap());
        let parsed = read_profile(&mut std::io::Cursor::new(json)).unwrap();
        assert_eq!(parsed.profile, frozen.profile);
    }
    #[test]
    fn missing_malformed_or_disconnected_permission_cannot_launch() {
        assert_eq!(
            read_permission(&mut std::io::Cursor::new(b"GO 5000\n")),
            Some(5000)
        );
        for bytes in [
            &b""[..],
            &b"GO\n"[..],
            &b"GO 5000"[..],
            &b"go 5000\n"[..],
            &b"GO anything\n"[..],
            &b"GO +5000\n"[..],
            &b"GO 5000 \n"[..],
            &b"GO 18446744073709551616\n"[..],
            &[b'x'; 40][..],
        ] {
            assert_eq!(read_permission(&mut std::io::Cursor::new(bytes)), None);
        }
        assert_eq!(request_permission(&Mutex::new(None)), None);
    }
}
