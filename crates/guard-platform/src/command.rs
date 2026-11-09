use crate::Fault;
use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
pub struct Output {
    pub code: i32,
    pub stdout: String,
}
pub trait CommandRunner: Send + Sync {
    fn run(&self, exe: &str, args: &[String], timeout: Duration) -> Result<Output, Fault>;
}
#[derive(Default)]
pub struct SystemRunner;
impl CommandRunner for SystemRunner {
    fn run(&self, exe: &str, args: &[String], timeout: Duration) -> Result<Output, Fault> {
        // No shell, inherited proxy variables, startup files, or arbitrary executable.
        let allowed = [
            "/usr/sbin/scutil",
            "/usr/sbin/lsof",
            "/bin/ps",
            "/usr/sbin/netstat",
            "/sbin/ifconfig",
            "/usr/bin/curl",
            "/usr/bin/codesign",
        ];
        if !allowed.contains(&exe) {
            return Err(Fault::Local);
        }
        let mut child = Command::new(exe)
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("LANG", "en_US.UTF-8")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| Fault::Local)?;
        let stdout = child.stdout.take().ok_or(Fault::Local)?;
        let reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let start = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|_| Fault::Local)? {
                break status;
            }
            if start.elapsed() >= timeout {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(if exe == "/usr/bin/curl" {
                    Fault::Transport(28)
                } else {
                    Fault::Local
                });
            }
            thread::sleep(Duration::from_millis(10));
        };
        let bytes = reader
            .join()
            .map_err(|_| Fault::Local)?
            .map_err(|_| Fault::Local)?;
        if bytes.len() > 1024 * 1024 {
            return Err(Fault::Local);
        }
        Ok(Output {
            code: status.code().unwrap_or(-1),
            stdout: String::from_utf8(bytes).map_err(|_| Fault::Local)?,
        })
    }
}
