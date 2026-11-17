use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{ring, CryptoProvider, WebPkiSupportedAlgorithms},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    server::Acceptor,
    ClientConfig, DigitallySignedStruct, ServerConfig, SignatureScheme,
};
use std::convert::TryFrom;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use super::stream::PrefixedStream;
use super::*;

pub async fn handle<TClient, TServer>(
    details: &mut ConnectionDetails,
    streams: Streams<TClient, TServer>,
    options: Arc<ConnectionOptions>,
    target_host: String,
) -> Result<
    Streams<
        tokio_rustls::server::TlsStream<PrefixedStream<TClient>>,
        tokio_rustls::client::TlsStream<TServer>,
    >,
>
where
    TClient: AsyncRead + AsyncWrite + Unpin,
    TServer: AsyncRead + AsyncWrite + Unpin,
{
    details.protocol_stack.push(Protocol::Tls);
    let Streams { mut client, server } = streams;

    let ca = match &options.ca {
        None => {
            return Err(Error::ConfigurationError {
                source: ConfigurationErrorKind::NoSource {},
                reason: "TLS connections require CA certificate",
            })
        }
        Some(ca) => ca,
    };

    // Peek at the client request to figure out the server/alpn the client sent.
    let HelloResult {
        data: client_data,
        sni,
        alpn,
    } = resolve_client_hello(&mut client).await?;
    log::debug!(
        "{} - Client SNI='{:?}', ALPN='{:?}'",
        details.uuid,
        sni,
        alpn
    );

    // Connect to the server.

    // If there is opaque redirect in place, use that as the outgoing SNI.
    let outgoing_sni = if let Some(redirect_sni) = &details.opaque_redirect {
        // Split the port off. That's not needed for the SNI.
        redirect_sni
            .split(':')
            .next()
            .expect("Any string has at least one (empty) segment")
            .to_owned()
    } else {
        sni.clone()
    };

    let server_stream_config =
        upstream_client_config(alpn)
            .context(TlsError {})
            .context(ServerError {
                scenario: "configuring TLS",
            })?;
    let server_stream_config = TlsConnector::from(Arc::new(server_stream_config));

    log::debug!(
        "{} - Establishing connection to {}",
        details.uuid,
        target_host
    );
    let server_stream = server_stream_config
        .connect(
            ServerName::try_from(outgoing_sni)
                .context(DNSError {})
                .context(ConfigurationError {
                    reason: "Invalid target server",
                })?,
            server,
        )
        .await
        .context(IoError {})
        .context(ServerError {
            scenario: "connecting TLS",
        })?;

    let alpn = server_stream.get_ref().1.alpn_protocol();
    log::debug!(
        "{} - Server connection done; ALPN='{:?}'",
        details.uuid,
        alpn.map(String::from_utf8_lossy)
    );

    // Establish the client connection.

    log::debug!("{} - Creating certificate for '{}'", details.uuid, &sni);
    let (cert_chain, private_key) = get_certificate(&sni, ca);

    let mut client_stream_config = ServerConfig::builder_with_provider(crypto_provider())
        .with_safe_default_protocol_versions()
        .context(TlsError {})
        .context(ClientError {
            scenario: "configuring TLS",
        })?
        .with_no_client_auth()
        .with_single_cert(cert_chain, private_key)
        .context(TlsError {})
        .context(ClientError {
            scenario: "configuring TLS",
        })?;
    if let Some(alpn) = alpn {
        client_stream_config.alpn_protocols = vec![alpn.to_vec()];
    }
    let client_stream_acceptor = TlsAcceptor::from(Arc::new(client_stream_config));
    let client_stream = client_stream_acceptor
        .accept(PrefixedStream::new(client_data, client))
        .await
        .context(IoError {})
        .context(ClientError {
            scenario: "connecting TLS",
        })?;

    log::debug!(
        "{} - TLS streams established with client and server",
        details.uuid
    );
    Ok(Streams {
        client: client_stream,
        server: server_stream,
    })
}

/// The crypto provider used for both sides of the intercepted connection.
///
/// Proxide uses the `ring` provider explicitly instead of relying on the process-wide default,
/// which rustls would otherwise choose based on the enabled crate features.
fn crypto_provider() -> Arc<CryptoProvider>
{
    Arc::new(ring::default_provider())
}

