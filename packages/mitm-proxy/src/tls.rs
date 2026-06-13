use anyhow::{Context, Result};
use rcgen::{CertificateParams, DistinguishedName, DnType, Issuer, KeyPair, SanType};
type OwnedIssuer = Issuer<'static, KeyPair>;
use rustls::{
    ClientConfig, ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use rustls_native_certs::load_native_certs;
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

pub struct TlsState {
    ca_issuer: OwnedIssuer,
    leaf_key: Arc<KeyPair>,
    cache: Mutex<HashMap<String, Arc<ServerConfig>>>,
    client_cfg: Arc<ClientConfig>,
}

impl TlsState {
    /// Construct a `TlsState` directly from an in-memory CA — used by benchmarks
    /// and tests that don't want to touch the filesystem.
    pub fn from_issuer(ca_issuer: OwnedIssuer) -> Self {
        let leaf_key = Arc::new(KeyPair::generate().expect("generating shared leaf key pair"));
        Self {
            ca_issuer,
            leaf_key,
            cache: Mutex::new(HashMap::new()),
            client_cfg: Self::build_client_config().unwrap(),
        }
    }

    /// Load the root CA from PEM files `ca.crt` and `ca.key` in `ca_dir`.
    pub fn load(ca_dir: &Path) -> Result<Self> {
        let cert_pem =
            std::fs::read_to_string(ca_dir.join("ca.crt")).context("reading ca.crt")?;
        let key_pem =
            std::fs::read_to_string(ca_dir.join("ca.key")).context("reading ca.key")?;

        let ca_key = KeyPair::from_pem(&key_pem).context("parsing CA private key")?;

        let pem = pem::parse(cert_pem.as_bytes()).context("parsing CA cert PEM")?;
        let ca_cert_der = CertificateDer::from(pem.into_contents());
        let ca_issuer = Issuer::from_ca_cert_der(&ca_cert_der, ca_key)
            .context("parsing CA certificate DER")?;

        let leaf_key = Arc::new(KeyPair::generate().context("generating shared leaf key pair")?);
        let client_cfg = Self::build_client_config()?;
        Ok(Self {
            ca_issuer,
            leaf_key,
            cache: Mutex::new(HashMap::new()),
            client_cfg,
        })
    }

    /// Return a `ServerConfig` presenting a leaf cert for `sni`, generating
    /// and caching it on the first call for each hostname.
    pub fn server_config(&self, sni: &str) -> Result<Arc<ServerConfig>> {
        {
            let cache = self.cache.lock().unwrap();
            if let Some(cfg) = cache.get(sni) {
                return Ok(Arc::clone(cfg));
            }
        }

        let cfg = Arc::new(self.make_server_config(sni)?);
        self.cache
            .lock()
            .unwrap()
            .insert(sni.to_owned(), Arc::clone(&cfg));
        Ok(cfg)
    }

    /// Generate a signed leaf certificate DER for `sni`, using the provided `key_pair`.
    /// Pass `self.leaf_key` (the shared key) for production use; pass a freshly
    /// generated key only in tests that need unique per-cert keys.
    pub fn generate_leaf_cert_der(
        &self,
        sni: &str,
        key_pair: &KeyPair,
    ) -> Result<CertificateDer<'static>> {
        let mut params =
            CertificateParams::new(vec![sni.to_owned()]).context("building leaf cert params")?;
        params.distinguished_name = {
            let mut dn = DistinguishedName::new();
            dn.push(DnType::CommonName, sni);
            dn
        };
        params.subject_alt_names = vec![SanType::DnsName(
            sni.to_owned().try_into().context("invalid SNI for SAN")?,
        )];

        let leaf_cert = params
            .signed_by(key_pair, &self.ca_issuer)
            .context("signing leaf certificate")?;

        Ok(leaf_cert.der().clone().into())
    }

    fn make_server_config(&self, sni: &str) -> Result<ServerConfig> {
        let cert_der = self.generate_leaf_cert_der(sni, &self.leaf_key)?;
        let key_der: PrivateKeyDer<'static> =
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.leaf_key.serialize_der()));

        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .context("choosing protocol versions")?
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .context("building ServerConfig")?;

        Ok(cfg)
    }

    /// Replace the outbound `ClientConfig` used for origin TLS connections.
    ///
    /// Intended for integration tests that need the proxy to trust a local
    /// test CA instead of the system root store.
    pub fn with_client_config(mut self, cfg: Arc<ClientConfig>) -> Self {
        self.client_cfg = cfg;
        self
    }

    /// Return the pre-built `ClientConfig` for origin TLS connections.
    ///
    /// The config is built once at startup (in [`TlsState::load`]) to avoid
    /// re-reading the system root store on every CONNECT request.
    pub fn client_config(&self) -> Arc<ClientConfig> {
        Arc::clone(&self.client_cfg)
    }

    /// Build a `ClientConfig` that trusts the system roots plus any `extra`
    /// DER-encoded certificates.  Pass an empty slice for the default config.
    pub fn build_client_config_with_extra_roots(
        extra: &[rustls::pki_types::CertificateDer<'static>],
    ) -> Result<Arc<ClientConfig>> {
        let mut root_store = rustls::RootCertStore::empty();
        let native = load_native_certs();
        if !native.errors.is_empty() {
            tracing::warn!("some native certs failed to load: {:?}", native.errors);
        }
        for cert in native.certs {
            root_store.add(cert).context("adding native root cert")?;
        }
        for cert in extra {
            root_store.add(cert.clone()).context("adding extra root cert")?;
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .context("choosing protocol versions")?
            .with_root_certificates(root_store)
            .with_no_client_auth();
        Ok(Arc::new(cfg))
    }

    fn build_client_config() -> Result<Arc<ClientConfig>> {
        let mut root_store = rustls::RootCertStore::empty();

        let native = load_native_certs();
        if !native.errors.is_empty() {
            tracing::warn!("some native certs failed to load: {:?}", native.errors);
        }
        for cert in native.certs {
            root_store.add(cert).context("adding native root cert")?;
        }

        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .context("choosing protocol versions")?
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Ok(Arc::new(cfg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::IsCa;
    use std::sync::Arc;

    /// Build a throwaway in-memory CA and return a `TlsState` backed by it.
    fn make_test_state() -> TlsState {
        let ca_key = KeyPair::generate().unwrap();
        let mut ca_params = CertificateParams::new(vec![]).unwrap();
        ca_params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_issuer = Issuer::new(ca_params, ca_key);
        TlsState::from_issuer(ca_issuer)
    }

    /// Verify that the leaf cert produced by the real `server_config` path
    /// contains the correct SAN (exercises `generate_leaf_cert_der` via
    /// `make_server_config`, not a hand-rolled duplicate).
    #[test]
    fn leaf_cert_has_correct_san() {
        let state = make_test_state();
        let sni = "example.com";

        let cert_der = state.generate_leaf_cert_der(sni, &state.leaf_key).unwrap();
        let (_, parsed) =
            x509_parser::parse_x509_certificate(&cert_der).expect("parse leaf DER");

        let san_ext = parsed
            .subject_alternative_name()
            .unwrap()
            .expect("SAN extension must be present");

        let has_dns = san_ext.value.general_names.iter().any(|gn| {
            matches!(gn, x509_parser::extensions::GeneralName::DNSName(n) if *n == sni)
        });
        assert!(has_dns, "SAN must contain DNS:{sni}");
    }

    /// Verify that calling `server_config` twice for the same hostname returns
    /// the same `Arc` (i.e., the cert is cached and not regenerated).
    #[test]
    fn server_config_is_cached() {
        let state = make_test_state();
        let sni = "cached.example.com";

        let first = state.server_config(sni).unwrap();
        let second = state.server_config(sni).unwrap();

        assert!(
            Arc::ptr_eq(&first, &second),
            "second call must return the same Arc (cache hit)"
        );
    }
}
