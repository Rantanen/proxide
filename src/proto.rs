use pest::{
    iterators::{Pair, Pairs},
    Parser,
};
use snafu::{ResultExt, Snafu};
use std::collections::HashMap;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error
{
    #[snafu(display("Parsing error: {}", source))]
    ParseError
    {
        source: pest::error::Error<Rule>
    },
}
pub type Result<S, E = Error> = std::result::Result<S, E>;

#[derive(Debug)]
pub struct Protobuf
{
    pub package: Option<String>,
    pub types: Vec<ProtobufType>,
    pub types_by_name: HashMap<String, usize>,
    pub services: Vec<Service>,
}

impl Protobuf
{
    fn add_type(&mut self, t: ProtobufType)
    {
        let full_name = t.full_name();
        self.types_by_name.insert(full_name, self.types.len());
        self.types.push(t);
    }

    fn resolve_type(&self, tr: TypeRef) -> &ProtobufType
    {
        &self.types[tr.idx]
    }

    pub fn resolve_message(&self, tr: MessageRef) -> &Message
    {
        match self.resolve_type(tr.0) {
            ProtobufType::Message(msg) => msg,
            _ => unreachable!("MessageRef did not refer to a Message"),
        }
    }

    pub fn resolve_enum(&self, tr: EnumRef) -> &Enum
    {
        match self.resolve_type(tr.0) {
            ProtobufType::Enum(e) => e,
            _ => unreachable!("EnumRef did not refer to an Enum"),
        }
    }

    pub fn get_service(&self, path: &str) -> Option<&Service>
    {
        let mut service_name = path;
        if let Some(pkg) = &self.package {
            if !service_name.starts_with(pkg) {
                return None;
            }
            service_name = &service_name[pkg.len()..];
            if !service_name.starts_with(".") {
                return None;
            }
            service_name = &service_name[1..];
        }

        self.services.iter().find(|s| s.name == service_name)
    }
}

#[derive(Debug)]
pub enum ProtobufType
{
    Message(Message),
    Enum(Enum),
}

impl ProtobufType
{
    pub fn name(&self) -> &str
    {
        match self {
            ProtobufType::Message(t) => &t.name,
            ProtobufType::Enum(t) => &t.name,
        }
    }

    pub fn full_name(&self) -> String
    {
        // The full name requires path and name from the underlying type.
        let (path, mut name) = match self {
            ProtobufType::Message(v) => (&v.path, &v.name),
            ProtobufType::Enum(v) => (&v.path, &v.name),
        };

        // If the path is empty the full name is nothing but the name. Otherwise conmbine the path
        // with the name.
        match path.is_empty() {
            true => name.to_string(),
            false => format!("{}.{}", path, name),
        }
    }
}

#[derive(pest_derive::Parser)]
#[grammar = "proto.pest"]
struct ProtoParser;

