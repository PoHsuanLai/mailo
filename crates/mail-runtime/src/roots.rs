//! The certificate authorities a server the person set up is checked against: the bundled
//! webpki roots, and the operating system's own.
//!
//! The system's are what `rustls-native-certs` finds: the Keychain's on macOS (with the trust
//! the person gave there), the system store on Windows, and on Linux the bundle OpenSSL would
//! read. A company's or a campus's authority installed there is believed for its mail servers
//! as the system believes it for everything else, and so is a self-signed certificate the person
//! marked trusted in Keychain Access. Before, only the bundled roots were, and such a server
//! could not be used at all.
//!
//! The servers an account is set up with use these: its IMAP, POP3, SMTP and ManageSieve
//! connections, its HTTP ones (JMAP, Graph, CardDAV, the token endpoint), and the discovery that
//! finds them. What a sender chooses (remote images, logos, WKD, an unsubscribe page) is on the
//! public internet, where the bundled roots are the whole story, and keeps to them.

use std::sync::{Arc, OnceLock};

use rustls::RootCertStore;
use rustls_pki_types::CertificateDer;

/// The operating system's certificate authorities, read once per process.
///
/// A certificate that cannot be read is skipped; a machine with no bundle has none, and the
/// bundled roots are all there is.
pub fn system() -> &'static [CertificateDer<'static>] {
    static SYSTEM: OnceLock<Vec<CertificateDer<'static>>> = OnceLock::new();
    SYSTEM.get_or_init(|| {
        let found = rustls_native_certs::load_native_certs();
        for error in &found.errors {
            log::debug!("a system certificate could not be read: {error}");
        }
        found.certs
    })
}

/// The bundled roots and the system's, for a TLS connection to a server an account is set up
/// with.
pub fn server_roots() -> Arc<RootCertStore> {
    static ROOTS: OnceLock<Arc<RootCertStore>> = OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let mut roots = RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            };
            let (_, unusable) = roots.add_parsable_certificates(system().iter().cloned());
            if unusable > 0 {
                log::debug!("{unusable} system certificates are not usable as roots");
            }
            Arc::new(roots)
        })
        .clone()
}

/// `builder`, also trusting the system's authorities: for an HTTP client that reaches a server
/// an account is set up with. reqwest keeps its bundled roots alongside.
pub fn trusting_system(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    system()
        .iter()
        // One reqwest cannot read is skipped, as `server_roots` skips one rustls cannot.
        .filter_map(|der| reqwest::Certificate::from_der(der.as_ref()).ok())
        .fold(builder, reqwest::ClientBuilder::add_root_certificate)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The system's authorities are added to the bundled ones, never in place of them.
    #[test]
    fn the_bundled_roots_are_always_there() {
        assert!(server_roots().len() >= webpki_roots::TLS_SERVER_ROOTS.len());
    }
}
