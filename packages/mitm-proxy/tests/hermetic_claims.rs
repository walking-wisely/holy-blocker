//! Hermetic checks for the `mitm-proxy.*` claims in docs/engineering/coverage.md.
//! The claims, and what a pass here does not prove, are in
//! docs/components/mitm-proxy/claim.md.
//!
//! Tests named `known_gap_*` assert the wrong behaviour that exists today and
//! go red when the gap is fixed, so the fix has to flip them. Each is paired
//! with a `control_*` test that succeeds through the same harness.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use common::*;
use mitm_proxy::scan::{ProtectionMode, ScanResult};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use x509_parser::oid_registry::OID_X509_EXT_AUTHORITY_KEY_IDENTIFIER;
use x509_parser::prelude::*;

async fn text(resp: reqwest::Response) -> (u16, Vec<u8>) {
    let status = resp.status().as_u16();
    (status, resp.bytes().await.unwrap().to_vec())
}

// mitm-proxy.https-scan-block

#[tokio::test]
async fn https_url_match_is_refused_before_the_origin_is_reached() {
    let h = start(Box::new(|_| ScanResult::Allow), ProtectionMode::Full).await;
    let (status, body) = text(client(&h).get(https_url(&h, BLOCKED_PATH)).send().await.unwrap()).await;

    assert_eq!((status, body.as_slice()), (403, b"Blocked\n".as_slice()));
    assert_eq!(h.origin_hits.load(Ordering::SeqCst), 0);
    let seen = h.url_seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!((seen[0].input.as_str(), seen[0].verdict), (BLOCKED_PATH, "block"));
    assert!(seen[0].score > 0);
}

#[tokio::test]
async fn https_clean_page_is_relayed_byte_for_byte() {
    let h = start(Box::new(|_| ScanResult::Allow), ProtectionMode::Full).await;
    let resp = client(&h).get(https_url(&h, "/clean")).send().await.unwrap();
    assert_eq!(resp.headers()["content-type"], "text/html");
    let (status, body) = text(resp).await;

    assert_eq!((status, body.as_slice()), (200, CLEAN_HTML));
    assert_eq!(h.url_seen.lock().unwrap()[0].verdict, "allow");
    let seen = h.body_seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        (seen[0].input.as_bytes(), seen[0].verdict),
        (CLEAN_HTML, "allow")
    );
}

#[tokio::test]
async fn https_body_match_is_refused_after_the_origin_answers() {
    let h = start(Box::new(|_| ScanResult::Allow), ProtectionMode::Full).await;
    let (status, body) = text(client(&h).get(https_url(&h, "/body")).send().await.unwrap()).await;

    assert_eq!((status, body.as_slice()), (403, b"Blocked\n".as_slice()));
    assert_eq!(h.origin_hits.load(Ordering::SeqCst), 1);
    let seen = h.body_seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!((seen[0].input.as_bytes(), seen[0].verdict), (BLOCKED_HTML, "block"));
    assert!(seen[0].score > 0);
}

#[tokio::test]
async fn https_image_verdict_block_is_refused() {
    let h = start(image_blocker(), ProtectionMode::Full).await;
    let (status, body) = text(client(&h).get(https_url(&h, "/pic.png")).send().await.unwrap()).await;
    assert_eq!((status, body.as_slice()), (403, b"Blocked\n".as_slice()));
}

