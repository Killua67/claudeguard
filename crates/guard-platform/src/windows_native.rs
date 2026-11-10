#[path = "windows_client.rs"]
mod client;
#[path = "windows_launch.rs"]
mod launch;
#[path = "windows_process.rs"]
mod process;
#[path = "windows_trust.rs"]
mod trust;
pub use client::{guard_parent, GuardParent};
pub use launch::{cli_session, submission_tick};

use super::rules::{listener_pid, manual_port, safe_bypass};
use crate::*;
use guard_core::{
    discovery::{Discovery, ProxyCandidate},
    policy,
};
use process::{process_owner, Owner};
use std::{
    collections::BTreeMap,
    ffi::c_void,
    mem::{offset_of, size_of},
    net::{Ipv4Addr, Ipv6Addr},
    ptr::null,
};
use windows_sys::Win32::{
    Foundation::{GlobalFree, ERROR_BUFFER_OVERFLOW, ERROR_INSUFFICIENT_BUFFER},
    NetworkManagement::{
        IpHelper::{
            GetAdaptersAddresses, GetExtendedTcpTable, GetIpForwardTable, GAA_FLAG_SKIP_ANYCAST,
            GAA_FLAG_SKIP_FRIENDLY_NAME, GAA_FLAG_SKIP_MULTICAST, GAA_FLAG_SKIP_UNICAST,
            IF_TYPE_SOFTWARE_LOOPBACK, IP_ADAPTER_ADDRESSES_LH, MIB_IPFORWARDROW,
            MIB_IPFORWARDTABLE, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID,
            MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID, MIB_TCP_STATE_LISTEN,
            TCP_TABLE_OWNER_PID_LISTENER,
        },
        Ndis::IfOperStatusUp,
    },
    Networking::{
        WinHttp::{WinHttpGetIEProxyConfigForCurrentUser, WINHTTP_CURRENT_USER_IE_PROXY_CONFIG},
        WinSock::{AF_INET, AF_INET6, AF_UNSPEC},
    },
    System::Time::{
        GetDynamicTimeZoneInformation, DYNAMIC_TIME_ZONE_INFORMATION, TIME_ZONE_ID_INVALID,
    },
};

pub struct WindowsPlatform;

struct ProxyConfig(WINHTTP_CURRENT_USER_IE_PROXY_CONFIG);

impl Drop for ProxyConfig {
    fn drop(&mut self) {
        for ptr in [
            self.0.lpszProxy,
            self.0.lpszProxyBypass,
            self.0.lpszAutoConfigUrl,
        ] {
            if !ptr.is_null() {
                // WinHTTP transfers ownership of each of these GlobalAlloc strings.
                unsafe { GlobalFree(ptr.cast()) };
            }
        }
    }
}

struct SystemProxy {
    server: String,
    bypass: String,
    automatic: bool,
}

impl SystemProxy {
    fn read() -> Result<Self, Fault> {
        let mut config = ProxyConfig(WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default());
        // Reads the current user's WinINet configuration; does not evaluate WPAD/PAC.
        if unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut config.0) } == 0 {
            return Err(Fault::ProxySettings);
        }
        fn text(ptr: *const u16) -> Result<String, Fault> {
            if ptr.is_null() {
                return Ok(String::new());
            }
            let mut units = Vec::new();
            for i in 0..8192 {
                // WinHTTP guarantees the input is a NUL-terminated UTF-16 string.
                let c = unsafe { ptr.add(i).read() };
                if c == 0 {
                    return String::from_utf16(&units).map_err(|_| Fault::ProxySettings);
                }
                units.push(c);
            }
            Err(Fault::ProxySettings)
        }
        Ok(Self {
            server: text(config.0.lpszProxy)?,
            bypass: text(config.0.lpszProxyBypass)?,
            automatic: config.0.fAutoDetect != 0 || !config.0.lpszAutoConfigUrl.is_null(),
        })
    }

    fn port(&self) -> Result<u16, Fault> {
        (!self.automatic)
            .then(|| manual_port(&self.server))
            .flatten()
            .ok_or(Fault::ProxySettings)
    }

    fn matches(&self, profile: &Profile) -> bool {
        self.port().is_ok_and(|p| {
            profile.proxy_url == format!("http://127.0.0.1:{p}")
                || profile.proxy_url == format!("http://127.0.0.1:{p}/")
        })
    }

    fn bypass_ok(&self) -> bool {
        !self.automatic && safe_bypass(&self.bypass)
    }
}

