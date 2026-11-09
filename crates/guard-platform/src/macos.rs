use crate::{
    command::{CommandRunner, SystemRunner},
    *,
};
use guard_core::policy;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
const SIGNATURE: &str = "=anchor apple generic and certificate leaf[subject.OU] = \"Q6L2SF6YDW\"";
const DESKTOP: &str = "/Applications/Claude.app";
pub const ENV_KEYS: [&str; 11] = [
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_PROCESS_WRAPPER",
    "NODE_OPTIONS",
    "NODE_EXTRA_CA_CERTS",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NODE_TLS_REJECT_UNAUTHORIZED",
    "ELECTRON_RUN_AS_NODE",
];
pub struct MacPlatform {
    runner: Arc<dyn CommandRunner>,
    applications: Arc<dyn ApplicationRegistry>,
}
pub const DESKTOP_ID: &str = "com.anthropic.claudefordesktop";
pub trait ApplicationRegistry: Send + Sync {
    fn has_running_bundle(&self, id: &str) -> Result<bool, Fault>;
}
struct NativeApplications;
impl ApplicationRegistry for NativeApplications {
    fn has_running_bundle(&self, id: &str) -> Result<bool, Fault> {
        use objc2_app_kit::NSRunningApplication;
        use objc2_foundation::NSString;
        // Identity lookup only. This API never starts or activates an app.
        let applications =
            NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(id));
        Ok(!applications.is_empty())
    }
}
impl Default for MacPlatform {
    fn default() -> Self {
        Self {
            runner: Arc::new(SystemRunner),
            applications: Arc::new(NativeApplications),
        }
    }
}
impl MacPlatform {
    pub fn with_runner(runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            runner,
            applications: Arc::new(NativeApplications),
        }
    }
    pub fn with_dependencies(
        runner: Arc<dyn CommandRunner>,
        applications: Arc<dyn ApplicationRegistry>,
    ) -> Self {
        Self {
            runner,
            applications,
        }
    }
    pub fn ensure_desktop_not_running(&self) -> Result<(), Fault> {
        if self.applications.has_running_bundle(DESKTOP_ID)? {
            Err(Fault::Client)
        } else {
            Ok(())
        }
    }
    pub(crate) fn run(&self, exe: &str, args: &[&str]) -> Result<String, Fault> {
        self.run_timeout(exe, args, Duration::from_secs(5))
    }
    fn run_timeout(&self, exe: &str, args: &[&str], timeout: Duration) -> Result<String, Fault> {
        let output = self.runner.run(
            exe,
            &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            timeout,
        )?;
        if output.code != 0 {
            return Err(Fault::Local);
        }
        Ok(output.stdout)
    }
    fn cli_path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        [
            home.join(".local/bin/claude"),
            PathBuf::from("/opt/homebrew/bin/claude"),
            PathBuf::from("/usr/local/bin/claude"),
        ]
        .into_iter()
        .find_map(|p| {
            use std::os::unix::fs::PermissionsExt;
            let meta = p.metadata().ok()?;
            if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
                return None;
            }
            p.canonicalize().ok()
        })
    }
    fn target_path(target: ClientTarget) -> Option<PathBuf> {
        match target {
            ClientTarget::Cli => Self::cli_path(),
            ClientTarget::Desktop => Path::new(DESKTOP).is_dir().then(|| PathBuf::from(DESKTOP)),
        }
    }
    pub fn verify(&self, target: ClientTarget) -> Result<PathBuf, Fault> {
        let path = Self::target_path(target).ok_or(Fault::Client)?;
        self.verify_file(&path, target)?;
        Ok(path)
    }
    pub fn verify_file(&self, path: &Path, target: ClientTarget) -> Result<(), Fault> {
        if !path.is_absolute() {
            return Err(Fault::Client);
        }
        let requirement = if target == ClientTarget::Desktop {
            format!("{SIGNATURE} and identifier \"com.anthropic.claudefordesktop\"")
        } else {
            SIGNATURE.into()
        };
        self.run_timeout(
            "/usr/bin/codesign",
            &[
                "--verify",
                "--strict",
                "-R",
                &requirement,
                path.to_str().ok_or(Fault::Client)?,
            ],
            Duration::from_secs(15),
        )
        .map_err(|_| Fault::Client)?;
        Ok(())
    }
    pub(crate) fn proxy_listener(&self, port: u16) -> Result<(u32, String), Fault> {
        let port_filter = format!("-iTCP:{port}");
        let args = ["-nP", "-a", &port_filter, "-sTCP:LISTEN", "-Fpcn"];
        let output = self
            .runner
            .run(
                "/usr/sbin/lsof",
                &args.map(str::to_owned),
                Duration::from_secs(5),
            )
            .map_err(|_| Fault::Listener)?;
        let endpoint = format!("n127.0.0.1:{port}");
        let pid = if output.code == 0 && output.stdout.lines().any(|line| line == endpoint) {
            guard_core::discovery::listener_pid(&output.stdout, port)
                .map_err(|_| Fault::Listener)?
        } else if output.code == 0 || (output.code == 1 && output.stdout.trim().is_empty()) {
            // An unprivileged lsof may omit sockets held by a root-owned service.
            let sockets = self
                .run("/usr/sbin/netstat", &["-anv", "-p", "tcp", "-f", "inet"])
                .map_err(|_| Fault::Listener)?;
            netstat_listener_pid(&sockets, port)?
        } else {
            return Err(Fault::Listener);
        };
        let path = self
            .run("/bin/ps", &["-p", &pid.to_string(), "-o", "comm="])
            .map_err(|_| Fault::ProcessIdentity)?;
        Ok((pid, path.trim().to_owned()))
    }
    fn owner(&self, profile: &Profile) -> Result<u32, Fault> {
        let (pid, path) = self.proxy_listener(profile.proxy_port())?;
        if path != profile.expected_proxy_executable {
            return Err(Fault::ProcessIdentity);
        }
        Ok(pid)
    }
    fn routes(&self) -> Result<String, Fault> {
        let raw = self
            .run("/usr/sbin/netstat", &["-rn", "-f", "inet"])
            .map_err(|_| Fault::Routes)?;
        let mut defaults: Vec<_> = raw
            .lines()
            .filter(|l| l.split_whitespace().next() == Some("default"))
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        defaults.sort();
        if defaults.is_empty() {
            return Err(Fault::Routes);
        }
        Ok(defaults.join("\n"))
    }
    fn ipv6_observation(
        &self,
    ) -> Result<guard_core::ipv6::Observation, crate::macos_ipv6::InspectionError> {
        use crate::macos_ipv6::InspectionError as E;
        let read = || {
            let interfaces = self
                .run("/sbin/ifconfig", &[])
                .map_err(|_| E::ReadInterfaces)?;
            let routes = self
                .run("/usr/sbin/netstat", &["-rn", "-f", "inet6"])
                .map_err(|_| E::ReadRoutes)?;
            crate::macos_ipv6::parse(&interfaces, &routes)
        };
        let before = read()?;
        let after = read()?;
        if before != after {
            return Err(E::Changed);
        }
        Ok(after)
    }
    fn processes(&self) -> Result<String, Fault> {
        self.run("/bin/ps", &["-axo", "comm="])
    }
}
impl Platform for MacPlatform {
    fn name(&self) -> &'static str {
        "macos"
    }
    fn discover_proxy(&self) -> guard_core::discovery::Discovery {
        self.discover_local_proxy()
    }
    fn clients(&self) -> ClientAvailability {
        ClientAvailability {
            cli: Self::cli_path().is_some(),
            desktop: Path::new(DESKTOP).is_dir(),
        }
    }
    fn local(&self, profile: &Profile) -> Result<LocalSnapshot, Fault> {
        profile.validate().map_err(|_| Fault::ProxySettings)?;
        let raw = self
            .run("/usr/sbin/scutil", &["--proxy"])
            .map_err(|_| Fault::ProxySettings)?;
        let (a, b) = policy::proxy_checks(&raw, profile);
        if a.status != Status::Pass || b.status != Status::Pass {
            return Err(Fault::ProxySettings);
        }
        Ok(LocalSnapshot {
            proxy_fingerprint: raw,
            owner_pid: self.owner(profile)?,
            routes: self.routes()?,
        })
    }
    fn probe(&self, profile: &Profile, clock: &dyn Clock) -> Result<Probe, Fault> {
        profile.validate().map_err(|_| Fault::ProxySettings)?;
        let start = clock.wall_seconds();
        let monotonic = clock.now_ms();
        let value = self
            .runner
            .run(
                "/usr/bin/curl",
                &profile.curl_arguments(),
                Duration::from_secs(10),
            )
            .map_err(|e| match e {
                Fault::Transport(code) => Fault::TransportTimed {
                    code,
                    elapsed_ms: clock.now_ms().saturating_sub(monotonic),
                },
                _ => Fault::ExitCommand,
            })?;
        if value.code != 0 {
            return Err(Fault::TransportTimed {
                code: value.code,
                elapsed_ms: clock.now_ms().saturating_sub(monotonic),
            });
        }
        policy::parse_probe(
            &value.stdout,
            value.code,
            start,
            clock.wall_seconds(),
            clock.now_ms(),
        )
        .map_err(|_| Fault::InvalidResponse)
    }
    fn inspect(&self, id: RuleId, profile: &Profile, _clock: &dyn Clock) -> Check {
        use RuleId as R;
        use Status as S;
        match id {
            R::SystemProxy | R::ProxyBypass => match self.run("/usr/sbin/scutil", &["--proxy"]) {
                Ok(raw) => {
                    let (a, b) = policy::proxy_checks(&raw, profile);
                    if id == R::SystemProxy {
                        a
                    } else {
                        b
                    }
                }
                Err(_) => Check::new(id, S::Fail, "系统代理无法确认"),
            },
            R::ProxyOwner => Check::new(
                id,
                if self.owner(profile).is_ok() {
                    S::Pass
                } else {
                    S::Fail
                },
                "核对指定本机端口和代理进程路径；不读取订阅",
            ),
            R::Dns => match self.run("/usr/sbin/scutil", &["--dns"]) {
                Ok(raw) => policy::dns(&raw),
                Err(_) => Check::new(id, S::Warning, "DNS本地配置无法确认；未测试泄漏"),
            },
            R::Route => Check::new(
                id,
                match self.routes() {
                    Ok(r)
                        if r.split_whitespace()
                            .any(|v| v.starts_with("en") || v.starts_with("utun")) =>
                    {
                        S::Info
                    }
                    _ => S::Warning,
                },
                "观察IPv4默认路由；utun不证明全流量接管",
            ),
            R::Ipv6 => match self.ipv6_observation() {
                Ok(observation) => observation.check(),
                Err(reason) => reason.check(),
            },
            R::Timezone => {
                let system = std::fs::read_link("/etc/localtime").ok().and_then(|p| {
                    p.to_str()
                        .and_then(|s| s.split_once("/zoneinfo/").map(|(_, z)| z.to_string()))
                });
                let process = objc2_foundation::NSTimeZone::localTimeZone()
                    .name()
                    .to_string();
                let valid = objc2_foundation::NSTimeZone::timeZoneWithName(
                    &objc2_foundation::NSString::from_str(&process),
                )
                .is_some();
                let mut check = policy::timezone(system.as_deref(), &process, valid, profile);
                check.detail = format!(
                    "系统：{}；进程：{process}。{}",
                    system.as_deref().unwrap_or("无法确认"),
                    check.detail
                );
                check
            }
            R::ClientSignature => {
                let available = self.clients();
                let ok = (!available.cli || self.verify(ClientTarget::Cli).is_ok())
                    && (!available.desktop || self.verify(ClientTarget::Desktop).is_ok());
                Check::new(
                    id,
                    if !available.cli && !available.desktop {
                        S::Info
                    } else if ok {
                        S::Pass
                    } else {
                        S::Fail
                    },
                    "静态验证已安装文件签名；从不运行客户端进行检测",
                )
            }
            R::Environment => {
                let vars: BTreeMap<_, _> = ENV_KEYS
                    .iter()
                    .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
                    .collect();
                policy::environment(&vars)
            }
            R::ExistingClients => match self.processes() {
                Ok(raw) => Check::new(
                    id,
                    S::Info,
                    format!(
                            "观察到 {} 个相关进程；旧进程的流量未验证，不自动终止",
                            raw.lines()
                                .filter(|l| l.ends_with("/claude")
                                    || l.contains("Claude.app/Contents/"))
                                .count()
                        ),
                ),
                Err(_) => Check::new(id, S::Warning, "无法确认已有进程"),
            },
            _ => Check::new(id, S::Skipped, "由共享检测引擎顺序执行"),
        }
    }
    fn launch(
        &self,
        target: ClientTarget,
        profile: &Profile,
        checker: &Path,
        directory: Option<&Path>,
        authorize: &dyn Fn() -> bool,
    ) -> Result<(), Fault> {
        crate::macos_launch::launch(self, target, profile, checker, directory, authorize)
    }
}

