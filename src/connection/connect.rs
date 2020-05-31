use snafu::ResultExt;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use tokio::io::{split, AsyncRead, AsyncReadExt, AsyncWrite, ReadHalf, WriteHalf};

use super::{ClientIoError, Error, Result};

pub struct ConnectData
{
    server: String,
}

pub async fn handle_connect<T: AsyncRead + AsyncWrite + Unpin>(mut client: &mut T)
    -> Result<String>
{
    let mut buffer = Vec::new();
    buffer.resize(255, 0_u8);
    let mut read = 0;
    loop {
        let count = client
            .read(&mut buffer[read..])
            .await
            .context(ClientIoError {
                scenario: "reading CONNECT",
            })?;
        read += count;
        if &buffer[read - 5..read - 1] == b"\r\n\r\n" {
            break;
        }

        if read > 1024 {
            return Err(Error::ClientIoError {
                scenario: "reading CONNECT (Too large request)",
                source: std::io::ErrorKind::Other.into(),
            });
        }
    }

    Ok("".to_string())
}
