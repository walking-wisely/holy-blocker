use bytes::Bytes;
use criterion::{Criterion, criterion_group, criterion_main};
use http_body_util::Full;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use mitm_proxy::{
    scan::ScanResult,
    tunnel::{ScanHooks, run},
};
use std::sync::Arc;
use tokio::runtime::Runtime;

fn make_rt() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

type TestBody = http_body_util::combinators::BoxBody<Bytes, std::convert::Infallible>;

fn tbody(b: &'static [u8]) -> TestBody {
    use http_body_util::BodyExt;
    Full::new(Bytes::from_static(b))
        .map_err(|e: std::convert::Infallible| match e {})
        .boxed()
}

/// Spawn a one-shot origin server on one side of a duplex pair and return the
/// client-side handle. The server responds with `handler(req)` once, then drops.
fn mock_origin<F, Fut>(handler: F) -> tokio::io::DuplexStream
where
    F: Fn(Request<hyper::body::Incoming>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<Response<TestBody>, std::convert::Infallible>>
        + Send,
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

/// Drive one full request/response round-trip through `run`.
async fn one_trip(scan: Arc<ScanHooks>, origin: tokio::io::DuplexStream, path: &'static str) {
    let (browser_client, browser_server) = tokio::io::duplex(65536);
    tokio::spawn(async move {
        run(browser_server, origin, scan).await.ok();
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
    let _ = res.status();
    // Drain the body so the connection is cleanly finished.
    use http_body_util::BodyExt;
    res.into_body().collect().await.unwrap();
}

// ── benchmarks ─────────────────────────────────────────────────────────────

/// Non-HTML, non-image content — body is streamed straight through, no buffering.
fn bench_pass_through(c: &mut Criterion) {
    let rt = make_rt();
    c.bench_function("tunnel/pass_through", |b| {
        b.to_async(&rt).iter(|| async {
            let origin = mock_origin(|_req| async {
                Ok(Response::builder()
                    .header("content-type", "application/octet-stream")
                    .body(tbody(b"binary data here"))
                    .unwrap())
            });
            one_trip(Arc::new(ScanHooks::default()), origin, "/file.bin").await;
        });
    });
}

/// HTML response — body is buffered and passed through the body scanner (stub
/// always returns Allow, so we measure the buffering overhead alone).
fn bench_html_body_scan(c: &mut Criterion) {
    let rt = make_rt();
    c.bench_function("tunnel/html_body_scan", |b| {
        b.to_async(&rt).iter(|| async {
            let origin = mock_origin(|_req| async {
                Ok(Response::builder()
                    .header("content-type", "text/html")
                    .body(tbody(b"<html><body><p>Hello world</p></body></html>"))
                    .unwrap())
            });
            one_trip(Arc::new(ScanHooks::default()), origin, "/page.html").await;
        });
    });
}

/// URL is blocked — the response is severed before any origin body is forwarded.
fn bench_url_block(c: &mut Criterion) {
    let rt = make_rt();
    let scan = Arc::new(ScanHooks {
        url_scanner: Box::new(|_| ScanResult::Block { score: 100 }),
        ..ScanHooks::default()
    });
    c.bench_function("tunnel/url_block", |b| {
        b.to_async(&rt).iter(|| async {
            let origin = mock_origin(|_req| async {
                Ok(Response::new(tbody(b"should never arrive")))
            });
            let s = Arc::clone(&scan);
            // We expect a 403; drive the trip normally — one_trip just discards status.
            let (browser_client, browser_server) = tokio::io::duplex(65536);
            tokio::spawn(async move {
                run(browser_server, origin, s).await.ok();
            });
            let (mut sender, conn) =
                hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(
                    browser_client,
                ))
                .await
                .unwrap();
            tokio::spawn(conn);
            let req = Request::builder()
                .uri("/blocked")
                .header("host", "example.com")
                .body(Full::new(Bytes::new()))
                .unwrap();
            let res = sender.send_request(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::FORBIDDEN);
            use http_body_util::BodyExt;
            res.into_body().collect().await.unwrap();
        });
    });
}

criterion_group!(benches, bench_pass_through, bench_html_body_scan, bench_url_block);
criterion_main!(benches);
