use clap::{Arg, ArgAction, ArgMatches, Command};
use protofish::context::{Context, MessageRef};
use protofish::decode::MessageValue;
use snafu::ResultExt;
use std::io::Read;
use std::rc::Rc;
use tui::text::{Span, Spans, Text};

pub mod reflection;

use super::{ConfigurationError, ConfigurationValueError, Decoder, DecoderFactory, Result};
use crate::session::{MessageData, RequestData, RequestPart};

pub struct GrpcDecoderFactory
{
    ctx: Rc<Context>,
}

pub fn setup_args(app: Command) -> Command
{
    app.arg(
        Arg::new("grpc")
            .long("grpc")
            .value_name("PROTO_FILE")
            // Several values per occurrence, so an unquoted `--grpc src/*.proto` expanded by the
            // shell works, and the option can be repeated.
            .num_args(1..)
            .action(ArgAction::Append)
            .help("Specify .proto file for decoding Protobuf messages"),
    )
    .arg(
        Arg::new("grpc-reflection")
            .long("grpc-reflection")
            .value_name("SERVER")
            .help("Load Protobuf definitions from a server with gRPC server reflection")
            .long_help(
                "\
Load the Protobuf definitions for decoding gRPC messages from SERVER with gRPC server reflection.
The server must have the reflection service enabled.

SERVER is host:port, http://host[:port] or https://host[:port]. Without a scheme, Proxide tries
an unencrypted connection first and then TLS. The server certificate is not validated.

The definitions are loaded once at startup. Files given with --grpc are added to them, and are used
instead of the server's definitions when both define the same type.",
            ),
    )
}

pub fn initialize(matches: &ArgMatches) -> Result<Option<Box<dyn DecoderFactory>>>
{
    let globs: Vec<&String> = matches
        .get_many::<String>("grpc")
        .map(|g| g.collect())
        .unwrap_or_default();
    let reflection_server = matches.get_one::<String>("grpc-reflection");

    // Avoid initialization if the grpc arguments arent given on the command line.
    if globs.is_empty() && reflection_server.is_none() {
        return Ok(None);
    }

    let mut reflected = Vec::new();
    if let Some(server) = reflection_server {
        let target = reflection::Target::parse(server)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send>)
            .context(ConfigurationError {
                option: "grpc-reflection",
            })?;
        let sources = reflection::fetch_proto_sources(&target)
            .map_err(|e| {
                Box::new(reflection::ReflectionError(format!(
                    "Failed to load definitions from '{}': {}",
                    server, e
                ))) as Box<dyn std::error::Error + Send>
            })
            .context(ConfigurationError {
                option: "grpc-reflection",
            })?;
        for file in &sources {
            log::debug!("gRPC reflection definitions:\n{}", file.source);
        }
        reflected = sources;
    }

    // Read all proto files.
    let mut content = Vec::new();
    for g in globs {
        let files = glob::glob(g)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send>)
            .context(ConfigurationValueError {
                option: "grpc",
                msg: format!("Invalid pattern '{}'", g),
            })?;
        for f in files {
            let path = match f {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("{}", e);
                    continue;
                }
            };
            let mut proto_file = String::new();
            std::fs::File::open(&path)
                .and_then(|mut file| file.read_to_string(&mut proto_file))
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send>)
                .context(ConfigurationValueError {
                    option: "grpc",
                    msg: format!("Failed to read '{}'", path.to_string_lossy()),
                })?;
            content.push(proto_file);
        }
    }

    let option = match matches.get_many::<String>("grpc") {
        Some(_) => "grpc",
        None => "grpc-reflection",
    };
    let context = parse_definitions(&content, reflected)
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send>)
        .context(ConfigurationError { option })?;

    Ok(Some(Box::new(GrpcDecoderFactory {
        ctx: Rc::new(context),
    })))
}

