use clap::ArgMatches;

use super::*;

pub fn initialize(_args: &ArgMatches) -> Result<Option<Box<dyn DecoderFactory>>>
{
    Ok(Some(Box::new(RawDecoderFactory)))
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
    fn name(&self) -> &'static str
    {
        "raw"
    }

    fn decode(&self, msg: &MessageData) -> Text
    {
        Text::raw(format!("{:?}", msg.content))
    }

    fn index(&self, _msg: &MessageData) -> Vec<String>
    {
        vec![]
    }
}
