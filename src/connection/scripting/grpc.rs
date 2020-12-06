use bytes::{Buf, Bytes};
use h2::{Reason, RecvStream};
use protofish::context::{Context, MessageRef};
use protofish::decode::Value;
use std::collections::VecDeque;
use std::sync::Arc;

use super::http2::{ScriptRecvStream, ScriptSendStream};

#[derive(runestick::Any)]
pub struct ScriptGrpcContext
{
    inner: Arc<Context>,
}

pub struct TransformStream
{
    ctx: Arc<Context>,
    msg: MessageRef,
    rx: ScriptRecvStream,
    tx: ScriptSendStream,
}

pub struct ScriptValue
{
    inner: Value,
}

impl ScriptGrpcContext
{
    fn register(module: &mut runestick::Module, ctx: Arc<Context>)
    {
        module
            .ty::<Self>()
            .expect("Failed to register ScriptGrpcContext");
        module
            .function(&["GrpcContext", "new"], Self::new)
            .expect("Failed to register ScriptGrpcContext::new");
    }

    fn new() -> runestick::Result<Self>
    {
        let ctx = crate::decoders::grpc::CONTEXT
            .lock()
            .unwrap()
            .as_ref()
            .expect("grpc module was registered with no Context")
            .clone();

        Ok(Self { inner: ctx })
    }
}

impl TransformStream
{
    fn new(request: http::request::Parts, rx: ScriptRecvStream, tx: ScriptSendStream) -> Self
    {
        let ctx = crate::decoders::grpc::CONTEXT
            .lock()
            .unwrap()
            .as_ref()
            .expect("grpc module was registered with no Context")
            .clone();
        let path = request.uri.path().rsplit('/');
        let function = path.next().unwrap();
        let service = path.next().unwrap();
        let rpc = match ctx.get_service(service).map(|s|

        Self { ctx, msg, rx, tx }
    }

    async fn run(mut self) -> runestick::Result<()>
    {
        let mut rx_buffers = VecDeque::new();

        loop {
            match self.run_once(&mut rx_buffers).await {
                Err(e) => {
                    self.tx
                        .inner
                        .send_reset(e.reason().unwrap_or(Reason::PROTOCOL_ERROR));
                    return Err(e.into());
                }
                Ok(false) => break,
                Ok(true) => {}
            }
        }

        Ok(())
    }

    async fn run_once(&mut self, buffers: &mut VecDeque<Bytes>) -> Result<bool, h2::Error>
    {
        let rx = &mut self.rx.inner;
        let tx = &mut self.tx.inner;

        let mut prefix_bytes = match read_buffered(5, rx, buffers).await? {
            Some(b) => b,
            None => {
                match rx.trailers().await? {
                    Some(t) => tx.send_trailers(t)?,
                    None => tx.send_reset(Reason::NO_ERROR),
                }
                return Ok(false);
            }
        };

        let compressed = prefix_bytes.get_u8();
        if compressed != 0 {
            return Err(Reason::COMPRESSION_ERROR.into());
        }
        let length = prefix_bytes.get_u32();
        let content_bytes = match read_buffered(length as usize, rx, buffers).await? {
            Some(b) => b,
            None => return Err(Reason::PROTOCOL_ERROR.into()),
        };

        let msg_value = self.ctx.decode(self.msg, &content_bytes);
        let output = msg_value.encode(&self.ctx);
        tx.send_data(output.freeze(), false)?;

        return Ok(true);
    }
}

async fn read_buffered(
    count: usize,
    rx: &mut RecvStream,
    buffers: &mut VecDeque<Bytes>,
) -> Result<Option<Bytes>, h2::Error>
{
    let mut current_len = buffers.iter().fold(0, |sum, buf| sum + buf.len());
    while current_len < count {
        let new_buffer = match rx.data().await {
            Some(b) => b?,
            None => break,
        };
        current_len += new_buffer.len();
        buffers.push_back(new_buffer);
    }

    // Check we got enough data.
    //
    // Getting no data is okay in this case as that might indicate end of streams, etc.
    // However getting partial data means there's data we do not understand.
    if current_len < count {
        return match current_len {
            0 => Ok(None),
            _ => Err(Reason::PROTOCOL_ERROR.into()),
        };
    }

    // Buffers should have enough data now. Extract the final buffer.
    let mut output = bytes::BytesMut::with_capacity(count);
    while output.len() < count {
        let remaining = count - output.len();
        let front = buffers
            .front_mut()
            .expect("Buffers did not have enough content");

        if front.len() > remaining {
            output.extend_from_slice(&front[..remaining]);
            *front = front.slice(remaining..);
        } else {
            output.extend_from_slice(&front);
            buffers.pop_front();
        }
    }

    Ok(Some(output.freeze()))
}

fn register(module: &mut runestick::Module)
{
    // Do not register the gRPC module if the decoder context is missing.
    if crate::decoders::grpc::CONTEXT.lock().unwrap().is_none() {
        return;
    }
}