// The native APIs fill caller-owned, aligned buffers. Cap growth and retry
// because the data can change between two calls.
struct Buffer {
    words: Vec<u64>,
    size: usize,
}
impl Buffer {
    fn read(
        mut size: u32,
        mut call: impl FnMut(*mut c_void, &mut u32) -> u32,
        fault: Fault,
    ) -> Result<Self, Fault> {
        for _ in 0..4 {
            if !(1..=4 * 1024 * 1024).contains(&size) {
                return Err(fault);
            }
            let capacity = size;
            let mut words = vec![0_u64; (capacity as usize).div_ceil(size_of::<u64>())];
            let result = call(words.as_mut_ptr().cast(), &mut size);
            if result == 0 && size <= capacity {
                return Ok(Self {
                    words,
                    size: size as usize,
                });
            }
            if !matches!(result, ERROR_BUFFER_OVERFLOW | ERROR_INSUFFICIENT_BUFFER)
                || size <= capacity
            {
                return Err(fault);
            }
        }
        Err(fault)
    }

    fn bytes(&self) -> &[u8] {
        // The allocation is initialized and read() bounds size to the allocation.
        unsafe { std::slice::from_raw_parts(self.words.as_ptr().cast(), self.size) }
    }

    /// The caller must supply the actual native row type; all bit patterns
    /// returned for that type must be valid Rust values.
    unsafe fn rows<T>(&self, offset: usize) -> Option<&[T]> {
        if self.size < size_of::<u32>() {
            return None;
        }
        let start = self.words.as_ptr().cast::<u8>();
        // The first DWORD is the row count, not the first row's byte offset.
        let count = unsafe { start.cast::<u32>().read() } as usize;
        let bytes = self.size.checked_sub(offset)?;
        if count.checked_mul(size_of::<T>())? > bytes {
            return None;
        }
        let first = unsafe { start.add(offset).cast::<T>() };
        if !first.is_aligned() {
            return None;
        }
        Some(unsafe { std::slice::from_raw_parts(first, count) })
    }
}

fn listener(port: u16) -> Result<u32, Fault> {
    let table = |family: u16| {
        Buffer::read(
            16_384,
            |data, size| unsafe {
                GetExtendedTcpTable(
                    data,
                    size,
                    0,
                    family.into(),
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            },
            Fault::Listener,
        )
    };
    let buffer = table(AF_INET)?;
    // GetExtendedTcpTable with this address family/table class returns this layout.
    let rows =
        unsafe { buffer.rows::<MIB_TCPROW_OWNER_PID>(offset_of!(MIB_TCPTABLE_OWNER_PID, table)) }
            .ok_or(Fault::Listener)?;
    let pid = listener_pid(
        rows.iter()
            .filter(|row| row.dwState == MIB_TCP_STATE_LISTEN as u32)
            .map(|row| {
                (
                    Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes()),
                    u16::from_be(row.dwLocalPort as u16),
                    row.dwOwningPid,
                )
            }),
        port,
    )
    .ok_or(Fault::Listener)?;

    // A dual-stack socket can receive IPv4 connections. The TCP table cannot
    // tell us IPV6_V6ONLY; reject any potential competing IPv6 owner.
    let buffer = table(AF_INET6)?;
    let rows =
        unsafe { buffer.rows::<MIB_TCP6ROW_OWNER_PID>(offset_of!(MIB_TCP6TABLE_OWNER_PID, table)) }
            .ok_or(Fault::Listener)?;
    if rows.iter().any(|row| {
        let address = Ipv6Addr::from(row.ucLocalAddr);
        row.dwState == MIB_TCP_STATE_LISTEN as u32
            && u16::from_be(row.dwLocalPort as u16) == port
            && row.dwOwningPid != pid
            && (address.is_unspecified() || address.to_ipv4_mapped() == Some(Ipv4Addr::LOCALHOST))
    }) {
        return Err(Fault::Listener);
    }
    Ok(pid)
}

fn observed_owner(port: u16) -> Result<Owner, Fault> {
    let owner = process_owner(listener(port)?)?;
    if process_owner(listener(port)?)? != owner {
        return Err(Fault::Drift);
    }
    Ok(owner)
}

fn confirmed_owner(profile: &Profile) -> Result<Owner, Fault> {
    let owner = observed_owner(profile.proxy_port())?;
    // Preserve case in the path: Windows can have case-sensitive directories.
    if owner.path != profile.expected_proxy_executable {
        return Err(Fault::ProcessIdentity);
    }
    Ok(owner)
}

