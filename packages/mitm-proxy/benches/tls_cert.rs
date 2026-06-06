use criterion::{Criterion, criterion_group, criterion_main};
use mitm_proxy::tls::TlsState;
use rcgen::{CertificateParams, IsCa, KeyPair};

/// Build an in-memory TlsState without reading any files.
fn make_state() -> TlsState {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(vec![]).unwrap();
    ca_params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    TlsState::from_parts(ca_cert, ca_key)
}

fn bench_cert_cold(c: &mut Criterion) {
    c.bench_function("tls/cert_generation_cold", |b| {
        b.iter(|| {
            // Fresh state each iteration so every call is a cache miss.
            let state = make_state();
            state.server_config("bench.example.com").unwrap();
        });
    });
}

fn bench_cert_cache_hit(c: &mut Criterion) {
    let state = make_state();
    // Warm the cache once before the measurement loop.
    state.server_config("cached.example.com").unwrap();

    c.bench_function("tls/cert_cache_hit", |b| {
        b.iter(|| {
            state.server_config("cached.example.com").unwrap();
        });
    });
}

criterion_group!(benches, bench_cert_cold, bench_cert_cache_hit);
criterion_main!(benches);
