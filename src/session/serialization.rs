use snafu::{ResultExt, Snafu};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Mutex;
use tokio::sync::oneshot::Sender;

use super::events::SessionEvent;
use super::*;

const TYPE_LENGTH: usize = 15; // "PROXIDE-SESSION", "PROXIDE-CAPTURE"
const VERSION_LENGTH: usize = 3; // "v01"

// File format versions
// ====================
//
// MessagePack files start with a header made of the file type and a format version. The payload
// is encoded with rmp-serde, and the format version identifies how it was encoded:
//
// | Header               | Written by       | Payload encoding                                    |
// |----------------------|------------------|-----------------------------------------------------|
// | `PROXIDE-SESSIONv01` | proxide <= 0.3.0 | rmp-serde 0.14, human-readable (`Encoding::Legacy`) |
// | `PROXIDE-CAPTUREv02` | proxide <= 0.3.0 | rmp-serde 0.14, human-readable (`Encoding::Legacy`) |
// | `PROXIDE-SESSIONv02` | proxide >= 0.4.0 | rmp-serde 1.x, compact (`Encoding::Current`)        |
// | `PROXIDE-CAPTUREv03` | proxide >= 0.4.0 | rmp-serde 1.x, compact (`Encoding::Current`)        |
//
// rmp-serde 0.14 reported itself to serde as "human readable", so types such as `Uuid` were
// written as strings. rmp-serde 1.x is not human readable by default, which switches those types
// to binary encodings, and it also encodes enums differently. The two encodings are not
// compatible in either direction, so the header versions were bumped when moving to rmp-serde 1.x:
// older proxide releases then report "unsupported version" for new files instead of failing
// with a decoding error. Old files are still read through rmp-serde 1.x's human-readable mode.
//
// Any future change to the payload encoding must bump the versions again and keep the old ones
// readable. The tests in this module load files recorded with proxide 0.3.0 from `test/data`.
//
// JSON output (`--json`) has no header and is not affected by any of this.
const SESSION_HEADER: &[u8; TYPE_LENGTH + VERSION_LENGTH] = b"PROXIDE-SESSIONv02";
const CAPTURE_HEADER: &[u8; TYPE_LENGTH + VERSION_LENGTH] = b"PROXIDE-CAPTUREv03";

/// How the MessagePack payload of a file is encoded. See "File format versions" above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding
{
    /// Session `v01` / capture `v02`: rmp-serde 0.14 in human-readable mode (proxide <= 0.3.0).
    Legacy,

    /// Session `v02` / capture `v03`: rmp-serde 1.x defaults.
    Current,
}

fn decode<T, R>(rd: R, encoding: Encoding) -> Result<T, SerializationError>
where
    T: serde::de::DeserializeOwned,
    R: Read,
{
    let mut de = rmp_serde::Deserializer::new(rd);
    match encoding {
        Encoding::Legacy => T::deserialize(&mut de.with_human_readable()),
        Encoding::Current => T::deserialize(&mut de),
    }
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send>)
    .context(FormatError {})
}

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

#[derive(Debug, Default, Clone, Copy)]
pub struct CaptureStatus
{
    pub connections: usize,
    pub active_connections: usize,
    pub requests: usize,
    pub active_requests: usize,
    pub data: usize,
}

#[derive(Clone, Copy)]
pub enum OutputFormat
{
    MessagePack,
    Json,
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

    let unsupported = |filetype| {
        Err(SerializationError::UnsupportedVersion {
            filetype,
            version: String::from_utf8_lossy(version).to_string(),
        })
    };
    match filetype {
        b"PROXIDE-SESSION" => match version {
            b"v01" => read_session_file(file, Encoding::Legacy),
            b"v02" => read_session_file(file, Encoding::Current),
            _ => unsupported("session"),
        },
        b"PROXIDE-CAPTURE" => match version {
            b"v02" => read_capture_file(file, Encoding::Legacy),
            b"v03" => read_capture_file(file, Encoding::Current),
            _ => unsupported("capture"),
        },
        _ => Err(SerializationError::UnrecognizedFile {}),
    }
}

impl Session
{
    pub fn write_to_file(
        &self,
        filename: &str,
        format: OutputFormat,
    ) -> Result<(), SerializationError>
    {
        let file = open_target_file(filename, format, SESSION_HEADER)?;
        serialize_format(file, self, format)
    }
}

fn read_session_file(file: std::fs::File, encoding: Encoding)
    -> Result<Session, SerializationError>
{
    decode(file, encoding)
}