pub fn parse(s: &str) -> Result<Protobuf>
{
    let pairs = ProtoParser::parse(Rule::proto, s).context(ParseError {})?;

    let mut pb = Protobuf {
        types: vec![],
        types_by_name: HashMap::new(),
        package: None,
        services: vec![],
    };
    for pair in pairs {
        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::syntax => {}
                Rule::topLevelDef => parse_top_level_def(inner, &mut pb)?,
                Rule::import => {}
                Rule::package => {
                    pb.package = Some(inner.into_inner().next().unwrap().as_str().to_string())
                }
                Rule::EOI => {}
                r => unreachable!("{:?}: {:?}", r, inner),
            }
        }
    }

    let mut type_assignments: Vec<(usize, usize, ValueType)> = Vec::new();
    for (type_idx, t) in pb.types.iter().enumerate() {
        match t {
            ProtobufType::Message(msg) => {
                for (field_idx, f) in msg.fields.iter().enumerate() {
                    let unk = match &f.field_type {
                        ValueType::Unknown(s) => s,
                        _ => continue,
                    };
                    if let Some(ty) = find_type_ref(&pb.types_by_name, Some(&msg), &unk)
                        .and_then(|tr| find_type(&pb.types, tr))
                    {
                        type_assignments.push((type_idx, field_idx, ty));
                    } else {
                        log::error!("Could not resolve {}", unk);
                    }
                }
            }
            ProtobufType::Enum(..) => {} // Ignore enums.
        }
    }
    for (type_idx, field_idx, ty) in type_assignments {
        let t = match &mut pb.types[type_idx] {
            ProtobufType::Enum(..) => unreachable!("We only processed Messages above"),
            ProtobufType::Message(msg) => msg,
        };

        t.fields[field_idx].field_type = ty;
    }

    for s in &mut pb.services {
        for r in &mut s.rpcs {
            match &r.param.param_type {
                ParamType::Unknown(s) => {
                    if let Some(tr) = find_type_ref(&pb.types_by_name, None, s.as_str()) {
                        r.param.param_type = ParamType::Message(MessageRef(tr));
                    }
                }
                _ => {}
            }
            match &r.retval.param_type {
                ParamType::Unknown(s) => {
                    if let Some(tr) = find_type_ref(&pb.types_by_name, None, s.as_str()) {
                        r.retval.param_type = ParamType::Message(MessageRef(tr));
                    }
                }
                _ => {}
            }
        }
    }

    Ok(pb)
}

fn find_type(types: &Vec<ProtobufType>, tr: TypeRef) -> Option<ValueType>
{
    Some(match types[tr.idx] {
        ProtobufType::Message(..) => ValueType::Message(MessageRef(tr)),
        ProtobufType::Enum(..) => ValueType::Enum(EnumRef(tr)),
    })
}

fn find_type_ref(
    types_by_name: &HashMap<String, usize>,
    msg: Option<&Message>,
    type_name: &str,
) -> Option<TypeRef>
{
    // Check for absolute type name.
    if type_name.starts_with(".") {
        let type_name = &type_name[1..];
        return types_by_name
            .get(type_name)
            .map(|idx| TypeRef { idx: *idx });
    }

    // Check if inner items exist that match the path.
    let mut path: Vec<_> = match msg {
        Some(msg) => {
            let mut parent_path: Vec<_> = if msg.path.is_empty() {
                vec![]
            } else {
                msg.path.split(".").collect()
            };
            parent_path.push(&msg.name);
            parent_path
        }
        None => vec![],
    };

    loop {
        let type_path = if path.is_empty() {
            type_name.to_string()
        } else {
            format!("{}.{}", path.join("."), type_name)
        };

        if let Some(&idx) = types_by_name.get(&type_path) {
            return Some(TypeRef { idx });
        }

        if path.is_empty() {
            return None;
        }
        path.pop();
    }
}

pub fn parse_top_level_def(p: Pair<Rule>, pb: &mut Protobuf) -> Result<()>
{
    let pair = p.into_inner().next().unwrap();
    match pair.as_rule() {
        Rule::message => parse_message(pair, &vec![], pb)?,
        Rule::enum_ => parse_enum(pair, &vec![], pb)?,
        Rule::service => parse_service(pair, pb)?,
        r => unreachable!("{:?}: {:?}", r, pair),
    };

    Ok(())
}

pub fn parse_message(p: Pair<Rule>, path: &[&str], pb: &mut Protobuf) -> Result<usize>
{
    let mut inner = p.into_inner();
    let name = inner.next().unwrap();
    let body = inner.next().unwrap();

    let name = name.as_str().to_string();
    let mut child_path: Vec<_> = path.iter().copied().collect();
    child_path.push(&name);
    let (fields, oneof, inner_types) = parse_message_body(body.into_inner(), &child_path, pb)?;

    pb.add_type(ProtobufType::Message(Message {
        path: path.iter().copied().collect::<Vec<_>>().join("."),
        name,
        fields,
        oneof,
        inner_types,
    }));
    Ok(pb.types.len() - 1)
}

