use snafu::Snafu;
use tui::widgets::Text;

use crate::session::{MessageData, RequestData};

pub mod grpc;
pub mod raw;

#[derive(Debug, Snafu)]
pub enum Error
{
    #[snafu(display("Parameter {}: {}", option, msg))]
    ConfigurationValueError
    {
        option: &'static str,
        msg: String,
        source: Box<dyn std::error::Error + Send>,
    },

    #[snafu(display("Parameter {}: {}", option, source))]
    ConfigurationError
    {
        option: &'static str,
        source: Box<dyn std::error::Error + Send>,
    },
}

type Result<S, E = Error> = std::result::Result<S, E>;

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
