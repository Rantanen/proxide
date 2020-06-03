use clap::ArgMatches;
use serde::{Deserialize, Serialize};
use snafu::ResultExt;

use super::session;
use super::{Error, SerializationError};

#[derive(Serialize, Deserialize)]
pub struct JsonSessionOutput
{
    session: session::Session,
}

pub fn view(matches: &ArgMatches) -> Result<(), Error>
{
    let filename = matches.value_of("file").unwrap();
    let session = session::serialization::read_file(&filename).context(SerializationError {})?;

    let output = JsonSessionOutput { session };

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
    Ok(())
}
