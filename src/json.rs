use clap::ArgMatches;
use snafu::ResultExt;

use super::session;
use super::{Error, SerializationError};

pub fn view(matches: &ArgMatches) -> Result<(), Error>
{
    let filename = matches.value_of("file").unwrap();
    let session = session::serialization::read_file(&filename).context(SerializationError {})?;
    println!("{}", serde_json::to_string_pretty(&session).unwrap());
    Ok(())
}