/// Parses the `--grpc` files together with the files loaded with reflection.
///
/// If both define the same type, the `--grpc` file wins and the reflected file is left out. This
/// allows overriding definitions the server provides, and passing the same files both ways.
fn parse_definitions(
    files: &[String],
    mut reflected: Vec<reflection::ReflectedFile>,
) -> std::result::Result<Context, protofish::context::ParseError>
{
    loop {
        let sources: Vec<&str> = files
            .iter()
            .map(String::as_str)
            .chain(reflected.iter().map(|f| f.source.as_str()))
            .collect();

        let duplicate = match Context::parse(sources) {
            Err(protofish::context::ParseError::DuplicateType { name }) => name,
            result => return result,
        };

        // Only leave out reflected files. A duplicate within the --grpc files is an error.
        match reflected.iter().position(|f| f.types.contains(&duplicate)) {
            Some(idx) => {
                let file = reflected.remove(idx);
                log::warn!(
                    "gRPC reflection: {} is also defined in --grpc files, leaving out {} from reflection",
                    duplicate,
                    file.name
                );
            }
            None => return Err(protofish::context::ParseError::DuplicateType { name: duplicate }),
        }
    }
}

impl DecoderFactory for GrpcDecoderFactory
{
    fn try_create(&self, request: &RequestData, msg: &MessageData) -> Option<Box<dyn Decoder>>
    {
        log::info!("Acquiring gRPC decoder: {:?}", msg.headers);
        match msg.headers.get("content-type")?.to_str() {
            Ok("application/grpc") => {}
            _ => return None,
        }

        let mut path = request.uri.path().rsplit('/');
        let function = path.next().unwrap();
        let service = path.next().unwrap();
        let service = self.ctx.get_service(service)?;
        let function = service.rpc_by_name(function)?;

        let ty = match msg.part {
            RequestPart::Request => &function.input.message,
            RequestPart::Response => &function.output.message,
        };

        Some(Box::new(GrpcDecoder::new(*ty, self.ctx.clone())))
    }
}

pub struct GrpcDecoder
{
    msg_ref: MessageRef,
    ctx: Rc<Context>,
}

impl GrpcDecoder
{
    pub fn new(msg_ref: MessageRef, rc: Rc<Context>) -> Self
    {
        Self { msg_ref, ctx: rc }
    }

    fn get_messages(&self, b: &[u8]) -> Vec<MessageValue>
    {
        let mut cursor = 0;
        let mut values = vec![];
        while b.len() >= cursor + 5 {
            let compressed = b[cursor];
            if compressed != 0 {
                return vec![];
            }

            let len = ((b[cursor + 1] as usize) << 24)
                + ((b[cursor + 2] as usize) << 16)
                + ((b[cursor + 3] as usize) << 8)
                + b[cursor + 4] as usize;

            if b.len() < cursor + 5 + len {
                break;
            }
            cursor += 5;

            values.push(self.ctx.decode(self.msg_ref, &b[cursor..cursor + len]));
            cursor += len;
        }

        values
    }
}

impl Decoder for GrpcDecoder
{
    fn name(&self) -> &'static str
    {
        "grpc"
    }

    fn decode(&self, msg: &MessageData) -> Text<'_>
    {
        let mut builder = TextBuilder::default();
        if !msg.headers.is_empty() {
            builder.push(Span::raw("Headers\n"));
            for (k, v) in &msg.headers {
                builder.push(Span::raw(format!(" - {}: {:?}\n", k, v)));
            }
            builder.push(Span::raw("\n"));
        }

        for v in &self.get_messages(&msg.content) {
            v.to_text(&self.ctx, 0, &mut builder);
            builder.push(Span::raw("\n"));
        }

        if !msg.trailers.is_empty() {
            builder.push(Span::raw("\n"));
            builder.push(Span::raw("\nTrailers\n"));
            for (k, v) in &msg.headers {
                builder.push(Span::raw(format!(" - {}: {:?}\n", k, v)));
            }
        }
        builder.build()
    }

    fn index(&self, msg: &MessageData) -> Vec<String>
    {
        self.get_messages(&msg.content)
            .into_iter()
            .flat_map(|msg| msg.to_index(&self.ctx))
            .collect()
    }
}

#[derive(Default)]
struct TextBuilder<'a>
{
    last_line: Vec<Span<'a>>,
    lines: Vec<Spans<'a>>,
}

