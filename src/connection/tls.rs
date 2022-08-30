use rustls::{
    client::{ServerCertVerified, ServerCertVerifier, ServerName},
    server::{ClientHello, ResolvesServerCert},
    sign::CertifiedKey,
    Certificate, ClientConfig, ServerConfig,
};
use std::convert::TryFrom;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
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

    // We'll avoid validating the server certificates, since we don't really know what certs the
    // client trusts.
    let mut server_stream_config = ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(Arc::new(NoVerify))
        .with_no_client_auth();
    server_stream_config.alpn_protocols = alpn;
    let server_stream_config = TlsConnector::from(Arc::new(server_stream_config));

    log::debug!(
        "{} - Establishing connection to {}",
        details.uuid,
        target_host
    );
    let server_stream = server_stream_config
        .connect(
            ServerName::try_from(outgoing_sni.as_ref())
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

    let mut client_stream_config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(cert_chain, private_key)
        .unwrap();
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

struct ClientHelloData
{
    sni: Option<String>,
    alpn: Vec<Vec<u8>>,
}
struct ClientHelloCapture
{
    channel: Mutex<mpsc::Sender<ClientHelloData>>,
}
impl ResolvesServerCert for ClientHelloCapture
{
    fn resolve(&self, client_hello: ClientHello) -> Option<Arc<CertifiedKey>>
    {
        log::trace!("Capturing ClientHello from cert resolver");
        let _ = self.channel.lock().unwrap().send(ClientHelloData {
            sni: client_hello.server_name().map(|m| m.to_owned()),
            alpn: client_hello
                .alpn()
                .into_iter()
                .flat_map(|arr| arr.map(|v| v.to_owned()).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
        });
        None
    }
}

struct NoVerify;
impl ServerCertVerifier for NoVerify
{
    fn verify_server_cert(
        &self,
        _end_entity: &Certificate,
        _intermediates: &[Certificate],
        _server_name: &ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<ServerCertVerified, rustls::Error>
    {
        Ok(ServerCertVerified::assertion())
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
    let (sender, receiver) = mpsc::channel();
    let config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(ClientHelloCapture {
            channel: Mutex::new(sender),
        }));
    let mut hello_session = rustls::ServerConnection::new(Arc::new(config))
        .context(TlsError {})
        .context(ClientError {
            scenario: "Connecting to server",
        })?;

    // Process data until we get the ClientHello.
    let mut hello_data: Vec<u8> = Vec::new();
    let mut buffer = [0_u8; 2048];
    let ClientHelloData { sni, alpn } = loop {
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
        hello_session
            .read_tls(&mut packet)
            .context(IoError {})
            .context(ClientError {
                scenario: "parsing ClientHello",
            })?;

        // Process packets and immediately follow that by checking if the ClientHello arrived. Our
        // `cert_resolver` doesn't really know how to do its job so it will cause a TLS error, but
        // if we got the ClientHello before that, we're good!
        let process_result = hello_session.process_new_packets();
        if let Ok(tuple) = receiver.try_recv() {
            break tuple;
        }

        // No ClientHello yet. Handle the result as normal.
        process_result
            .context(super::TlsError {})
            .context(ClientError {
                scenario: "parsing ClientHello",
            })?;
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
) -> (Vec<rustls::Certificate>, rustls::PrivateKey)
{
    let ca_key = rcgen::KeyPair::from_pem(&ca.key).unwrap();
    let ca_params = rcgen::CertificateParams::from_ca_cert_pem(&ca.certificate, ca_key).unwrap();
    let ca_cert = rcgen::Certificate::from_params(ca_params).unwrap();

    let mut cert_params = rcgen::CertificateParams::new(vec![]);
    cert_params.use_authority_key_identifier_extension = false;
    cert_params.distinguished_name = rcgen::DistinguishedName::new();
    cert_params.distinguished_name.push(
        rcgen::DnType::OrganizationName,
        "UNSAFE Proxide Certificate",
    );
    cert_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, common_name);
    let cert = rcgen::Certificate::from_params(cert_params).unwrap();
    (
        vec![rustls::Certificate(
            cert.serialize_der_with_signer(&ca_cert).unwrap(),
        )],
        rustls::PrivateKey(cert.serialize_private_key_der()),
    )
}