fn routes() -> Result<String, Fault> {
    let buffer = Buffer::read(
        16_384,
        |data, size| unsafe { GetIpForwardTable(data.cast(), size, 0) },
        Fault::Routes,
    )?;
    let rows = unsafe { buffer.rows::<MIB_IPFORWARDROW>(offset_of!(MIB_IPFORWARDTABLE, table)) }
        .ok_or(Fault::Routes)?;
    let mut defaults: Vec<_> = rows
        .iter()
        .filter(|row| row.dwForwardDest == 0 && row.dwForwardMask == 0)
        .map(|row| {
            (
                row.dwForwardIfIndex,
                row.dwForwardNextHop,
                row.dwForwardMetric1,
            )
        })
        .collect();
    if defaults.is_empty() {
        return Err(Fault::Routes);
    }
    defaults.sort_unstable();
    // Private drift key, never included in check detail or report export.
    Ok(format!("{defaults:?}"))
}

fn dns_configured() -> Result<bool, Fault> {
    let buffer = Buffer::read(
        16_384,
        |data, size| unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC.into(),
                GAA_FLAG_SKIP_UNICAST
                    | GAA_FLAG_SKIP_ANYCAST
                    | GAA_FLAG_SKIP_MULTICAST
                    | GAA_FLAG_SKIP_FRIENDLY_NAME,
                null(),
                data.cast(),
                size,
            )
        },
        Fault::Local,
    )?;
    let start = buffer.words.as_ptr().addr();
    let mut next = buffer.words.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
    for _ in 0..buffer.size / size_of::<IP_ADAPTER_ADDRESSES_LH>() {
        let offset = next.addr().checked_sub(start).ok_or(Fault::Local)?;
        if !next.is_aligned() || offset > buffer.size - size_of::<IP_ADAPTER_ADDRESSES_LH>() {
            return Err(Fault::Local);
        }
        // All addresses come from the adapter API. Bounds above keep the
        // dereference within the caller's buffer; only test DNS pointer presence.
        let adapter = unsafe { &*next };
        if adapter.OperStatus == IfOperStatusUp
            && adapter.IfType != IF_TYPE_SOFTWARE_LOOPBACK
            && !adapter.FirstDnsServerAddress.is_null()
        {
            return Ok(true);
        }
        next = adapter.Next;
        if next.is_null() {
            return Ok(false);
        }
    }
    Err(Fault::Local)
}

fn timezone(profile: &Profile) -> Check {
    let mut data = DYNAMIC_TIME_ZONE_INFORMATION::default();
    let value = unsafe { GetDynamicTimeZoneInformation(&mut data) };
    let key = data
        .TimeZoneKeyName
        .split(|c| *c == 0)
        .next()
        .unwrap_or(&[]);
    let mainland = "China Standard Time".encode_utf16().eq(key.iter().copied());
    let (status, detail) = if value == TIME_ZONE_ID_INVALID || key.is_empty() {
        (Status::Warning, "无法确认 Windows 当前系统时区")
    } else if profile.non_mainland_timezone && mainland {
        (Status::Warning, "Windows 当前系统时区与已选个人偏好不符")
    } else {
        (
            Status::Info,
            "已读取 Windows 当前系统时区；未读取客户端时区",
        )
    };
    Check::new(RuleId::Timezone, status, detail)
}

fn environment() -> Check {
    let vars: BTreeMap<_, _> = [
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
    ]
    .into_iter()
    .filter_map(|key| {
        std::env::var_os(key).map(|value| (key.to_string(), value.to_string_lossy().into_owned()))
    })
    .collect();
    policy::environment(&vars)
}

