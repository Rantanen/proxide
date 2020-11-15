use snafu::ResultExt;
use std::net::SocketAddr;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use uuid::Uuid;

use crate::session::events::*;
use crate::session::*;
use crate::{CADetails, ConnectionOptions};

mod connect;
mod demux;
pub mod error;
mod http2;
pub mod scripting;
mod stream;
mod tls;

use error::*;

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
            .context(EndpointError {
                endpoint: EndpointType::Client,
                scenario: "demuxing stream",
            })?;
    log::debug!("{} - Top level protocol: {:?}", details.uuid, protocol);

    if protocol == demux::Protocol::Connect {
        // Ensure Proxide is set up as a CONNECT proxy.
        let connect_filter = match &options.proxy {
            Some(f) => f,
            None => {
                return Err(EndpointErrorKind::ProxideError {
                    reason: "CONNECT proxy requests are not allowed",
                })
                .context(EndpointError {
                    endpoint: EndpointType::Client,
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
                .context(EndpointError {
                    endpoint: EndpointType::Client,
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
                return Err(EndpointErrorKind::ProxideError {
                    reason: "Direct connections are not allowed",
                })
                .context(EndpointError {
                    endpoint: EndpointType::Client,
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
            .context(EndpointError {
                endpoint: EndpointType::Server,
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
        http2::handle(details, options, src_addr, streams, ui_clone).await?;
    } else {
        http2::handle(details, options, src_addr, streams, ui_clone).await?;
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
            if let Err(e) = write.write(&b[..count]).await {
                log::error!("Error writing data: {}", e);
                break;
            }
        }
        log::info!("Exit");
    });
}
