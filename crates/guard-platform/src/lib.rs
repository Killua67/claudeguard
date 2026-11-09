//! Read-only OS inspection and narrowly scoped native launch adapters.
pub mod background;
pub mod command;
pub mod demo;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
mod macos_discovery;
#[cfg(target_os = "macos")]
pub mod macos_events;
#[cfg(target_os = "macos")]
mod macos_ipv6;
pub mod windows;
use guard_core::{config::Profile, domain::*};
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
    fn wall_seconds(&self) -> f64;
}
#[derive(Default)]
pub struct SystemClock {
    origin: OnceLock<Instant>,
}
impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        self.origin.get_or_init(Instant::now).elapsed().as_millis() as u64
    }
    fn wall_seconds(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }
}
#[derive(Debug, Clone, Copy)]
pub enum Fault {
    Unsupported,
    Local,
    Transport(i32),
    TransportTimed { code: i32, elapsed_ms: u64 },
    ProxySettings,
    Listener,
    ProcessIdentity,
    Routes,
    LocalCommand,
    ExitCommand,
    ProxyTransport,
    InvalidResponse,
    Drift,
    Client,
    Busy,
}
impl Fault {
    pub fn message(self) -> &'static str {
        match self {
            Self::Unsupported => "此平台能力尚未实现，不能判定通过",
            Self::Local => "本地代理、进程或路由检查未通过",
            Self::Transport(_) | Self::TransportTimed { .. } => "出口请求失败或超时（未回退直连）",
            Self::ProxySettings => "本地检查失败：系统代理已关闭、改变或无法读取",
            Self::Listener => "本地检查失败：代理监听已消失或身份无法确认",
            Self::ProcessIdentity => "本地检查失败：代理监听进程身份发生变化",
            Self::Routes => "本地检查失败：无法确认默认网络路由",
            Self::LocalCommand => "本地检查失败：系统检测命令超时或无法执行",
            Self::ExitCommand => "出口检查失败：探测命令无法执行（未回退直连）",
            Self::ProxyTransport => {
                "出口检查失败：本机代理 CONNECT、HTTPS 传输或 TLS 验证未通过（未回退直连）"
            }
            Self::InvalidResponse => "HTTP、TLS或出口响应无法确认",
            Self::Drift => "检测期间代理、路由或出口发生变化",
            Self::Client => "客户端不可用、身份未通过或已有旧实例",
            Self::Busy => "已有检查或启动正在进行",
        }
    }
    /// Construct only from bounded codes/timing, never subprocess output or IPs.
    pub fn detail(self) -> String {
        let (code, elapsed) = match self {
            Self::Transport(code) => (code, None),
            Self::TransportTimed { code, elapsed_ms } => (code, Some(elapsed_ms)),
            _ => return self.message().into(),
        };
        let reason = match code {
            5 => "代理地址解析失败",
            6 => "目标域名解析失败",
            7 => "无法连接本地代理",
            22 => "服务器返回HTTP错误",
            28 => "Cloudflare HTTPS请求超时",
            35 => "TLS握手失败",
            60 => "TLS证书验证失败",
            56 => "连接接收中断或代理隧道失败",
            63 => "响应超过大小上限",
            _ => "HTTPS传输失败",
        };
        let timing = elapsed
            .map(|ms| format!("，耗时 {:.2} 秒", ms as f64 / 1000.0))
            .unwrap_or_default();
        format!("出口检查失败：{reason}（curl {code}{timing}，未回退直连）")
    }
}
pub trait Platform: Send + Sync {
    fn name(&self) -> &'static str;
    fn discover_proxy(&self) -> guard_core::discovery::Discovery {
        guard_core::discovery::Discovery::unsupported()
    }
    fn clients(&self) -> ClientAvailability;
    fn inspect(&self, id: RuleId, profile: &Profile, clock: &dyn Clock) -> Check;
    fn local(&self, profile: &Profile) -> Result<LocalSnapshot, Fault>;
    fn probe(&self, profile: &Profile, clock: &dyn Clock) -> Result<Probe, Fault>;
    fn launch(
        &self,
        _target: ClientTarget,
        _profile: &Profile,
        _checker: &std::path::Path,
        _directory: Option<&std::path::Path>,
        _authorize: &dyn Fn() -> bool,
    ) -> Result<(), Fault> {
        Err(Fault::Unsupported)
    }
}
#[cfg(target_os = "macos")]
mod macos_launch;
