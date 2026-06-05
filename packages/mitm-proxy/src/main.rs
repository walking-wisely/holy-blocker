mod forward;
mod proxy;
mod tls;

use anyhow::Result;
use tokio::net::TcpListener;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("mitm_proxy=debug,warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let addr = "127.0.0.1:8080";
    let listener = TcpListener::bind(addr).await?;
    info!("proxy listening on {addr}");

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        tokio::spawn(async move {
            if let Err(e) = proxy::handle(stream).await {
                tracing::warn!(%peer_addr, "connection closed with error: {e}");
            }
        });
    }
}
