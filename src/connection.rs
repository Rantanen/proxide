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
    pub listen_port: String,
    pub target_server: String,
    pub ca: Option<CADetails>,
}

pub struct CADetails
{
    pub certificate: String,
    pub key: String,
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
    log::info!("{} - Top level protocol: {:?}", details.uuid, protocol);

    if protocol == demux::Protocol::Connect {
        details.protocol_stack.push(Protocol::Connect);
        let connect_data = connect::handle_connect(client).await?;

        // Perform a new demux on the inner stream. We'll do this here so we can reuse the original
        // demux result in `handle_protocol` without having to perform another one there again.
        let (protocol, client_stream) = demux::recognize(connect_data.client_stream)
            .await
            .context(IoError {})
            .context(ClientError {
                scenario: "demuxing stream",
            })?;
        log::info!("{} - Next protocol: {:?}", details.uuid, protocol);

        handle_protocol(
            details,
            protocol,
            Streams::new(client_stream, connect_data.server_stream),
            src_addr,
            options,
            ui,
        )
        .await
    } else {
        // Not a CONNECT request; Use the user supplied target server as the server address and
        // redirect the whole client stream there.
        details.opaque_redirect = Some(options.target_server.to_string());
        let server = TcpStream::connect(AsRef::<str>::as_ref(&options.target_server))
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
    options: Arc<ConnectionOptions>,
    ui: Sender<SessionEvent>,
) -> Result<()>
where
    TClient: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    TServer: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let ui_clone = ui.clone();
    if protocol == demux::Protocol::TLS {
        let streams = tls::handle(&mut details, streams, options.clone()).await?;
        http2::handle(details, src_addr, streams, ui_clone).await?;
    } else {
        http2::handle(details, src_addr, streams, ui_clone).await?;
    }

    Ok(())
}