pub fn capture_to_file<F: FnMut(&CaptureStatus) + Send + 'static>(
    rx: Receiver<SessionEvent>,
    abort: Sender<()>,
    filename: &str,
    format: OutputFormat,
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

    // Set up a thread that invokes 'status_callback'.
    //
    // This allows the status handling to proceed without pending UI updates and
    // the UI updates will only update the "last" state to the UI without having
    // to go through every single intermediate state.
    enum StatusAction
    {
        Status(CaptureStatus),
        Callback,
        Quit,
    }
    let (status_tx, status_rx) = channel();
    let status_thread = std::thread::spawn({
        let status_tx = status_tx.clone();
        move || {
            let mut pending_callback = false;
            let mut current_status = CaptureStatus::default();
            while let Ok(s) = status_rx.recv() {
                match s {
                    StatusAction::Callback => {
                        pending_callback = false;
                        status_callback(&current_status);
                    }
                    StatusAction::Status(status) => {
                        current_status = status;
                        if !pending_callback {
                            pending_callback = true;
                            status_tx.send(StatusAction::Callback).unwrap();
                        }
                    }
                    StatusAction::Quit => {
                        break;
                    }
                }
            }
        }
    });

    let mut file = open_target_file(filename, format, CAPTURE_HEADER)?;
    let mut buffer: Vec<u8> = Vec::new();
    let mut status = CaptureStatus::default();
    while let Ok(event) = rx.recv() {
        // Handle status updates with certain events.
        match &event {
            SessionEvent::NewConnection(_) => {
                status.connections += 1;
                status.active_connections += 1;
            }
            SessionEvent::NewRequest(_) => {
                status.requests += 1;
                status.active_requests += 1;
            }
            SessionEvent::ConnectionDone(_) => status.active_connections -= 1,
            SessionEvent::RequestDone(_) => status.active_requests -= 1,
            SessionEvent::MessageData(d) => status.data += d.data.len(),
            _ => {}
        }

        // Print errors out, but otherwise ignore them.
        if let Err(e) = serialize_format(&mut buffer, event, format) {
            eprintln!("{}", e);
        } else {
            match format {
                OutputFormat::Json => writeln!(
                    file,
                    "{}",
                    std::str::from_utf8(&buffer)
                        .expect("JSON serialization produced invalid UTF-8")
                )
                .context(IoError {
                    operation: "writing",
                    file: filename.to_string(),
                })?,
                OutputFormat::MessagePack => {
                    // Convert the data length as varint (each byte has 7 bytes of payload and the MSB
                    // indicates whether the length continues in the next byte.
                    let mut len_buffer: Vec<u8> = Vec::new();
                    let mut len = buffer.len();
                    while len >= 0x80 {
                        len_buffer.push((len & 0x7f | 0x80) as u8);
                        len >>= 7;
                    }
                    len_buffer.push(len as u8);

                    // Write the event. Length followed by the payload.
                    file.write_all(&len_buffer)
                        .and_then(|_| file.write_all(&buffer))
                        .context(IoError {
                            operation: "writing",
                            file: filename.to_string(),
                        })?;
                }
            }
            status_tx.send(StatusAction::Status(status)).unwrap();
            buffer.clear();
        }
    }

    // Join the status thread.
    status_tx.send(StatusAction::Quit).unwrap();
    status_thread.join().unwrap();

    Ok(())
}

fn read_capture_file(
    mut file: std::fs::File,
    encoding: Encoding,
) -> Result<Session, SerializationError>
{
    let mut session = Session::default();

    let mut payload: Vec<u8> = Vec::new();
    let byte = &mut [0u8];
    loop {
        // Read length header byte by byte. We'll need to read this one byte at a time to avoid
        // over-reading into the actual payload
        let mut idx = 0;
        let mut payload_len = 0_usize;

        // Handle the first byte separately since this is a valid moment for the stream to end. If
        // the read here fails, it means we reached the end of the stream when we read the last
        // event.
        if file.read_exact(byte).is_err() {
            return Ok(session);
        }
        loop {
            payload_len += ((byte[0] & 0x7f) as usize) << (7 * idx);
            idx += 1;
            if byte[0] & 0x80 == 0 {
                break;
            }

            // An error here would indicate that the input file was cut in the middle of the length
            // data.
            if file.read_exact(byte).is_err() {
                log::error!("Incomplete input file");
                return Ok(session);
            }
        }

        // An error here indicates incomplete payload.
        payload.clear();
        payload.resize(payload_len, 0);
        if file.read_exact(&mut payload).is_err() {
            log::error!("Incomplete input file");
            return Ok(session);
        }

        // Deserialize the event and process it by the session.
        //
        // The events should include all the information required to replicate the session so this
        // is as good as receiving those events live.
        let event: SessionEvent = decode(&payload[..], encoding)?;
        session.handle(event);
    }
}

