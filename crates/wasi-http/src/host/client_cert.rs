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

#[cfg(test)]
mod tests {
    use rcgen::{
        BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
        KeyUsagePurpose, date_time_ymd,
    };

    use super::*;

    fn params(configure: impl FnOnce(&mut CertificateParams)) -> CertificateParams {
        let mut params = CertificateParams::new(Vec::<String>::new()).expect("parameters");
        configure(&mut params);
        params
    }

    fn bundle(configure: impl FnOnce(&mut CertificateParams)) -> Vec<u8> {
        let key = KeyPair::generate().expect("key pair");
        let certificate = params(configure).self_signed(&key).expect("certificate");
        format!("{}{}", certificate.pem(), key.serialize_pem()).into_bytes()
    }

    fn client_auth_params(params: &mut CertificateParams) {
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    }

    #[test]
    fn client_auth() {
        assert_eq!(validate_bundle(&bundle(client_auth_params)), Ok(()));
    }

    #[test]
    fn server_auth() {
        let bundle = bundle(|params| {
            params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
            params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        });
        assert_eq!(validate_bundle(&bundle), Err(Rejection::NotForClientAuth));
    }

    #[test]
    fn no_eku() {
        assert_eq!(validate_bundle(&bundle(|_| {})), Ok(()));
    }

    #[test]
    fn ca() {
        let bundle = bundle(|params| params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained));
        assert_eq!(validate_bundle(&bundle), Err(Rejection::CertificateAuthority));
    }

    #[test]
    fn client_auth_with_chain() {
        let ca_key = KeyPair::generate().expect("CA key pair");
        let ca_params = params(|params| {
            params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
            params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        });
        let ca = ca_params.self_signed(&ca_key).expect("CA certificate");

        let leaf_key = KeyPair::generate().expect("leaf key pair");
        let leaf = params(client_auth_params)
            .signed_by(&leaf_key, &Issuer::from_params(&ca_params, &ca_key))
            .expect("leaf certificate");

        let bundle = format!("{}{}{}", leaf.pem(), ca.pem(), leaf_key.serialize_pem());
        assert_eq!(validate_bundle(bundle.as_bytes()), Ok(()));
    }

    #[test]
    fn key_encipherment_only() {
        let bundle = bundle(|params| params.key_usages = vec![KeyUsagePurpose::KeyEncipherment]);
        assert_eq!(validate_bundle(&bundle), Err(Rejection::NoDigitalSignature));
    }

    #[test]
    fn expired() {
        let bundle = bundle(|params| {
            params.not_before = date_time_ymd(2020, 1, 1);
            params.not_after = date_time_ymd(2021, 1, 1);
        });
        assert_eq!(validate_bundle(&bundle), Err(Rejection::Expired));
    }

    #[test]
    fn not_yet_valid() {
        let bundle = bundle(|params| {
            params.not_before = date_time_ymd(2100, 1, 1);
            params.not_after = date_time_ymd(2101, 1, 1);
        });
        assert_eq!(validate_bundle(&bundle), Err(Rejection::NotYetValid));
    }

    #[test]
    fn empty_bundle() {
        assert_eq!(validate_bundle(b""), Err(Rejection::NoCertificate));
    }

    #[test]
    fn key_only() {
        let key = KeyPair::generate().expect("key pair");
        assert_eq!(validate_bundle(key.serialize_pem().as_bytes()), Err(Rejection::NoCertificate));
    }

    #[test]
    fn garbage() {
        let bundle =
            b"-----BEGIN CERTIFICATE-----\nbm90IGEgY2VydGlmaWNhdGU=\n-----END CERTIFICATE-----\n";
        assert_eq!(validate_bundle(bundle), Err(Rejection::Malformed));
    }
}
