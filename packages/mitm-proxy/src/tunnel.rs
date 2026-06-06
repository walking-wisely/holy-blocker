use crate::scan::ScanResult;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Hooks called for each intercepted request/response pair.
///
/// All fields default to allow-everything stubs so callers only need to
/// override the hooks they care about.
pub struct ScanHooks {
    pub url_scanner: Box<dyn Fn(&str) -> ScanResult + Send + Sync>,
    pub body_scanner: Box<dyn Fn(&str) -> ScanResult + Send + Sync>,
    pub image_scanner: Box<dyn Fn(&[u8]) -> ScanResult + Send + Sync>,
    /// Phase 5 sink: a copy of every HLS/DASH segment is pushed here.
    pub video_tx: mpsc::Sender<Bytes>,
    /// Body bytes scanned per response (bodies larger than this are forwarded
    /// without scanning rather than blocked).
    pub body_limit: usize,
}

impl Default for ScanHooks {
    fn default() -> Self {
        let (tx, _rx) = mpsc::channel(16);
        Self {
            url_scanner: Box::new(|_| ScanResult::Allow),
            body_scanner: Box::new(|_| ScanResult::Allow),
            image_scanner: Box::new(|_| ScanResult::Allow),
            video_tx: tx,
            body_limit: 1024 * 1024,
        }
    }
}

type ResBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

fn blocked() -> Response<ResBody> {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(
            Full::new(Bytes::from_static(b"Blocked\n"))
                .map_err(|e: Infallible| match e {})
                .boxed(),
        )
        .unwrap()
}

fn bad_gateway() -> Response<ResBody> {
    Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .body(
            Full::new(Bytes::from_static(b"Bad Gateway\n"))
                .map_err(|e: Infallible| match e {})
                .boxed(),
        )
        .unwrap()
}

fn bytes_body(bytes: Bytes) -> ResBody {
    Full::new(bytes)
        .map_err(|e: Infallible| match e {})
        .boxed()
}

/// Run an HTTP/1.1 request/response loop over two already-decrypted streams.
///
/// `browser` is the TLS stream accepted from the client; `origin` is the TLS
/// stream connected to the real server.  Every request passes through the
/// phase-3/4/5 scan hooks before being forwarded.
pub async fn run<B, O>(browser: B, origin: O, scan: Arc<ScanHooks>) -> anyhow::Result<()>
where
    B: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    O: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(origin)).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::debug!("origin conn driver: {e:#}");
        }
    });
    let sender = Arc::new(Mutex::new(sender));

    hyper::server::conn::http1::Builder::new()
        .serve_connection(
            TokioIo::new(browser),
            hyper::service::service_fn(move |req: Request<Incoming>| {
                let scan = Arc::clone(&scan);
                let sender = Arc::clone(&sender);
                async move { forward(req, sender, scan).await }
            }),
        )
        .await?;

    Ok(())
}

