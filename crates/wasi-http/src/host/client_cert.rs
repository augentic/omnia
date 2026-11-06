//! Pre-flight checks on a `Client-Cert` PEM bundle before it becomes a TLS
//! identity: the leaf must be an end-entity certificate usable for TLS client
//! authentication and inside its validity window.

use std::fmt;
use std::time::SystemTime;

use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use x509_cert::der::Decode;
use x509_cert::der::oid::AssociatedOid;
use x509_cert::der::oid::db::rfc5280::{ANY_EXTENDED_KEY_USAGE, ID_KP_CLIENT_AUTH};
use x509_cert::ext::pkix::{BasicConstraints, ExtendedKeyUsage, KeyUsage};
use x509_cert::{Certificate, TbsCertificate};

/// Why a `Client-Cert` bundle was refused before any connection was attempted.
#[derive(Debug, PartialEq, Eq)]
pub enum Rejection {
    /// No certificate block found in the PEM bundle.
    NoCertificate,
    /// Certificate or extension could not be decoded.
    Malformed,
    /// Certificate is a CA certificate rather than an end-entity certificate.
    CertificateAuthority,
    /// Extended key usage excludes TLS client authentication.
    NotForClientAuth,
    /// Key usage excludes digital signatures.
    NoDigitalSignature,
    /// Certificate validity window has not started yet.
    NotYetValid,
    /// Certificate validity window has passed.
    Expired,
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self {
            Self::NoCertificate => "client certificate bundle contains no certificate",
            Self::Malformed => "client certificate is malformed",
            Self::CertificateAuthority => "client certificate is a CA certificate",
            Self::NotForClientAuth => {
                "client certificate extended key usage does not permit client authentication"
            }
            Self::NoDigitalSignature => "client certificate key usage does not permit signing",
            Self::NotYetValid => "client certificate is not yet valid",
            Self::Expired => "client certificate has expired",
        };
        f.write_str(reason)
    }
}

/// Check the leaf certificate of a PEM bundle for TLS client authentication.
///
/// # Errors
///
/// Returns the [`Rejection`] reason when the leaf is missing, malformed, a CA,
/// not permitted for client authentication or signing, or outside its validity
/// window.
pub fn validate_bundle(pem: &[u8]) -> Result<(), Rejection> {
    let leaf = match CertificateDer::pem_slice_iter(pem).next() {
        None => return Err(Rejection::NoCertificate),
        Some(Err(_)) => return Err(Rejection::Malformed),
        Some(Ok(der)) => der,
    };
    let certificate = Certificate::from_der(&leaf).map_err(|_decode| Rejection::Malformed)?;
    let tbs = certificate.tbs_certificate();

    if let Some(constraints) = extension::<BasicConstraints>(tbs)?
        && constraints.ca
    {
        return Err(Rejection::CertificateAuthority);
    }

    if let Some(eku) = extension::<ExtendedKeyUsage>(tbs)?
        && !eku.0.iter().any(|oid| *oid == ID_KP_CLIENT_AUTH || *oid == ANY_EXTENDED_KEY_USAGE)
    {
        return Err(Rejection::NotForClientAuth);
    }

    if let Some(usage) = extension::<KeyUsage>(tbs)?
        && !usage.digital_signature()
    {
        return Err(Rejection::NoDigitalSignature);
    }

    let now = SystemTime::now();
    let validity = tbs.validity();
    if now < validity.not_before.to_system_time() {
        return Err(Rejection::NotYetValid);
    }
    if now > validity.not_after.to_system_time() {
        return Err(Rejection::Expired);
    }

    Ok(())
}

fn extension<'a, T>(tbs: &'a TbsCertificate) -> Result<Option<T>, Rejection>
where
    T: Decode<'a> + AssociatedOid,
{
    tbs.get_extension::<T>()
        .map(|found| found.map(|(_critical, value)| value))
        .map_err(|_decode| Rejection::Malformed)
}