pub fn parse_message_body(
    pairs: Pairs<Rule>,
    path: &[&str],
    pb: &mut Protobuf,
) -> Result<(Vec<MessageField>, Vec<Oneof>, Vec<usize>)>
{
    let mut f = vec![];
    let mut oneof = vec![];
    let mut i = vec![];
    for p in pairs {
        match p.as_rule() {
            Rule::field => f.push(parse_message_field(p)?),
            Rule::oneof => oneof.push(parse_message_oneof(p, oneof.len(), &mut f)?),
            Rule::message => i.push(parse_message(p, path, pb)?),
            Rule::enum_ => i.push(parse_enum(p, path, pb)?),
            r => unreachable!("{:?}: {:?}", r, p),
        }
    }
    Ok((f, oneof, i))
}

pub fn parse_message_field(p: Pair<Rule>) -> Result<MessageField>
{
    let mut repeated = false;
    let mut type_ = "";
    let mut name = String::new();
    let mut number = 0;
    let mut options = Vec::new();
    for p in p.into_inner() {
        match p.as_rule() {
            Rule::repeated => repeated = true,
            Rule::type_ => type_ = p.as_str(),
            Rule::fieldName => name = p.as_str().to_string(),
            Rule::fieldNumber => number = parse_uint_literal(p)?,
            Rule::fieldOptions => options = parse_options(p)?,
            r => unreachable!("{:?}: {:?}", r, p),
        }
    }
    let field_type = parse_field_type(type_);
    Ok(MessageField {
        repeated,
        field_type,
        name,
        number,
        options,
        oneof: None,
    })
}

pub fn parse_message_oneof(
    p: Pair<Rule>,
    oneof_idx: usize,
    fields: &mut Vec<MessageField>,
) -> Result<Oneof>
{
    let mut inner = p.into_inner();
    let name = inner.next().unwrap().as_str().to_string();
    let mut options = Vec::new();
    let mut field_idx = vec![];
    for p in inner {
        match p.as_rule() {
            Rule::option => options.push(parse_option(p)?),
            Rule::oneofField => {
                field_idx.push(fields.len());
                fields.push(parse_oneof_field(p, oneof_idx)?);
            }
            Rule::emptyStatement => {}
            r => unreachable!("{:?}: {:?}", r, p),
        }
    }
    Ok(Oneof {
        name,
        fields: field_idx,
        options,
    })
}

pub fn parse_oneof_field(p: Pair<Rule>, oneof_idx: usize) -> Result<MessageField>
{
    let mut inner = p.into_inner();
    let field_type = parse_field_type(inner.next().unwrap().as_str());
    let name = inner.next().unwrap().as_str().to_string();
    let number = parse_uint_literal(inner.next().unwrap())?;
    let options = match inner.next() {
        Some(opt) => parse_options(opt)?,
        None => vec![],
    };

    Ok(MessageField {
        repeated: false,
        field_type,
        name,
        number,
        options,
        oneof: Some(oneof_idx),
    })
}

pub fn parse_enum(p: Pair<Rule>, path: &[&str], pb: &mut Protobuf) -> Result<usize>
{
    let mut inner = p.into_inner();
    let name = inner.next().unwrap();
    let body = inner.next().unwrap();

    let name = name.as_str().to_string();

    let fields = parse_enum_body(body.into_inner())?;
    pb.add_type(ProtobufType::Enum(Enum {
        name,
        path: path.iter().copied().collect::<Vec<_>>().join("."),
        fields,
    }));
    Ok(pb.types.len() - 1)
}

pub fn parse_enum_body(pairs: Pairs<Rule>) -> Result<Vec<EnumField>>
{
    let mut v = vec![];
    for p in pairs {
        match p.as_rule() {
            Rule::enumField => v.push(parse_enum_field(p)?),
            r => unreachable!("{:?}: {:?}", r, p),
        }
    }
    Ok(v)
}

