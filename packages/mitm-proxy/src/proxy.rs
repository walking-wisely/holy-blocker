use crate::connect;
use crate::forward::{self, ResBody};
use crate::tls::TlsState;
use crate::tunnel::ScanHooks;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, Response, StatusCode, body::Incoming};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::net::TcpStream;

pub fn text(s: &'static str) -> ResBody {
    Full::new(Bytes::from_static(s.as_bytes()))
        .map_err(|never| match never {})
        .boxed()
}

/// Accepts one HTTP/1.1 connection and serves all requests on it.
pub async fn handle(stream: TcpStream, tls: Arc<TlsState>, scan: Arc<ScanHooks>) -> anyhow::Result<()> {
    let io = TokioIo::new(stream);
    hyper::server::conn::http1::Builder::new()
        .serve_connection(
            io,
            hyper::service::service_fn(move |req| {
                let tls = Arc::clone(&tls);
                let scan = Arc::clone(&scan);
                async move { dispatch(req, tls, scan).await }
            }),
        )
        .with_upgrades()
        .await?;
    Ok(())
}

async fn dispatch(req: Request<Incoming>, tls: Arc<TlsState>, scan: Arc<ScanHooks>) -> Result<Response<ResBody>, Infallible> {
    let res = if req.method() == Method::CONNECT {
        let authority = req.uri().authority().cloned();
        match authority {
            Some(authority) => {
                let upgrade = hyper::upgrade::on(req);
                tokio::spawn(async move {
                    match upgrade.await {
                        Ok(upgraded) => {
                            if let Err(e) =
                                connect::handle_connect(authority, TokioIo::new(upgraded), tls, scan)
                                    .await
                            {
                                tracing::warn!("CONNECT tunnel error: {e:#}");
                            }
                        }
                        Err(e) => tracing::warn!("upgrade error: {e:#}"),
                    }
                });
                Response::builder()
                    .status(StatusCode::OK)
                    .body(text(""))
                    .unwrap()
            }
            None => Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(text("CONNECT target must be host:port\n"))
                .unwrap(),
        }
    } else {
        match forward::forward_http(req).await {
            Ok(res) => res,
            Err(e) => {
                tracing::warn!("upstream error: {e:#}");
                Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(text("Bad Gateway\n"))
                    .unwrap()
            }
        }
    };
    Ok(res)
}