#[tokio::test]
async fn body_exactly_at_the_scan_limit_is_scanned() {
    let h = start_with_limit(
        Box::new(|_| ScanResult::Allow),
        ProtectionMode::Full,
        Some(BLOCKED_HTML.len()),
    )
    .await;
    let (status, _) = text(client(&h).get(https_url(&h, "/body")).send().await.unwrap()).await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn body_over_the_scan_limit_is_relayed_unscanned() {
    let h = start_with_limit(
        Box::new(|_| ScanResult::Allow),
        ProtectionMode::Full,
        Some(BLOCKED_HTML.len() - 1),
    )
    .await;
    let (status, body) = text(client(&h).get(https_url(&h, "/body")).send().await.unwrap()).await;
    assert_eq!((status, body.as_slice()), (200, BLOCKED_HTML));
    assert!(h.body_seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn video_segments_are_teed_by_path_not_query() {
    let h = start(Box::new(|_| ScanResult::Allow), ProtectionMode::Full).await;
    let c = client(&h);
    for path in ["/seg.ts?token=abc", "/seg.m4s"] {
        let (status, body) = text(c.get(https_url(&h, path)).send().await.unwrap()).await;
        assert_eq!((status, body.as_slice()), (200, CLEAN_HTML), "{path}");
        let teed = h.video_rx.lock().await.try_recv().expect("segment teed");
        assert_eq!(teed.as_ref(), CLEAN_HTML, "{path}");
    }
}

#[tokio::test]
async fn text_verdicts_follow_protection_mode_changes_at_runtime() {
    let h = start(Box::new(|_| ScanResult::Allow), ProtectionMode::Full).await;
    let c = client(&h);
    let url = https_url(&h, "/body");

    assert_eq!(text(c.get(&url).send().await.unwrap()).await.0, 403);
    for relaxed in [ProtectionMode::WarnOnly, ProtectionMode::Off] {
        h.set_mode(relaxed);
        let (status, body) = text(c.get(&url).send().await.unwrap()).await;
        assert_eq!((status, body.as_slice()), (200, BLOCKED_HTML), "{relaxed:?}");
    }
    h.set_mode(ProtectionMode::Full);
    assert_eq!(text(c.get(&url).send().await.unwrap()).await.0, 403);
}

// Row: Plain HTTP request content

#[tokio::test]
async fn control_https_blocks_the_content_plain_http_lets_through() {
    let h = start(Box::new(|_| ScanResult::Allow), ProtectionMode::Full).await;
    let c = client(&h);
    assert_eq!(text(c.get(https_url(&h, BLOCKED_PATH)).send().await.unwrap()).await.0, 403);
    assert_eq!(text(c.get(https_url(&h, "/body")).send().await.unwrap()).await.0, 403);
}

#[tokio::test]
async fn known_gap_plain_http_is_forwarded_unscanned() {
    let h = start(image_blocker(), ProtectionMode::Full).await;
    let c = client(&h);

    let (status, body) = text(c.get(http_url(&h, BLOCKED_PATH)).send().await.unwrap()).await;
    assert_eq!((status, body.as_slice()), (200, CLEAN_HTML));
    let (status, body) = text(c.get(http_url(&h, "/body")).send().await.unwrap()).await;
    assert_eq!((status, body.as_slice()), (200, BLOCKED_HTML));
    let (status, body) = text(c.get(http_url(&h, "/pic.png")).send().await.unwrap()).await;
    assert_eq!((status, body.as_slice()), (200, IMAGE_BYTES));

    assert_eq!(h.origin_hits.load(Ordering::SeqCst), 3);
    assert!(h.url_seen.lock().unwrap().is_empty(), "no hook ran for plain HTTP");
    assert!(h.body_seen.lock().unwrap().is_empty());
}

// Row: ProtectionMode gate on the image path

#[tokio::test]
async fn control_text_gate_honours_off_and_image_blocks_in_full() {
    let off = start(image_blocker(), ProtectionMode::Off).await;
    let (status, body) = text(client(&off).get(https_url(&off, "/body")).send().await.unwrap()).await;
    assert_eq!((status, body.as_slice()), (200, BLOCKED_HTML));

    let full = start(image_blocker(), ProtectionMode::Full).await;
    let (status, _) = text(client(&full).get(https_url(&full, "/pic.png")).send().await.unwrap()).await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn known_gap_image_scan_ignores_protection_mode() {
    for mode in [ProtectionMode::Off, ProtectionMode::WarnOnly] {
        let h = start(image_blocker(), mode).await;
        let (status, body) =
            text(client(&h).get(https_url(&h, "/pic.png")).send().await.unwrap()).await;
        assert_eq!((status, body.as_slice()), (403, b"Blocked\n".as_slice()), "{mode:?}");
    }
}

// Row: OpenSSL-based clients through the proxy

/// Open a CONNECT tunnel with a hand-written request, then TLS over it, and
/// return the leaf the proxy presented.
async fn presented_leaf(h: &Harness) -> Vec<u8> {
    let mut tcp = TcpStream::connect(("127.0.0.1", h.proxy_port)).await.unwrap();
    let target = format!("localhost:{}", h.https_port);
    tcp.write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(tcp.read_u8().await.unwrap());
    }
    assert!(head.starts_with(b"HTTP/1.1 200"), "CONNECT refused");

    let mut roots = RootCertStore::empty();
    roots.add(h.proxy_ca_der.clone()).unwrap();
    let cfg = ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let tls = tokio_rustls::TlsConnector::from(Arc::new(cfg))
        .connect(ServerName::try_from("localhost").unwrap(), tcp)
        .await
        .expect("handshake against the proxy CA");
    tls.get_ref().1.peer_certificates().unwrap()[0].to_vec()
}

#[tokio::test]
async fn control_leaf_is_issued_by_the_ca_for_the_requested_host() {
    let h = start(Box::new(|_| ScanResult::Allow), ProtectionMode::Full).await;
    let der = presented_leaf(&h).await;
    let (_, leaf) = X509Certificate::from_der(&der).unwrap();
    let (_, ca) = X509Certificate::from_der(&h.proxy_ca_der).unwrap();

    leaf.verify_signature(Some(ca.public_key())).expect("leaf signed by the proxy CA");
    let san = leaf.subject_alternative_name().unwrap().expect("SAN present");
    assert!(
        san.value.general_names.iter().any(|n| matches!(n, GeneralName::DNSName("localhost")))
    );
}

#[tokio::test]
async fn known_gap_leaf_has_no_authority_key_identifier() {
    let h = start(Box::new(|_| ScanResult::Allow), ProtectionMode::Full).await;
    let der = presented_leaf(&h).await;
    let (_, leaf) = X509Certificate::from_der(&der).unwrap();

    let aki = leaf.get_extension_unique(&OID_X509_EXT_AUTHORITY_KEY_IDENTIFIER).unwrap();
    assert!(aki.is_none());
}
