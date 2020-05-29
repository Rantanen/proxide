use std::io::Result;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use tokio::io::{split, AsyncRead, AsyncReadExt, AsyncWrite, ReadHalf, WriteHalf};

#[derive(Debug)]
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
    println!("{:?}", String::from_utf8_lossy(&buffer));

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

    let (read_half, write_half) = split(stream);

    Ok((
        protocol,
        DemultiplexingStream {
            prelude: Some(buffer.into()),
            read: read_half,
            write: write_half,
        },
    ))
}

/// A stream that supports HTTP, HTTP2 or TLS connections.
#[allow(dead_code)]
struct DemultiplexingStream<S>
{
    prelude: Option<Vec<u8>>,
    read: ReadHalf<S>,
    write: WriteHalf<S>,
}

impl<S: AsyncWrite + AsyncRead + Unpin> DemultiplexingStream<S> {}

impl<S: AsyncRead> AsyncRead for DemultiplexingStream<S>
{
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context, buf: &mut [u8])
        -> Poll<Result<usize>>
    {
        if let Some(p) = &mut self.prelude {
            if p.len() <= buf.len() {
                buf[..p.len()].copy_from_slice(&p);
                let copied = p.len();
                self.prelude = None;
                return Poll::Ready(Ok(copied));
            } else {
                let mut taken = p.split_off(buf.len());
                std::mem::swap(&mut taken, p);
                buf.copy_from_slice(&taken);
                return Poll::Ready(Ok(buf.len()));
            }
        }

        // `self` is pinned, which results in `s.stream` being pinned.
        // This makes the `map_unchecked_mut` safe here.
        let inner_pin = unsafe { self.map_unchecked_mut(|s| &mut s.read) };
        inner_pin.poll_read(cx, buf)
    }
}

impl<S: AsyncWrite> AsyncWrite for DemultiplexingStream<S>
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<Result<usize>>
    {
        // `self` is pinned, which results in `s.stream` being pinned.
        // This makes the `map_unchecked_mut` safe here.
        let inner_pin = unsafe { self.map_unchecked_mut(|s| &mut s.write) };
        inner_pin.poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<()>>
    {
        let inner_pin = unsafe { self.map_unchecked_mut(|s| &mut s.write) };
        inner_pin.poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<()>>
    {
        let inner_pin = unsafe { self.map_unchecked_mut(|s| &mut s.write) };
        inner_pin.poll_shutdown(cx)
    }
}
