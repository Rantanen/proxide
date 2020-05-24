use snafu::{ResultExt, Snafu};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::Receiver;
use std::sync::Mutex;
use tokio::sync::oneshot::Sender;

use super::events::SessionEvent;
use super::*;

const TYPE_LENGTH: usize = 15; // "PROXIDE-SESSION", "PROXIDE-CAPTURE"
const VERSION_LENGTH: usize = 3; // "v01"

#[derive(Debug, Snafu)]
pub enum SerializationError
{
    #[snafu(display("Error {} file {}\n{}", operation, file, source))]
    IoError
    {
        operation: &'static str,
        file: String,
        source: std::io::Error,
    },

    #[snafu(display("Unrecognized file format"))]
    UnrecognizedFile {},

    #[snafu(display("Unsupported {} file version '{}'", filetype, version))]
    UnsupportedVersion
    {
        filetype: &'static str,
        version: String,
    },

    #[snafu(display("Error deserializing data: {}", source))]
    FormatError
    {
        source: Box<dyn std::error::Error + Send>,
    },
}

#[derive(Debug, Default)]
pub struct CaptureStatus
{
    pub connections: usize,
    pub requests: usize,
    pub data: usize,
}

pub fn read_file<P: AsRef<Path> + ToString>(filename: &P) -> Result<Session, SerializationError>
{
    let mut file = std::fs::File::open(filename).context(IoError {
        operation: "reading",
        file: filename.to_string(),
    })?;

    let mut header = [0; TYPE_LENGTH + VERSION_LENGTH];
    file.read_exact(&mut header)
        .map_err(|_| SerializationError::UnrecognizedFile {})?;

    let filetype = &header[..TYPE_LENGTH];
    let version = &header[TYPE_LENGTH..];

    match filetype {
        b"PROXIDE-SESSION" => match version {
            b"v01" => read_session_file(file),
            _ => Err(SerializationError::UnsupportedVersion {
                filetype: "session",
                version: String::from_utf8_lossy(version).to_string(),
            }),
        },
        b"PROXIDE-CAPTURE" => match version {
            b"v01" => read_capture_file(file),
            _ => Err(SerializationError::UnsupportedVersion {
                filetype: "session",
                version: String::from_utf8_lossy(version).to_string(),
            }),
        },
        _ => Err(SerializationError::UnrecognizedFile {}),
    }
}

impl Session
{
    pub fn write_to_file<P: AsRef<Path> + ToString>(
        &self,
        filename: &P,
    ) -> Result<(), SerializationError>
    {
        let mut file = open_target_file(filename, b"PROXIDE-SESSIONv01")?;

        // We are using FormatError here even if the error message for that states
        // 'deserializing'. Since we are controlling the data, a serialization error shouldn't
        // occur here.
        match self.serialize(&mut rmp_serde::Serializer::new(&mut file)) {
            Ok(_) => Ok(()),
            Err(e) => {
                return Err(SerializationError::FormatError {
                    source: Box::new(e),
                })
            }
        }
    }
}

pub fn read_session_file(file: std::fs::File) -> Result<Session, SerializationError>
{
    rmp_serde::from_read(file)
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send>)
        .context(FormatError {})
}

