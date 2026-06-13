mod connect;
mod forward;
mod proxy;
mod scan;
mod tls;
mod tunnel;

use anyhow::Result;
use std::sync::{
    atomic::AtomicU8,
    Arc,
};
use tokio::net::TcpListener;
use tracing::info;

/// Shared runtime state.
///
/// `Arc<ProxyState>` is cloned into every connection task and can also be handed
/// to an IPC handler so the desktop can call `ProtectionMode::store(&state.mode, …)`
/// without knowing about the internal u8 encoding.
pub struct ProxyState {
    pub tls: Arc<tls::TlsState>,
    pub scan: Arc<tunnel::ScanHooks>,
    /// Current protection mode. Update via `scan::ProtectionMode::store`.
    pub mode: Arc<AtomicU8>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("mitm_proxy=debug,warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let ca_dir = std::path::PathBuf::from("data/ca");
    let tls = Arc::new(tls::TlsState::load(&ca_dir)?);

    let engine = Arc::new(scan::build_default_engine());
    let mode = scan::ProtectionMode::Full.to_atomic();
    let scan = {
        let url_engine = Arc::clone(&engine);
        let body_engine = Arc::clone(&engine);
        let url_mode = Arc::clone(&mode);
        let body_mode = Arc::clone(&mode);
        Arc::new(tunnel::ScanHooks {
            url_scanner: Box::new(move |url| {
                scan::scan_url(&url_engine, url, scan::ProtectionMode::from_atomic(&url_mode))
            }),
            body_scanner: Box::new(move |html| {
                scan::scan_body(&body_engine, html, scan::ProtectionMode::from_atomic(&body_mode))
            }),
            ..tunnel::ScanHooks::default()
        })
    };

    let state = Arc::new(ProxyState { tls, scan, mode });

    let addr = "127.0.0.1:8080";
    let listener = TcpListener::bind(addr).await?;
    info!("proxy listening on {addr}");

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        let tls = Arc::clone(&state.tls);
        let scan = Arc::clone(&state.scan);
        tokio::spawn(async move {
            if let Err(e) = proxy::handle(stream, tls, scan).await {
                tracing::warn!(%peer_addr, "connection closed with error: {e}");
            }
        });
    }
}