impl Platform for WindowsPlatform {
    fn name(&self) -> &'static str {
        "windows-preflight"
    }

    fn discover_proxy(&self) -> Discovery {
        let Ok(settings) = SystemProxy::read() else {
            return Discovery::unavailable("无法读取 Windows 当前用户的系统代理");
        };
        let Ok(port) = settings.port() else {
            return Discovery::unavailable(
                "请关闭 PAC 和自动检测，并启用明确的 127.0.0.1 HTTP 代理",
            );
        };
        let Ok(owner) = observed_owner(port) else {
            return Discovery::unavailable("无法确认本机代理端口的监听进程和完整路径");
        };
        let profile = Profile {
            proxy_url: format!("http://127.0.0.1:{port}"),
            expected_proxy_executable: owner.path,
            ..Profile::default()
        };
        if profile.validate().is_err() {
            return Discovery::unavailable("识别到的系统代理地址或进程路径格式不受支持");
        }
        Discovery {
            supported: true,
            candidates: vec![ProxyCandidate {
                profile,
                source: "Windows 当前用户的系统代理和 IPv4 TCP 监听进程".into(),
            }],
            message: "已找到 Windows 本机代理候选；请核对后确认".into(),
        }
    }

    fn clients(&self) -> ClientAvailability {
        client::availability()
    }

    fn inspect(&self, id: RuleId, profile: &Profile, _: &dyn Clock) -> Check {
        use RuleId as R;
        use Status as S;
        let status = |ok| if ok { S::Pass } else { S::Fail };
        let (result, message) = match id {
            R::SystemProxy => (
                status(SystemProxy::read().is_ok_and(|settings| settings.matches(profile))),
                "核对 Windows 当前用户的手动 HTTP/HTTPS 代理；两者必须指向已确认的本机地址",
            ),
            R::ProxyBypass => (
                status(SystemProxy::read().is_ok_and(|settings| settings.bypass_ok())),
                "仅接受已知本机或内网旁路；PAC、自动检测和任意主机通配不受支持",
            ),
            R::ProxyOwner => (
                status(confirmed_owner(profile).is_ok()),
                "核对 127.0.0.1 TCP 监听 PID 和已确认的完整进程路径",
            ),
            R::Dns => match dns_configured() {
                Ok(true) => (S::Info, "在活动 Windows 网卡上看到 DNS 配置；未发 DNS 请求"),
                _ => (S::Warning, "无法确认活动 Windows 网卡的 DNS 配置"),
            },
            R::Route => match routes() {
                Ok(_) => (
                    S::Info,
                    "已读取 Windows IPv4 默认路由；不能证明客户端流量走代理",
                ),
                Err(_) => (S::Warning, "无法确认 Windows IPv4 默认路由"),
            },
            R::Ipv6 => (S::Warning, "Windows IPv4/IPv6 客户端实际流量覆盖未验证"),
            R::Timezone => return timezone(profile),
            R::Environment => return environment(),
            R::ClientSignature => return client::signatures(),
            R::ExistingClients => return client::existing(),
            _ => (S::Skipped, "由共享检测引擎顺序执行"),
        };
        Check::new(id, result, message)
    }

    fn local(&self, profile: &Profile) -> Result<LocalSnapshot, Fault> {
        profile.validate().map_err(|_| Fault::ProxySettings)?;
        let proxy = SystemProxy::read()?;
        if !proxy.matches(profile) || !proxy.bypass_ok() {
            return Err(Fault::ProxySettings);
        }
        let owner = confirmed_owner(profile)?;
        Ok(LocalSnapshot {
            proxy_fingerprint: format!("{};{};{}", owner.created, proxy.server, proxy.bypass),
            owner_pid: owner.pid,
            routes: routes()?,
        })
    }

    fn probe(&self, profile: &Profile, clock: &dyn Clock) -> Result<Probe, Fault> {
        // Recheck before each request, including intermediate stability samples.
        let before = self.local(profile)?;
        let sample = super::probe::sample(profile.proxy_port(), clock)?;
        if self.local(profile)? != before {
            return Err(Fault::Drift);
        }
        Ok(sample)
    }

    fn launch(
        &self,
        target: ClientTarget,
        profile: &Profile,
        checker: &std::path::Path,
        directory: Option<&std::path::Path>,
        authorize: &dyn Fn() -> bool,
    ) -> Result<(), Fault> {
        launch::launch(target, profile, checker, directory, authorize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, path::PathBuf};

    #[test]
    fn real_tcp_table_and_process_identity_resolve_a_loopback_fixture() {
        let socket = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let owner = observed_owner(socket.local_addr().unwrap().port()).unwrap();
        assert_eq!(owner.pid, std::process::id());
        assert_eq!(
            PathBuf::from(owner.path).file_name(),
            std::env::current_exe().unwrap().file_name(),
        );
        assert_ne!(owner.created, 0);
    }

    #[test]
    fn matching_manual_proxy_does_not_override_pac_or_unknown_bypass() {
        let mut proxy = SystemProxy {
            server: "127.0.0.1:7897".into(),
            bypass: "localhost".into(),
            automatic: false,
        };
        let profile = Profile {
            proxy_url: "http://127.0.0.1:7897".into(),
            ..Profile::default()
        };
        assert!(proxy.matches(&profile));
        assert!(proxy.bypass_ok());
        proxy.automatic = true;
        assert!(!proxy.matches(&profile));
        assert!(!proxy.bypass_ok());
        proxy.automatic = false;
        proxy.bypass = "127.*".into();
        assert!(proxy.matches(&profile));
        assert!(!proxy.bypass_ok());
    }
}
