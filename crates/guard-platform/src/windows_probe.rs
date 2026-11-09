//! Fixed HTTPS probe over the confirmed loopback HTTP CONNECT endpoint.
use crate::{Clock, Fault, Probe};
use guard_core::{config::PROBE_URL, policy};
use reqwest::{
    blocking::Client,
    dns::{Name, Resolve, Resolving},
    redirect::Policy,
    Proxy, StatusCode,
};
use std::{
    io::{self, Read},
    net::{Ipv4Addr, SocketAddr},
    sync::{Arc, OnceLock},
    time::Duration,
};

/// The proxy is a numeric IP and needs no DNS. A direct attempt to reach any
/// name fails here before reaching the network, even if proxy routing changes.
struct NoDns;
impl Resolve for NoDns {
    fn resolve(&self, _: Name) -> Resolving {
        Box::pin(async { Err(io::Error::from(io::ErrorKind::PermissionDenied).into()) })
    }
}

fn tls() -> rustls::ClientConfig {
    static CONFIG: OnceLock<rustls::ClientConfig> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            // Bundled roots and in-process verification: no OS certificate
            // lookups, AIA, CRL, proxy discovery, or user client certificates.
            let roots =
                rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .expect("ring supplies the Rustls protocol defaults")
            .with_root_certificates(roots)
            .with_no_client_auth()
        })
        .clone()
}

pub(super) fn sample(port: u16, clock: &dyn Clock) -> Result<Probe, Fault> {
    sample_with_tls(port, clock, tls())
}

fn sample_with_tls(
    port: u16,
    clock: &dyn Clock,
    tls: rustls::ClientConfig,
) -> Result<Probe, Fault> {
    if port == 0 {
        return Err(Fault::ProxySettings);
    }
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let proxy = Proxy::all(format!("http://{address}")).map_err(|_| Fault::ProxySettings)?;
    let client = Client::builder()
        .no_proxy()
        .proxy(proxy)
        .dns_resolver(Arc::new(NoDns))
        .tls_backend_preconfigured(tls)
        .https_only(true)
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|_| Fault::ProxyTransport)?;
    let start = clock.wall_seconds();
    let started = clock.now_ms();
    let response = client
        .get(PROBE_URL)
        .send()
        .map_err(|_| Fault::ProxyTransport)?;
    if response.status() != StatusCode::OK || response.remote_addr() != Some(address) {
        return Err(Fault::InvalidResponse);
    }
    let mut body = Vec::new();
    const BODY_LIMIT: u64 = 65_536;
    response
        .take(BODY_LIMIT + 1)
        .read_to_end(&mut body)
        .map_err(|_| Fault::ProxyTransport)?;
    if body.len() as u64 > BODY_LIMIT {
        return Err(Fault::InvalidResponse);
    }
    let raw = std::str::from_utf8(&body).map_err(|_| Fault::InvalidResponse)?;
    let sampled_at = clock.now_ms();
    policy::parse_verified_trace(
        raw,
        sampled_at.saturating_sub(started),
        start,
        clock.wall_seconds(),
        sampled_at,
    )
    .map_err(|_| Fault::InvalidResponse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SystemClock;
    use std::{
        io::Write,
        net::{TcpListener, TcpStream},
        thread,
        time::Instant,
    };

    fn fixture(serve: impl FnOnce(TcpStream) + Send + 'static) -> (u16, thread::JoinHandle<()>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let proxy = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e)
                        if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("loopback proxy did not receive CONNECT: {e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream.set_nonblocking(false).unwrap();
            let request = request_header(&mut stream).to_ascii_lowercase();
            assert!(request.starts_with("connect www.cloudflare.com:443 http/1.1\r\n"));
            assert!(!request.contains("authorization:"));
            drop(listener);
            serve(stream);
        });
        (port, proxy)
    }

    fn request_header(stream: &mut impl Read) -> String {
        let mut request = Vec::new();
        let mut byte = [0; 1];
        while !request.ends_with(b"\r\n\r\n") && request.len() < 1024 {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        assert!(request.ends_with(b"\r\n\r\n"));
        String::from_utf8(request).unwrap()
    }

    #[test]
    fn fixed_connect_does_not_authenticate_follow_redirect_or_accept_plaintext() {
        for response in [
            "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=fixture\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 302 Found\r\nLocation: https://unexpected.invalid/\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 200 Connection Established\r\n\r\nHTTP/1.1 200 OK\r\n\r\nloc=US\nip=192.0.2.1\n",
        ] {
            let (port, proxy) = fixture(move |mut stream| {
                stream.write_all(response.as_bytes()).unwrap();
            });
            assert!(matches!(sample(port, &SystemClock::default()), Err(Fault::ProxyTransport)));
            proxy.join().unwrap();
        }
    }

    #[test]
    fn verified_loopback_tunnel_requires_trusted_tls_and_http_200() {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["www.cloudflare.com".into()]).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let server = rustls::ServerConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.der().clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
            )
            .unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert.der().clone()).unwrap();
        let client = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let server = Arc::new(server);
        for ok in [true, false] {
            let server = server.clone();
            let (port, proxy) = fixture(move |mut stream| {
                stream
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .unwrap();
                let tls = rustls::ServerConnection::new(server).unwrap();
                let mut stream = rustls::StreamOwned::new(tls, stream);
                let request = request_header(&mut stream);
                assert!(request.starts_with("GET /cdn-cgi/trace HTTP/1.1\r\n"));
                if ok {
                    let body = "loc=US\nip=192.0.2.1\n";
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                } else {
                    stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: https://unexpected.invalid/\r\nContent-Length: 0\r\n\r\n").unwrap();
                }
                stream.flush().unwrap();
            });
            let result = sample_with_tls(port, &SystemClock::default(), client.clone());
            proxy.join().unwrap();
            if ok {
                let result = result.unwrap();
                assert_eq!(result.country, "US");
                assert_eq!(result.ip.to_string(), "192.0.2.1");
            } else {
                assert!(matches!(result, Err(Fault::InvalidResponse)));
            }
        }
    }
}