pub fn open_target_file(
    filename: &str,
    format: OutputFormat,
    filetype: &[u8; TYPE_LENGTH + VERSION_LENGTH],
) -> Result<Box<dyn Write>, SerializationError>
{
    let mut file: Box<dyn Write> = match filename {
        "-" => Box::new(std::io::stdout()),
        _ => match std::fs::File::create(filename) {
            Ok(f) => Box::new(f),
            Err(e) => {
                return Err(SerializationError::IoError {
                    operation: "opening",
                    file: filename.to_string(),
                    source: e,
                });
            }
        },
    };

    // Write the file header
    //
    // We want to keep this as a match-statement to ensure new output formats need to be taken into
    // account here.
    #[allow(clippy::single_match)]
    match format {
        OutputFormat::MessagePack => {
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
        }
        _ => (),
    }

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

fn serialize_format(
    file: impl Write,
    data: impl Serialize,
    format: OutputFormat,
) -> Result<(), SerializationError>
{
    // We are using FormatError here even if the error message for that states 'deserializing'.
    // Since we are controlling the data, a serialization error shouldn't occur here so we
    // don't _really_ care about an error message; This is essentially just an unwrap.
    match format {
        OutputFormat::MessagePack => data
            .serialize(&mut rmp_serde::Serializer::new(file))
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send>),
        OutputFormat::Json => data
            .serialize(&mut serde_json::Serializer::new(file))
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send>),
    }
    .map_err(|source| SerializationError::FormatError { source })
}

#[cfg(test)]
mod test
{
    use super::*;
    use crate::session::events::*;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    /// Files recorded with proxide 0.3.0 (rmp-serde 0.14): two connections and five gRPC
    /// requests through `capture --no-ca`. The session file was saved from the same capture.
    const LEGACY_CAPTURE: &str = "test/data/capture-v02-proxide-0.3.0.bin";
    const LEGACY_SESSION: &str = "test/data/session-v01-proxide-0.3.0.bin";

    fn data_path(relative: &str) -> String
    {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(relative)
            .to_string_lossy()
            .to_string()
    }

    /// A file in the temp directory that is removed when dropped.
    struct TempFile(String);
    impl TempFile
    {
        fn new() -> Self
        {
            let name = format!("proxide-test-{}.bin", Uuid::new_v4());
            TempFile(
                std::env::temp_dir()
                    .join(name)
                    .to_string_lossy()
                    .to_string(),
            )
        }