/// TLS client configuration for connections from Proxide to servers.
///
/// We'll avoid validating the server certificates, since we don't really know what certs the
/// client trusts.
pub fn upstream_client_config(
    alpn: Vec<Vec<u8>>,
) -> std::result::Result<ClientConfig, rustls::Error>
{
    let provider = crypto_provider();
    let mut config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerify {
            algorithms: provider.signature_verification_algorithms,
        }))
        .with_no_client_auth();
    config.alpn_protocols = alpn;
    Ok(config)
}

/// Server certificate verifier that accepts any certificate.
///
/// Proxide doesn't know which certificates the client would trust, so it can't validate the
/// server's certificate chain. Handshake signatures are still verified so that the TLS session
/// itself is sound.
#[derive(Debug)]
struct NoVerify
{
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for NoVerify
{
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error>
    {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error>
    {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error>
    {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme>
    {
        self.algorithms.supported_schemes()
    }
}

struct HelloResult
{
    data: Vec<u8>,
    sni: String,
    alpn: Vec<Vec<u8>>,
}

async fn resolve_client_hello<TStream>(client: &mut TStream) -> Result<HelloResult>
where
    TStream: AsyncRead + Unpin,
{
    // Feed the client bytes to a rustls `Acceptor` until it has parsed the ClientHello. The bytes
    // are also kept so they can be replayed to the real TLS acceptor once the certificate for the
    // requested server name has been generated.
    let mut acceptor = Acceptor::default();
    let mut hello_data: Vec<u8> = Vec::new();
    let mut buffer = [0_u8; 2048];
    let (sni, alpn) = loop {
        log::trace!("Waiting for client bytes for ClientHello");
        let read = client
            .read(&mut buffer)
            .await
            .context(IoError {})
            .context(ClientError {
                scenario: "reading ClientHello",
            })?;
        if read == 0 {
            return Err(std::io::ErrorKind::ConnectionReset.into())
                .context(IoError {})
                .context(ClientError {
                    scenario: "reading ClientHello",
                });
        }
        let mut packet: &[u8] = &buffer[..read];
        log::trace!("Got {} bytes for ClientHello: {:?}", read, &buffer[..read]);

        hello_data.extend(packet);
        while !packet.is_empty() {
            acceptor
                .read_tls(&mut packet)
                .context(IoError {})
                .context(ClientError {
                    scenario: "parsing ClientHello",
                })?;
        }

        match acceptor.accept() {
            Ok(Some(accepted)) => {
                let hello = accepted.client_hello();
                break (
                    hello.server_name().map(str::to_owned),
                    hello
                        .alpn()
                        .into_iter()
                        .flatten()
                        .map(<[u8]>::to_vec)
                        .collect::<Vec<_>>(),
                );
            }
            Ok(None) => continue,
            Err((e, _alert)) => {
                return Err(e).context(super::TlsError {}).context(ClientError {
                    scenario: "parsing ClientHello",
                })
            }
        }
    };

    let sni = match sni {
        Some(sni) => sni,
        None => {
            return Err(EndpointError::ProxideError {
                reason: "Client is required to support SNI",
            })
            .context(ClientError {
                scenario: "resolving ClientHello",
            })
        }
    };

    Ok(HelloResult {
        data: hello_data,
        sni,
        alpn,
    })
}

fn get_certificate(
    common_name: &str,
    ca: &CADetails,
) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)
{
    let ca_key = rcgen::KeyPair::from_pem(&ca.key).unwrap();
    let issuer = rcgen::Issuer::from_ca_cert_pem(&ca.certificate, ca_key).unwrap();

    // Clients such as Go (grpcurl) and browsers ignore the common name and only match
    // the host against the subject alternative names.
    let mut cert_params = rcgen::CertificateParams::new(vec![common_name.to_string()]).unwrap();
    cert_params.use_authority_key_identifier_extension = false;
    cert_params.distinguished_name = rcgen::DistinguishedName::new();
    cert_params.distinguished_name.push(
        rcgen::DnType::OrganizationName,
        "UNSAFE Proxide Certificate",
    );
    cert_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, common_name);
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = cert_params.signed_by(&key, &issuer).unwrap();
    (
        vec![cert.der().clone()],
        PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    )
}

#[cfg(test)]
mod test
{
    use super::*;
    use rustls::client::WebPkiServerVerifier;

    fn test_ca() -> (CADetails, rustls::RootCertStore)
    {
        let mut params = rcgen::CertificateParams::new(vec![]).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Proxide Test CA");
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap();

        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert.der().clone()).unwrap();
        let ca = CADetails {
            certificate: cert.pem(),
            key: key.serialize_pem(),
        };
        (ca, roots)
    }

    fn verify(host: &'static str) -> std::result::Result<ServerCertVerified, rustls::Error>
    {
        let (ca, roots) = test_ca();
        let (chain, _) = get_certificate(host, &ca);
        WebPkiServerVerifier::builder_with_provider(Arc::new(roots), crypto_provider())
            .build()
            .unwrap()
            .verify_server_cert(
                &chain[0],
                &[],
                &ServerName::try_from(host).unwrap(),
                &[],
                UnixTime::now(),
            )
    }

    #[test]
    fn certificate_is_valid_for_dns_name()
    {
        verify("localhost").expect("certificate should be valid for 'localhost'");
        verify("example.com").expect("certificate should be valid for 'example.com'");
    }

    #[test]
    fn certificate_is_not_valid_for_other_names()
    {
        let (ca, roots) = test_ca();
        let (chain, _) = get_certificate("example.com", &ca);
        let result =
            WebPkiServerVerifier::builder_with_provider(Arc::new(roots), crypto_provider())
                .build()
                .unwrap()
                .verify_server_cert(
                    &chain[0],
                    &[],
                    &ServerName::try_from("other.example.com").unwrap(),
                    &[],
                    UnixTime::now(),
                );
        assert!(result.is_err());
    }

    /// Run a full TLS handshake through `handle` against a real TLS server, and check that the
    /// client sees a certificate signed by the proxide CA and the server's ALPN choice.
    #[tokio::test]
    async fn intercepts_tls_connection()
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (ca, roots) = test_ca();
        let roots = Arc::new(roots);

        // The "real" server, with its own certificate that nobody trusts.
        let server_key = rcgen::KeyPair::generate().unwrap();
        let server_cert = rcgen::CertificateParams::new(vec!["upstream.test".to_string()])
            .unwrap()
            .self_signed(&server_key)
            .unwrap();
        let mut server_config = ServerConfig::builder_with_provider(crypto_provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![server_cert.der().clone()],
                PrivatePkcs8KeyDer::from(server_key.serialize_der()).into(),
            )
            .unwrap();
        server_config.alpn_protocols = vec![b"h2".to_vec()];

        let (client_side, proxy_client_side) = tokio::io::duplex(64 * 1024);
        let (proxy_server_side, server_side) = tokio::io::duplex(64 * 1024);

        let upstream_task = tokio::spawn(async move {
            let mut stream = tokio_rustls::TlsAcceptor::from(Arc::new(server_config))
                .accept(server_side)
                .await
                .unwrap();
            let mut buf = [0u8; 4];
            stream.read_exact(&mut buf).await.unwrap();
            stream.write_all(b"pong").await.unwrap();
            stream.flush().await.unwrap();
            buf
        });

        let client_task = tokio::spawn(async move {
            let mut config = ClientConfig::builder_with_provider(crypto_provider())
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth();
            config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
            let mut stream = tokio_rustls::TlsConnector::from(Arc::new(config))
                .connect(ServerName::try_from("upstream.test").unwrap(), client_side)
                .await
                .expect("client should trust the proxide certificate");
            assert_eq!(stream.get_ref().1.alpn_protocol(), Some(&b"h2"[..]));
            stream.write_all(b"ping").await.unwrap();
            stream.flush().await.unwrap();
            let mut buf = [0u8; 4];
            stream.read_exact(&mut buf).await.unwrap();
            buf
        });

        let mut details = ConnectionDetails {
            uuid: uuid::Uuid::new_v4(),
            protocol_stack: vec![],
            opaque_redirect: None,
        };
        let options = Arc::new(ConnectionOptions {
            allow_remote: false,
            listen_port: "0".to_string(),
            target_server: None,
            proxy: None,
            ca: Some(ca),
        });
        let Streams {
            mut client,
            mut server,
        } = handle(
            &mut details,
            Streams::new(proxy_client_side, proxy_server_side),
            options,
            "upstream.test".to_string(),
        )
        .await
        .unwrap();

        // Relay one message each way.
        let mut buf = [0u8; 4];
        client.read_exact(&mut buf).await.unwrap();
        server.write_all(&buf).await.unwrap();
        server.flush().await.unwrap();
        server.read_exact(&mut buf).await.unwrap();
        client.write_all(&buf).await.unwrap();
        client.flush().await.unwrap();

        assert_eq!(&upstream_task.await.unwrap(), b"ping");
        assert_eq!(&client_task.await.unwrap(), b"pong");
        assert!(matches!(details.protocol_stack[..], [Protocol::Tls]));
    }
}