pub fn parse_enum_field(p: Pair<Rule>) -> Result<EnumField>
{
    let mut inner = p.into_inner();
    let name = inner.next().unwrap();
    let value = inner.next().unwrap();

    Ok(EnumField {
        name: name.as_str().to_string(),
        value: value.as_str().to_string(),
    })
}

pub fn parse_field_type(t: &str) -> ValueType
{
    match t {
        "double" => ValueType::Double,
        "float" => ValueType::Float,
        "int32" => ValueType::Int32,
        "int64" => ValueType::Int64,
        "uint32" => ValueType::UInt32,
        "uint64" => ValueType::UInt64,
        "sint32" => ValueType::SInt32,
        "sint64" => ValueType::SInt64,
        "fixed32" => ValueType::Fixed32,
        "fixed64" => ValueType::Fixed64,
        "sfixed32" => ValueType::SFixed32,
        "sfixed64" => ValueType::SFixed64,
        "bool" => ValueType::Bool,
        "string" => ValueType::String,
        "bytes" => ValueType::Bytes,
        _ => ValueType::Unknown(t.to_string()),
    }
}

pub fn parse_service(p: Pair<Rule>, pb: &mut Protobuf) -> Result<usize>
{
    let mut inner = p.into_inner();
    let name = inner.next().unwrap();
    let mut rpc = vec![];
    let mut options = vec![];
    for p in inner {
        match p.as_rule() {
            Rule::option => options.push(parse_option(p)?),
            Rule::rpc => rpc.push(parse_rpc(p)?),
            Rule::emptyStatement => {}
            r => unreachable!("{:?}: {:?}", r, p),
        }
    }

    pb.services.push(Service {
        name: name.as_str().to_string(),
        rpcs: rpc,
        options: options,
    });
    Ok(pb.services.len() - 1)
}

pub fn parse_rpc(p: Pair<Rule>) -> Result<Rpc>
{
    let mut inner = p.into_inner();
    let name = inner.next().unwrap();

    let param = inner.next().unwrap();
    let param = if param.as_rule() == Rule::stream {
        RpcParam {
            stream: true,
            param_type: ParamType::Unknown(inner.next().unwrap().as_str().to_string()),
        }
    } else {
        RpcParam {
            stream: false,
            param_type: ParamType::Unknown(param.as_str().to_string()),
        }
    };
    let retval = inner.next().unwrap();
    let retval = if retval.as_rule() == Rule::stream {
        RpcParam {
            stream: true,
            param_type: ParamType::Unknown(inner.next().unwrap().as_str().to_string()),
        }
    } else {
        RpcParam {
            stream: false,
            param_type: ParamType::Unknown(retval.as_str().to_string()),
        }
    };

    let mut options = vec![];
    for p in inner {
        match p.as_rule() {
            Rule::option => options.push(parse_option(p)?),
            Rule::emptyStatement => {}
            r => unreachable!("{:?}: {:?}", r, p),
        }
    }

    Ok(Rpc {
        name: name.as_str().to_string(),
        param,
        retval,
        options,
    })
}

pub fn parse_uint_literal(p: Pair<Rule>) -> Result<u64>
{
    match p.as_rule() {
        Rule::fieldNumber => parse_uint_literal(p.into_inner().next().unwrap()),
        Rule::intLit => {
            let mut inner = p.into_inner();
            let lit = inner.next().unwrap();
            Ok(match lit.as_rule() {
                Rule::decimalLit => u64::from_str_radix(lit.as_str(), 10).unwrap(),
                Rule::octalLit => u64::from_str_radix(&lit.as_str()[1..], 8).unwrap(),
                Rule::hexLit => u64::from_str_radix(&lit.as_str()[2..], 16).unwrap(),
                r => unreachable!("{:?}: {:?}", r, lit),
            })
        }
        r => unreachable!("{:?}: {:?}", r, p),
    }
}

