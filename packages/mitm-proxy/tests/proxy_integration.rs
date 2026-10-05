//! End-to-end integration tests for the proxy.
//!
//! Each test spins up real TCP listeners (proxy + origin) on ephemeral ports
//! and uses a reqwest client configured to route traffic through the proxy.
//!
//! HTTP test  — exercises plain-HTTP forwarding (the `forward_http` path).
//! HTTPS test — exercises CONNECT tunnelling, TLS interception, leaf-cert
//!              generation, and the tunnel HTTP loop.

use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, body::Incoming};
use hyper_util::rt::TokioIo;
use mitm_proxy::{blocklist::HostBlocklist, proxy, tls::TlsState, tunnel::ScanHooks};
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use tokio::net::TcpListener;

// ── CA helpers ───────────────────────────────────────────────────────────────

struct TestCa {
    issuer: Issuer<'static, KeyPair>,
    /// DER of the CA cert — handed to TLS root stores.
    der: CertificateDer<'static>,
}

fn make_ca() -> TestCa {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec![]).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let cert = params.self_signed(&key).unwrap();
    let der = CertificateDer::from(cert.der().to_vec());
    let issuer = Issuer::new(params, key);
    TestCa { issuer, der }
}

// ── Origin server helpers ────────────────────────────────────────────────────

type PlainBody = http_body_util::combinators::BoxBody<Bytes, Infallible>;

fn plain_body(s: &'static [u8]) -> PlainBody {
    Full::new(Bytes::from_static(s))
        .map_err(|e: Infallible| match e {})
        .boxed()
}

/// Spawn a plain HTTP/1.1 server that always returns `200 OK` with `body`.
/// Returns the bound port.
async fn spawn_http_origin(body: &'static [u8]) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { break };
            tokio::spawn(async move {
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(stream),
                        hyper::service::service_fn(move |_req: Request<Incoming>| async move {
                            Ok::<_, Infallible>(
                                Response::builder()
                                    .header("content-type", "text/plain")
                                    .body(plain_body(body))
                                    .unwrap(),
                            )
                        }),
                    )
                    .await
                    .ok();
            });
        }
    });
    port
}

/// Spawn an HTTPS/1.1 server whose certificate is signed by `origin_ca`.
/// The cert covers `localhost`. Returns the bound port.
async fn spawn_https_origin(origin_ca: &TestCa, body: &'static [u8]) -> u16 {
    // Issue a leaf cert for "localhost" signed by the origin CA.
    let leaf_key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
    params.subject_alt_names = vec![rcgen::SanType::DnsName(
        "localhost".to_owned().try_into().unwrap(),
    )];
    let leaf_cert = params
        .signed_by(&leaf_key, &origin_ca.issuer)
        .unwrap();

    let cert_der: CertificateDer<'static> = leaf_cert.der().clone();
    let key_der: PrivateKeyDer<'static> =
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server_cfg = Arc::new(
        ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .unwrap(),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        let acceptor = tokio_rustls::TlsAcceptor::from(server_cfg);
        loop {
            let Ok((stream, _)) = listener.accept().await else { break };
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let tls_stream = acceptor.accept(stream).await.unwrap();
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(tls_stream),
                        hyper::service::service_fn(move |_req: Request<Incoming>| async move {
                            Ok::<_, Infallible>(
                                Response::builder()
                                    .header("content-type", "text/plain")
                                    .body(plain_body(body))
                                    .unwrap(),
                            )
                        }),
                    )
                    .await
                    .ok();
            });
        }
    });
    port
}

// ── Proxy helper ─────────────────────────────────────────────────────────────

/// Spawn the proxy using `tls`. Returns the bound port.
async fn spawn_proxy(tls: Arc<TlsState>) -> u16 {
    spawn_proxy_with(tls, ScanHooks::default()).await
}

async fn spawn_proxy_with(tls: Arc<TlsState>, scan: ScanHooks) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let scan = Arc::new(scan);
    tokio::spawn(async move {
        loop {
            let Ok((stream, peer)) = listener.accept().await else { break };
            let tls = Arc::clone(&tls);
            let scan = Arc::clone(&scan);
            tokio::spawn(async move {
                if let Err(e) = proxy::handle(stream, tls, scan).await {
                    tracing::debug!(%peer, "connection closed: {e:#}");
                }
            });
        }
    });
    port
}

