use http::header::{HeaderName, HeaderValue};
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

pub struct Decoders
{
    factories: Vec<Box<dyn DecoderFactory>>,
}

impl Decoders
{
    pub fn new<T: IntoIterator<Item = Box<dyn DecoderFactory>>>(decoders: T) -> Self
    {
        Self {
            factories: decoders.into_iter().collect(),
        }
    }

    pub fn get_decoders<'a>(
        &'a self,
        request: &'a RequestData,
        message: &'a MessageData,
    ) -> impl Iterator<Item = Box<dyn Decoder>> + 'a
    {
        self.factories
            .iter()
            .map(move |d| d.try_create(request, message))
            .filter_map(|o| o)
    }

    pub fn index(&self, request: &RequestData, message: &MessageData) -> Vec<String>
    {
        self.factories
            .iter()
            .map(|d| d.try_create(request, message))
            .filter_map(|o| o)
            .flat_map(|d| d.index(message).into_iter())
            .collect()
    }
}

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
    fn index(&self, msg: &MessageData) -> Vec<String>;
}

struct HeaderDecoder;
impl Decoder for HeaderDecoder
{
    fn decode(&self, msg: &MessageData) -> Vec<Text>
    {
        self.process(
            msg,
            |s| Some(Text::raw(s)),
            |k, v| Text::raw(format!(" - {}: {:?}\n", k, v)),
        )
    }

    fn index(&self, msg: &MessageData) -> Vec<String>
    {
        self.process(msg, |_| None, |k, v| format!("{}: {:?}", k, v))
    }
}

impl HeaderDecoder
{
    fn process<T>(
        &self,
        msg: &MessageData,
        title: fn(&'static str) -> Option<T>,
        ctor: fn(&HeaderName, &HeaderValue) -> T,
    ) -> Vec<T>
    {
        let mut output = vec![];

        if !msg.headers.is_empty() {
            output.extend(title("Headers\n"));
            for (k, v) in &msg.headers {
                output.push(ctor(k, v));
            }
        }

        if !msg.trailers.is_empty() {
            output.extend(title("\nTrailers\n"));
            for (k, v) in &msg.headers {
                output.push(ctor(k, v));
            }
        }

        output
    }
}
