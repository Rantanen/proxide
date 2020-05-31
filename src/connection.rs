use snafu::{ResultExt, Snafu};
use std::net::SocketAddr;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use tokio::net::TcpStream;

use crate::session::events::*;
use crate::session::*;

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
//) -> Result<http2::Http2Connection<impl AsyncWrite + AsyncRead + Unpin + 'static, TcpStream>>
{
    let (protocol, client) =
        demux::recognize(client)
            .await
            .context(IoError {})
            .context(ClientError {
                scenario: "demuxing stream",
            })?;

    let ui_clone = ui.clone();
    if protocol == demux::Protocol::TLS {
        log::info!("New TLS connection from {}", src_addr);

        let streams = tls::TlsProxy::new(client, options.clone()).await?;
        let mut conn = http2::Http2Connection::new(
            src_addr,
            streams.client_stream,
            streams.server_stream,
            ui_clone,
        )
        .await?;
        conn.run(ui).await?;
    } else {
        let server = TcpStream::connect(&options.target_server)
            .await
            .context(IoError {})
            .context(ServerError {
                scenario: "connecting",
            })?;
        let mut conn = http2::Http2Connection::new(src_addr, client, server, ui_clone).await?;
        conn.run(ui).await?;
    }

    Ok(())
}
