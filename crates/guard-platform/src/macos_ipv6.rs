//! macOS text adapters only. Policy consumes normalized, address-free evidence.
use guard_core::ipv6::{Interface, LinkState, Observation};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::Ipv6Addr,
};

fn name_valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
}
fn local_scope(ip: Ipv6Addr) -> bool {
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_unicast_link_local()
        || ip.is_unique_local()
}
struct ParsedInterface {
    up: bool,
    status: Option<LinkState>,
    usable: BTreeSet<Ipv6Addr>,
    unavailable: BTreeSet<Ipv6Addr>,
}
fn interfaces(raw: &str) -> Option<Vec<Interface>> {
    let mut parsed = BTreeMap::<String, ParsedInterface>::new();
    let mut current = None;
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        if !line.starts_with(char::is_whitespace) {
            let (name, rest) = line.split_once(':')?;
            if !name_valid(name)
                || !rest.trim_start().starts_with("flags=")
                || parsed.contains_key(name)
            {
                return None;
            }
            let (numeric, flags) = rest.trim_start().strip_prefix("flags=")?.split_once('<')?;
            let numeric = u32::from_str_radix(numeric, 16).ok()?;
            let (flags, attributes) = flags.split_once('>')?;
            let attributes: Vec<_> = attributes.split_whitespace().collect();
            let mtu_at = attributes.iter().position(|v| *v == "mtu")?;
            let mtu: u32 = attributes.get(mtu_at + 1)?.parse().ok()?;
            if mtu == 0 {
                return None;
            }
            // Native dormant interfaces (for example stf0) legitimately have
            // flags=0<>. Empty textual flags with nonzero bits stay invalid.
            if (flags.is_empty() && numeric != 0)
                || (numeric & 1 != 0) != flags.split(',').any(|s| s == "UP")
            {
                return None;
            }
            parsed.insert(
                name.into(),
                ParsedInterface {
                    up: flags.split(',').any(|s| s == "UP"),
                    status: None,
                    usable: BTreeSet::new(),
                    unavailable: BTreeSet::new(),
                },
            );
            current = Some(name);
            continue;
        }
        let i = parsed.get_mut(current?)?;
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.first() == Some(&"status:") {
            if i.status.is_some() || fields.len() != 2 {
                return None;
            }
            i.status = Some(match fields[1] {
                "active" => LinkState::Active,
                "inactive" => LinkState::Inactive,
                _ => return None,
            });
        }
        if fields.first() != Some(&"inet6") {
            if fields.first().is_some_and(|s| s.starts_with("inet6")) {
                return None;
            }
            continue;
        }
        let address: Ipv6Addr = fields.get(1)?.split('%').next()?.parse().ok()?;
        let prefix_at = fields.iter().position(|f| *f == "prefixlen")?;
        let prefix: u8 = fields.get(prefix_at + 1)?.parse().ok()?;
        if prefix > 128 {
            return None;
        }
        if local_scope(address) {
            continue;
        }
        // Deprecated addresses remain usable for existing connections. Optimistic
        // DAD can transmit even while tentative, so it is never excluded.
        let unavailable = fields
            .iter()
            .any(|f| matches!(*f, "duplicated" | "detached"))
            || (fields.contains(&"tentative") && !fields.contains(&"optimistic"));
        if unavailable {
            i.unavailable.insert(address);
        } else {
            i.usable.insert(address);
        }
    }
    if parsed.is_empty() {
        return None;
    }
    Some(
        parsed
            .into_iter()
            .map(|(name, i)| Interface {
                name,
                state: if !i.up || i.status == Some(LinkState::Inactive) {
                    LinkState::Inactive
                } else {
                    i.status.unwrap_or(LinkState::Unknown)
                },
                usable_public_addresses: i.usable.len(),
                unavailable_public_addresses: i.unavailable.len(),
            })
            .collect(),
    )
}
fn external_destination(raw: &str) -> Option<bool> {
    if raw == "default" {
        return Some(true);
    }
    let (host, prefix) = raw.split_once('/').unwrap_or((raw, "128"));
    let ip: Ipv6Addr = host.split('%').next()?.parse().ok()?;
    let prefix: u8 = prefix.parse().ok()?;
    if prefix > 128 {
        return None;
    }
    // Only networks entirely inside local scopes can be ignored. A broad
    // ::/1 or 8000::/1 remains external even when its base address is special.
    let local = (prefix >= 128 && (ip.is_loopback() || ip.is_unspecified()))
        || (prefix >= 8 && ip.is_multicast())
        || (prefix >= 10 && ip.is_unicast_link_local())
        || (prefix >= 7 && ip.is_unique_local());
    Some(!local)
}
fn routes(raw: &str, interfaces: &[Interface]) -> Option<Vec<String>> {
    let mut section = false;
    let mut columns = None;
    let mut external = BTreeSet::new();
    for line in raw.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line == "Routing tables" {
            continue;
        }
        if line == "Internet6:" {
            if section {
                return None;
            }
            section = true;
            continue;
        }
        if !section {
            return None;
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.first() == Some(&"Destination") {
            if columns.is_some() {
                return None;
            }
            columns = Some((
                fields.iter().position(|s| *s == "Flags")?,
                fields.iter().position(|s| *s == "Netif")?,
            ));
            continue;
        }
        let (flag_at, iface_at) = columns?;
        let flags = *fields.get(flag_at)?;
        let interface = *fields.get(iface_at)?;
        if !interfaces.iter().any(|i| i.name == interface)
            || flags.is_empty()
            || !flags
                .bytes()
                .all(|c| b"123BbCcDGHIiLlMmRrSUWXYg!".contains(&c))
        {
            return None;
        }
        let outside = external_destination(fields[0])?;
        if outside && flags.contains('U') && !flags.contains(['R', 'B']) {
            external.insert(interface.to_string());
        }
    }
    columns?;
    Some(external.into_iter().collect())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InspectionError {
    ReadInterfaces,
    ReadRoutes,
    ParseInterfaces,
    ParseRoutes,
    Changed,
}
impl InspectionError {
    pub(crate) fn check(self) -> guard_core::domain::Check {
        // Closed, static messages only: never expose raw command output,
        // addresses, routes or subprocess errors to the UI/report.
        let reason = match self {
            Self::ReadInterfaces => "接口读取失败或超时（ifconfig）",
            Self::ReadRoutes => "IPv6 路由读取失败或超时（netstat）",
            Self::ParseInterfaces => "接口格式无法解析（ifconfig）",
            Self::ParseRoutes => "IPv6 路由表格式无法解析（netstat）",
            Self::Changed => "两轮本地采样结果发生变化，请网络稳定后重新检测",
        };
        let mut check = guard_core::ipv6::unavailable();
        check.detail = format!("本地证据：{reason}。\n{}", check.detail);
        check
    }
}
pub(crate) fn parse(ifconfig: &str, netstat: &str) -> Result<Observation, InspectionError> {
    let interfaces = interfaces(ifconfig).ok_or(InspectionError::ParseInterfaces)?;
    let external_route_interfaces =
        routes(netstat, &interfaces).ok_or(InspectionError::ParseRoutes)?;
    Ok(Observation {
        interfaces,
        external_route_interfaces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use guard_core::domain::Status;
    fn iface(name: &str, state: &str, address: &str) -> String {
        let flags = if state == "down" {
            "8822<BROADCAST,SIMPLEX,MULTICAST>"
        } else {
            "8863<UP,BROADCAST,RUNNING,SIMPLEX,MULTICAST>"
        };
        let status = match state {
            "active" => "\tstatus: active\n",
            "inactive" | "down" => "\tstatus: inactive\n",
            _ => "",
        };
        format!("{name}: flags={flags} mtu 1500\n{address}{status}")
    }
    fn table(rows: &str) -> String {
        format!("Routing tables\n\nInternet6:\nDestination Gateway Flags Netif Expire\n{rows}")
    }
    fn status(interfaces: &str, routes: &str) -> Status {
        parse(interfaces, &table(routes)).map_or(Status::Warning, |o| o.check().status)
    }
    #[test]
    fn zero_flag_system_interface_is_valid_and_does_not_hide_active_public_ipv6() {
        let dormant = "stf0: flags=0<> mtu 1280\n";
        let local = iface("en0", "active", "\tinet6 fe80::1%en0 prefixlen 64\n");
        let no_public =
            parse(&format!("{dormant}{local}"), &table("")).expect("native zero flags are valid");
        assert_eq!(no_public.check().status, Status::Info);
        let public = iface("en0", "active", "\tinet6 2001:db8::7 prefixlen 64 autoconf secured\n\tinet6 2001:db8::8 prefixlen 64 autoconf temporary\n");
        let observed = parse(
            &format!("{dormant}{public}"),
            &table("default fe80::1%en0 UGcg en0\n"),
        )
        .expect("valid interface set");
        assert_eq!(observed.check().status, Status::Warning);
        assert_eq!(
            observed
                .interfaces
                .iter()
                .find(|i| i.name == "en0")
                .unwrap()
                .usable_public_addresses,
            2
        );
        assert!(observed.check().detail.contains("非阻断提醒"));
        assert!(parse("stf0: flags=1<> mtu 1280\n", &table("")).is_err());
    }
    #[test]
    fn active_public_deprecated_virtual_and_unknown_links_stay_blocked() {
        for name in ["en0", "utun9", "bridge1"] {
            for state in ["active", "unknown"] {
                for suffix in ["", " deprecated", " tentative optimistic"] {
                    let raw = iface(
                        name,
                        state,
                        &format!("\tinet6 2001:db8::7 prefixlen 64{suffix}\n"),
                    );
                    assert_eq!(status(&raw, ""), Status::Warning);
                }
            }
        }
    }
    #[test]
    fn only_confirmed_unavailable_addresses_or_interfaces_are_excluded() {
        for state in ["down", "inactive"] {
            let raw = iface("en0", state, "\tinet6 2001:db8::7 prefixlen 64\n");
            let check = parse(&raw, &table("")).unwrap().check();
            assert_eq!(check.status, Status::Info);
            assert!(check.detail.contains("1 个停用/断开接口地址"));
            // A contradictory route is still evidence needing confirmation.
            assert_eq!(
                status(&raw, "default fe80::1%en0 UG en0\n"),
                Status::Warning
            );
        }
        for flag in ["tentative", "duplicated", "detached"] {
            let raw = iface(
                "en0",
                "active",
                &format!("\tinet6 2001:db8::7 prefixlen 64 {flag}\n"),
            );
            assert_eq!(status(&raw, ""), Status::Info);
            assert_eq!(
                status(&raw, "default fe80::1%en0 UG en0\n"),
                Status::Warning
            );
        }
    }
    #[test]
    fn external_routes_include_split_default_host_and_nat64_without_global_source() {
        let raw = iface("utun9", "unknown", "\tinet6 fd00::1 prefixlen 64\n");
        for destination in [
            "default",
            "::/0",
            "::/1",
            "8000::/1",
            "2000::/3",
            "2001:db8::/32",
            "2001:db8::9",
            "64:ff9b::/96",
        ] {
            assert_eq!(
                status(&raw, &format!("{destination} link#9 U utun9\n")),
                Status::Warning,
                "{destination}"
            );
        }
        for destination in ["::1", "fe80::%utun9/64", "fc00::/7", "ff00::/8"] {
            assert_eq!(
                status(&raw, &format!("{destination} link#9 U utun9\n")),
                Status::Info,
                "{destination}"
            );
        }
        for flags in ["UGR", "UGB", "G"] {
            assert_eq!(
                status(&raw, &format!("default link#9 {flags} utun9\n")),
                Status::Info
            );
        }
    }
    #[test]
    fn malformed_or_incomplete_output_is_unknown_not_no_ipv6() {
        let valid = iface("en0", "active", "\tinet6 fe80::1%en0 prefixlen 64\n");
        for raw in [
            "en0: flags=1<UP>",
            "en0: flags=1<UP> mtu zero",
            "en0: flags=1<UP> mtu 0",
            "en0: flags=1<UP> mtu 1500\n\tinet6:",
            "en0: flags=1<UP> mtu 1500\n\tinet6garbage 2001:db8::1 prefixlen 64",
        ] {
            assert!(parse(raw, &table("")).is_err(), "{raw}");
        }

        for raw in [
            "",
            "garbage",
            "inet6 2001:db8::1",
            "en0: flags=1<UP> mtu 1500\n\tinet6 invalid prefixlen 64",
            "en0: flags=1<UP> mtu 1500\n\tinet6 2001:db8::1 prefixlen 129",
            "en0: flags=1<UP> mtu 1500\n\tstatus: unknown",
            "en0: flags=1<UP> mtu 1500\nen0: flags=1<UP> mtu 1500\n",
            "en0: flags=8863<BOGUS> mtu 1500\n\tinet6 2001:db8::1 prefixlen 64\n\tstatus: active",
            "en0: flags=0<UP> mtu 1500\n",
            "en0: flags=bogus<UP> mtu 1500\n",
        ] {
            assert!(parse(raw, &table("")).is_err(), "{raw}");
        }
        for raw in ["", "Internet6:", "Destination Gateway Flags Netif", "Routing tables\nInternet6:\nDestination Gateway Flags Netif\ndefault", "Routing tables\nInternet6:\nDestination Gateway Flags Netif\ndefault link#9 U missing0", "Routing tables\nInternet6:\nDestination Gateway Flags Netif\ngarbage link#1 U en0"] {
            assert!(parse(&valid,raw).is_err(),"{raw}");
        }
    }
    #[test]
    fn raw_addresses_never_reach_details_or_change_tokens() {
        let raw = iface(
            "en0",
            "active",
            "\tinet6 2001:db8::1234 prefixlen 64\n\tinet6 2001:db8::1234 prefixlen 64\n",
        );
        let o = parse(&raw, &table("default fe80::9%en0 UG en0\n")).unwrap();
        assert_eq!(o.interfaces[0].usable_public_addresses, 1);
        assert!(!o.check().detail.contains("2001:db8"));
        assert!(!o.fingerprint().contains("2001:db8"));
        assert!(!o.fingerprint().contains("fe80::9"));
    }
}
