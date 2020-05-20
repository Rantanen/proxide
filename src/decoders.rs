use tui::widgets::Text;

use crate::ui_state::{MessageData, RequestData};

mod grpc;
pub use grpc::GrpcDecoderFactory;

/// A factory for constructing decoders.
pub trait DecoderFactory
{
    /// Attempt to create a decoder for the request.
    fn try_create(&self, req: &RequestData, msg: &MessageData) -> Option<Box<dyn Decoder>>;
}

/// Generic decoder trait that is invoked to acquire the decoded output.
pub trait Decoder
{
    fn decode(&self, msg: &MessageData) -> Vec<Text>;
}

pub struct RawDecoderFactory;
impl DecoderFactory for RawDecoderFactory
{
    fn try_create(&self, _: &RequestData, _: &MessageData) -> Option<Box<dyn Decoder>>
    {
        Some(Box::new(RawDecoder))
    }
}

pub struct RawDecoder;
impl Decoder for RawDecoder
{
    fn decode(&self, msg: &MessageData) -> Vec<Text>
    {
        vec![Text::raw(format!("{:?}", msg.content))]
    }
}

struct HeaderDecoder;
impl Decoder for HeaderDecoder
{
    fn decode(&self, msg: &MessageData) -> Vec<Text>
    {
        let mut output = vec![];

        if msg.headers.len() > 0 {
            output.push(Text::raw("Headers\n"));
            for (k, v) in &msg.headers {
                output.push(Text::raw(format!(" - {}: {:?}\n", k, v)));
            }
        }

        if msg.trailers.len() > 0 {
            output.push(Text::raw("\nTrailers\n"));
            for (k, v) in &msg.headers {
                output.push(Text::raw(format!(" - {}: {:?}\n", k, v)));
            }
        }

        output
    }
}
