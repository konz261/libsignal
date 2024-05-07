//
// Copyright 2023 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

use std::borrow::Cow;

use boring::error::ErrorStack;
use boring::ssl::{SslAlert, SslConnectorBuilder, SslVerifyError, SslVerifyMode};
use boring::x509::store::X509StoreBuilder;
use boring::x509::X509;
use rustls::client::danger::ServerCertVerifier;

const SIGNAL_ROOT_CERT_DER: &[u8] = include_bytes!("../../res/signal.cer");

#[derive(thiserror::Error, Debug, displaydoc::Display)]
pub enum Error {
    /// Bad certificate
    BadCertificate,
    /// Bad hostname
    BadHostname,
}

impl From<ErrorStack> for Error {
    fn from(_value: ErrorStack) -> Self {
        Self::BadCertificate
    }
}

#[derive(Debug, Clone)]
pub enum RootCertificates {
    Native,
    Signal,
    FromDer(Cow<'static, [u8]>),
}

impl RootCertificates {
    pub fn apply_to_connector(
        &self,
        connector: &mut SslConnectorBuilder,
        host_name: &str,
    ) -> Result<(), Error> {
        let der = match self {
            RootCertificates::Native => {
                return set_up_platform_verifier(
                    connector,
                    host_name,
                    rustls_platform_verifier::Verifier::new(),
                );
            }
            RootCertificates::Signal => SIGNAL_ROOT_CERT_DER,
            RootCertificates::FromDer(der) => der,
        };
        let mut store_builder = X509StoreBuilder::new()?;
        store_builder.add_cert(X509::from_der(der)?)?;
        connector.set_verify_cert_store(store_builder.build())?;
        Ok(())
    }
}

/// Configures [rustls_platform_verifier] as a BoringSSL [custom verify
/// callback](boring::ssl::SslContextBuilder::set_custom_verify_callback).
fn set_up_platform_verifier(
    connector: &mut SslConnectorBuilder,
    host_name: &str,
    verifier: impl ServerCertVerifier + 'static,
) -> Result<(), Error> {
    let host_as_server_name = rustls::pki_types::ServerName::try_from(host_name)
        .map_err(|_| Error::BadHostname)?
        .to_owned();

    connector.set_custom_verify_callback(SslVerifyMode::PEER, move |ssl| {
        // Get the certificate chain, lazily convert each certificate to DER (as expected by rustls).
        let mut cert_chain = ssl
            .peer_cert_chain()
            .ok_or(SslVerifyError::Invalid(SslAlert::NO_CERTIFICATE))?
            .into_iter()
            .map(|cert| Ok(cert.to_der()?.into()));

        // The head of the chain should be the leaf certificate.
        let end_entity = match cert_chain.next() {
            Some(Ok(leaf_cert)) => leaf_cert,
            None | Some(Err(_)) => {
                return Err(SslVerifyError::Invalid(SslAlert::BAD_CERTIFICATE));
            }
        };

        // The rest of the chain should be valid intermediate certificates.
        let intermediates: Vec<_> = cert_chain
            .collect::<Result<_, boring::error::ErrorStack>>()
            .map_err(|_| SslVerifyError::Invalid(SslAlert::BAD_CERTIFICATE))?;

        // We don't do our own OCSP. Either the platform will do its own checks, or it won't.
        let ocsp_responses = [];

        verifier
            .verify_server_cert(
                &end_entity,
                &intermediates,
                &host_as_server_name,
                &ocsp_responses,
                rustls::pki_types::UnixTime::now(),
            )
            .map_err(|e| {
                // The most important thing is to reject the certificate. Mapping the errors over
                // only affects what message gets reported in logs. Which isn't *unimportant*, but
                // isn't critical for correctness either.
                //
                // From RFC 5246:
                // - bad_certificate: A certificate was corrupt, contained signatures that did not
                //   verify correctly, etc.
                // - certificate_expired: A certificate has expired or is not currently valid.
                // - certificate_unknown: Some other (unspecified) issue arose in processing the
                //   certificate, rendering it unacceptable.
                // - certificate_revoked: A certificate was revoked by its signer.
                // - unknown_ca: A valid certificate chain or partial chain was received, but the
                //   certificate was not accepted because the CA certificate could not be located or
                //   couldn't be matched with a known, trusted CA.
                // - internal_error: An internal error unrelated to the peer or the correctness of
                //   the protocol (such as a memory allocation failure) makes it impossible to
                //   continue.
                log::debug!(
                    "TLS certificate for {} failed verification: {e}",
                    host_as_server_name.to_str()
                );
                SslVerifyError::Invalid(match e {
                    rustls::Error::InvalidCertificate(e) => match e {
                        rustls::CertificateError::BadEncoding => SslAlert::BAD_CERTIFICATE,
                        rustls::CertificateError::Expired => SslAlert::CERTIFICATE_EXPIRED,
                        rustls::CertificateError::NotValidYet => SslAlert::CERTIFICATE_UNKNOWN,
                        rustls::CertificateError::Revoked => SslAlert::CERTIFICATE_REVOKED,
                        rustls::CertificateError::UnhandledCriticalExtension => {
                            SslAlert::CERTIFICATE_UNKNOWN
                        }
                        rustls::CertificateError::UnknownIssuer => SslAlert::UNKNOWN_CA,
                        rustls::CertificateError::UnknownRevocationStatus => {
                            SslAlert::CERTIFICATE_UNKNOWN
                        }
                        rustls::CertificateError::BadSignature => SslAlert::BAD_CERTIFICATE,
                        rustls::CertificateError::NotValidForName => SslAlert::CERTIFICATE_UNKNOWN,
                        rustls::CertificateError::InvalidPurpose => SslAlert::CERTIFICATE_UNKNOWN,
                        rustls::CertificateError::ApplicationVerificationFailure => {
                            SslAlert::INTERNAL_ERROR
                        }
                        rustls::CertificateError::Other(_) => SslAlert::CERTIFICATE_UNKNOWN,

                        // CertificateError is marked non_exhaustive, so we also have to have an explicit fallback:
                        _ => SslAlert::CERTIFICATE_UNKNOWN,
                    },
                    _ => SslAlert::BAD_CERTIFICATE,
                })
            })?;

        Ok(())
    });

    Ok(())
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use assert_matches::assert_matches;
    use boring::ssl::{ErrorCode, SslConnector, SslMethod};
    use rustls::RootCertStore;
    use tokio::net::TcpStream;

    use crate::infra::tcp_ssl::testutil::{
        localhost_http_server, make_http_request_response_over, PROXY_CERTIFICATE,
        SERVER_CERTIFICATE, SERVER_HOSTNAME,
    };

    use super::*;

    #[tokio::test]
    async fn verify_certificate_via_rustls() {
        let (addr, server) = localhost_http_server();
        let _server_handle = tokio::spawn(server);

        let mut root_cert_store = RootCertStore::empty();
        root_cert_store
            .add(SERVER_CERTIFICATE.cert.der().clone())
            .expect("valid");
        let verifier = rustls::client::WebPkiServerVerifier::builder(Arc::new(root_cert_store))
            .build()
            .expect("valid");

        let mut ssl = SslConnector::builder(SslMethod::tls_client()).expect("valid");
        set_up_platform_verifier(
            &mut ssl,
            SERVER_HOSTNAME,
            Arc::into_inner(verifier).expect("only one referent"),
        )
        .expect("valid");

        let transport = TcpStream::connect(addr).await.expect("can connect");
        let connection = tokio_boring::connect(
            ssl.build().configure().expect("valid"),
            SERVER_HOSTNAME,
            transport,
        )
        .await
        .expect("successful handshake");

        make_http_request_response_over(connection).await;
    }

    #[tokio::test]
    async fn verify_certificate_failure_via_rustls() {
        let (addr, server) = localhost_http_server();
        let _server_handle = tokio::spawn(server);

        let mut root_cert_store = RootCertStore::empty();
        // Wrong certificate here!
        root_cert_store
            .add(PROXY_CERTIFICATE.cert.der().clone())
            .expect("valid");
        let verifier = rustls::client::WebPkiServerVerifier::builder(Arc::new(root_cert_store))
            .build()
            .expect("valid");

        let mut ssl = SslConnector::builder(SslMethod::tls_client()).expect("valid");
        set_up_platform_verifier(
            &mut ssl,
            SERVER_HOSTNAME,
            Arc::into_inner(verifier).expect("only one referent"),
        )
        .expect("valid");

        let transport = TcpStream::connect(addr).await.expect("can connect");
        assert_matches!(
            tokio_boring::connect(
                ssl.build().configure().expect("valid"),
                SERVER_HOSTNAME,
                transport,
            )
            .await,
            Err(e) if e.code() == Some(ErrorCode::SSL)
        );
    }
}
