use crate::proto::{MessageRef, ParamType, Protobuf, ValueType};
use bytes::{Bytes, BytesMut};
use http::Uri;
use std::convert::{TryFrom, TryInto};
use std::rc::Rc;
use tui::widgets::Text;

pub trait Decoder
{
    fn extend(&mut self, b: Bytes);
    fn as_text(&self) -> Vec<Text>;
}

pub struct RawDecoder(BytesMut);

impl RawDecoder
{
    pub fn decoders() -> (Box<dyn Decoder>, Box<dyn Decoder>)
    {
        (
            Box::new(RawDecoder(Default::default())),
            Box::new(RawDecoder(Default::default())),
        )
    }
}

impl Decoder for RawDecoder
{
    fn extend(&mut self, b: Bytes)
    {
        self.0.extend(b)
    }

    fn as_text(&self) -> Vec<Text>
    {
        vec![Text::raw(format!("{:?}", self.0))]
    }
}

pub struct GrpcDecoder
{
    pub raw: BytesMut,
    pub values: Vec<ProtobufMessage>,
    pub cursor: usize,
    pub msg_ref: MessageRef,
    pub pb: Rc<Protobuf>,
}

impl GrpcDecoder
{
    pub fn new(msg_ref: MessageRef, rc: Rc<Protobuf>) -> Self
    {
        Self {
            raw: Default::default(),
            values: vec![],
            cursor: 0,
            msg_ref,
            pb: rc,
        }
    }

    pub fn decoders(uri: &Uri, rc: Rc<Protobuf>) -> (Box<dyn Decoder>, Box<dyn Decoder>)
    {
        let mut path = uri.path().rsplit('/');
        let function = path.next().unwrap();
        let service = path.next().unwrap();
        let service = match rc.get_service(service) {
            None => return RawDecoder::decoders(),
            Some(s) => s,
        };
        let function = match service.rpcs.iter().find(|f| f.name == function) {
            None => return RawDecoder::decoders(),
            Some(f) => f,
        };

        let req = match &function.param.param_type {
            ParamType::Unknown(_) => panic!("Unknown request"),
            ParamType::Message(msg_ref) => msg_ref,
        };
        let resp = match &function.retval.param_type {
            ParamType::Unknown(_) => panic!("Unknown response"),
            ParamType::Message(msg_ref) => msg_ref,
        };

        (
            Box::new(GrpcDecoder::new(*req, rc.clone())),
            Box::new(GrpcDecoder::new(*resp, rc)),
        )
    }
}
impl Decoder for GrpcDecoder
{
    fn extend(&mut self, b: Bytes)
    {
        self.raw.extend(b);

        if self.raw.len() >= self.cursor + 5 {
            let compressed = self.raw[self.cursor];
            if compressed != 0 {
                panic!("Compressed messages not supported");
            }

            let len = ((self.raw[self.cursor + 1] as usize) << 24)
                + ((self.raw[self.cursor + 2] as usize) << 16)
                + ((self.raw[self.cursor + 3] as usize) << 8)
                + self.raw[self.cursor + 4] as usize;

            if self.raw.len() < self.cursor + 5 + len {
                return;
            }

            let slice = &self.raw[self.cursor + 5..self.cursor + 5 + len];
            self.values
                .push(ProtobufMessage::from(slice, self.msg_ref, &self.pb));
        }
    }

    fn as_text(&self) -> Vec<Text>
    {
        let type_name = &self.pb.resolve_message(self.msg_ref).name;
        let mut output = vec![];
        for v in &self.values {
            v.describe(0, &mut output, self.pb.as_ref());
        }
        output
    }
}

#[derive(Debug)]
pub enum ProtobufValue
{
    Double(f64),
    Float(f32),
    Int32(i32),
    Int64(i64),
    UInt32(u32),
    UInt64(u64),
    SInt32(i32),
    SInt64(i64),
    Fixed32(u32),
    Fixed64(u64),
    SFixed32(i32),
    SFixed64(i64),
    Bool(bool),
    String(String),
    Bytes(Bytes),
    Message(Box<ProtobufMessage>),
    Enum(usize),
    Invalid(ValueType, Bytes),

    UnknownVarint(u128),
    Unknown64(u64),
    UnknownLengthDelimited(Bytes),
    Unknown32(u32),
}

impl ProtobufValue
{
    pub fn parse(
        data: &mut &[u8],
        vt: &ValueType,
        pb: &Protobuf,
    ) -> Result<ProtobufValue, ProtobufValue>
    {
        Self::parse_maybe(data, vt, pb)
            .ok_or_else(|| ProtobufValue::Invalid(vt.clone(), Bytes::copy_from_slice(data)))
    }