        fn header(&self) -> Vec<u8>
        {
            std::fs::read(&self.0).unwrap()[..TYPE_LENGTH + VERSION_LENGTH].to_vec()
        }
    }
    impl Drop for TempFile
    {
        fn drop(&mut self)
        {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// Order-independent representation of a session for comparisons.
    fn to_json(session: &Session) -> serde_json::Value
    {
        serde_json::to_value(session).unwrap()
    }

    fn uuid(s: &str) -> Uuid
    {
        Uuid::parse_str(s).unwrap()
    }

    fn assert_legacy_fixture_content(session: &Session)
    {
        let connections = &session.connections.items;
        assert_eq!(connections.len(), 2);
        for c in connections {
            assert_eq!(c.client_addr.ip().to_string(), "127.0.0.1");
            assert!(matches!(c.protocol_stack[..], [Protocol::Http2]));
            assert_eq!(c.status, Status::Succeeded);
        }
        assert!(session
            .connections
            .get_index_by_uuid(uuid("5df70714-44fc-4ad8-b6b9-aff258a953ca"))
            .is_some());

        let requests = &session.requests.items;
        assert_eq!(requests.len(), 5);
        for r in requests {
            assert_eq!(r.request_data.method, http::Method::POST);
            assert_eq!(r.request_data.status, Status::Succeeded);
            assert_eq!(&r.request_msg.content[..], &[0, 0, 0, 0, 0]);
            assert_eq!(r.response_msg.headers["content-type"], "application/grpc");
        }

        let first = &requests[0];
        assert_eq!(
            first.request_data.uuid,
            uuid("9c79bb00-2b04-4162-b089-c009aa5547fd")
        );
        assert_eq!(
            first.request_data.connection_uuid,
            uuid("5df70714-44fc-4ad8-b6b9-aff258a953ca")
        );
        assert_eq!(
            first.request_data.uri,
            "http://127.0.0.1:50551/rust_grpc.TestService/GetDiagnostics"
        );
        assert_eq!(
            &first.response_msg.content[..],
            &[0, 0, 0, 0, 9, 10, 7, 8, 2, 16, 177, 192, 244, 4]
        );
        assert_eq!(
            requests[1].request_data.uri,
            "http://127.0.0.1:50551/rust_grpc.TestService/SendMessage"
        );
    }

    /// Events for one connection with one completed request.
    fn sample_events() -> Vec<SessionEvent>
    {
        let connection = uuid("11111111-2222-4333-8444-555555555555");
        let request = uuid("66666666-7777-4888-9999-aaaaaaaaaaaa");
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let t = |ms| t0 + Duration::from_millis(ms);

        let mut request_headers = http::HeaderMap::new();
        request_headers.insert("content-type", "application/grpc".parse().unwrap());
        let mut trailers = http::HeaderMap::new();
        trailers.insert("grpc-status", "0".parse().unwrap());

        vec![
            SessionEvent::NewConnection(NewConnectionEvent {
                uuid: connection,
                protocol_stack: vec![Protocol::Tls, Protocol::Http2],
                client_addr: "[::1]:40000".parse().unwrap(),
                timestamp: t(0),
            }),
            SessionEvent::NewRequest(NewRequestEvent {
                connection_uuid: connection,
                uuid: request,
                uri: "https://example.test/pkg.Service/Method".parse().unwrap(),
                method: http::Method::POST,
                headers: request_headers.clone(),
                timestamp: t(1),
            }),
            SessionEvent::MessageData(MessageDataEvent {
                uuid: request,
                data: bytes::Bytes::from_static(&[0, 0, 0, 0, 2, 8, 1]),
                part: RequestPart::Request,
            }),
            SessionEvent::MessageDone(MessageDoneEvent {
                uuid: request,
                part: RequestPart::Request,
                status: Status::Succeeded,
                timestamp: t(2),
                trailers: None,
            }),
            SessionEvent::NewResponse(NewResponseEvent {
                connection_uuid: connection,
                uuid: request,
                headers: request_headers,
                timestamp: t(3),
            }),
            SessionEvent::MessageData(MessageDataEvent {
                uuid: request,
                data: bytes::Bytes::from_static(&[0, 0, 0, 0, 0]),
                part: RequestPart::Response,
            }),
            SessionEvent::MessageDone(MessageDoneEvent {
                uuid: request,
                part: RequestPart::Response,
                status: Status::Succeeded,
                timestamp: t(4),
                trailers: Some(trailers),
            }),
            SessionEvent::RequestDone(RequestDoneEvent {
                uuid: request,
                status: Status::Succeeded,
                timestamp: t(5),
            }),
            SessionEvent::ConnectionDone(ConnectionDoneEvent {
                uuid: connection,
                status: Status::Succeeded,
                timestamp: t(6),
            }),
        ]
    }

    fn session_from(events: Vec<SessionEvent>) -> Session
    {
        let mut session = Session::default();
        for e in events {
            session.handle(e);
        }
        session
    }

    #[test]
    fn legacy_capture_from_0_3_0_loads()
    {
        let session = read_file(&data_path(LEGACY_CAPTURE)).unwrap();
        assert_legacy_fixture_content(&session);
    }

    #[test]
    fn legacy_session_from_0_3_0_loads()
    {
        let session = read_file(&data_path(LEGACY_SESSION)).unwrap();
        assert_legacy_fixture_content(&session);
    }

    #[test]
    fn legacy_capture_and_session_have_same_content()
    {
        let capture = read_file(&data_path(LEGACY_CAPTURE)).unwrap();
        let session = read_file(&data_path(LEGACY_SESSION)).unwrap();

        // Capture files store event times as `SystemTime`, which has a 100 ns resolution on
        // Windows, while the session file keeps the nanoseconds recorded on Linux.
        let mut capture = to_json(&capture);
        let mut session = to_json(&session);
        truncate_timestamps_to_100ns(&mut capture);
        truncate_timestamps_to_100ns(&mut session);
        assert_eq!(capture, session);
    }

    fn truncate_timestamps_to_100ns(value: &mut serde_json::Value)
    {
        use chrono::{DateTime, SecondsFormat, Timelike};
        match value {
            serde_json::Value::String(s) => {
                if let Ok(ts) = DateTime::parse_from_rfc3339(s) {
                    let nanos = ts.nanosecond() / 100 * 100;
                    *s = ts
                        .with_nanosecond(nanos)
                        .unwrap()
                        .to_rfc3339_opts(SecondsFormat::Nanos, true);
                }
            }
            serde_json::Value::Array(items) => {
                items.iter_mut().for_each(truncate_timestamps_to_100ns)
            }
            serde_json::Value::Object(map) => {
                map.values_mut().for_each(truncate_timestamps_to_100ns)
            }
            _ => {}
        }
    }

    #[test]
    fn capture_round_trip_uses_current_format()
    {
        let file = TempFile::new();
        let (tx, rx) = channel();
        for e in sample_events() {
            tx.send(e).unwrap();
        }
        drop(tx);
        let (abort, _abort_rx) = tokio::sync::oneshot::channel();
        capture_to_file(rx, abort, &file.0, OutputFormat::MessagePack, |_| {}).unwrap();

        assert_eq!(file.header(), CAPTURE_HEADER);
        let loaded = read_file(&file.0).unwrap();
        assert_eq!(to_json(&loaded), to_json(&session_from(sample_events())));
    }

    #[test]
    fn session_round_trip_uses_current_format()
    {
        let file = TempFile::new();
        let session = session_from(sample_events());
        session
            .write_to_file(&file.0, OutputFormat::MessagePack)
            .unwrap();

        assert_eq!(file.header(), SESSION_HEADER);
        let loaded = read_file(&file.0).unwrap();
        assert_eq!(to_json(&loaded), to_json(&session));
    }

    #[test]
    fn legacy_session_resaved_in_current_format_keeps_content()
    {
        let file = TempFile::new();
        let legacy = read_file(&data_path(LEGACY_SESSION)).unwrap();
        legacy
            .write_to_file(&file.0, OutputFormat::MessagePack)
            .unwrap();

        assert_eq!(file.header(), SESSION_HEADER);
        let loaded = read_file(&file.0).unwrap();
        assert_legacy_fixture_content(&loaded);
        assert_eq!(to_json(&loaded), to_json(&legacy));
    }

    /// The legacy and current encodings can't read each other's data. This is why the header
    /// versions were bumped: older proxide releases must not try to decode new files.
    #[test]
    fn legacy_and_current_encodings_are_incompatible()
    {
        let session = session_from(sample_events());
        let mut current = Vec::new();
        session
            .serialize(&mut rmp_serde::Serializer::new(&mut current))
            .unwrap();
        assert!(decode::<Session, _>(&current[..], Encoding::Current).is_ok());
        assert!(decode::<Session, _>(&current[..], Encoding::Legacy).is_err());

        let legacy = std::fs::read(data_path(LEGACY_SESSION)).unwrap();
        let payload = &legacy[TYPE_LENGTH + VERSION_LENGTH..];
        assert!(decode::<Session, _>(payload, Encoding::Legacy).is_ok());
        assert!(decode::<Session, _>(payload, Encoding::Current).is_err());
    }

    #[test]
    fn unknown_versions_are_rejected()
    {
        for (header, expected_type) in [
            (&b"PROXIDE-SESSIONv00"[..], "session"),
            (&b"PROXIDE-SESSIONv03"[..], "session"),
            (&b"PROXIDE-CAPTUREv01"[..], "capture"),
            (&b"PROXIDE-CAPTUREv04"[..], "capture"),
        ] {
            let file = TempFile::new();
            std::fs::write(&file.0, header).unwrap();
            match read_file(&file.0) {
                Err(SerializationError::UnsupportedVersion { filetype, version }) => {
                    assert_eq!(filetype, expected_type);
                    assert_eq!(version.as_bytes(), &header[TYPE_LENGTH..]);
                }
                Err(e) => panic!("Unexpected error for {:?}: {}", header, e),
                Ok(_) => panic!("Unexpected success for {:?}", header),
            }
        }
    }

    #[test]
    fn unrecognized_files_are_rejected()
    {
        for content in [&b"PROXIDE-NOTHINGv01"[..], b"short"] {
            let file = TempFile::new();
            std::fs::write(&file.0, content).unwrap();
            assert!(matches!(
                read_file(&file.0),
                Err(SerializationError::UnrecognizedFile {})
            ));
        }
    }
}
