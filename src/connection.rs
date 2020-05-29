use bytes::Bytes;
use chrono::prelude::*;
use h2::{
    client::{self, ResponseFuture},
    server::{self, SendResponse},
    Reason, RecvStream, SendStream,
};
use http::{HeaderMap, Request, Response};
use log::error;
use snafu::{ResultExt, Snafu};
use std::net::SocketAddr;
use std::sync::mpsc::Sender;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::TcpStream;
use uuid::Uuid;

use crate::session::events::*;
use crate::session::*;

mod demux;
mod http2;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error
{
    #[snafu(display("HTTP error occurred with the server in {}: {}", scenario, source))]
    ServerError
    {
        scenario: &'static str,
        source: h2::Error,
    },

    #[snafu(display("HTTP error occurred with the client in {}: {}", scenario, source))]
    ClientError
    {
        scenario: &'static str,
        source: h2::Error,
    },

    #[snafu(display("HTTP error occurred with the client in {}: {}", scenario, source))]
    ClientIoError
    {
        scenario: &'static str,
        source: std::io::Error,
    },
}

pub type Result<S, E = Error> = std::result::Result<S, E>;

pub async fn connect(
    client: TcpStream,
    server: TcpStream,
    src_addr: SocketAddr,
    ui: Sender<SessionEvent>,
) -> Result<http2::Http2Connection<impl AsyncWrite + AsyncRead + Unpin + 'static, TcpStream>>
{
    let (protocol, client) = demux::recognize(client).await.context(ClientIoError {
        scenario: "demuxing stream",
    })?;
    http2::Http2Connection::new(src_addr, client, server, ui).await
}