// ── reqwest client helpers ────────────────────────────────────────────────────

/// Build a reqwest client that routes all traffic through the proxy at
/// `proxy_port` and trusts `trusted_ca_der` in addition to system roots.
fn proxy_client(proxy_port: u16, trusted_ca_der: &CertificateDer<'static>) -> reqwest::Client {
    let ca_cert = reqwest::Certificate::from_der(trusted_ca_der).unwrap();
    reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{proxy_port}")).unwrap())
        .add_root_certificate(ca_cert)
        // Disable built-in roots so only our CA (+ any system-added ones) is trusted;
        // keeps the test self-contained and avoids depending on the system store.
        .tls_built_in_root_certs(false)
        .build()
        .unwrap()
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Plain HTTP forwarding: the proxy receives an absolute-URI GET request,
/// opens a TCP connection to the origin, and relays the response.
#[tokio::test]
async fn http_proxy_forwards_plain_request() {
    // The proxy CA is used only for CONNECT/TLS — for plain HTTP it is not
    // exercised, but TlsState still needs to be constructed.
    let proxy_ca = make_ca();
    let tls = Arc::new(TlsState::from_issuer(proxy_ca.issuer));

    let origin_port = spawn_http_origin(b"hello from origin").await;
    let proxy_port = spawn_proxy(tls).await;

    let client = proxy_client(proxy_port, &proxy_ca.der);

    let resp = client
        .get(format!("http://127.0.0.1:{origin_port}/hello"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"hello from origin");
}

/// HTTPS CONNECT tunnelling: the proxy intercepts a TLS connection, presents
/// a dynamically-generated leaf cert signed by the proxy CA, forwards the
/// decrypted HTTP traffic to the real TLS origin, and relays the response.
#[tokio::test]
async fn https_proxy_intercepts_and_forwards() {
    // Two independent CAs:
    //   proxy_ca  — signs the leaf certs the proxy presents to the browser
    //   origin_ca — signs the cert of the real origin server
    let proxy_ca = make_ca();
    let origin_ca = make_ca();

    // Build a ClientConfig that the proxy will use for its outbound leg;
    // it must trust `origin_ca` since the origin cert is not system-trusted.
    let mut origin_root_store = RootCertStore::empty();
    origin_root_store.add(origin_ca.der.clone()).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let proxy_outbound_cfg = Arc::new(
        ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(origin_root_store)
            .with_no_client_auth(),
    );

    let tls = Arc::new(
        TlsState::from_issuer(proxy_ca.issuer)
            .with_client_config(proxy_outbound_cfg),
    );

    let origin_port = spawn_https_origin(&origin_ca, b"hello over tls").await;
    let proxy_port = spawn_proxy(tls).await;

    // reqwest trusts proxy_ca so it accepts the MITM leaf cert.
    let client = proxy_client(proxy_port, &proxy_ca.der);

    let resp = client
        .get(format!("https://localhost:{origin_port}/hello"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"hello over tls");
}

// ── Domain blocklist ─────────────────────────────────────────────────────────

fn hosts_blocklist(apexes: &[&str]) -> (tempfile::TempDir, Arc<HostBlocklist>) {
    use domain_blocklist::{Category, KeyId, LicenseId, MergedEntry, SourceId};
    use domain_normalize::RuleScope;
    use net_shield::blocklist::{ARTIFACT_FILE, MANIFEST_FILE};

    let signing = ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]);
    let entries: Vec<MergedEntry> = apexes
        .iter()
        .map(|d| MergedEntry {
            domain: (*d).to_owned(),
            scope: RuleScope::Apex,
            sources: vec![SourceId::StevenBlack],
            categories: vec![Category::Adult],
        })
        .collect();
    let artifact = domain_blocklist::build(
        &entries,
        LicenseId("MIT".to_owned()),
        Vec::new(),
        1,
        1_700_000_000,
        &[(KeyId("k1".to_owned()), signing.clone())],
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let slot = dir.path().join("current");
    std::fs::create_dir_all(&slot).unwrap();
    std::fs::write(slot.join(ARTIFACT_FILE), &artifact.fst_bytes).unwrap();
    std::fs::write(
        slot.join(MANIFEST_FILE),
        bincode::serialize(&artifact.manifest).unwrap(),
    )
    .unwrap();
    let keys = [(KeyId("k1".to_owned()), signing.verifying_key())];
    let list = HostBlocklist::load(dir.path(), &keys).unwrap();
    (dir, Arc::new(list))
}

fn hooks_with(list: Arc<HostBlocklist>) -> ScanHooks {
    ScanHooks {
        host_blocklist: Some(list),
        ..ScanHooks::default()
    }
}

async fn raw_connect_status(proxy_port: u16, authority: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", proxy_port))
        .await
        .unwrap();
    let request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut buf = [0u8; 64];
    let n = stream.read(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or("").to_owned()
}

#[tokio::test]
async fn a_listed_host_is_refused_on_plain_http_without_contacting_the_origin() {
    let proxy_ca = make_ca();
    let tls = Arc::new(TlsState::from_issuer(proxy_ca.issuer));
    let origin_port = spawn_http_origin(b"must not be served").await;
    let (_dir, list) = hosts_blocklist(&["blocked.example"]);
    let proxy_port = spawn_proxy_with(tls, hooks_with(list)).await;

    let client = proxy_client(proxy_port, &proxy_ca.der);
    let resp = client
        .get(format!("http://www.blocked.example:{origin_port}/"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 403);
    assert_ne!(resp.bytes().await.unwrap().as_ref(), b"must not be served");
}

#[tokio::test]
async fn an_unlisted_host_is_still_forwarded_when_a_list_is_loaded() {
    let proxy_ca = make_ca();
    let tls = Arc::new(TlsState::from_issuer(proxy_ca.issuer));
    let origin_port = spawn_http_origin(b"hello from origin").await;
    let (_dir, list) = hosts_blocklist(&["blocked.example"]);
    let proxy_port = spawn_proxy_with(tls, hooks_with(list)).await;

    let client = proxy_client(proxy_port, &proxy_ca.der);
    let resp = client
        .get(format!("http://127.0.0.1:{origin_port}/"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"hello from origin");
}

#[tokio::test]
async fn a_listed_connect_authority_is_refused_before_any_tunnel() {
    let proxy_ca = make_ca();
    let tls = Arc::new(TlsState::from_issuer(proxy_ca.issuer));
    let (_dir, list) = hosts_blocklist(&["blocked.example"]);
    let proxy_port = spawn_proxy_with(tls, hooks_with(list)).await;

    let status = raw_connect_status(proxy_port, "www.blocked.example:443").await;

    assert!(status.contains("403"), "got {status:?}");
}

async fn tls_handshake_through_connect(
    proxy_port: u16,
    proxy_ca_der: &CertificateDer<'static>,
    authority: &str,
    sni: &'static str,
) -> std::io::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", proxy_port))
        .await
        .unwrap();
    let request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).await?;
        head.push(byte[0]);
    }
    assert!(String::from_utf8_lossy(&head).contains("200"));

    let mut roots = RootCertStore::empty();
    roots.add(proxy_ca_der.clone()).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let name = rustls::pki_types::ServerName::try_from(sni).unwrap();
    connector.connect(name, stream).await.map(|_| ())
}

#[tokio::test]
async fn an_unlisted_connect_authority_with_a_listed_sni_is_refused() {
    let proxy_ca = make_ca();
    let tls = Arc::new(TlsState::from_issuer(proxy_ca.issuer));
    let (_dir, list) = hosts_blocklist(&["blocked.example"]);
    let proxy_port = spawn_proxy_with(tls, hooks_with(list)).await;

    let unlisted =
        tls_handshake_through_connect(proxy_port, &proxy_ca.der, "127.0.0.1:9", "fine.example").await;
    let listed =
        tls_handshake_through_connect(proxy_port, &proxy_ca.der, "127.0.0.1:9", "www.blocked.example").await;

    assert!(unlisted.is_ok(), "control handshake failed: {unlisted:?}");
    assert!(listed.is_err());
}
