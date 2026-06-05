use crate::forward;
use bytes::Bytes;
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::{Method, Request, Response, StatusCode, body::Incoming};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use tokio::net::TcpStream;

/// Unified response body type used throughout the proxy.
pub type ResBody = BoxBody<Bytes, hyper::Error>;

pub fn text(s: &'static str) -> ResBody {
    Full::new(Bytes::from_static(s.as_bytes()))
        .map_err(|never| match never {})
        .boxed()
}

/// Accepts one HTTP/1.1 connection and serves all requests on it.
pub async fn handle(stream: TcpStream) -> anyhow::Result<()> {
    let io = TokioIo::new(stream);
    hyper::server::conn::http1::Builder::new()
        .serve_connection(io, hyper::service::service_fn(dispatch))
        .await?;
    Ok(())
}

async fn dispatch(req: Request<Incoming>) -> Result<Response<ResBody>, Infallible> {
    let res = if req.method() == Method::CONNECT {
        // HTTPS CONNECT tunnel — not implemented in the HTTP-only phase.
        Response::builder()
            .status(StatusCode::NOT_IMPLEMENTED)
            .body(text("HTTPS tunneling is not yet implemented\n"))
            .unwrap()
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
