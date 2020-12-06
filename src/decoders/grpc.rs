use clap::{App, Arg, ArgMatches};
use lazy_static::lazy_static;
use protofish::context::{Context, MessageRef};
use protofish::decode::MessageValue;
use snafu::ResultExt;
use std::io::Read;
use std::sync::{Arc, Mutex};
use tui::widgets::Text;

use super::{ConfigurationError, ConfigurationValueError, Decoder, DecoderFactory, Result};
use crate::session::{MessageData, RequestData, RequestPart};

lazy_static! {
    pub static ref CONTEXT: Mutex<Option<Arc<Context>>> = Mutex::new(None);
}

pub struct GrpcDecoderFactory
{
    ctx: Arc<Context>,
}

pub fn setup_args<'a, 'b>(app: App<'a, 'b>) -> App<'a, 'b>
{
    app.arg(
        Arg::with_name("grpc")
            .long("grpc")
            .value_name("PROTO_FILE")
            .multiple(true)
            .help("Specify .proto file for decoding Protobuf messages")
            .takes_value(true),
    )
}

pub fn initialize(matches: &ArgMatches) -> Result<Option<Box<dyn DecoderFactory>>>
{
    // Avoid initialization if the grpc arguments arent given on the command line.
    let globs = match matches.values_of("grpc") {
        Some(globs) => globs,
        None => return Ok(None),
    };

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

    let content_ref: Vec<_> = content.iter().map(|s| s.as_str()).collect();

    let context = Context::parse(&content_ref)
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send>)
        .context(ConfigurationError { option: "grpc" })?;
    let context = Arc::new(context);
    *CONTEXT.lock().unwrap() = Some(context.clone());

    Ok(Some(Box::new(GrpcDecoderFactory { ctx: context })))
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
        let service = match self.ctx.get_service(service) {
            None => return None,
            Some(s) => s,
        };
        let function = match service.rpcs.iter().find(|f| f.name == function) {
            None => return None,
            Some(f) => f,
        };

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
    ctx: Arc<Context>,
}

impl GrpcDecoder
{
    pub fn new(msg_ref: MessageRef, rc: Arc<Context>) -> Self
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

    fn decode(&self, msg: &MessageData) -> Vec<Text>
    {
        let mut output = vec![];
        if !msg.headers.is_empty() {
            output.push(Text::raw("Headers\n"));
            for (k, v) in &msg.headers {
                output.push(Text::raw(format!(" - {}: {:?}\n", k, v)));
            }
            output.push(Text::raw("\n"));
        }

        for v in &self.get_messages(&msg.content) {
            output.append(&mut v.to_text(&self.ctx, 0));
            output.push(Text::raw("\n"));
        }

        if !msg.trailers.is_empty() {
            output.push(Text::raw("\n"));
            output.push(Text::raw("\nTrailers\n"));
            for (k, v) in &msg.headers {
                output.push(Text::raw(format!(" - {}: {:?}\n", k, v)));
            }
        }
        output
    }

    fn index(&self, msg: &MessageData) -> Vec<String>
    {
        self.get_messages(&msg.content)
            .into_iter()
            .flat_map(|msg| msg.to_index(&self.ctx))
            .collect()
    }
}

trait ToText
{
    fn to_text<'a>(&self, ctx: &'a Context, indent: usize) -> Vec<Text<'a>>;

    fn to_index(&self, ctx: &Context) -> Vec<String>;
}

impl ToText for protofish::decode::MessageValue
{
    fn to_text<'a>(&self, ctx: &'a Context, mut indent: usize) -> Vec<Text<'a>>
    {
        // Panic here should indicate that msg_ref is for a different context.
        let msg = ctx.resolve_message(self.msg_ref);

        let mut v = Vec::with_capacity(2 + 5 * self.fields.len());
        v.push(Text::raw(format!("{} {{\n", msg.name)));
        indent += 1;
        for f in &self.fields {
            v.push(Text::raw("  ".repeat(indent)));
            v.push(match msg.get_field(f.number) {
                Some(f) => Text::raw(&f.name),
                None => Text::raw(format!("[#{}]", f.number)),
            });
            v.push(Text::raw(": "));
            v.append(&mut f.value.to_text(ctx, indent));
            v.push(Text::raw("\n"));
        }
        indent -= 1;
        v.push(Text::raw(format!("{}}}", "  ".repeat(indent))));
        v
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
    fn to_text<'a>(&self, ctx: &'a Context, _indent: usize) -> Vec<Text<'a>>
    {
        // Panic here should indicate that msg_ref is for a different context.
        let e = ctx.resolve_enum(self.enum_ref);

        match e.get_field_by_value(self.value) {
            Some(field) => vec![Text::raw(&field.name)],
            None => vec![Text::raw(self.value.to_string())],
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
    fn to_text<'a>(&self, ctx: &'a Context, indent: usize) -> Vec<Text<'a>>
    {
        vec![match self {
            Self::Double(v) => Text::raw(format!("{}", v)),
            Self::Float(v) => Text::raw(format!("{}", v)),
            Self::Int32(v) => Text::raw(format!("{}", v)),
            Self::Int64(v) => Text::raw(format!("{}", v)),
            Self::UInt32(v) => Text::raw(format!("{}", v)),
            Self::UInt64(v) => Text::raw(format!("{}", v)),
            Self::SInt32(v) => Text::raw(format!("{}", v)),
            Self::SInt64(v) => Text::raw(format!("{}", v)),
            Self::Fixed32(v) => Text::raw(format!("{}", v)),
            Self::Fixed64(v) => Text::raw(format!("{}", v)),
            Self::SFixed32(v) => Text::raw(format!("{}", v)),
            Self::SFixed64(v) => Text::raw(format!("{}", v)),
            Self::Bool(v) => Text::raw(format!("{}", v)),
            Self::String(v) => Text::raw(format!("{:?}", v)),
            Self::Bytes(v) => Text::raw(format!("{:?}", v)),
            Self::Packed(v) => return v.to_text(ctx, indent),

            Self::Enum(v) => return v.to_text(ctx, indent),
            Self::Message(v) => return v.to_text(ctx, indent),

            Self::Unknown(unk) => Text::raw(format!("!! {:?}", unk)),
            Self::Incomplete(vt, bytes) => Text::raw(format!("Incomplete({}, {:X})", vt, bytes)),
        }]
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
            Self::Incomplete(vt, bytes) => format!("Incomplete({}, {:X})", vt, bytes),
        }]
    }
}

impl ToText for protofish::decode::PackedArray
{
    fn to_text<'a>(&self, _ctx: &'a Context, _indent: usize) -> Vec<Text<'a>>
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
            return vec![Text::raw("[]")];
        }

        let mut output = vec![Text::raw("[ ")];
        output.push(Text::raw(v.join(", ")));
        output.push(Text::raw(" ]"));
        output
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
