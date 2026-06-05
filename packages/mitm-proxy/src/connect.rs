use crate::tls::TlsState;
use crate::tunnel;
use anyhow::{Context, Result};
use hyper::http::uri::Authority;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use std::sync::Arc;
use tokio::net::TcpStream;

/// Parse a TLS 1.x ClientHello record and return the SNI hostname, if present.
///
/// Returns `None` when the buffer is too short, does not start with a TLS
/// handshake record, or contains no SNI extension — without panicking.
pub fn extract_sni(buf: &[u8]) -> Option<String> {
    // TLS record header: content_type(1) + legacy_version(2) + length(2)
    if buf.len() < 5 {
        return None;
    }
    if buf[0] != 0x16 {
        return None; // not a Handshake record
    }
    let record_len = u16::from_be_bytes([buf[3], buf[4]]) as usize;
    if buf.len() < 5 + record_len {
        return None;
    }

    let hs = &buf[5..5 + record_len];
    // Handshake header: msg_type(1) + length(3)
    if hs.len() < 4 || hs[0] != 0x01 {
        return None; // not a ClientHello
    }

    let hello = &hs[4..];
    // ClientHello body: client_version(2) + random(32) + ...
    let mut pos: usize = 34; // skip version + random

    // session_id: length(1) + data
    let sid_len = *hello.get(pos)? as usize;
    pos = pos.checked_add(1 + sid_len)?;

    // cipher_suites: length(2) + data
    if hello.len() < pos + 2 {
        return None;
    }
    let cs_len = u16::from_be_bytes([hello[pos], hello[pos + 1]]) as usize;
    pos = pos.checked_add(2 + cs_len)?;

    // compression_methods: length(1) + data
    let cm_len = *hello.get(pos)? as usize;
    pos = pos.checked_add(1 + cm_len)?;

    // extensions: total_length(2) + [type(2) + length(2) + data...]
    if hello.len() < pos + 2 {
        return None;
    }
    let ext_total = u16::from_be_bytes([hello[pos], hello[pos + 1]]) as usize;
    pos += 2;
    let ext_end = pos + ext_total;
    if hello.len() < ext_end {
        return None;
    }

    while pos + 4 <= ext_end {
        let ext_type = u16::from_be_bytes([hello[pos], hello[pos + 1]]);
        let ext_data_len = u16::from_be_bytes([hello[pos + 2], hello[pos + 3]]) as usize;
        pos += 4;
        if pos + ext_data_len > ext_end {
            return None;
        }

        if ext_type == 0x0000 {
            // SNI extension body: list_length(2) + name_type(1) + name_length(2) + name
            let data = &hello[pos..pos + ext_data_len];
            if data.len() < 5 {
                return None;
            }
            if data[2] != 0x00 {
                return None; // only host_name(0) is defined
            }
            let name_len = u16::from_be_bytes([data[3], data[4]]) as usize;
            if data.len() < 5 + name_len {
                return None;
            }
            return std::str::from_utf8(&data[5..5 + name_len])
                .ok()
                .map(str::to_owned);
        }

        pos += ext_data_len;
    }

    None
}

/// Handle an HTTP CONNECT tunnel:
///
/// 1. Accept the TLS ClientHello from the browser, reading the SNI.
/// 2. Complete a TLS handshake with the browser using a leaf cert for that SNI.
/// 3. Open a TLS connection to the real origin.
/// 4. Relay bytes between the two legs via `tunnel::run`.
pub async fn handle_connect(
    target: Authority,
    upgraded: TokioIo<Upgraded>,
    tls: Arc<TlsState>,
) -> Result<()> {
    let acceptor = tokio_rustls::LazyConfigAcceptor::new(
        rustls::server::Acceptor::default(),
        upgraded,
    );
    let start = acceptor.await.context("waiting for TLS ClientHello")?;

    let sni = start
        .client_hello()
        .server_name()
        .map(str::to_owned)
        .unwrap_or_else(|| target.host().to_owned());

    let server_cfg = tls.server_config(&sni)?;
    let browser_tls = start
        .into_stream(server_cfg)
        .await
        .context("TLS handshake with browser")?;

    let host = target.host();
    let port = target.port_u16().unwrap_or(443);
    let origin_tcp = TcpStream::connect((host, port))
        .await
        .with_context(|| format!("connecting to {host}:{port}"))?;

    let client_cfg = TlsState::client_config()?;
    let connector = tokio_rustls::TlsConnector::from(client_cfg);
    let server_name = rustls::pki_types::ServerName::try_from(sni.clone())
        .with_context(|| format!("invalid server name: {sni}"))?
        .to_owned();
    let origin_tls = connector
        .connect(server_name, origin_tcp)
        .await
        .context("TLS handshake with origin")?;

    tunnel::run(browser_tls, origin_tls).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::crypto::ring::default_provider;
    use rustls::pki_types::ServerName;
    use rustls::{ClientConfig, ClientConnection, RootCertStore};
    use std::sync::Arc;

    /// Generate the raw bytes of a TLS ClientHello for the given hostname using
    /// rustls, so the test exercises a realistic wire-format record.
    fn client_hello_bytes(hostname: &str) -> Vec<u8> {
        let config = Arc::new(
            ClientConfig::builder_with_provider(Arc::new(default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(RootCertStore::empty())
                .with_no_client_auth(),
        );
        let sni = ServerName::try_from(hostname).unwrap().to_owned();
        let mut conn = ClientConnection::new(config, sni).unwrap();
        let mut buf = Vec::new();
        conn.write_tls(&mut buf).unwrap();
        buf
    }

    #[test]
    fn sni_extracted_from_realistic_client_hello() {
        let bytes = client_hello_bytes("example.com");
        assert_eq!(extract_sni(&bytes), Some("example.com".to_owned()));
    }

    #[test]
    fn sni_extracted_for_subdomain() {
        let bytes = client_hello_bytes("sub.example.org");
        assert_eq!(extract_sni(&bytes), Some("sub.example.org".to_owned()));
    }

    #[test]
    fn empty_buffer_returns_none() {
        assert_eq!(extract_sni(&[]), None);
    }

    #[test]
    fn non_handshake_record_type_returns_none() {
        // Application-data record (0x17), not a handshake
        let data = [0x17u8, 0x03, 0x03, 0x00, 0x01, 0x00];
        assert_eq!(extract_sni(&data), None);
    }

    #[test]
    fn truncated_after_record_header_returns_none() {
        // Claims a 100-byte body but supplies nothing
        let data = [0x16u8, 0x03, 0x01, 0x00, 0x64];
        assert_eq!(extract_sni(&data), None);
    }

    #[test]
    fn non_client_hello_handshake_returns_none() {
        // Handshake type 0x02 = ServerHello
        let body = [0x02u8, 0x00, 0x00, 0x01, 0xFF];
        let mut data = vec![0x16u8, 0x03, 0x01, 0x00, body.len() as u8];
        data.extend_from_slice(&body);
        assert_eq!(extract_sni(&data), None);
    }

    #[test]
    fn garbage_bytes_return_none() {
        assert_eq!(extract_sni(b"GET / HTTP/1.1\r\n"), None);
        assert_eq!(extract_sni(&[0u8; 64]), None);
    }
}
