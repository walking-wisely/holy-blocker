/// Standalone timing breakdown for cold cert generation.
///
/// Measures each step independently so we know exactly where time goes
/// without needing a working flame graph on Windows.
use std::time::{Duration, Instant};

use rcgen::{CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, SanType};
use rustls::{ServerConfig, pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer}};

const ITERS: u32 = 200;

fn mean(samples: &[Duration]) -> Duration {
    samples.iter().sum::<Duration>() / samples.len() as u32
}

fn main() {
    // ── 1. KeyPair::generate ──────────────────────────────────────────────
    let mut samples = Vec::with_capacity(ITERS as usize);
    for _ in 0..ITERS {
        let t = Instant::now();
        let _ = KeyPair::generate().unwrap();
        samples.push(t.elapsed());
    }
    println!("KeyPair::generate()          mean = {:>8.3} ms", mean(&samples).as_secs_f64() * 1000.0);

    // ── 2. Build CA (one-time cost at startup) ────────────────────────────
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(vec![]).unwrap();
    ca_params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();

    // Pre-generate one shared leaf key (our optimisation)
    let leaf_key = KeyPair::generate().unwrap();

    // ── 3. CertificateParams::signed_by (signing only, shared key) ────────
    let mut samples = Vec::with_capacity(ITERS as usize);
    for i in 0..ITERS {
        let sni = format!("host{i}.example.com");
        let mut params = CertificateParams::new(vec![sni.clone()]).unwrap();
        params.distinguished_name = {
            let mut dn = DistinguishedName::new();
            dn.push(DnType::CommonName, &sni);
            dn
        };
        params.subject_alt_names = vec![SanType::DnsName(sni.try_into().unwrap())];

        let t = Instant::now();
        let cert = params.signed_by(&leaf_key, &ca_cert, &ca_key).unwrap();
        samples.push(t.elapsed());
        let _ = cert;
    }
    println!("signed_by() (shared key)     mean = {:>8.3} ms", mean(&samples).as_secs_f64() * 1000.0);

    // ── 4. signed_by with a fresh key per call (old behaviour) ────────────
    let mut samples = Vec::with_capacity(ITERS as usize);
    for i in 0..ITERS {
        let sni = format!("host{i}.example.com");
        let fresh_key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(vec![sni.clone()]).unwrap();
        params.distinguished_name = {
            let mut dn = DistinguishedName::new();
            dn.push(DnType::CommonName, &sni);
            dn
        };
        params.subject_alt_names = vec![SanType::DnsName(sni.try_into().unwrap())];

        let t = Instant::now();
        let cert = params.signed_by(&fresh_key, &ca_cert, &ca_key).unwrap();
        samples.push(t.elapsed());
        let _ = cert;
    }
    println!("signed_by() (fresh key/call) mean = {:>8.3} ms  ← old behaviour (keygen excluded)", mean(&samples).as_secs_f64() * 1000.0);

    // ── 5. ServerConfig construction ──────────────────────────────────────
    let sni = "bench.example.com";
    let mut params = CertificateParams::new(vec![sni.to_owned()]).unwrap();
    params.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, sni);
        dn
    };
    params.subject_alt_names = vec![SanType::DnsName(sni.to_owned().try_into().unwrap())];
    let cert = params.signed_by(&leaf_key, &ca_cert, &ca_key).unwrap();
    let cert_der = cert.der().clone();

    let mut samples = Vec::with_capacity(ITERS as usize);
    for _ in 0..ITERS {
        let cert_der = rustls::pki_types::CertificateDer::from(cert_der.to_vec());
        let key_der: PrivateKeyDer<'static> =
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
        let t = Instant::now();
        let cfg = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .unwrap();
        samples.push(t.elapsed());
        let _ = cfg;
    }
    println!("ServerConfig construction    mean = {:>8.3} ms", mean(&samples).as_secs_f64() * 1000.0);

    // ── 6. Full cold miss: new state + server_config (current code) ───────
    let mut samples = Vec::with_capacity(ITERS as usize);
    for i in 0..ITERS {
        let ca_key = KeyPair::generate().unwrap();
        let mut ca_params = CertificateParams::new(vec![]).unwrap();
        ca_params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();
        let leaf_key = KeyPair::generate().unwrap(); // generated once in from_parts

        let sni = format!("host{i}.example.com");
        let t = Instant::now();
        {
            let mut params = CertificateParams::new(vec![sni.clone()]).unwrap();
            params.distinguished_name = {
                let mut dn = DistinguishedName::new();
                dn.push(DnType::CommonName, &sni);
                dn
            };
            params.subject_alt_names = vec![SanType::DnsName(sni.try_into().unwrap())];
            let cert = params.signed_by(&leaf_key, &ca_cert, &ca_key).unwrap();
            let cert_der = rustls::pki_types::CertificateDer::from(cert.der().to_vec());
            let key_der: PrivateKeyDer<'static> =
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
            let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
            let _ = ServerConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![cert_der], key_der)
                .unwrap();
        }
        samples.push(t.elapsed());
    }
    println!("server_config() cold miss    mean = {:>8.3} ms  ← what Criterion measures in make_server_config", mean(&samples).as_secs_f64() * 1000.0);
}
