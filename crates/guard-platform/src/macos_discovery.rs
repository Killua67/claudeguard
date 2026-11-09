//! The only discovery source is the current system HTTP/HTTPS proxy and its owner.
use crate::{macos::MacPlatform, Fault};
use guard_core::{
    config::Profile,
    discovery::{system_proxy_url, Discovery, ProxyCandidate},
};
impl MacPlatform {
    pub(crate) fn discover_local_proxy(&self) -> Discovery {
        let result = (|| {
            let before = self
                .run("/usr/sbin/scutil", &["--proxy"])
                .map_err(|_| "系统代理读取失败，请稍后重试")?;
            let url = system_proxy_url(&before)?;
            let mut profile = Profile {
                proxy_url: url,
                ..Profile::default()
            };
            let read_owner = || -> Result<(u32, String), &'static str> {
                self.proxy_listener(profile.proxy_port())
                    .map_err(|fault| match fault {
                        Fault::ProcessIdentity => "本机代理进程路径无法读取",
                        _ => "本机代理端口归属无法读取",
                    })
            };
            let owner = read_owner()?;
            if self
                .run("/usr/sbin/scutil", &["--proxy"])
                .map_err(|_| "系统代理复核失败")?
                != before
                || read_owner()? != owner
            {
                return Err("识别期间代理发生变化，请重新识别");
            }
            profile.expected_proxy_executable = owner.1;
            profile.validate()?;
            Ok(profile)
        })();
        match result {
            Ok(profile) => Discovery { supported:true, candidates:vec![ProxyCandidate { profile, source:"系统 HTTP/HTTPS 代理与唯一回环监听进程；读取前后已复核".into() }], message:"已发现本机代理，请核对地址与程序路径后确认。识别不代表信任该程序，也不代表网络检查通过；本次识别未发起外网请求。".into() },
            Err(message) => Discovery::unavailable(message),
        }
    }
}
