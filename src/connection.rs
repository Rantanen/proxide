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

pub async fn run(
    client: TcpStream,
    src_addr: SocketAddr,
    options: Arc<ConnectionOptions>,
    ui: Sender<SessionEvent>,
) -> Result<()>
{
    // Peek into the client stream to demux the protocol.
    connect_phase(Uuid::new_v4(), client, src_addr, Vec::new(), options, ui).await
}

pub async fn connect_phase(
    uuid: Uuid,
    client: TcpStream,
    src_addr: SocketAddr,
    mut protocol_stack: Vec<Protocol>,
    options: Arc<ConnectionOptions>,
    ui: Sender<SessionEvent>,
) -> Result<()>
{
    log::info!("{} - New connection from {:?}", uuid, src_addr);

    let (protocol, client) =
        demux::recognize(client)
            .await
            .context(IoError {})
            .context(ClientError {
                scenario: "demuxing stream",
            })?;
    log::info!("{} - Top level protocol: {:?}", uuid, protocol);

    if protocol == demux::Protocol::Connect {
        protocol_stack.push(Protocol::Connect);
        let connect_data = connect::handle_connect(client).await?;

        // Perform a new demux on the inner stream. We'll do this here so we can reuse the original
        // demux result in `handle_protocol` without having to perform another one there again.
        let (protocol, client_stream) = demux::recognize(connect_data.client_stream)
            .await
            .context(IoError {})
            .context(ClientError {
                scenario: "demuxing stream",
            })?;
        log::info!("{} - Next protocol: {:?}", uuid, protocol);

        handle_protocol(
            uuid,
            protocol,
            client_stream,
            connect_data.server_stream,
            src_addr,
            protocol_stack,
            options,
            ui,
        )
        .await
    } else {
        let server = TcpStream::connect(AsRef::<str>::as_ref(&options.target_server))
            .await
            .context(IoError {})
            .context(ServerError {
                scenario: "connecting",
            })?;
        handle_protocol(
            uuid,
            protocol,
            client,
            server,
            src_addr,
            protocol_stack,
            options,
            ui,
        )
        .await
    }
}

pub async fn handle_protocol<TClient, TServer>(
    uuid: Uuid,
    protocol: demux::Protocol,
    client: TClient,
    server: TServer,
    src_addr: SocketAddr,
    mut protocol_stack: Vec<Protocol>,
    options: Arc<ConnectionOptions>,
    ui: Sender<SessionEvent>,
) -> Result<()>
where
    TClient: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    TServer: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let ui_clone = ui.clone();
    if protocol == demux::Protocol::TLS {
        protocol_stack.push(Protocol::Tls);
        let streams = tls::TlsProxy::new(uuid, client, server, options.clone()).await?;
        let mut conn = http2::Http2Connection::new(
            uuid,
            src_addr,
            streams.client_stream,
            streams.server_stream,
            protocol_stack,
            ui_clone,
        )
        .await?;
        conn.run(ui).await?;
    } else {
        let mut conn =
            http2::Http2Connection::new(uuid, src_addr, client, server, protocol_stack, ui_clone)
                .await?;
        conn.run(ui).await?;
    }

    Ok(())
}
