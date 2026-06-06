use criterion::{Criterion, criterion_group, criterion_main};
use mitm_proxy::forward::strip_hop_by_hop;

fn make_headers() -> hyper::HeaderMap {
    let mut map = hyper::HeaderMap::new();
    map.insert(hyper::header::HOST, "example.com".parse().unwrap());
    map.insert(hyper::header::CONTENT_TYPE, "text/html".parse().unwrap());
    map.insert(hyper::header::CONTENT_LENGTH, "1234".parse().unwrap());
    map.insert(hyper::header::ACCEPT_ENCODING, "gzip, br".parse().unwrap());
    map.insert(
        hyper::header::CONNECTION,
        "keep-alive, x-custom".parse().unwrap(),
    );
    map.insert("keep-alive", "timeout=5, max=100".parse().unwrap());
    map.insert("x-custom", "value".parse().unwrap());
    map.insert("transfer-encoding", "chunked".parse().unwrap());
    map
}

fn bench_strip_hop_by_hop(c: &mut Criterion) {
    c.bench_function("headers/strip_hop_by_hop", |b| {
        b.iter(|| {
            let mut headers = make_headers();
            strip_hop_by_hop(&mut headers);
            headers
        });
    });
}

criterion_group!(benches, bench_strip_hop_by_hop);
criterion_main!(benches);