impl<'a> TextBuilder<'a>
{
    fn push(&mut self, span: Span<'a>)
    {
        let has_linebreak = span.content.contains('\n');
        self.last_line.push(span);
        if has_linebreak {
            let mut line = vec![];
            std::mem::swap(&mut line, &mut self.last_line);
            self.lines.push(Spans::from(line));
        }
    }

    fn build(mut self) -> Text<'a>
    {
        self.lines.push(Spans::from(self.last_line));
        Text::from(self.lines)
    }
}

trait ToText
{
    fn to_text<'a>(&self, ctx: &'a Context, indent: usize, builder: &mut TextBuilder<'a>);

    fn to_index(&self, ctx: &Context) -> Vec<String>;
}

impl ToText for protofish::decode::MessageValue
{
    fn to_text<'a>(&self, ctx: &'a Context, mut indent: usize, builder: &mut TextBuilder<'a>)
    {
        // Panic here should indicate that msg_ref is for a different context.
        let msg = ctx.resolve_message(self.msg_ref);

        builder.push(Span::raw(format!("{} {{\n", msg.name)));
        indent += 1;
        for f in &self.fields {
            builder.push(Span::raw("  ".repeat(indent)));
            builder.push(match msg.get_field(f.number) {
                Some(f) => Span::raw(&f.name),
                None => Span::raw(format!("[#{}]", f.number)),
            });
            builder.push(Span::raw(": "));
            f.value.to_text(ctx, indent, builder);
            builder.push(Span::raw("\n"));
        }
        indent -= 1;
        builder.push(Span::raw(format!("{}}}", "  ".repeat(indent))));
    }

    fn to_index(&self, ctx: &Context) -> Vec<String>
    {
        let msg = ctx.resolve_message(self.msg_ref);
        std::iter::once(msg.name.clone())
            .chain(self.fields.iter().flat_map(|field| {
                msg.get_field(field.number)
                    .map(|f| f.name.clone())
                    .into_iter()
                    .chain(field.value.to_index(ctx))
            }))
            .collect()
    }
}

impl ToText for protofish::decode::EnumValue
{
    fn to_text<'a>(&self, ctx: &'a Context, _indent: usize, builder: &mut TextBuilder<'a>)
    {
        // Panic here should indicate that msg_ref is for a different context.
        let e = ctx.resolve_enum(self.enum_ref);

        match e.get_field_by_value(self.value) {
            Some(field) => builder.push(Span::raw(&field.name)),
            None => builder.push(Span::raw(self.value.to_string())),
        }
    }

    fn to_index(&self, ctx: &Context) -> Vec<String>
    {
        let e = ctx.resolve_enum(self.enum_ref);

        match e.get_field_by_value(self.value) {
            Some(field) => vec![field.name.to_string()],
            None => vec![],
        }
    }
}

impl ToText for protofish::decode::Value
{
    fn to_text<'a>(&self, ctx: &'a Context, indent: usize, builder: &mut TextBuilder<'a>)
    {
        builder.push(match self {
            Self::Double(v) => Span::raw(format!("{}", v)),
            Self::Float(v) => Span::raw(format!("{}", v)),
            Self::Int32(v) => Span::raw(format!("{}", v)),
            Self::Int64(v) => Span::raw(format!("{}", v)),
            Self::UInt32(v) => Span::raw(format!("{}", v)),
            Self::UInt64(v) => Span::raw(format!("{}", v)),
            Self::SInt32(v) => Span::raw(format!("{}", v)),
            Self::SInt64(v) => Span::raw(format!("{}", v)),
            Self::Fixed32(v) => Span::raw(format!("{}", v)),
            Self::Fixed64(v) => Span::raw(format!("{}", v)),
            Self::SFixed32(v) => Span::raw(format!("{}", v)),
            Self::SFixed64(v) => Span::raw(format!("{}", v)),
            Self::Bool(v) => Span::raw(format!("{}", v)),
            Self::String(v) => Span::raw(format!("{:?}", v)),
            Self::Bytes(v) => Span::raw(format!("{:?}", v)),
            Self::Packed(v) => return v.to_text(ctx, indent, builder),

            Self::Enum(v) => return v.to_text(ctx, indent, builder),
            Self::Message(v) => return v.to_text(ctx, indent, builder),

            Self::Unknown(unk) => Span::raw(format!("!! {:?}", unk)),
            Self::Incomplete(_, bytes) => Span::raw(format!("Incomplete({:X})", bytes)),
        })
    }