pub fn parse_int_literal(p: Pair<Rule>) -> Result<i64>
{
    match p.as_rule() {
        Rule::intLit => {
            let mut inner = p.into_inner();
            let sign = inner.next().unwrap();
            let (sign, lit) = match sign.as_rule() {
                Rule::sign if sign.as_str() == "-" => (-1, inner.next().unwrap()),
                Rule::sign if sign.as_str() == "+" => (1, inner.next().unwrap()),
                _ => (1, sign),
            };
            Ok(match lit.as_rule() {
                Rule::decimalLit => sign * i64::from_str_radix(lit.as_str(), 10).unwrap(),
                Rule::octalLit => sign * i64::from_str_radix(&lit.as_str()[1..], 8).unwrap(),
                Rule::hexLit => sign * i64::from_str_radix(&lit.as_str()[2..], 16).unwrap(),
                r => unreachable!("{:?}: {:?}", r, lit),
            })
        }
        r => unreachable!("{:?}: {:?}", r, p),
    }
}

pub fn parse_options(p: Pair<Rule>) -> Result<Vec<ProtoOption>>
{
    Ok(vec![])
}

pub fn parse_option(p: Pair<Rule>) -> Result<ProtoOption>
{
    Ok(ProtoOption {})
}

#[derive(Debug)]
pub struct Message
{
    pub name: String,
    pub path: String,
    pub fields: Vec<MessageField>,
    pub oneof: Vec<Oneof>,
    pub inner_types: Vec<usize>,
}

impl Message
{
    pub fn get_field(&self, number: u64) -> Option<&MessageField>
    {
        self.fields.iter().find(|f| f.number == number)
    }
}

#[derive(Debug)]
pub struct MessageField
{
    pub repeated: bool,
    pub field_type: ValueType,
    pub name: String,
    pub number: u64,
    pub options: Vec<ProtoOption>,
    pub oneof: Option<usize>,
}

#[derive(Debug)]
pub struct Oneof
{
    pub name: String,
    pub fields: Vec<usize>,
    pub options: Vec<ProtoOption>,
}

#[derive(Debug)]
pub struct Enum
{
    pub name: String,
    pub path: String,
    pub fields: Vec<EnumField>,
}

#[derive(Debug)]
pub struct EnumField
{
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug)]
pub enum ValueType
{
    Double,
    Float,
    Int32,
    Int64,
    UInt32,
    UInt64,
    SInt32,
    SInt64,
    Fixed32,
    Fixed64,
    SFixed32,
    SFixed64,
    Bool,
    String,
    Bytes,
    Message(MessageRef),
    Enum(EnumRef),
    Unknown(String),
}

impl ValueType
{
    pub fn tag(&self) -> u8
    {
        match self {
            Self::Double => 1,
            Self::Float => 5,
            Self::Int32 => 0,
            Self::Int64 => 0,
            Self::UInt32 => 0,
            Self::UInt64 => 0,
            Self::SInt32 => 0,
            Self::SInt64 => 0,
            Self::Fixed32 => 5,
            Self::Fixed64 => 1,
            Self::SFixed32 => 5,
            Self::SFixed64 => 1,
            Self::Bool => 0,
            Self::String => 2,
            Self::Bytes => 2,
            Self::Message(..) => 2,
            Self::Enum(..) => 0,
            Self::Unknown(..) => 2,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct TypeRef
{
    pub idx: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct MessageRef(TypeRef);

#[derive(Debug, Clone, Copy)]
pub struct EnumRef(TypeRef);

#[derive(Debug)]
pub struct Service
{
    pub name: String,
    pub rpcs: Vec<Rpc>,
    pub options: Vec<ProtoOption>,
}

#[derive(Debug)]
pub struct Rpc
{
    pub name: String,
    pub param: RpcParam,
    pub retval: RpcParam,
    pub options: Vec<ProtoOption>,
}

#[derive(Debug)]
pub struct RpcParam
{
    pub stream: bool,
    pub param_type: ParamType,
}

#[derive(Debug)]
pub enum ParamType
{
    Message(MessageRef),
    Unknown(String),
}

#[derive(Debug)]
pub struct ProtoOption {}
