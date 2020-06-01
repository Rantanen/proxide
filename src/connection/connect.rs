use snafu::ResultExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use super::stream::PrefixedStream;
use super::{ClientError, ConnectError, IoError, Result, ServerError};

pub struct ConnectData<TClient>
{
    pub client_stream: PrefixedStream<TClient>,
    pub server_stream: TcpStream,
}

pub async fn handle_connect<T: AsyncRead + AsyncWrite + Unpin>(
    mut client: T,
) -> Result<ConnectData<T>>
{
    let mut buffer = Vec::new();
    let (host, remainder) = loop {
        let mut chunk = [0_u8; 256];
        let count = client
            .read(&mut chunk)
            .await
            .context(IoError {})
            .context(ClientError {
                scenario: "reading CONNECT",
            })?;
        buffer.extend(chunk[..count].iter().copied());

        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut req = httparse::Request::new(&mut headers);
        let res = req
            .parse(&buffer)
            .context(ConnectError {})
            .context(ClientError {
                scenario: "parsing CONNECT request",
            })?;
        if let httparse::Status::Complete(count) = res {
            break (req.path.unwrap().to_string(), buffer[count..].to_vec());
        }
    };

    let server = TcpStream::connect(AsRef::<str>::as_ref(&host))
        .await
        .context(IoError {})
        .context(ServerError {
            scenario: "connecting",
        })?;
    client
        .write(b"HTTP/1.1 200 OK\r\n\r\n")
        .await
        .context(IoError {})
        .context(ServerError {
            scenario: "responding to CONNECT",
        })?;

    Ok(ConnectData {
        client_stream: PrefixedStream::new(remainder, client),
        server_stream: server,
    })
}