async fn forward(
    req: Request<Incoming>,
    sender: Arc<Mutex<hyper::client::conn::http1::SendRequest<Incoming>>>,
    scan: Arc<ScanHooks>,
) -> Result<Response<ResBody>, Infallible> {
    let uri = req.uri().to_string();
    let path = req.uri().path().to_owned();

    // Phase 3 — URL scan
    if matches!((scan.url_scanner)(&uri), ScanResult::Block { .. }) {
        return Ok(blocked());
    }

    let mut guard = sender.lock().await;
    let res = match guard.send_request(req).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("origin error: {e:#}");
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(bytes_body(Bytes::from_static(b"Bad Gateway\n")))
                .unwrap());
        }
    };
    drop(guard);

    let content_type = res
        .headers()
        .get(hyper::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();

    let is_html = content_type.starts_with("text/html");
    let is_image = content_type.starts_with("image/");
    // Phase 5 — detect HLS (.ts) and DASH (.m4s) segments by path, not the
    // full URI, so query strings like `/seg.ts?token=abc` are handled correctly.
    let is_video_segment = path.ends_with(".ts") || path.ends_with(".m4s");

    let (parts, body) = res.into_parts();

    if is_video_segment {
        // Phase 5 — tee a copy to the video watchdog queue (stub consumer)
        let bytes = match body.collect().await {
            Ok(c) => c.to_bytes(),
            Err(e) => {
                tracing::warn!("video body error: {e:#}");
                return Ok(bad_gateway());
            }
        };
        let _ = scan.video_tx.try_send(bytes.clone());
        return Ok(Response::from_parts(parts, bytes_body(bytes)));
    }

    if is_html || is_image {
        // Fast path: skip buffering entirely when Content-Length already exceeds
        // the scan limit — avoids allocating e.g. 80 MB for a large image only
        // to decide we won't scan it.
        let content_length: Option<usize> = parts
            .headers
            .get(hyper::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok());
        if content_length.is_some_and(|len| len > scan.body_limit) {
            return Ok(Response::from_parts(parts, body.map_err(|e| e).boxed()));
        }

        let bytes = match body.collect().await {
            Ok(c) => c.to_bytes(),
            Err(e) => {
                tracing::warn!("body read error: {e:#}");
                return Ok(bad_gateway());
            }
        };

        // Only scan bodies within the configured limit; oversized bodies pass
        // through without a verdict rather than being incorrectly blocked.
        if bytes.len() <= scan.body_limit {
            // Phase 3 — body scan (HTML)
            if is_html {
                if let Ok(text) = std::str::from_utf8(&bytes) {
                    if matches!((scan.body_scanner)(text), ScanResult::Block { .. }) {
                        return Ok(blocked());
                    }
                }
            }

            // Phase 4 — image scan
            if is_image {
                if matches!((scan.image_scanner)(&bytes), ScanResult::Block { .. }) {
                    return Ok(blocked());
                }
            }
        }

        return Ok(Response::from_parts(parts, bytes_body(bytes)));
    }

    // Default: stream through unchanged
    Ok(Response::from_parts(parts, body.map_err(|e| e).boxed()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};
    use hyper::{Request, Response, StatusCode};
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use std::sync::Arc;

    type TestBody = http_body_util::combinators::BoxBody<Bytes, Infallible>;

    fn tbody(s: &'static [u8]) -> TestBody {
        Full::new(Bytes::from_static(s)).map_err(|e: Infallible| match e {}).boxed()
    }

    fn owned_body(b: Bytes) -> TestBody {
        Full::new(b).map_err(|e: Infallible| match e {}).boxed()
    }

    /// Spawn a one-shot hyper HTTP/1.1 server on the duplex stream, using
    /// `handler` to respond to each request.
    fn mock_origin<F, Fut>(handler: F) -> tokio::io::DuplexStream
    where
        F: Fn(Request<Incoming>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<Response<TestBody>, Infallible>> + Send,
    {
        let (client_side, server_side) = tokio::io::duplex(65536);
        tokio::spawn(async move {
            hyper::server::conn::http1::Builder::new()
                .serve_connection(
                    TokioIo::new(server_side),
                    hyper::service::service_fn(move |req| handler(req)),
                )
                .await
                .ok();
        });
        client_side
    }

    /// Feed a request through the tunnel and return the status + collected body.
    async fn tunnel_request(
        scan: Arc<ScanHooks>,
        origin_stream: tokio::io::DuplexStream,
        path: &'static str,
    ) -> (StatusCode, Bytes) {
        let (browser_client, browser_server) = tokio::io::duplex(65536);
        tokio::spawn(async move {
            run(browser_server, origin_stream, scan).await.ok();
        });

        let (mut sender, conn) =
            hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(browser_client))
                .await
                .unwrap();
        tokio::spawn(conn);

        let req = Request::builder()
            .uri(path)
            .header("host", "example.com")
            .body(Full::new(Bytes::new()))
            .unwrap();

        let res = sender.send_request(req).await.unwrap();
        let status = res.status();
        let body = res.collect().await.unwrap().to_bytes();
        (status, body)
    }

    #[tokio::test]
    async fn allow_all_forwards_headers_and_body() {
        let origin = mock_origin(|_req| async {
            Ok(Response::builder()
                .header("content-type", "text/html")
                .body(tbody(b"<html>hello</html>"))
                .unwrap())
        });

        let (status, body) =
            tunnel_request(Arc::new(ScanHooks::default()), origin, "/").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_ref(), b"<html>hello</html>");
    }

    #[tokio::test]
    async fn url_block_returns_403_before_origin_body() {
        let origin = mock_origin(|_req| async {
            Ok(Response::new(tbody(b"secret data")))
        });

        let scan = Arc::new(ScanHooks {
            url_scanner: Box::new(|url| {
                if url.contains("/bad") {
                    ScanResult::Block { score: 100 }
                } else {
                    ScanResult::Allow
                }
            }),
            ..ScanHooks::default()
        });

        let (status, body) = tunnel_request(scan, origin, "/bad").await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(!body.windows(b"secret".len()).any(|w| w == b"secret"));
    }

    #[tokio::test]
    async fn body_block_returns_403_for_html() {
        let origin = mock_origin(|_req| async {
            Ok(Response::builder()
                .header("content-type", "text/html")
                .body(tbody(b"<html>bad content</html>"))
                .unwrap())
        });

        let scan = Arc::new(ScanHooks {
            body_scanner: Box::new(|_| ScanResult::Block { score: 100 }),
            ..ScanHooks::default()
        });

        let (status, _) = tunnel_request(scan, origin, "/page").await;

        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn over_body_limit_skips_scan_and_forwards_all_bytes() {
        // Body is larger than the configured limit — scanner always blocks, but
        // the limit check means it never runs and the response goes through.
        let large = Bytes::from(vec![b'x'; 200]);
        let large_clone = large.clone();

        let origin = mock_origin(move |_req| {
            let b = large_clone.clone();
            async move {
                Ok(Response::builder()
                    .header("content-type", "text/html")
                    .body(owned_body(b))
                    .unwrap())
            }
        });

        let scan = Arc::new(ScanHooks {
            body_scanner: Box::new(|_| ScanResult::Block { score: 100 }),
            body_limit: 50, // smaller than 200 bytes above
            ..ScanHooks::default()
        });

        let (status, body) = tunnel_request(scan, origin, "/large").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.len(), 200);
    }
}
