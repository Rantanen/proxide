use std::io::Result;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use tokio::io::{split, AsyncRead, AsyncWrite, ReadHalf, WriteHalf};

pub struct PrefixedRead<S>
{
    prefix: Option<Vec<u8>>,
    read: ReadHalf<S>,
}

pub struct PrefixedStream<S>
{
    read: PrefixedRead<S>,
    write: WriteHalf<S>,
}

impl<S> PrefixedStream<S>
where
    S: AsyncWrite + AsyncRead,
{
    pub fn new(prefix: Vec<u8>, stream: S) -> Self
    {
        let (read, write) = split(stream);
        Self {
            read: PrefixedRead {
                prefix: Some(prefix),
                read,
            },
            write,
        }
    }

    pub fn into_split(self) -> (PrefixedRead<S>, WriteHalf<S>)
    {
        (self.read, self.write)
    }
}

impl<S: AsyncWrite + AsyncRead + Unpin> PrefixedStream<S> {}

impl<S: AsyncRead> AsyncRead for PrefixedStream<S>
{
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context, buf: &mut [u8]) -> Poll<Result<usize>>
    {
        let inner_pin = unsafe { self.map_unchecked_mut(|s| &mut s.read) };
        inner_pin.poll_read(cx, buf)
    }
}

impl<S: AsyncRead> AsyncRead for PrefixedRead<S>
{
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context, buf: &mut [u8])
        -> Poll<Result<usize>>
    {
        if let Some(p) = &mut self.prefix {
            if p.is_empty() {
                // If the Vec was empty, we'll want to avoid returning zero bytes. Tokio considers
                // a return of zero bytes as stream having ended, which is not what is happening
                // here.
                self.prefix = None;
            } else if p.len() <= buf.len() {
                buf[..p.len()].copy_from_slice(p);
                let copied = p.len();
                self.prefix = None;
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

impl<S: AsyncWrite> AsyncWrite for PrefixedStream<S>
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