    fn parse_maybe(data: &mut &[u8], vt: &ValueType, pb: &Protobuf) -> Option<ProtobufValue>
    {
        match vt {
            ValueType::Double => {
                into_8_bytes(data).map(|b| ProtobufValue::Double(f64::from_le_bytes(b)))
            }
            ValueType::Float => {
                into_4_bytes(data).map(|b| ProtobufValue::Float(f32::from_le_bytes(b)))
            }
            ValueType::Int32 => i32::from_signed_varint(data).map(ProtobufValue::Int32),
            ValueType::Int64 => i64::from_signed_varint(data).map(ProtobufValue::Int64),
            ValueType::UInt32 => u32::from_unsigned_varint(data).map(ProtobufValue::UInt32),
            ValueType::UInt64 => u64::from_unsigned_varint(data).map(ProtobufValue::UInt64),
            ValueType::SInt32 => u32::from_unsigned_varint(data).map(|u| {
                let sign = if u % 2 == 0 { 1i32 } else { -1i32 };
                let magnitude = (u / 2) as i32;
                ProtobufValue::SInt32(sign * magnitude)
            }),
            ValueType::SInt64 => u64::from_unsigned_varint(data).map(|u| {
                let sign = if u % 2 == 0 { 1i64 } else { -1i64 };
                let magnitude = (u / 2) as i64;
                ProtobufValue::SInt64(sign * magnitude)
            }),
            ValueType::Fixed32 => {
                into_4_bytes(data).map(|b| ProtobufValue::Fixed32(u32::from_le_bytes(b)))
            }
            ValueType::Fixed64 => {
                into_8_bytes(data).map(|b| ProtobufValue::Fixed64(u64::from_le_bytes(b)))
            }
            ValueType::SFixed32 => {
                into_4_bytes(data).map(|b| ProtobufValue::SFixed32(i32::from_le_bytes(b)))
            }
            ValueType::SFixed64 => {
                into_8_bytes(data).map(|b| ProtobufValue::SFixed64(i64::from_le_bytes(b)))
            }
            ValueType::Bool => {
                usize::from_unsigned_varint(data).map(|u| ProtobufValue::Bool(u != 0))
            }
            ValueType::Enum(_) => usize::from_unsigned_varint(data).map(ProtobufValue::Enum),
            ValueType::String => read_string(data).map(ProtobufValue::String),
            ValueType::Bytes => read_bytes(data).map(ProtobufValue::Bytes),
            ValueType::Message(mref) => {
                let length = usize::from_unsigned_varint(data)?;
                let (consumed, remainder) = data.split_at(length);
                *data = remainder;
                Some(ProtobufValue::Message(Box::new(ProtobufMessage::from(
                    consumed, *mref, pb,
                ))))
            }
            _ => Self::parse_unknown(data, vt.tag()),
        }
    }

    pub fn parse_unknown(data: &mut &[u8], vt: u8) -> Option<ProtobufValue>
    {
        Some(match vt {
            0 => ProtobufValue::UnknownVarint(u128::from_unsigned_varint(data)?),
            1 => ProtobufValue::Unknown64(u64::from_le_bytes(into_8_bytes(data)?)),
            2 => {
                let length = usize::from_unsigned_varint(data)?;
                if length > data.len() {
                    return None;
                }
                let (consumed, remainder) = data.split_at(length);
                *data = remainder;
                ProtobufValue::UnknownLengthDelimited(Bytes::copy_from_slice(consumed))
            }
            5 => ProtobufValue::Unknown32(u32::from_le_bytes(into_4_bytes(data)?)),
            _ => return None,
        })
    }

    pub fn describe(&self, indent: usize, output: &mut Vec<Text>, pb: &Protobuf)
    {
        output.push(match self {
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
            Self::Invalid(vt, v) => Text::raw(format!("!! {:?} -> {:?}", vt, v)),
            Self::Enum(v) => Text::raw(format!("{}", v)),

            Self::UnknownVarint(v) => Text::raw(format!("! {}", v)),
            Self::Unknown64(v) => Text::raw(format!("! {}", v)),
            Self::UnknownLengthDelimited(v) => Text::raw(format!("! {:?}", v)),
            Self::Unknown32(v) => Text::raw(format!("! {}", v)),

            Self::Message(v) => {
                return v.describe(indent, output, pb);
            }
        })
    }
}

fn into_8_bytes(data: &mut &[u8]) -> Option<[u8; 8]>
{
    match (*data).try_into() {
        Ok(v) => {
            *data = &data[8..];
            Some(v)
        }
        Err(_) => None,
    }
}

fn into_4_bytes(data: &mut &[u8]) -> Option<[u8; 4]>
{
    match (*data).try_into() {
        Ok(v) => {
            *data = &data[4..];
            Some(v)
        }
        Err(_) => None,
    }
}

fn read_string(data: &mut &[u8]) -> Option<String>
{
    let original = *data;
    let len = usize::from_unsigned_varint(data)?;
    if len > data.len() {
        *data = original;
        return None;
    }
    let (str_data, remainder) = data.split_at(len);
    *data = remainder;
    Some(String::from_utf8_lossy(str_data).to_string())
}

