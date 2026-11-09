//! Real curl + local mock CONNECT proxy/upstreams only. The upstream IPv4/IPv6
//! families are exercised while every socket and resolver override is confined
//! to loopback. These are transport simulations, not TLS or Claude acceptance,
//! and do not claim an arbitrary program is forcibly isolated from the network.
use guard_core::{config::Profile, proxy::ClientProxy};
use std::{
    io::{self, Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
const BODY: &str = "loopback-proxy-fixture";
const SOCKET_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Default)]
struct Records {
    connections: AtomicUsize,
    tunneled_requests: AtomicUsize,
    connects: Mutex<Vec<String>>,
    errors: Mutex<Vec<String>>,
}
struct Server {
    address: SocketAddr,
    records: Arc<Records>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Server {
    fn start(
        ip: IpAddr,
        handler: impl Fn(TcpStream, &Records) -> io::Result<()> + Send + 'static,
    ) -> Self {
        assert!(ip.is_loopback());
        let listener = TcpListener::bind(SocketAddr::new(ip, 0)).expect("loopback listener");
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let records = Arc::new(Records::default());
        let stop = Arc::new(AtomicBool::new(false));
        let thread_records = records.clone();
        let thread_stop = stop.clone();
        let worker = thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, peer)) => {
                        if !peer.ip().is_loopback() {
                            thread_records
                                .errors
                                .lock()
                                .unwrap()
                                .push("non-loopback peer".into());
                            continue;
                        }
                        thread_records.connections.fetch_add(1, Ordering::SeqCst);
                        // macOS may inherit the listener's nonblocking flag;
                        // accepted sockets use bounded blocking I/O instead.
                        if let Err(error) = stream
                            .set_nonblocking(false)
                            .and_then(|_| stream.set_read_timeout(Some(SOCKET_TIMEOUT)))
                            .and_then(|_| stream.set_write_timeout(Some(SOCKET_TIMEOUT)))
                            .and_then(|_| handler(stream, &thread_records))
                        {
                            thread_records
                                .errors
                                .lock()
                                .unwrap()
                                .push(error.to_string());
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => {
                        thread_records
                            .errors
                            .lock()
                            .unwrap()
                            .push(error.to_string());
                        break;
                    }
                }
            }
        });
        Self {
            address,
            records,
            stop,
            worker: Some(worker),
        }
    }
    fn assert_healthy(&self) {
        let errors = self.records.errors.lock().unwrap();
        assert!(errors.is_empty(), "{errors:?}");
    }
    fn connections(&self) -> usize {
        self.records.connections.load(Ordering::SeqCst)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let joined = worker.join();
            assert!(
                joined.is_ok() || thread::panicking(),
                "mock server thread panicked"
            );
        }
    }
}
fn header(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() >= 8192 {
            return Err(io::Error::other("mock header size limit"));
        }
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
    }
    Ok(bytes)
}
fn upstream(ip: IpAddr) -> Server {
    Server::start(ip, |mut stream, _| {
        let request = header(&mut stream)?;
        if !request.starts_with(b"GET /fixture HTTP/1.1\r\n") {
            return Err(io::Error::other(format!(
                "unexpected mock upstream request: {}",
                String::from_utf8_lossy(&request)
                    .lines()
                    .next()
                    .unwrap_or("")
            )));
        }
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
            BODY.len()
        )?;
        // Dropping the accepted stream closes it after the complete response.
        // A redundant shutdown can race a peer close (ENOTCONN on macOS).
        Ok(())
    })
}
#[derive(Clone, Copy)]
enum ProxyBehavior {
    Forward,
    Reject,
    Disconnect,
    DisconnectAfterTunnel,
}
fn proxy(upstream: SocketAddr, host: &'static str, behavior: ProxyBehavior) -> Server {
    assert!(upstream.ip().is_loopback());
    Server::start(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        move |mut client, records| {
            let request = header(&mut client)?;
            let first = String::from_utf8_lossy(&request)
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            records.connects.lock().unwrap().push(first.clone());
            if first != format!("CONNECT {host}:{} HTTP/1.1", upstream.port()) {
                return Err(io::Error::other("unexpected mock CONNECT authority"));
            }
            match behavior {
                ProxyBehavior::Reject => {
                    client.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
                }
                ProxyBehavior::Disconnect => {}
                ProxyBehavior::DisconnectAfterTunnel => {
                    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
                    let request = header(&mut client)?;
                    if !request.starts_with(b"GET /fixture HTTP/1.1\r\n") {
                        return Err(io::Error::other("unexpected mock tunneled request"));
                    }
                    records.tunneled_requests.fetch_add(1, Ordering::SeqCst);
                    // Interrupt only after the tunnel is established and the
                    // client has sent its request. No upstream socket is opened.
                }
                ProxyBehavior::Forward => {
                    // The CONNECT authority is validated, never resolved. The only
                    // forwarding target is this prebound loopback SocketAddr.
                    let mut target = TcpStream::connect_timeout(&upstream, SOCKET_TIMEOUT)?;
                    target.set_read_timeout(Some(SOCKET_TIMEOUT))?;
                    target.set_write_timeout(Some(SOCKET_TIMEOUT))?;
                    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
                    target.write_all(&header(&mut client)?)?;
                    let mut response = Vec::new();
                    target.take(16 * 1024).read_to_end(&mut response)?;
                    client.write_all(&response)?;
                }
            }
            Ok(()) // Drop closes the tunnel, including intentional disconnects.
        },
    )
}
fn fixture_profile(port: u16) -> Profile {
    Profile {
        schema_version: 1,
        proxy_url: format!("http://127.0.0.1:{port}"),
        expected_country: None,
        expected_proxy_executable: "/fixture/mock-proxy".into(),
        non_mainland_timezone: false,
    }
}
fn curl_path() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(std::env::var_os("SystemRoot").expect("Windows system root"))
            .join("System32/curl.exe")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/usr/bin/curl")
    }
}
fn client(target: SocketAddr, host: &str, configuration: Option<&ClientProxy>) -> Output {
    assert!(target.ip().is_loopback());
    let resolved = match target.ip() {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    };
    let mut command = Command::new(curl_path());
    command
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.env(
        "SystemRoot",
        std::env::var_os("SystemRoot").expect("Windows system root"),
    );
    if let Some(configuration) = configuration {
        command.envs(&configuration.environment);
    }
    command.args([
        "-q",
        "--silent",
        "--show-error",
        "--fail",
        "--max-time",
        "2",
        "--connect-timeout",
        "1",
        "--max-filesize",
        "1024",
        "--retry",
        "0",
        "--proxytunnel",
        "--proto",
        "=http",
        "--resolve",
        &format!("{host}:{}:{resolved}", target.port()),
        &format!("http://{host}:{}/fixture", target.port()),
    ]);
    let mut child = command.spawn().expect("system curl mock client");
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("mock curl client exceeded its deadline");
        }
        thread::sleep(Duration::from_millis(5));
    }
    child.wait_with_output().unwrap()
}
fn families() -> [(IpAddr, &'static str); 2] {
    [
        (IpAddr::V4(Ipv4Addr::LOCALHOST), "v4.fixture.invalid"),
        (IpAddr::V6(Ipv6Addr::LOCALHOST), "v6.fixture.invalid"),
    ]
}
fn assert_direct_control_reachable(upstream: &Server, host: &str, expected_connections: usize) {
    // This explicit control is intentionally direct to the mock loopback server.
    // It proves failed proxy cases could have reached the target if a client
    // fallback existed; --resolve prevents DNS even in this direct control.
    let direct = client(upstream.address, host, None);
    assert!(
        direct.status.success(),
        "{}",
        String::from_utf8_lossy(&direct.stderr)
    );
    assert_eq!(direct.stdout, BODY.as_bytes());
    assert_eq!(upstream.connections(), expected_connections);
    upstream.assert_healthy();
}
#[test]
fn ipv4_and_ipv6_upstreams_are_reached_via_the_shared_launch_proxy_environment() {
    for (ip, host) in families() {
        let upstream = upstream(ip);
        let proxy = proxy(upstream.address, host, ProxyBehavior::Forward);
        let configuration =
            ClientProxy::from_profile(&fixture_profile(proxy.address.port())).unwrap();
        let response = client(upstream.address, host, Some(&configuration));
        assert!(
            response.status.success(),
            "{}; proxy errors: {:?}; upstream errors: {:?}",
            String::from_utf8_lossy(&response.stderr),
            proxy.records.errors.lock().unwrap(),
            upstream.records.errors.lock().unwrap()
        );
        assert_eq!(response.stdout, BODY.as_bytes());
        assert_eq!(proxy.connections(), 1);
        assert_eq!(proxy.records.connects.lock().unwrap().len(), 1);
        assert_eq!(upstream.connections(), 1);
        proxy.assert_healthy();
        upstream.assert_healthy();
    }
}
#[test]
fn unavailable_proxy_never_falls_back_to_a_reachable_ipv4_or_ipv6_target() {
    for (ip, host) in families() {
        let upstream = upstream(ip);
        let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let configuration = ClientProxy::from_profile(&fixture_profile(port)).unwrap();
        let response = client(upstream.address, host, Some(&configuration));
        assert!(!response.status.success());
        assert_eq!(upstream.connections(), 0);
        assert_direct_control_reachable(&upstream, host, 1);
    }
}
fn assert_proxy_failure_no_fallback(behavior: ProxyBehavior) {
    for (ip, host) in families() {
        let upstream = upstream(ip);
        let proxy = proxy(upstream.address, host, behavior);
        let configuration =
            ClientProxy::from_profile(&fixture_profile(proxy.address.port())).unwrap();
        let response = client(upstream.address, host, Some(&configuration));
        assert!(!response.status.success());
        assert_eq!(proxy.connections(), 1);
        assert_eq!(proxy.records.connects.lock().unwrap().len(), 1);
        assert_eq!(
            proxy.records.tunneled_requests.load(Ordering::SeqCst),
            usize::from(matches!(behavior, ProxyBehavior::DisconnectAfterTunnel))
        );
        assert_eq!(upstream.connections(), 0);
        assert_direct_control_reachable(&upstream, host, 1);
        proxy.assert_healthy();
    }
}
#[test]
fn rejected_connect_never_falls_back_to_ipv4_or_ipv6_direct_connections() {
    assert_proxy_failure_no_fallback(ProxyBehavior::Reject);
}
#[test]
fn disconnected_connect_never_falls_back_to_ipv4_or_ipv6_direct_connections() {
    assert_proxy_failure_no_fallback(ProxyBehavior::Disconnect);
}

#[test]
fn established_tunnel_interruption_never_falls_back_to_ipv4_or_ipv6_direct_connections() {
    assert_proxy_failure_no_fallback(ProxyBehavior::DisconnectAfterTunnel);
}