pub fn capture_to_file<F: FnMut(&CaptureStatus), P: AsRef<Path> + ToString>(
    rx: Receiver<SessionEvent>,
    abort: Sender<()>,
    filename: &P,
    mut status_callback: F,
) -> Result<(), SerializationError>
{
    // Handle Ctrl-c gracefully since that's the intended way to stop the capture.
    let abort = Mutex::new(Some(abort));
    let _ = ctrlc::set_handler(move || {
        if let Ok(mut g) = abort.lock() {
            if let Some(tx) = g.take() {
                let _ = tx.send(());
            }
        }
    });

    let mut file = open_target_file(filename, b"PROXIDE-CAPTUREv01")?;
    let mut buffer: Vec<u8> = Vec::new();
    let mut status = CaptureStatus::default();
    while let Ok(event) = rx.recv() {
        // Handle status updates with certain events.
        match &event {
            SessionEvent::NewConnection(_) => status.connections += 1,
            SessionEvent::NewRequest(_) => status.requests += 1,
            SessionEvent::MessageData(d) => status.data += d.data.len(),
            _ => {}
        }

        // Print errors out, but otherwise ignore them.
        if let Err(e) = event.serialize(&mut rmp_serde::Serializer::new(&mut buffer)) {
            eprintln!("{}", e);
        } else {
            // Convert the data length as varint (each byte has 7 bytes of payload and the MSB
            // indicates whether the length continues in the next byte.
            let mut len_buffer: Vec<u8> = Vec::new();
            let mut len = buffer.len();
            while len >= 0x80 {
                len_buffer.push((len & 0x7f | 0x80) as u8);
                len = len >> 7;
            }
            len_buffer.push(len as u8);

            // Write the event. Length followed by the payload.
            file.write_all(&len_buffer)
                .and_then(|_| file.write_all(&buffer))
                .context(IoError {
                    operation: "writing",
                    file: filename.to_string(),
                })?;
            status_callback(&status);
            buffer.clear();
        }
    }

    Ok(())
}

pub fn read_capture_file(mut file: std::fs::File) -> Result<Session, SerializationError>
{
    let mut session = Session::default();

    let byte = &mut [0u8];
    let mut payload: Vec<u8> = Vec::new();
    loop {
        // Read length header byte by byte. We'll need to read this one byte at a time to avoid
        // over-reading into the actual payload
        let mut idx = 0;
        let mut payload_len = 0_usize;

        // Handle the first byte separately since this is a valid moment for the stream to end. If
        // the read here fails, it means we reached the end of the stream when we read the last
        // event.
        if let Err(_) = file.read_exact(byte) {
            return Ok(session);
        }
        loop {
            payload_len = payload_len + (((byte[0] & 0x7f) as usize) << (7 * idx));
            idx += 1;
            if byte[0] & 0x80 == 0 {
                break;
            }

            // An error here would indicate that the input file was cut in the middle of the length
            // data.
            if let Err(_) = file.read_exact(byte) {
                log::error!("Incomplete input file");
                return Ok(session);
            }
        }

        // An error here indicates incomplete payload.
        payload.clear();
        payload.resize(payload_len, 0);
        if let Err(_) = file.read_exact(&mut payload) {
            log::error!("Incomplete input file");
            return Ok(session);
        }

        // Deserialize the event and process it by the session.
        //
        // The events should include all the information required to replicate the session so this
        // is as good as receiving those events live.
        let event: SessionEvent = rmp_serde::from_slice(&payload)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send>)
            .context(FormatError {})?;
        session.handle(event);
    }
}

pub fn open_target_file<P: AsRef<Path> + ToString>(
    filename: &P,
    filetype: &[u8; TYPE_LENGTH + VERSION_LENGTH],
) -> Result<std::fs::File, SerializationError>
{
    let mut file = match std::fs::File::create(&filename) {
        Ok(f) => f,
        Err(e) => {
            return Err(SerializationError::IoError {
                operation: "opening",
                file: filename.to_string(),
                source: e,
            });
        }
    };

    // Write the file header
    match file.write_all(filetype) {
        Ok(_) => {}
        Err(e) => {
            return Err(SerializationError::IoError {
                operation: "writing",
                file: filename.to_string(),
                source: e,
            })
        }
    };

    Ok(file)
}

pub mod opt_header_map
{
    use http::HeaderMap;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(value: &Option<HeaderMap>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct Helper<'a>(#[serde(with = "http_serde::header_map")] &'a HeaderMap);

        value.as_ref().map(Helper).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<HeaderMap>, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper(#[serde(with = "http_serde::header_map")] HeaderMap);

        let helper = Option::deserialize(deserializer)?;
        Ok(helper.map(|Helper(external)| external))
    }
}