    fn to_index(&self, ctx: &Context) -> Vec<String>
    {
        vec![match self {
            Self::Double(v) => format!("{}", v),
            Self::Float(v) => format!("{}", v),
            Self::Int32(v) => format!("{}", v),
            Self::Int64(v) => format!("{}", v),
            Self::UInt32(v) => format!("{}", v),
            Self::UInt64(v) => format!("{}", v),
            Self::SInt32(v) => format!("{}", v),
            Self::SInt64(v) => format!("{}", v),
            Self::Fixed32(v) => format!("{}", v),
            Self::Fixed64(v) => format!("{}", v),
            Self::SFixed32(v) => format!("{}", v),
            Self::SFixed64(v) => format!("{}", v),
            Self::Bool(v) => format!("{}", v),
            Self::String(v) => format!("{:?}", v),
            Self::Bytes(v) => format!("{:?}", v),
            Self::Packed(v) => return v.to_index(ctx),

            Self::Enum(v) => return v.to_index(ctx),
            Self::Message(v) => return v.to_index(ctx),

            Self::Unknown(unk) => format!("!! {:?}", unk),
            Self::Incomplete(_, bytes) => format!("Incomplete({:X})", bytes),
        }]
    }
}

impl ToText for protofish::decode::PackedArray
{
    fn to_text<'a>(&self, _ctx: &'a Context, mut _indent: usize, builder: &mut TextBuilder<'a>)
    {
        let v: Vec<_> = match self {
            Self::Double(v) => v.iter().map(ToString::to_string).collect(),
            Self::Float(v) => v.iter().map(ToString::to_string).collect(),
            Self::Int32(v) => v.iter().map(ToString::to_string).collect(),
            Self::Int64(v) => v.iter().map(ToString::to_string).collect(),
            Self::UInt32(v) => v.iter().map(ToString::to_string).collect(),
            Self::UInt64(v) => v.iter().map(ToString::to_string).collect(),
            Self::SInt32(v) => v.iter().map(ToString::to_string).collect(),
            Self::SInt64(v) => v.iter().map(ToString::to_string).collect(),
            Self::Fixed32(v) => v.iter().map(ToString::to_string).collect(),
            Self::Fixed64(v) => v.iter().map(ToString::to_string).collect(),
            Self::SFixed32(v) => v.iter().map(ToString::to_string).collect(),
            Self::SFixed64(v) => v.iter().map(ToString::to_string).collect(),
            Self::Bool(v) => v.iter().map(ToString::to_string).collect(),
        };

        if v.is_empty() {
            return builder.push(Span::raw("[]"));
        }

        builder.push(Span::raw("[ "));
        builder.push(Span::raw(v.join(", ")));
        builder.push(Span::raw(" ]"));
    }

    fn to_index(&self, _ctx: &Context) -> Vec<String>
    {
        let v: Vec<_> = match self {
            Self::Double(v) => v.iter().map(ToString::to_string).collect(),
            Self::Float(v) => v.iter().map(ToString::to_string).collect(),
            Self::Int32(v) => v.iter().map(ToString::to_string).collect(),
            Self::Int64(v) => v.iter().map(ToString::to_string).collect(),
            Self::UInt32(v) => v.iter().map(ToString::to_string).collect(),
            Self::UInt64(v) => v.iter().map(ToString::to_string).collect(),
            Self::SInt32(v) => v.iter().map(ToString::to_string).collect(),
            Self::SInt64(v) => v.iter().map(ToString::to_string).collect(),
            Self::Fixed32(v) => v.iter().map(ToString::to_string).collect(),
            Self::Fixed64(v) => v.iter().map(ToString::to_string).collect(),
            Self::SFixed32(v) => v.iter().map(ToString::to_string).collect(),
            Self::SFixed64(v) => v.iter().map(ToString::to_string).collect(),
            Self::Bool(v) => v.iter().map(ToString::to_string).collect(),
        };

        v
    }
}

#[cfg(test)]
pub(super) mod test
{
    use super::*;
    use crate::session::{RequestPart, Status};
    use chrono::Local;
    use http::{HeaderMap, HeaderValue, Method};
    use uuid::Uuid;

