use clap::ArgMatches;
use serde::Serialize;
use snafu::ResultExt;
use std::collections::HashMap;
use uuid::Uuid;

use super::session;
use super::{DecoderError, Error, SerializationError};

#[derive(Serialize)]
pub struct JsonSessionOutput
{
    session: session::Session,
    request_data: HashMap<Uuid, JsonRequestData>,
}

#[derive(Serialize)]
pub struct JsonRequestData
{
    request: JsonMessageData,
    response: JsonMessageData,
}

#[derive(Serialize)]
pub struct JsonMessageData
{
    decoded: HashMap<&'static str, String>,
}

pub fn view(matches: &ArgMatches) -> Result<(), Error>
{
    let filename = matches.value_of("file").unwrap();
    let session = session::serialization::read_file(&filename).context(SerializationError {})?;

    let decoders = crate::decoders::get_decoders(matches).context(DecoderError {})?;

    let mut output = JsonSessionOutput {
        session,
        request_data: Default::default(),
    };
    for r in output.session.requests.iter() {
        output.request_data.insert(
            r.request_data.uuid,
            JsonRequestData {
                request: get_message_data(&r.request_data, &r.request_msg, &decoders),
                response: get_message_data(&r.request_data, &r.response_msg, &decoders),
            },
        );
    }

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
    Ok(())
}

fn get_message_data(
    req: &session::RequestData,
    msg: &session::MessageData,
    decoders: &crate::decoders::Decoders,
) -> JsonMessageData
{
    JsonMessageData {
        decoded: decoders
            .get_decoders(req, msg)
            .map(|dec| {
                (
                    dec.name(),
                    dec.decode(msg)
                        .into_iter()
                        .map(|text| text_to_string(text))
                        .collect::<String>(),
                )
            })
            .collect(),
    }
}

fn text_to_string(t: tui::widgets::Text) -> String
{
    match t {
        tui::widgets::Text::Raw(s) | tui::widgets::Text::Styled(s, _) => s.into(),
    }
}