fn read_bytes(data: &mut &[u8]) -> Option<Bytes>
{
    let original = *data;
    let len = usize::from_unsigned_varint(data)?;
    if len > data.len() {
        *data = original;
        return None;
    }
    let (str_data, remainder) = data.split_at(len);
    *data = remainder;
    Some(Bytes::copy_from_slice(str_data))
}

#[derive(Debug)]
pub struct ProtobufMessage
{
    msg_ref: MessageRef,
    fields: Vec<ProtobufMessageField>,
    garbage: Option<bytes::Bytes>,
}

impl ProtobufMessage
{
    fn from(mut data: &[u8], msg_ref: MessageRef, pb: &Protobuf) -> ProtobufMessage
    {
        let msg_desc = pb.resolve_message(msg_ref);
        let mut msg = ProtobufMessage {
            msg_ref,
            fields: vec![],
            garbage: None,
        };

        loop {
            let l = data.len();
            if data.len() == 0 {
                break;
            }

            let tag = match u64::from_unsigned_varint(&mut data) {
                Some(tag) => tag,
                None => {
                    msg.garbage = Some(Bytes::copy_from_slice(data));
                    break;
                }
            };

            let field_id = tag >> 3;
            let field_type = (tag & 0x07) as u8;

            let value = match msg_desc.get_field(field_id) {
                Some(field) => {
                    ProtobufValue::parse(&mut data, &field.field_type, pb).unwrap_or_else(|e| e)
                }
                None => match ProtobufValue::parse_unknown(&mut data, field_type) {
                    Some(v) => v,
                    None => {
                        let invalid = ProtobufValue::Invalid(
                            ValueType::Unknown(format!("f:{},{}", field_type, l)),
                            Bytes::copy_from_slice(data),
                        );
                        data = &[];
                        invalid
                    }
                },
            };

            msg.fields.push(ProtobufMessageField {
                number: field_id,
                value: value,
            })
        }

        msg
    }

    pub fn describe(&self, indent: usize, output: &mut Vec<Text>, pb: &Protobuf)
    {
        let message = &pb.resolve_message(self.msg_ref);
        output.push(Text::raw(format!("{} {{\n", message.name)));
        {
            let indent = indent + 1;
            for f in &self.fields {
                let field_name = match message.get_field(f.number) {
                    Some(f) => f.name.to_string(),
                    None => format!("[#{}]", f.number),
                };
                output.push(Text::raw(format!(
                    "{}{}: ",
                    "  ".repeat(indent),
                    field_name
                )));
                f.value.describe(indent, output, pb);
                output.push(Text::raw("\n"));
            }
        }
        output.push(Text::raw(format!("{}}}", "  ".repeat(indent))));
    }
}

#[derive(Debug)]
pub struct ProtobufMessageField
{
    number: u64,
    value: ProtobufValue,
}

trait FromUnsignedVarint: Sized
{
    fn from_unsigned_varint(data: &mut &[u8]) -> Option<Self>;
}

impl<T: Default + TryFrom<u64>> FromUnsignedVarint for T
{
    fn from_unsigned_varint(data: &mut &[u8]) -> Option<Self>
    {
        let mut result = 0u64;
        let mut idx = 0;
        loop {
            if idx >= data.len() {
                return None;
            }

            let b = data[idx];
            let value = (b & 0x7f) as u64;
            result += value << (idx * 7);

            idx += 1;
            if b & 0x80 == 0 {
                break;
            }
        }
        let result = T::try_from(result).ok()?;

        *data = &data[idx..];
        Some(result)
    }
}

trait FromSignedVarint: Sized
{
    fn from_signed_varint(data: &mut &[u8]) -> Option<Self>;
}

impl<T: Default + TryFrom<i64>> FromSignedVarint for T
{
    fn from_signed_varint(data: &mut &[u8]) -> Option<Self>
    {
        let mut result = 0i64;
        let mut idx = 0;
        loop {
            if idx >= data.len() {
                return None;
            }

            let b = data[idx];
            let value = (b & 0x7f) as i64;
            result += value << (idx * 7);

            idx += 1;
            if b & 0x80 == 0 {
                break;
            }
        }
        let result = T::try_from(result).ok()?;

        *data = &data[idx..];
        Some(result)
    }
}

/*
fn read_varint64(data: &mut &[u8]) -> Option<u64>
{
    let mut result = 0u64;
    let mut idx = 0;
    loop {
        let b = data[idx];
        result += ((b & 0x7f) as u64) << (idx * 7);

        idx += 1;
        if b & 0x80 == 0 {
            break;
        } else if idx >= data.len() {
            // End of data before the varint ended.
            return None;
        }
    }

    *data = &data[idx..];
    Some(result)
}

fn read_varint128(data: &mut &[u8]) -> Option<u128>
{
    let mut result = 0u128;
    let mut idx = 0;
    loop {
        let b = data[idx];
        result += ((b & 0x7f) as u128) << (idx * 7);

        idx += 1;
        if b & 0x80 == 0 {
            break;
        } else if idx >= data.len() {
            // End of data before the varint ended.
            return None;
        }
    }

    *data = &data[idx..];
    Some(result)
}
*/
