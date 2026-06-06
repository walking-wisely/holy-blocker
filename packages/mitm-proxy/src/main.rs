mod connect;
mod forward;
mod proxy;
mod scan;
mod tunnel;
mod tls;

use anyhow::Result;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("mitm_proxy=debug,warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let ca_dir = std::path::PathBuf::from("data/ca");
    let tls = Arc::new(tls::TlsState::load(&ca_dir)?);
    let scan = Arc::new(tunnel::ScanHooks::default());

    let addr = "127.0.0.1:8080";
    let listener = TcpListener::bind(addr).await?;
    info!("proxy listening on {addr}");

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        let tls = Arc::clone(&tls);
        let scan = Arc::clone(&scan);
        tokio::spawn(async move {
            if let Err(e) = proxy::handle(stream, tls, scan).await {
                tracing::warn!(%peer_addr, "connection closed with error: {e}");
            }
        });
    }
}
