use snafu::ResultExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use super::stream::PrefixedStream;
use super::{ClientError, ConnectError, IoError, ProxyFilter, Result, ServerError};

pub struct ConnectData<TClient>
{
    pub client_stream: PrefixedStream<TClient>,
    pub server_stream: TcpStream,
    pub target_server: String,
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

    let host = AsRef::<str>::as_ref(&host);
    let server = TcpStream::connect(host)
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
        target_server: host.to_string(),
    })
}

pub fn check_filter(filter: &[ProxyFilter], target: &str) -> bool
{
    // If there are no filters, everything ought to be accepted.
    if filter.is_empty() {
        return true;
    }

    // Split the target uri into host and port.
    let mut split = target.split(':');
    let host = split.next().unwrap();
    let port: u16 = split.next().unwrap().parse().unwrap();

    for f in filter {
        if !f.host_filter.is_match(host) {
            continue;
        }

        if let Some(port_filter) = f.port_filter {
            if port != port_filter.get() {
                continue;
            }
        }

        // None of the filters rejected this.
        return true;
    }

    false
}
