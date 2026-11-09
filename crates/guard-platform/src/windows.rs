//! Windows current-user proxy preflight. Does not launch clients or inspect WSL.
#[cfg(windows)]
#[path = "windows_native.rs"]
mod native;
#[cfg(any(windows, test))]
#[path = "windows_probe.rs"]
mod probe;
#[cfg(windows)]
pub use native::WindowsPlatform;

#[cfg(not(windows))]
mod unsupported {
    use crate::*;
    pub struct WindowsPlatform;
    impl Platform for WindowsPlatform {
        fn name(&self) -> &'static str {
            "unsupported"
        }
        fn clients(&self) -> ClientAvailability {
            ClientAvailability::default()
        }
        fn inspect(&self, id: RuleId, _: &Profile, _: &dyn Clock) -> Check {
            Check::new(id, Status::Skipped, Fault::Unsupported.message())
        }
        fn local(&self, _: &Profile) -> Result<LocalSnapshot, Fault> {
            Err(Fault::Unsupported)
        }
        fn probe(&self, _: &Profile, _: &dyn Clock) -> Result<Probe, Fault> {
            Err(Fault::Unsupported)
        }
    }
}
#[cfg(not(windows))]
pub use unsupported::WindowsPlatform;

#[cfg(any(windows, test))]
mod rules {
    use std::net::Ipv4Addr;

    pub(super) fn manual_port(server: &str) -> Option<u16> {
        fn endpoint(value: &str) -> Option<u16> {
            let digits = value.trim().strip_prefix("127.0.0.1:")?;
            let port = digits.parse::<u16>().ok()?;
            (port != 0 && digits == port.to_string()).then_some(port)
        }
        if !server.contains('=') {
            return endpoint(server);
        }
        let (mut http, mut https) = (None, None);
        for item in server.split(';') {
            let (scheme, value) = item.trim().split_once('=')?;
            let slot = if scheme.eq_ignore_ascii_case("http") {
                &mut http
            } else if scheme.eq_ignore_ascii_case("https") {
                &mut https
            } else {
                return None;
            };
            if slot.replace(endpoint(value)?).is_some() {
                return None;
            }
        }
        http.filter(|port| Some(*port) == https)
    }

    /// Fixed allowlist. In WinINet a wildcard such as "127.*" is a host pattern,
    /// not an address range. Never accept arbitrary domain or textual IP globs.
    pub(super) fn safe_bypass(bypass: &str) -> bool {
        bypass.trim().is_empty()
            || bypass.split(';').all(|part| {
                matches!(
                    part.trim().to_ascii_lowercase().as_str(),
                    "localhost"
                        | "loopback"
                        | "127.0.0.1"
                        | "[::1]"
                        | "<local>"
                        | "*.local"
                        | "10.0.0.0/8"
                        | "172.16.0.0/12"
                        | "192.168.0.0/16"
                )
            })
    }

    pub(super) fn listener_pid(
        rows: impl IntoIterator<Item = (Ipv4Addr, u16, u32)>,
        port: u16,
    ) -> Option<u32> {
        let mut owner = None;
        for (addr, row_port, pid) in rows {
            if port != row_port || (!addr.is_unspecified() && addr != Ipv4Addr::LOCALHOST) {
                continue;
            }
            if addr.is_unspecified() || pid == 0 || owner.is_some_and(|prev| prev != pid) {
                return None;
            }
            owner = Some(pid);
        }
        owner
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn only_one_explicit_loopback_http_endpoint_is_supported() {
            for s in ["127.0.0.1:7897", "HTTPS=127.0.0.1:7897;http=127.0.0.1:7897"] {
                assert_eq!(manual_port(s), Some(7897));
            }
            for s in [
                "",
                "localhost:7897",
                "127.0.0.1:0",
                "127.0.0.1:07897",
                "example.invalid:7897",
                "user@127.0.0.1:7897",
                "http=127.0.0.1:7897",
                "http=127.0.0.1:7897;https=127.0.0.1:7898",
                "http=127.0.0.1:7897;https=127.0.0.1:7897;HTTP=127.0.0.1:7897",
                "http=127.0.0.1:7897;https=127.0.0.1:7897;socks=127.0.0.1:7897",
                "http=127.0.0.1:7897;https=https://127.0.0.1:7897",
            ] {
                assert_eq!(manual_port(s), None, "{s}");
            }
        }

        #[test]
        fn local_bypass_list_does_not_accept_domain_patterns_or_unknown_syntax() {
            assert!(safe_bypass(""));
            assert!(safe_bypass(
                "LOCALHOST;127.0.0.1;[::1];<local>;*.local;10.0.0.0/8;192.168.0.0/16"
            ));
            for s in [
                "127.*",
                "192.168.*",
                "localhost;*.example.invalid",
                "localhost,*",
                "localhost;;127.0.0.1",
                "<-loopback>",
            ] {
                assert!(!safe_bypass(s), "{s}");
            }
        }

        #[test]
        fn listener_cannot_be_wildcard_missing_or_ambiguous() {
            let port = 7897;
            let row = (Ipv4Addr::LOCALHOST, port, 42);
            let wildcard = (Ipv4Addr::UNSPECIFIED, port, 42);
            assert_eq!(
                listener_pid([row, (Ipv4Addr::LOCALHOST, 7898, 43)], port),
                Some(42)
            );
            assert_eq!(listener_pid([wildcard], port), None);
            assert_eq!(listener_pid([row, wildcard], port), None);
            assert_eq!(
                listener_pid([row, (Ipv4Addr::LOCALHOST, port, 43)], port),
                None
            );
            assert_eq!(listener_pid([(Ipv4Addr::LOCALHOST, port, 0)], port), None);
            assert_eq!(listener_pid([], port), None);
        }
    }
}
