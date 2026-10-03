//! Hermetic harness: in-process CAs, loopback origins, the shipped hook wiring.
//! No OS trust store, no browser, no files.

use std::convert::Infallible;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, body::Incoming};
use hyper_util::rt::TokioIo;
use mitm_proxy::scan::{self, ProtectionMode, ScanResult};
use mitm_proxy::tls::TlsState;
use mitm_proxy::tunnel::ImageScanner;
use mitm_proxy::proxy;
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use tokio::net::TcpListener;

pub const CLEAN_HTML: &[u8] = b"<html>hello world</html>";
pub const BLOCKED_HTML: &[u8] = b"<html>explicit act shown here</html>";
pub const IMAGE_BYTES: &[u8] = b"stand-in image bytes";
pub const BLOCKED_PATH: &str = "/adult-platform/index.html";

pub struct TestCa {
    pub issuer: Issuer<'static, KeyPair>,
    pub der: CertificateDer<'static>,
}

pub fn make_ca() -> TestCa {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec![]).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let cert = params.self_signed(&key).unwrap();
    let der = CertificateDer::from(cert.der().to_vec());
    TestCa { issuer: Issuer::new(params, key), der }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub input: String,
    pub verdict: &'static str,
    pub score: u32,
}

pub struct Harness {
    pub proxy_port: u16,
    pub http_port: u16,
    pub https_port: u16,
    pub proxy_ca_der: CertificateDer<'static>,
    pub mode: Arc<AtomicU8>,
    pub url_seen: Arc<Mutex<Vec<Seen>>>,
    pub body_seen: Arc<Mutex<Vec<Seen>>>,
    pub origin_hits: Arc<AtomicUsize>,
    pub video_rx: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Bytes>>,
}

impl Harness {
    pub fn set_mode(&self, mode: ProtectionMode) {
        ProtectionMode::store(&self.mode, mode);
    }
}

fn record(
    log: &Arc<Mutex<Vec<Seen>>>,
    input: &str,
    result: &ScanResult,
) {
    let (verdict, score) = match result {
        ScanResult::Allow => ("allow", 0),
        ScanResult::Warn { score } => ("warn", *score),
        ScanResult::Block { score } => ("block", *score),
    };
    log.lock().unwrap().push(Seen { input: input.to_owned(), verdict, score });
}

/// Image scanner that blocks exactly `IMAGE_BYTES`, standing in for a model
/// so the claim does not depend on one.
pub fn image_blocker() -> ImageScanner {
    Box::new(|bytes| {
        if bytes == IMAGE_BYTES {
            ScanResult::Block { score: 99 }
        } else {
            ScanResult::Allow
        }
    })
}

pub async fn start(image: ImageScanner, mode: ProtectionMode) -> Harness {
    start_with_limit(image, mode, None).await
}

pub async fn start_with_limit(
    image: ImageScanner,
    mode: ProtectionMode,
    body_limit: Option<usize>,
) -> Harness {
    let proxy_ca = make_ca();
    let origin_ca = make_ca();
    let origin_hits = Arc::new(AtomicUsize::new(0));

    let mut roots = RootCertStore::empty();
    roots.add(origin_ca.der.clone()).unwrap();
    let outbound = Arc::new(
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    let tls = Arc::new(TlsState::from_issuer(proxy_ca.issuer).with_client_config(outbound));

    let http_port = spawn_origin(None, Arc::clone(&origin_hits)).await;
    let https_port = spawn_origin(Some(&origin_ca), Arc::clone(&origin_hits)).await;

    let mode_cell = mode.to_atomic();
    let mut hooks =
        scan::build_hooks(Arc::new(scan::build_default_engine()), image, Arc::clone(&mode_cell));
    if let Some(limit) = body_limit {
        hooks.body_limit = limit;
    }
    let url_seen = Arc::new(Mutex::new(Vec::new()));
    let body_seen = Arc::new(Mutex::new(Vec::new()));
    let inner_url = std::mem::replace(&mut hooks.url_scanner, Box::new(|_| ScanResult::Allow));
    let inner_body = std::mem::replace(&mut hooks.body_scanner, Box::new(|_| ScanResult::Allow));
    let log = Arc::clone(&url_seen);
    hooks.url_scanner = Box::new(move |url| {
        let r = inner_url(url);
        record(&log, url, &r);
        r
    });
    let log = Arc::clone(&body_seen);
    hooks.body_scanner = Box::new(move |html| {
        let r = inner_body(html);
        record(&log, html, &r);
        r
    });
    let (video_tx, video_rx) = tokio::sync::mpsc::channel(16);
    hooks.video_tx = video_tx;
    let hooks = Arc::new(hooks);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { break };
            let (tls, hooks) = (Arc::clone(&tls), Arc::clone(&hooks));
            tokio::spawn(async move {
                let _ = proxy::handle(stream, tls, hooks).await;
            });
        }
    });

    Harness {
        proxy_port,
        http_port,
        https_port,
        proxy_ca_der: proxy_ca.der,
        mode: mode_cell,
        url_seen,
        body_seen,
        origin_hits,
        video_rx: tokio::sync::Mutex::new(video_rx),
    }
}

type Body = http_body_util::combinators::BoxBody<Bytes, Infallible>;

fn respond(req: &Request<Incoming>) -> Response<Body> {
    let (ctype, body): (&str, &'static [u8]) = match req.uri().path() {
        "/body" => ("text/html", BLOCKED_HTML),
        "/pic.png" => ("image/png", IMAGE_BYTES),
        _ => ("text/html", CLEAN_HTML),
    };
    Response::builder()
        .header("content-type", ctype)
        .body(Full::new(Bytes::from_static(body)).map_err(|e: Infallible| match e {}).boxed())
        .unwrap()
}

/// Loopback origin. TLS when `ca` is given, plain HTTP otherwise.
async fn spawn_origin(ca: Option<&TestCa>, hits: Arc<AtomicUsize>) -> u16 {
    let acceptor = ca.map(|ca| {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
        params.subject_alt_names = vec![SanType::DnsName("localhost".to_owned().try_into().unwrap())];
        let cert = params.signed_by(&key, &ca.issuer).unwrap();
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        let cfg = ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert.der().clone()], key_der)
            .unwrap();
        tokio_rustls::TlsAcceptor::from(Arc::new(cfg))
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { break };
            let (acceptor, hits) = (acceptor.clone(), Arc::clone(&hits));
            tokio::spawn(async move {
                let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
                    hits.fetch_add(1, Ordering::SeqCst);
                    async move { Ok::<_, Infallible>(respond(&req)) }
                });
                match acceptor {
                    Some(a) => {
                        let Ok(tls) = a.accept(stream).await else { return };
                        let _ = hyper::server::conn::http1::Builder::new()
                            .serve_connection(TokioIo::new(tls), svc)
                            .await;
                    }
                    None => {
                        let _ = hyper::server::conn::http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), svc)
                            .await;
                    }
                }
            });
        }
    });
    port
}

/// reqwest over rustls: shares no code with the proxy's own helpers.
pub fn client(h: &Harness) -> reqwest::Client {
    reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}", h.proxy_port)).unwrap())
        .add_root_certificate(reqwest::Certificate::from_der(&h.proxy_ca_der).unwrap())
        .tls_built_in_root_certs(false)
        .build()
        .unwrap()
}

pub fn https_url(h: &Harness, path: &str) -> String {
    format!("https://localhost:{}{path}", h.https_port)
}

pub fn http_url(h: &Harness, path: &str) -> String {
    format!("http://127.0.0.1:{}{path}", h.http_port)
}