    // proto3 `optional` fields (protoc 3.15+) used to fail with
    // "expected emptyStatement, option, repeated, ..." (issue #25).
    const PROTO: &str = r#"
        syntax = "proto3";
        package test;

        enum Kind {
            KIND_UNKNOWN = 0;
            KIND_ADMIN = 1;
        }

        message Person {
            string name = 1;
            optional string nickname = 2;
            optional int32 age = 3;
            Kind kind = 4;
            optional Kind role = 5;
        }

        service People {
            rpc Get (Person) returns (Person);
        }
    "#;

    pub(super) fn factory_from_files(protos: &[&str]) -> Result<Option<Box<dyn DecoderFactory>>>
    {
        let dir = std::env::temp_dir().join(format!("proxide-test-{}", Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        for (i, proto) in protos.iter().enumerate() {
            std::fs::write(dir.join(format!("{}.proto", i)), proto).unwrap();
        }

        let glob = dir.join("*.proto");
        let app = setup_args(Command::new("test"));
        let matches = app.get_matches_from(["test", "--grpc", glob.to_str().unwrap()]);
        let result = initialize(&matches);

        std::fs::remove_dir_all(&dir).unwrap();
        result
    }

    fn factory_from_file(proto: &str) -> Result<Option<Box<dyn DecoderFactory>>>
    {
        factory_from_files(&[proto])
    }

    /// Wraps a protobuf payload into an uncompressed gRPC frame.
    pub(super) fn grpc_frame(payload: &[u8]) -> Vec<u8>
    {
        let mut frame = vec![0];
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(payload);
        frame
    }

    pub(super) fn request(path: &str) -> RequestData
    {
        RequestData {
            uuid: Uuid::new_v4(),
            connection_uuid: Uuid::new_v4(),
            method: Method::POST,
            uri: path.parse().unwrap(),
            start_timestamp: Local::now(),
            end_timestamp: None,
            status: Status::Succeeded,
        }
    }

    pub(super) fn message(part: RequestPart, content: &[u8]) -> MessageData
    {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", HeaderValue::from_static("application/grpc"));
        let mut msg = MessageData::new(part).with_headers(headers);
        msg.content.extend_from_slice(content);
        msg
    }

    pub(super) fn text_to_string(text: &Text) -> String
    {
        text.lines
            .iter()
            .flat_map(|line| line.0.iter().map(|span| span.content.as_ref()))
            .collect()
    }

    #[test]
    fn proto3_optional_fields_are_supported()
    {
        let factory = factory_from_file(PROTO)
            .expect("proto with optional fields should parse")
            .expect("--grpc was given, factory should exist");

        // name = "ann", nickname = "a", age = 0 (explicitly present), kind = KIND_ADMIN
        let payload = [
            0x0A, 0x03, b'a', b'n', b'n', // field 1, string
            0x12, 0x01, b'a', // field 2, string
            0x18, 0x00, // field 3, varint
            0x20, 0x01, // field 4, enum
        ];
        let req = request("/test.People/Get");

        for part in [RequestPart::Request, RequestPart::Response] {
            let msg = message(part, &grpc_frame(&payload));
            let decoder = factory
                .try_create(&req, &msg)
                .expect("gRPC decoder should be created for a known method");

            let text = text_to_string(&decoder.decode(&msg));
            for expected in [
                "Person {",
                "name: \"ann\"",
                "nickname: \"a\"",
                "age: 0",
                "kind: KIND_ADMIN",
            ] {
                assert!(text.contains(expected), "{:?} not in:\n{}", expected, text);
            }

            let index = decoder.index(&msg);
            for expected in [
                "Person",
                "name",
                "\"ann\"",
                "nickname",
                "age",
                "kind",
                "KIND_ADMIN",
            ] {
                assert!(
                    index.iter().any(|i| i == expected),
                    "{:?} not in {:?}",
                    expected,
                    index
                );
            }
        }
    }

    /// Decodes `payload` as a `test.Person` request and returns the message body as trimmed lines,
    /// from `Person {` to the closing `}`, together with the search index.
    fn decode_person(factory: &dyn DecoderFactory, payload: &[u8]) -> (Vec<String>, Vec<String>)
    {
        let msg = message(RequestPart::Request, &grpc_frame(payload));
        let decoder = factory
            .try_create(&request("/test.People/Get"), &msg)
            .unwrap();

        let text = text_to_string(&decoder.decode(&msg));
        let lines: Vec<String> = text
            .lines()
            .map(|l| l.trim().to_string())
            .skip_while(|l| l != "Person {")
            .take_while(|l| !l.is_empty())
            .collect();
        (lines, decoder.index(&msg))
    }

    #[test]
    fn proto3_optional_fields_unset_are_omitted()
    {
        let factory = factory_from_file(PROTO).unwrap().unwrap();

        /// (description, encoded fields on the wire, expected field lines in the output)
        type Case<'a> = (&'a str, &'a [&'a [u8]], &'a [&'a str]);

