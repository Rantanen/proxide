use rustls::{
    sign::CertifiedKey, Certificate, ClientConfig, ClientHello, DangerousClientConfig,
    NoClientAuth, ResolvesServerCert, RootCertStore, ServerCertVerified, ServerCertVerifier,
    ServerConfig, Session, TLSError,
};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use uuid::Uuid;
use webpki::{DNSName, DNSNameRef};

use super::stream::PrefixedStream;
use super::*;

pub async fn handle<TClient, TServer>(
    uuid: Uuid,
    streams: Streams<TClient, TServer>,
    options: Arc<ConnectionOptions>,
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
    log::debug!("{} - Client SNI='{:?}', ALPN='{:?}'", uuid, sni, alpn);

    // Connect to the server.

    // Resolve the server hostname from the SNI.
    log::debug!("{} - Final target server='{}", uuid, options.target_server);

    let mut server_stream_config = ClientConfig::new();
    server_stream_config.set_protocols(&alpn);
    DangerousClientConfig {
        cfg: &mut server_stream_config,
    }
    .set_certificate_verifier(Arc::new(NoVerify));
    let server_stream_config = TlsConnector::from(Arc::new(server_stream_config));

    log::debug!(
        "{} - Establishing connection to {}",
        uuid,
        options.target_server
    );
    let server_stream = server_stream_config
        .connect(sni.as_ref(), server)
        .await
        .context(IoError {})
        .context(ServerError {
            scenario: "connecting TLS",
        })?;

    let alpn = server_stream.get_ref().1.get_alpn_protocol();
    log::debug!(
        "{} - Server connection done; ALPN='{:?}'",
        uuid,
        alpn.map(|o| String::from_utf8_lossy(o))
    );

    let mut client_stream_config = ServerConfig::new(NoClientAuth::new());
    let (cert_chain, private_key) =
        get_certificate(AsRef::<str>::as_ref(&options.target_server), ca);
    log::trace!(
        "{} - Certificate: {}",
        uuid,
        cert_chain[0]
            .0
            .iter()
            .map(|u| format!("{:02x} ", u))
            .collect::<String>()
    );
    client_stream_config
        .set_single_cert(cert_chain, private_key)
        .unwrap();
    alpn.map(|alpn| client_stream_config.set_protocols(&[alpn.to_vec()]));
    let client_stream_acceptor = TlsAcceptor::from(Arc::new(client_stream_config));
    let client_stream = client_stream_acceptor
        .accept(PrefixedStream::new(client_data, client))
        .await
        .context(IoError {})
        .context(ClientError {
            scenario: "connecting TLS",
        })?;

    log::debug!("{} - TLS stream established", uuid);
    Ok(Streams {
        client: client_stream,
        server: server_stream,
    })
}

struct ClientHelloCapture
{
    channel: Mutex<mpsc::Sender<(Option<DNSName>, Vec<Vec<u8>>)>>,
}
impl ResolvesServerCert for ClientHelloCapture
{
    fn resolve(&self, client_hello: ClientHello) -> Option<CertifiedKey>
    {
        log::debug!("Capturing ClientHello from cert resolver");
        let _ = self.channel.lock().unwrap().send((
            client_hello.server_name().map(|m| m.to_owned()),
            client_hello
                .alpn()
                .into_iter()
                .flat_map(|arr| arr.into_iter().map(|v| (*v).into()))
                .collect::<Vec<_>>(),
        ));
        None
    }
}

struct NoVerify;
impl ServerCertVerifier for NoVerify
{
    fn verify_server_cert(
        &self,
        _roots: &RootCertStore,
        _presented_certs: &[Certificate],
        _dns_name: DNSNameRef,
        _ocsp_response: &[u8],
    ) -> Result<ServerCertVerified, TLSError>
    {
        Ok(ServerCertVerified::assertion())
    }
}

struct HelloResult
{
    data: Vec<u8>,
    sni: DNSName,
    alpn: Vec<Vec<u8>>,
}

async fn resolve_client_hello<TStream>(client: &mut TStream) -> Result<HelloResult>
where
    TStream: AsyncRead + Unpin,
{
    let (sender, receiver) = mpsc::channel();
    let mut config = ServerConfig::new(NoClientAuth::new());
    config.cert_resolver = Arc::new(ClientHelloCapture {
        channel: Mutex::new(sender),
    });
    let mut hello_session = rustls::ServerSession::new(&Arc::new(config));

    // Process data until we get the ClientHello.
    let mut hello_data: Vec<u8> = Vec::new();
    let mut buffer = [0_u8; 2048];
    let (sni, alpn) = loop {
        log::debug!("Waiting for client bytes for ClientHello");
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
        log::debug!("Got {} bytes for ClientHello: {:?}", read, &buffer[..read]);

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
            .context(super::TLSError {})
            .context(ClientError {
                scenario: "parsing ClientHello",
            })?;
        log::debug!("SNI hostname? '{:?}'", hello_session.get_sni_hostname());
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
    // Split the port off. That's not needed for the certificate.
    let common_name = common_name
        .split(':')
        .next()
        .expect("Any string has at least one (empty) segment");

    log::debug!("Creating certificate for '{}'", common_name);

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
        vec![
            rustls::Certificate(cert.serialize_der_with_signer(&ca_cert).unwrap()), //
        ],
        rustls::PrivateKey(cert.serialize_private_key_der()),
    )
}