fn netstat_listener_pid(sockets: &str, port: u16) -> Result<u32, Fault> {
    let mut lines = sockets.lines();
    let header = lines
        .find(|line| line.starts_with("Proto "))
        .ok_or(Fault::Listener)?;
    let (_, trailing) = header.split_once(" process:pid ").ok_or(Fault::Listener)?;
    let trailing_columns = trailing.split_ascii_whitespace().count();
    let endpoint = format!("127.0.0.1.{port}");
    let mut owner = None;
    for line in lines {
        let mut columns = line.split_ascii_whitespace();
        if !matches!(columns.next(), Some("tcp4" | "tcp46"))
            || columns.nth(2) != Some(endpoint.as_str())
            || columns.nth(1) != Some("LISTEN")
        {
            continue;
        }
        // Count from the right so spaces in the process name cannot shift the PID.
        let pid = columns
            .rev()
            .nth(trailing_columns)
            .and_then(|column| column.rsplit_once(':'))
            .and_then(|(_, pid)| pid.parse::<u32>().ok())
            .filter(|pid| *pid > 0)
            .ok_or(Fault::Listener)?;
        if owner.is_some_and(|previous| previous != pid) {
            return Err(Fault::Listener);
        }
        owner = Some(pid);
    }
    owner.ok_or(Fault::Listener)
}
