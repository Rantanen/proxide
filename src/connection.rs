use snafu::{ResultExt, Snafu};
use std::net::SocketAddr;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use uuid::Uuid;

use crate::session::events::*;
use crate::session::*;

mod connect;
mod demux;
mod http2;
mod stream;
mod tls;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum ConfigurationErrorKind
{
    DNSError
    {
        source: webpki::InvalidDNSNameError,
    },
    UriError
    {
        source: http::uri::InvalidUri,
    },
    UriPartsError
    {
        source: http::uri::InvalidUriParts,
    },
    NoSource {},
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum EndpointError
{
    IoError
    {
        source: std::io::Error
    },
    ConnectError
    {
        source: httparse::Error
    },
    H2Error
    {
        source: h2::Error
    },
    TLSError
    {
        source: rustls::TLSError
    },

    #[snafu(display("{}", reason))]
    ProxideError
    {
        reason: &'static str
    },
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error
{
    #[snafu(display("Configuration error: {}", reason))]
    ConfigurationError
    {
        reason: &'static str,
        source: ConfigurationErrorKind,
    },

    #[snafu(display("Error occurred with the server in {}: {}", scenario, source))]
    ServerError
    {
        scenario: &'static str,
        source: EndpointError,
    },

    #[snafu(display("Error occurred with the client in {}: {}", scenario, source))]
    ClientError
    {
        scenario: &'static str,
        source: EndpointError,
    },
}

pub type Result<S, E = Error> = std::result::Result<S, E>;

pub struct ConnectionOptions
{
    pub allow_remote: bool,
    pub listen_port: String,
    pub target_server: Option<String>,
    pub proxy: Option<Vec<ProxyFilter>>,
    pub ca: Option<CADetails>,
}

pub struct CADetails
{
    pub certificate: String,
    pub key: String,
}

pub struct ProxyFilter
{
    pub host_filter: wildmatch::WildMatch,
    pub port_filter: Option<std::num::NonZeroU16>,
}

pub struct ConnectionDetails
{
    /// Connection ID
    pub uuid: Uuid,

    /// A stack of protocols that the connection is using.
    pub protocol_stack: Vec<Protocol>,

    /// Server name for opaque redirect purposes.
    ///
    /// Presence of opaque redirect implies the client thinks it's connecting to a different
    /// server than Proxide is redirecting it to. This might result in the need to rewrite
    /// Host/authority headers, etc. in the outgoing requests.
    pub opaque_redirect: Option<String>,
}

pub struct Streams<TClient, TServer>
{
    pub client: TClient,
    pub server: TServer,
}

impl<TClient, TServer> Streams<TClient, TServer>
{
    pub fn new(client: TClient, server: TServer) -> Self
    {
        Self { client, server }
    }
}

/// Handles a single client connection.
///
/// The connection handling is split into multiple functions, but the functions are chained in a
/// deep call stack. This is done to handle the generics properly as each function performs
/// decisions that may affect the stream types going forward.
///
/// Avoiding having to return the streams allows us to avoid dynamic dispatch in the stream
/// handling.
pub async fn run(
    client: TcpStream,
    src_addr: SocketAddr,
    options: Arc<ConnectionOptions>,
    ui: Sender<SessionEvent>,
) -> Result<()>
{
    let details = ConnectionDetails {
        uuid: Uuid::new_v4(),
        protocol_stack: vec![],
        opaque_redirect: None,
    };
    connect_phase(details, client, src_addr, options, ui).await
}

/// Establishes the connection to the server.
///
/// The server may be either a hard coded one as specified by the user or one specified through a
/// CONNECT proxy request by the client.
pub async fn connect_phase(
    mut details: ConnectionDetails,
    client: TcpStream,
    src_addr: SocketAddr,
    options: Arc<ConnectionOptions>,
    ui: Sender<SessionEvent>,
) -> Result<()>
{
    log::info!("{} - New connection from {:?}", details.uuid, src_addr);

    // Resolve the top-level protocol.
    let (protocol, client) =
        demux::recognize(client)
            .await
            .context(IoError {})
            .context(ClientError {
                scenario: "demuxing stream",
            })?;
    log::debug!("{} - Top level protocol: {:?}", details.uuid, protocol);

    if protocol == demux::Protocol::Connect {
        // Ensure Proxide is set up as a CONNECT proxy.
        let connect_filter = match &options.proxy {
            Some(f) => f,
            None => {
                return Err(EndpointError::ProxideError {
                    reason: "CONNECT proxy requests are not allowed",
                })
                .context(ClientError {
                    scenario: "setting up server connection",
                })
            }
        };

        details.protocol_stack.push(Protocol::Connect);
        let connect_data = connect::handle_connect(client).await?;

        // Check what to do with the CONNECT target.
        if connect::check_filter(connect_filter, &connect_data.target_server) {
            log::info!("{} - Intercepting CONNECT", details.uuid);

            // The connection matches filter and should be decoded.

            // Perform a new demux on the inner stream. We'll do this here so we can reuse the original
            // demux result in `handle_protocol` without having to perform another one there again.
            let (protocol, client_stream) = demux::recognize(connect_data.client_stream)
                .await
                .context(IoError {})
                .context(ClientError {
                    scenario: "demuxing stream",
                })?;
            log::debug!("{} - Next protocol: {:?}", details.uuid, protocol);

            handle_protocol(
                details,
                protocol,
                Streams::new(client_stream, connect_data.server_stream),
                src_addr,
                connect_data.target_server,
                options,
                ui,
            )
            .await
        } else {
            log::info!("{} - Proxying CONNECT without decoding", details.uuid);
            // Connection does NOT match the filter. We should just pipe the
            // streams together.
            let (server_read, server_write) = connect_data.server_stream.into_split();
            let (client_read, client_write) = connect_data.client_stream.into_split();
            pipe_stream(client_read, server_write);
            pipe_stream(server_read, client_write);
            Ok(())
        }
    } else {
        let target_server = match &options.target_server {
            Some(t) => t,
            None => {
                return Err(EndpointError::ProxideError {
                    reason: "Direct connections are not allowed",
                })
                .context(ClientError {
                    scenario: "setting up server connection",
                })
            }
        };

        // Not a CONNECT request; Use the user supplied target server as the server address and
        // redirect the whole client stream there.
        details.opaque_redirect = Some(target_server.to_string());
        let server = TcpStream::connect(target_server)
            .await
            .context(IoError {})
            .context(ServerError {
                scenario: "connecting",
            })?;

        handle_protocol(
            details,
            protocol,
            Streams::new(client, server),
            src_addr,
            target_server.to_string(),
            options,
            ui,
        )
        .await
    }
}

pub async fn handle_protocol<TClient, TServer>(
    mut details: ConnectionDetails,
    protocol: demux::Protocol,
    streams: Streams<TClient, TServer>,
    src_addr: SocketAddr,
    target: String,
    options: Arc<ConnectionOptions>,
    ui: Sender<SessionEvent>,
) -> Result<()>
where
    TClient: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    TServer: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let ui_clone = ui.clone();
    if protocol == demux::Protocol::TLS {
        let streams = tls::handle(&mut details, streams, options.clone(), target).await?;
        http2::handle(details, src_addr, streams, ui_clone).await?;
    } else {
        http2::handle(details, src_addr, streams, ui_clone).await?;
    }

    Ok(())
}

fn pipe_stream<TRead, TWrite>(mut read: TRead, mut write: TWrite)
where
    TRead: AsyncRead + Unpin + Send + 'static,
    TWrite: AsyncWrite + Unpin + Send + 'static,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    tokio::spawn(async move {
        let mut b = [0_u8; 1024];
        log::info!("Enter");
        loop {
            let count = match read.read(&mut b).await {
                Err(e) => {
                    log::error!("Error reading data: {}", e);
                    break;
                }
                Ok(c) if c == 0 => break,
                Ok(c) => c,
            };
            match write.write(&b[..count]).await {
                Err(e) => {
                    log::error!("Error writing data: {}", e);
                    break;
                }
                Ok(_) => (),
            }
        }
        log::info!("Exit");
    });
}