        // Field encodings for test.Person.
        const NAME: &[u8] = &[0x0A, 0x03, b'a', b'n', b'n']; // name = "ann"
        const NICK: &[u8] = &[0x12, 0x01, b'a']; // nickname = "a"
        const NICK_EMPTY: &[u8] = &[0x12, 0x00]; // nickname = "" (present)
        const AGE_ZERO: &[u8] = &[0x18, 0x00]; // age = 0 (present)
        const AGE: &[u8] = &[0x18, 0x2A]; // age = 42
        const ROLE_DEFAULT: &[u8] = &[0x28, 0x00]; // role = KIND_UNKNOWN (present)

        #[rustfmt::skip]
        let cases: &[Case] = &[
            ("all unset, empty message", &[], &[]),
            ("only non-optional set", &[NAME], &["name: \"ann\""]),
            ("nickname set, age unset", &[NAME, NICK], &["name: \"ann\"", "nickname: \"a\""]),
            ("age set, nickname unset", &[AGE], &["age: 42"]),
            // Explicit default values are on the wire for optional fields and must be shown,
            // unlike unset ones.
            ("explicit empty string", &[NICK_EMPTY], &["nickname: \"\""]),
            ("explicit zero", &[AGE_ZERO], &["age: 0"]),
            ("explicit default enum", &[ROLE_DEFAULT], &["role: KIND_UNKNOWN"]),
            ("all optionals present", &[NICK, AGE_ZERO, ROLE_DEFAULT],
                &["nickname: \"a\"", "age: 0", "role: KIND_UNKNOWN"]),
        ];

        for (name, fields, expected_fields) in cases {
            let payload: Vec<u8> = fields.concat();
            let (lines, index) = decode_person(factory.as_ref(), &payload);

            let mut expected = vec!["Person {".to_string()];
            expected.extend(expected_fields.iter().map(|f| f.to_string()));
            expected.push("}".to_string());
            assert_eq!(lines, expected, "case: {}", name);

            // Unset fields must not show up in the search index either.
            for field in ["nickname", "age", "role"] {
                let is_set = expected_fields
                    .iter()
                    .any(|f| f.starts_with(&format!("{}:", field)));
                assert_eq!(
                    index.iter().any(|i| i == field),
                    is_set,
                    "case: {}: index entry for {:?} in {:?}",
                    name,
                    field,
                    index
                );
            }
        }
    }

    fn reflected(name: &str, source: &str, types: &[&str]) -> reflection::ReflectedFile
    {
        reflection::ReflectedFile {
            name: name.to_string(),
            source: source.to_string(),
            types: types.iter().map(|t| t.to_string()).collect(),
        }
    }

