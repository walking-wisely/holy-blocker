mod cli;
mod connect;
mod forward;
mod proxy;
mod scan;
mod tls;
mod tunnel;

use anyhow::Result;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    let options = match cli::Options::from_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n\n{}", cli::USAGE);
            std::process::exit(2);
        }
    };

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("mitm_proxy=debug,warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let tls = Arc::new(tls::TlsState::load(&options.ca_dir)?);

    let engine = Arc::new(scan::build_default_engine());
    let sandbox = Arc::new(scan::build_image_sandbox(options.image_model.as_deref(), options.image_threshold));
    // mode_cell can be swapped at runtime (e.g. from a desktop config_update IPC message)
    // without rebuilding ScanHooks; default to Full protection. Image scanning is not yet
    // gated by mode — it predates ProtectionMode and runs unconditionally, same as before.
    let mode_cell = scan::ProtectionMode::Full.to_atomic();
    let scan = {
        let url_engine = Arc::clone(&engine);
        let body_engine = Arc::clone(&engine);
        let image_sandbox = Arc::clone(&sandbox);
        let url_mode = Arc::clone(&mode_cell);
        let body_mode = Arc::clone(&mode_cell);
        Arc::new(tunnel::ScanHooks {
            url_scanner: Box::new(move |url| {
                scan::scan_url(&url_engine, url, scan::ProtectionMode::from_atomic(&url_mode))
            }),
            body_scanner: Box::new(move |html| {
                scan::scan_body(&body_engine, html, scan::ProtectionMode::from_atomic(&body_mode))
            }),
            image_scanner: Box::new(move |bytes| scan::scan_image(&image_sandbox, bytes)),
            ..tunnel::ScanHooks::default()
        })
    };

    let listener = TcpListener::bind(options.listen).await?;
    info!("proxy listening on {}", options.listen);

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
