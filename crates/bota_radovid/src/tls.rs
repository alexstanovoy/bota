//! Which certificates a client trusts, for the control connection and for
//! `wss://` game servers alike.

/// The public web's roots, or a development server's own certificate. The
/// browser does its own TLS, so there it is a marker only.
#[derive(Clone)]
pub struct Tls {
    #[cfg(feature = "native")]
    pub(crate) control: radovid_client::TlsConfig,
    #[cfg(feature = "native")]
    pub(crate) game: bota_client_core::TlsConfig,
}

#[cfg(feature = "native")]
impl Tls {
    /// Trusts the certificates the public web does.
    pub fn webpki() -> Tls {
        Tls {
            control: radovid_client::TlsConfig::webpki(),
            game: bota_client_core::TlsConfig::webpki(),
        }
    }

    /// Trusts exactly the certificates in `pem`.
    pub fn trust_pem(pem: &str) -> std::io::Result<Tls> {
        Ok(Tls {
            control: radovid_client::TlsConfig::trust_pem(pem)?,
            game: bota_client_core::TlsConfig::trust_pem(pem)?,
        })
    }
}

#[cfg(not(feature = "native"))]
impl Tls {
    /// The browser trusts what it trusts.
    pub fn webpki() -> Tls {
        Tls {}
    }
}