    #[test]
    fn grpc_files_override_reflected_definitions()
    {
        let user = r#"syntax = "proto3"; package test;
            message Person { string name = 1; string email = 9; }"#;
        let server = r#"syntax = "proto3"; package test;
            message Person { string name = 1; }
            message Group { repeated Person members = 1; }"#;
        let other = r#"syntax = "proto3"; package other;
            message Thing { test.Group group = 1; }"#;

        let ctx = parse_definitions(
            &[user.to_string()],
            vec![
                reflected("test.proto", server, &["test.Person", "test.Group"]),
                reflected("other.proto", other, &["other.Thing"]),
            ],
        );

        // The whole reflected test.proto is replaced by the --grpc file, so test.Group is gone
        // and other.Thing can't resolve it.
        let err = ctx.expect_err("should fail").to_string();
        assert!(err.contains("test.Group"), "{}", err);

        // Without the dependency, the --grpc version of test.Person is used.
        let ctx = parse_definitions(
            &[user.to_string()],
            vec![reflected(
                "test.proto",
                server,
                &["test.Person", "test.Group"],
            )],
        )
        .unwrap();
        let person = ctx.get_message("test.Person").unwrap();
        assert!(person.get_field_by_name("email").is_some());
        assert!(ctx.get_message("test.Group").is_none());
    }

    #[test]
    fn duplicate_grpc_files_are_an_error()
    {
        let user = r#"syntax = "proto3"; package test; message Person { string name = 1; }"#;
        let err = parse_definitions(&[user.to_string(), user.to_string()], vec![])
            .expect_err("should fail");
        assert_eq!(err.to_string(), "Duplicate type: test.Person");
    }

    #[test]
    fn unknown_method_has_no_decoder()
    {
        let factory = factory_from_file(PROTO).unwrap().unwrap();
        let msg = message(RequestPart::Request, &grpc_frame(&[]));
        assert!(factory
            .try_create(&request("/test.People/Missing"), &msg)
            .is_none());
        assert!(factory
            .try_create(&request("/test.Missing/Get"), &msg)
            .is_none());
    }

    #[test]
    fn invalid_proto_is_a_configuration_error()
    {
        let err = factory_from_file("message {").err().expect("should fail");
        assert!(err.to_string().contains("grpc"), "{}", err);
    }

    #[test]
    fn decodes_recorded_grpc_traffic()
    {
        // Minimal stand-in for google/protobuf/duration.proto, which rust_grpc.proto imports.
        const DURATION: &str = r#"
            syntax = "proto3";
            package google.protobuf;
            message Duration {
                int64 seconds = 1;
                int32 nanos = 2;
            }
        "#;
        let rust_grpc = std::fs::read_to_string("test/rust_grpc/proto/rust_grpc.proto").unwrap();
        let factory = factory_from_files(&[&rust_grpc, DURATION])
            .unwrap()
            .unwrap();

        // Recorded with proxide 0.3.0 against the test/rust_grpc server.
        let session =
            crate::session::serialization::read_file(&"test/data/capture-v02-proxide-0.3.0.bin")
                .unwrap();

        let call = session
            .requests
            .items
            .iter()
            .find(|r| r.request_data.uri.path() == "/rust_grpc.TestService/GetDiagnostics")
            .expect("capture should contain a GetDiagnostics call");
        let (req, response) = (&call.request_data, &call.response_msg);

        let decoder = factory.try_create(req, response).unwrap();
        let text = text_to_string(&decoder.decode(response));
        for expected in [
            // send_message_calls and clients were zero/empty in this recording, and proto3 omits
            // default values from the wire.
            "DiagnosticsResponse {",
            "uptime: Duration {",
            "seconds: 2",
            "nanos: ",
        ] {
            assert!(text.contains(expected), "{:?} not in:\n{}", expected, text);
        }
        assert!(!text.contains("Incomplete"), "{}", text);
        assert!(!text.contains("[#"), "unknown fields in:\n{}", text);
    }
}
