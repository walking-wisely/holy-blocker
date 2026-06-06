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
        // State is created once; each iteration is a cache miss for a new hostname.
        // This isolates the signing cost from the one-time startup keygen.
        let state = make_state();
        let mut counter: u64 = 0;
        b.iter(|| {
            counter += 1;
            state
                .server_config(&format!("host{counter}.example.com"))
                .unwrap();
        });
    });
}

fn assert_cold_miss_under_2ms(_c: &mut Criterion) {
    use std::time::Instant;
    let iterations = 20;
    let mut total = std::time::Duration::ZERO;
    for i in 0..iterations {
        let state = make_state();
        let t = Instant::now();
        state
            .server_config(&format!("host{i}.example.com"))
            .unwrap();
        total += t.elapsed();
    }
    let mean = total / iterations;
    assert!(
        mean < std::time::Duration::from_millis(2),
        "cold cert generation mean {mean:?} exceeded 2 ms — leaf key reuse may be broken"
    );
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

criterion_group!(benches, bench_cert_cold, bench_cert_cache_hit, assert_cold_miss_under_2ms);
criterion_main!(benches);
