use std::io::Result;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

use super::stream::PrefixedStream;

#[derive(Debug, PartialEq)]
pub enum Protocol
{
    Http2,
    Connect,
    TLS,
}

pub async fn recognize(
    mut stream: impl AsyncWrite + AsyncRead + Unpin + 'static,
) -> Result<(Protocol, impl AsyncWrite + AsyncRead + Unpin + 'static)>
{
    let mut buffer = [0_u8; 10];
    stream.read_exact(&mut buffer).await?;

    let protocol = match &buffer {
        // Content type: Handshake (22)
        // TLS version: 1.x (3, _)
        // Length: _, _
        // Handshake type: ClientHello (1)
        // Handshake length: _, _, _
        // Protocol version: 1.x (3, _)
        &[22, 3, _, _, _, 1, _, _, _, 3] => Protocol::TLS,
        b"PRI * HTTP" => Protocol::Http2,
        &[b'C', b'O', b'N', b'N', b'E', b'C', b'T', b' ', _, _] => Protocol::Connect,
        _ => return Err(std::io::ErrorKind::InvalidData.into()),
    };

    Ok((protocol, PrefixedStream::new(buffer.to_vec(), stream)))
}
