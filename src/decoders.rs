use snafu::Snafu;
use tui::text::Text;

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

pub fn setup_args(app: clap::App) -> clap::App
{
    grpc::setup_args(app)
}

pub fn get_decoders(matches: &clap::ArgMatches) -> Result<Decoders, Error>
{
    let decoders = vec![raw::initialize(matches)?, grpc::initialize(matches)?];

    Ok(Decoders::new(decoders.into_iter().flatten()))
}

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
            .filter_map(move |d| d.try_create(request, message))
    }

    pub fn index(&self, request: &RequestData, message: &MessageData) -> Vec<String>
    {
        self.factories
            .iter()
            .filter_map(|d| d.try_create(request, message))
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
    fn name(&self) -> &'static str;
    fn decode(&self, msg: &MessageData) -> Text<'_>;
    fn index(&self, msg: &MessageData) -> Vec<String>;
}
