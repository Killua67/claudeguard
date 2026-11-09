//! Platform-neutral, local-only IPv6 evidence. No addresses or gateways enter
//! reports. A tunnel or a successful proxy probe never proves IPv6 coverage.
use crate::domain::{Check, RuleId, Status};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LinkState {
    Active,
    Inactive,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Interface {
    pub name: String,
    pub state: LinkState,
    pub usable_public_addresses: usize,
    pub unavailable_public_addresses: usize,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Observation {
    pub interfaces: Vec<Interface>,
    /// Interfaces with usable default, split-default or non-local IPv6 routes.
    /// No interface name (including utun) is evidence of proxy coverage.
    pub external_route_interfaces: Vec<String>,
}
impl Observation {
    pub fn check(&self) -> Check {
        if self.interfaces.is_empty() {
            return unavailable();
        }
        let relevant: usize = self
            .interfaces
            .iter()
            .filter(|i| i.state != LinkState::Inactive)
            .map(|i| i.usable_public_addresses)
            .sum();
        let inactive: usize = self
            .interfaces
            .iter()
            .filter(|i| i.state == LinkState::Inactive)
            .map(|i| i.usable_public_addresses + i.unavailable_public_addresses)
            .sum();
        let unavailable: usize = self
            .interfaces
            .iter()
            .filter(|i| i.state != LinkState::Inactive)
            .map(|i| i.unavailable_public_addresses)
            .sum();
        let routes = self.external_route_interfaces.len();
        let warning = relevant > 0 || routes > 0;
        let observation = if warning {
            format!(
                "公网 IPv6 路径：{} 个可用或活动状态待确认的地址，{} 个接口存在公网范围目标的候选路由。",
                relevant, routes
            )
        } else {
            "公网 IPv6 路径：未观察到当前可用的公网地址或外网路由。".into()
        };
        Check::new(RuleId::Ipv6, if warning { Status::Warning } else { Status::Info },
            format!("{observation}\n本次排除：{inactive} 个停用/断开接口地址，{unavailable} 个暂不可用地址。\n代理覆盖：尚未验证{}。未进行 IPv6 外联，不能据此断言已发生直连或泄漏；本次未发现也不保证未来不会改变。",
                if warning { "；这是非阻断提醒，不要求关闭 IPv6" } else { "；当前没有发现需要验证的公网路径" }))
    }
    /// Canonical, address-free observation token; not a proxy authorization fingerprint.
    pub fn fingerprint(&self) -> String {
        let mut interfaces = self.interfaces.clone();
        interfaces.sort();
        let mut routes = self.external_route_interfaces.clone();
        routes.sort();
        routes.dedup();
        format!("{interfaces:?}|{routes:?}")
    }
}
pub fn unavailable() -> Check {
    Check::new(RuleId::Ipv6, Status::Warning,
        "公网 IPv6 路径：无法确认，接口/路由读取失败、格式不完整或观测期间发生变化。\n代理覆盖：尚未验证；代理预检模式下仅提醒，不据此阻止启动。未进行 IPv6 外联，也未证实直连或泄漏。")
}
