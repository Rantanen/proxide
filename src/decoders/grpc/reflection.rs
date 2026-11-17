//! Loading Protobuf definitions from a server with gRPC server reflection.
//!
//! Reflection returns the definitions as compiled `FileDescriptorProto` messages. Protofish only
//! parses `.proto` source, so the descriptors are rendered back into proto3 source and parsed
//! together with any files given with `--grpc`.
//!
//! See https://github.com/grpc/grpc/blob/master/doc/server-reflection.md

use bytes::{Buf, Bytes, BytesMut};
use http::{Request, StatusCode};
use prost::Message;
use prost_types::field_descriptor_proto::{Label, Type};
use prost_types::{
    DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
    ServiceDescriptorProto,
};
use std::collections::{BTreeMap, VecDeque};
use std::convert::TryFrom;
use std::fmt::Write;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

/// Time limit for fetching all definitions from the server.
pub const TIMEOUT: Duration = Duration::from_secs(15);

/// Reflection service versions, newest first. Servers may implement either or both.
const SERVICE_PATHS: [&str; 2] = [
    "/grpc.reflection.v1.ServerReflection/ServerReflectionInfo",
    "/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo",
];

const GRPC_STATUS_NOT_FOUND: u32 = 5;
const GRPC_STATUS_UNIMPLEMENTED: u32 = 12;

#[derive(Debug)]
pub struct ReflectionError(pub String);

impl std::fmt::Display for ReflectionError
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
    {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ReflectionError {}

fn err<T>(msg: impl Into<String>) -> Result<T, ReflectionError>
{
    Err(ReflectionError(msg.into()))
}

/// The server to query, parsed from the `--grpc-reflection` value.
#[derive(Debug, PartialEq, Eq)]
pub struct Target
{
    /// `host:port` used for the TCP connection and the HTTP/2 `:authority`.
    pub authority: String,

    /// Host name for TLS SNI.
    pub host: String,

    /// `Some(true)` for `https://`, `Some(false)` for `http://`. `None` tries plaintext first and
    /// falls back to TLS.
    pub tls: Option<bool>,
}

impl Target
{
    pub fn parse(value: &str) -> Result<Self, ReflectionError>
    {
        let (tls, rest) = if let Some(rest) = value.strip_prefix("https://") {
            (Some(true), rest)
        } else if let Some(rest) = value.strip_prefix("http://") {
            (Some(false), rest)
        } else if value.contains("://") {
            return err(format!(
                "Unsupported scheme in '{}', use http:// or https://",
                value
            ));
        } else {
            (None, value)
        };

        let rest = rest.trim_end_matches('/');
        if rest.is_empty() || rest.contains('/') {
            return err(format!(
                "Invalid server '{}', expected [http://|https://]host:port",
                value
            ));
        }

        // Split the port off. IPv6 addresses are written in brackets: [::1]:50051.
        let (host, port) = match rest.rfind(':') {
            Some(idx) if !rest[idx..].contains(']') => (&rest[..idx], Some(&rest[idx + 1..])),
            _ => (rest, None),
        };
        let port = match (port, tls) {
            (Some(p), _) => match p.parse::<u16>() {
                Ok(_) => p.to_string(),
                Err(_) => return err(format!("Invalid port in '{}'", value)),
            },
            (None, Some(true)) => "443".to_string(),
            (None, Some(false)) => "80".to_string(),
            (None, None) => {
                return err(format!(
                    "Missing port in '{}', expected host:port, http://host or https://host",
                    value
                ))
            }
        };
        if host.is_empty() {
            return err(format!("Missing host in '{}'", value));
        }

        Ok(Self {
            authority: format!("{}:{}", host, port),
            host: host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_string(),
            tls,
        })
    }
}

/// A `.proto` file loaded with reflection, rendered as proto3 source.
#[derive(Debug)]
pub struct ReflectedFile
{
    /// File name, such as `google/protobuf/duration.proto`.
    pub name: String,

    /// proto3 source for protofish.
    pub source: String,

    /// Full names of the messages and enums defined in the file, including nested ones.
    pub types: Vec<String>,
}

/// Fetches the definitions of all services from the server and renders them as proto3 source.
///
/// Returns one entry per `.proto` file.
pub fn fetch_proto_sources(target: &Target) -> Result<Vec<ReflectedFile>, ReflectionError>
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_else(|e| err(format!("Failed to start async runtime: {}", e)))?;

    let files = runtime.block_on(async {
        match tokio::time::timeout(TIMEOUT, fetch_files(target)).await {
            Ok(result) => result,
            Err(_) => err(format!("Timed out after {} seconds", TIMEOUT.as_secs())),
        }
    })?;

    Ok(files
        .descriptors
        .values()
        .map(reflected_file)
        .chain(
            files
                .builtin
                .iter()
                .map(|(name, body)| builtin_file(name, body)),
        )
        .collect())
}

#[derive(Default)]
struct Files
{
    /// Files from the server, by file name.
    descriptors: BTreeMap<String, FileDescriptorProto>,

    /// Built-in well-known types used for dependencies the server didn't have.
    builtin: BTreeMap<String, &'static str>,
}

impl Files
{
    fn contains(&self, name: &str) -> bool
    {
        self.descriptors.contains_key(name) || self.builtin.contains_key(name)
    }

    fn add(&mut self, response: MessageResponse) -> Result<(), ReflectionError>
    {
        let encoded = match response {
            MessageResponse::FileDescriptorResponse(r) => r.file_descriptor_proto,
            other => return err(format!("Unexpected reflection response: {:?}", other)),
        };
        for bytes in encoded {
            let fd = FileDescriptorProto::decode(bytes.as_slice())
                .or_else(|e| err(format!("Invalid file descriptor from the server: {}", e)))?;
            self.descriptors.entry(fd.name().to_string()).or_insert(fd);
        }
        Ok(())
    }

    /// Dependencies that are neither fetched nor built in, sorted.
    fn missing_dependencies(&self) -> Vec<String>
    {
        let mut missing: Vec<String> = self
            .descriptors
            .values()
            .flat_map(|f| f.dependency.iter())
            .filter(|d| !self.contains(d))
            .cloned()
            .collect();
        missing.sort();
        missing.dedup();
        missing
    }
}

/// Fetches the file descriptors of all services and their dependencies.
async fn fetch_files(target: &Target) -> Result<Files, ReflectionError>
{
    let mut client = match target.tls {
        Some(tls) => Client::connect(target, tls).await?,
        None => match Client::connect(target, false).await {
            Ok(client) => client,
            // No point trying TLS if the server can't be reached at all.
            Err(ConnectError::Tcp(e)) => return Err(e),
            Err(ConnectError::Protocol(plain_err)) => {
                log::debug!(
                    "Plaintext reflection to {} failed, trying TLS: {}",
                    target.authority,
                    plain_err
                );
                Client::connect(target, true).await.or_else(|tls_err| {
                    err(format!(
                        "Failed with both plaintext ({}) and TLS ({})",
                        plain_err,
                        ReflectionError::from(tls_err)
                    ))
                })?
            }
        },
    };

    let services = client.list_services().await?;
    log::info!("gRPC reflection: services {:?}", services);
    if services.is_empty() {
        return err("The server did not list any services");
    }

    let mut files = Files::default();
    for service in &services {
        let response = client
            .request(MessageRequest::FileContainingSymbol(service.clone()))
            .await?;
        files.add(response)?;
    }

    // Servers may or may not include dependencies in the responses. Fetch the missing ones.
    // Unresolvable dependencies are skipped and logged.
    let mut not_found = Vec::new();
    let mut missing: VecDeque<String> = files.missing_dependencies().into();
    while let Some(name) = missing.pop_front() {
        if files.contains(&name) || not_found.contains(&name) {
            continue;
        }

        match client
            .request(MessageRequest::FileByFilename(name.clone()))
            .await
        {
            Ok(response) => files.add(response)?,
            Err(RequestError::NotFound(msg)) => match well_known_type(&name) {
                Some(body) => {
                    log::info!("gRPC reflection: using built-in {}", name);
                    files.builtin.insert(name.clone(), body);
                }
                None => {
                    // Not fatal: the types may be given with --grpc. If they aren't, parsing
                    // reports the unknown type.
                    log::warn!("gRPC reflection: dependency {} not found: {}", name, msg);
                    not_found.push(name);
                    continue;
                }
            },
            Err(RequestError::Other(e)) => return Err(e),
        }

        for dep in files.missing_dependencies() {
            if !missing.contains(&dep) && !not_found.contains(&dep) {
                missing.push_back(dep);
            }
        }
    }

    Ok(files)
}

/// Reflection messages from `grpc/reflection/v1/reflection.proto`. The v1alpha messages are
/// identical on the wire.
#[derive(Clone, PartialEq, prost::Message)]
struct ServerReflectionRequest
{
    #[prost(string, tag = "1")]
    host: String,
    #[prost(oneof = "MessageRequest", tags = "3, 4, 7")]
    message_request: Option<MessageRequest>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
enum MessageRequest
{
    #[prost(string, tag = "3")]
    FileByFilename(String),
    #[prost(string, tag = "4")]
    FileContainingSymbol(String),
    #[prost(string, tag = "7")]
    ListServices(String),
}

#[derive(Clone, PartialEq, prost::Message)]
struct ServerReflectionResponse
{
    #[prost(oneof = "MessageResponse", tags = "4, 6, 7")]
    message_response: Option<MessageResponse>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
#[allow(clippy::enum_variant_names, reason = "names follow reflection.proto")]
enum MessageResponse
{
    #[prost(message, tag = "4")]
    FileDescriptorResponse(FileDescriptorResponse),
    #[prost(message, tag = "6")]
    ListServicesResponse(ListServiceResponse),
    #[prost(message, tag = "7")]
    ErrorResponse(ErrorResponse),
}

#[derive(Clone, PartialEq, prost::Message)]
struct FileDescriptorResponse
{
    #[prost(bytes = "vec", repeated, tag = "1")]
    file_descriptor_proto: Vec<Vec<u8>>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ListServiceResponse
{
    #[prost(message, repeated, tag = "1")]
    service: Vec<ServiceResponse>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ServiceResponse
{
    #[prost(string, tag = "1")]
    name: String,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ErrorResponse
{
    #[prost(int32, tag = "1")]
    error_code: i32,
    #[prost(string, tag = "2")]
    error_message: String,
}

enum ConnectError
{
    /// The TCP connection failed.
    Tcp(ReflectionError),

    /// TLS, HTTP/2 or the reflection service failed.
    Protocol(ReflectionError),
}

impl From<ConnectError> for ReflectionError
{
    fn from(e: ConnectError) -> Self
    {
        match e {
            ConnectError::Tcp(e) | ConnectError::Protocol(e) => e,
        }
    }
}

enum RequestError
{
    /// The server reported the requested file or symbol doesn't exist.
    NotFound(String),
    Other(ReflectionError),
}

impl From<ReflectionError> for RequestError
{
    fn from(e: ReflectionError) -> Self
    {
        RequestError::Other(e)
    }
}

impl From<RequestError> for ReflectionError
{
    fn from(e: RequestError) -> Self
    {
        match e {
            RequestError::NotFound(msg) => ReflectionError(msg),
            RequestError::Other(e) => e,
        }
    }
}

/// gRPC reflection client.
///
/// Each request is sent on its own stream, which the reflection protocol allows: the stream is a
/// sequence of independent request/response pairs.
struct Client
{
    send: h2::client::SendRequest<Bytes>,
    scheme: &'static str,
    authority: String,
    path: &'static str,
}

impl Client
{
    async fn connect(target: &Target, tls: bool) -> Result<Self, ConnectError>
    {
        let tcp = TcpStream::connect(&target.authority).await.map_err(|e| {
            ConnectError::Tcp(ReflectionError(format!(
                "Failed to connect to {}: {}",
                target.authority, e
            )))
        })?;
        Self::connect_over(target, tls, tcp)
            .await
            .map_err(ConnectError::Protocol)
    }

    async fn connect_over(
        target: &Target,
        tls: bool,
        tcp: TcpStream,
    ) -> Result<Self, ReflectionError>
    {
        let send = if tls {
            let config = crate::connection::tls::upstream_client_config(vec![b"h2".to_vec()])
                .or_else(|e| err(format!("Failed to configure TLS: {}", e)))?;
            let server_name = rustls::pki_types::ServerName::try_from(target.host.clone())
                .or_else(|e| err(format!("Invalid TLS server name '{}': {}", target.host, e)))?;
            let stream = tokio_rustls::TlsConnector::from(Arc::new(config))
                .connect(server_name, tcp)
                .await
                .or_else(|e| err(format!("TLS handshake failed: {}", e)))?;
            handshake(stream).await?
        } else {
            handshake(tcp).await?
        };

        let mut client = Self {
            send,
            scheme: if tls { "https" } else { "http" },
            authority: target.authority.clone(),
            path: SERVICE_PATHS[0],
        };

        // Pick the reflection version and make sure the connection works, so that a failing
        // plaintext attempt can fall back to TLS.
        client.select_version().await?;
        Ok(client)
    }

    async fn select_version(&mut self) -> Result<(), ReflectionError>
    {
        for path in SERVICE_PATHS.iter() {
            self.path = path;
            match self.call(MessageRequest::ListServices(String::new())).await {
                Ok(_) => return Ok(()),
                Err(CallError::Status(GRPC_STATUS_UNIMPLEMENTED, _)) => {
                    log::debug!("gRPC reflection: {} not implemented", path);
                    continue;
                }
                Err(e) => return err(e.to_string()),
            }
        }
        err("The server does not support gRPC server reflection")
    }

    async fn list_services(&mut self) -> Result<Vec<String>, ReflectionError>
    {
        match self
            .request(MessageRequest::ListServices(String::new()))
            .await?
        {
            MessageResponse::ListServicesResponse(r) => Ok(r
                .service
                .into_iter()
                .map(|s| s.name)
                .filter(|s| !s.starts_with("grpc.reflection."))
                .collect()),
            other => err(format!("Unexpected reflection response: {:?}", other)),
        }
    }

    async fn request(&mut self, request: MessageRequest) -> Result<MessageResponse, RequestError>
    {
        let description = format!("{:?}", request);
        match self.call(request).await {
            Ok(MessageResponse::ErrorResponse(e))
                if e.error_code as u32 == GRPC_STATUS_NOT_FOUND =>
            {
                Err(RequestError::NotFound(e.error_message))
            }
            Ok(MessageResponse::ErrorResponse(e)) => {
                Err(RequestError::Other(ReflectionError(format!(
                    "{} failed with status {}: {}",
                    description, e.error_code, e.error_message
                ))))
            }
            Ok(response) => Ok(response),
            Err(CallError::Status(GRPC_STATUS_NOT_FOUND, msg)) => Err(RequestError::NotFound(msg)),
            Err(e) => Err(RequestError::Other(ReflectionError(format!(
                "{} failed: {}",
                description, e
            )))),
        }
    }

    async fn call(&mut self, request: MessageRequest) -> Result<MessageResponse, CallError>
    {
        let request = ServerReflectionRequest {
            host: String::new(),
            message_request: Some(request),
        };
        let http_request =
            Request::post(format!("{}://{}{}", self.scheme, self.authority, self.path))
                .header("content-type", "application/grpc")
                .header("te", "trailers")
                .body(())
                .expect("Static request is valid");

        let send = self.send.clone().ready().await?;
        let mut send = send;
        let (response, mut stream) = send.send_request(http_request, false)?;
        stream.send_data(grpc_frame(&request.encode_to_vec()), true)?;

        let response = response.await?;
        if response.status() != StatusCode::OK {
            return Err(CallError::Protocol(format!(
                "HTTP status {}",
                response.status()
            )));
        }

        // A "trailers-only" response carries the gRPC status in the headers.
        if let Some(status) = grpc_status(response.headers()) {
            if status.0 != 0 {
                return Err(CallError::Status(status.0, status.1));
            }
        }

        let mut body = response.into_body();
        let mut data = BytesMut::new();
        while let Some(chunk) = body.data().await {
            let chunk = chunk?;
            let _ = body.flow_control().release_capacity(chunk.len());
            data.extend_from_slice(&chunk);
        }
        if let Some(trailers) = body.trailers().await? {
            if let Some(status) = grpc_status(&trailers) {
                if status.0 != 0 {
                    return Err(CallError::Status(status.0, status.1));
                }
            }
        }

        let message = first_grpc_message(data.freeze())
            .ok_or_else(|| CallError::Protocol("Empty response".to_string()))?;
        let response = ServerReflectionResponse::decode(message)
            .map_err(|e| CallError::Protocol(format!("Invalid response: {}", e)))?;
        response
            .message_response
            .ok_or_else(|| CallError::Protocol("Empty response".to_string()))
    }
}

async fn handshake<T>(io: T) -> Result<h2::client::SendRequest<Bytes>, ReflectionError>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (send, connection) = h2::client::handshake(io)
        .await
        .or_else(|e| err(format!("HTTP/2 handshake failed: {}", e)))?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            log::debug!("gRPC reflection connection closed: {}", e);
        }
    });
    Ok(send)
}

enum CallError
{
    Status(u32, String),
    Protocol(String),
}

impl From<h2::Error> for CallError
{
    fn from(e: h2::Error) -> Self
    {
        CallError::Protocol(e.to_string())
    }
}

impl std::fmt::Display for CallError
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
    {
        match self {
            CallError::Status(code, msg) => write!(f, "gRPC status {}: {}", code, msg),
            CallError::Protocol(msg) => f.write_str(msg),
        }
    }
}

fn grpc_status(headers: &http::HeaderMap) -> Option<(u32, String)>
{
    let code = headers.get("grpc-status")?.to_str().ok()?.parse().ok()?;
    let message = headers
        .get("grpc-message")
        .and_then(|m| m.to_str().ok())
        .unwrap_or_default()
        .to_string();
    Some((code, message))
}

fn grpc_frame(payload: &[u8]) -> Bytes
{
    let mut frame = BytesMut::with_capacity(payload.len() + 5);
    frame.extend_from_slice(&[0]);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    frame.freeze()
}

fn first_grpc_message(mut data: Bytes) -> Option<Bytes>
{
    if data.len() < 5 || data[0] != 0 {
        return None;
    }
    data.advance(1);
    let len = data.get_u32() as usize;
    if data.len() < len {
        return None;
    }
    Some(data.split_to(len))
}

/// Renders a file descriptor as proto3 source that protofish can parse.
///
/// Only what's needed for decoding is rendered: options, comments, imports, extensions and
/// reserved ranges are left out. proto2 files are rendered as proto3, which describes the same
/// wire format. proto2 groups are not supported by protofish and are left out.
///
/// Map fields are rendered the way they are encoded: as repeated `XxxEntry` messages with `key`
/// and `value` fields. protofish 0.5 doesn't support `map<K, V>` syntax.
pub fn render_file(fd: &FileDescriptorProto) -> String
{
    let mut out = String::new();
    let _ = writeln!(out, "// {}", fd.name());
    out.push_str("syntax = \"proto3\";\n");
    if !fd.package().is_empty() {
        let _ = writeln!(out, "package {};", fd.package());
    }

    let proto3 = fd.syntax() == "proto3";
    for m in &fd.message_type {
        render_message(&mut out, m, proto3, 0);
    }
    for e in &fd.enum_type {
        render_enum(&mut out, e, 0);
    }
    for s in &fd.service {
        render_service(&mut out, s);
    }
    out
}

fn reflected_file(fd: &FileDescriptorProto) -> ReflectedFile
{
    fn collect(
        prefix: &str,
        messages: &[DescriptorProto],
        enums: &[EnumDescriptorProto],
        out: &mut Vec<String>,
    )
    {
        for e in enums {
            out.push(format!("{}{}", prefix, e.name()));
        }
        for m in messages {
            let name = format!("{}{}", prefix, m.name());
            collect(&format!("{}.", name), &m.nested_type, &m.enum_type, out);
            out.push(name);
        }
    }

    let prefix = match fd.package() {
        "" => String::new(),
        package => format!("{}.", package),
    };
    let mut types = vec![];
    collect(&prefix, &fd.message_type, &fd.enum_type, &mut types);

    ReflectedFile {
        name: fd.name().to_string(),
        source: render_file(fd),
        types,
    }
}

fn indent(out: &mut String, level: usize)
{
    for _ in 0..level {
        out.push_str("  ");
    }
}

fn render_message(out: &mut String, m: &DescriptorProto, proto3: bool, level: usize)
{
    indent(out, level);
    let _ = writeln!(out, "message {} {{", m.name());

    // Map entry messages are rendered as ordinary nested messages.
    for nested in &m.nested_type {
        render_message(out, nested, proto3, level + 1);
    }
    for e in &m.enum_type {
        render_enum(out, e, level + 1);
    }

    // proto3 `optional` fields are placed in synthetic oneofs, which are not real oneofs.
    let is_real_oneof_field =
        |f: &FieldDescriptorProto| f.oneof_index.is_some() && !f.proto3_optional();

    for f in m.field.iter().filter(|f| !is_real_oneof_field(f)) {
        if f.r#type() == Type::Group {
            indent(out, level + 1);
            let _ = writeln!(
                out,
                "// group {} = {} is not supported",
                f.name(),
                f.number()
            );
            continue;
        }

        let label = match f.label() {
            Label::Repeated => "repeated ",
            Label::Optional if f.proto3_optional() || !proto3 => "optional ",
            _ => "",
        };
        indent(out, level + 1);
        let _ = writeln!(
            out,
            "{}{} {} = {};",
            label,
            field_type(f),
            f.name(),
            f.number()
        );
    }

    for (idx, oneof) in m.oneof_decl.iter().enumerate() {
        let fields: Vec<_> = m
            .field
            .iter()
            .filter(|f| is_real_oneof_field(f) && f.oneof_index() as usize == idx)
            .collect();
        if fields.is_empty() {
            continue;
        }

        indent(out, level + 1);
        let _ = writeln!(out, "oneof {} {{", oneof.name());
        for f in fields {
            indent(out, level + 2);
            if f.r#type() == Type::Group {
                let _ = writeln!(
                    out,
                    "// group {} = {} is not supported",
                    f.name(),
                    f.number()
                );
            } else {
                let _ = writeln!(out, "{} {} = {};", field_type(f), f.name(), f.number());
            }
        }
        indent(out, level + 1);
        out.push_str("}\n");
    }

    indent(out, level);
    out.push_str("}\n");
}

fn field_type(f: &FieldDescriptorProto) -> String
{
    match f.r#type() {
        Type::Double => "double",
        Type::Float => "float",
        Type::Int64 => "int64",
        Type::Uint64 => "uint64",
        Type::Int32 => "int32",
        Type::Fixed64 => "fixed64",
        Type::Fixed32 => "fixed32",
        Type::Bool => "bool",
        Type::String => "string",
        Type::Bytes => "bytes",
        Type::Uint32 => "uint32",
        Type::Sfixed32 => "sfixed32",
        Type::Sfixed64 => "sfixed64",
        Type::Sint32 => "sint32",
        Type::Sint64 => "sint64",
        // Descriptors use fully qualified names with a leading dot, which protofish resolves.
        Type::Message | Type::Enum | Type::Group => return f.type_name().to_string(),
    }
    .to_string()
}

fn render_enum(out: &mut String, e: &EnumDescriptorProto, level: usize)
{
    indent(out, level);
    let _ = writeln!(out, "enum {} {{", e.name());
    for v in &e.value {
        indent(out, level + 1);
        let _ = writeln!(out, "{} = {};", v.name(), v.number());
    }
    indent(out, level);
    out.push_str("}\n");
}

fn render_service(out: &mut String, s: &ServiceDescriptorProto)
{
    let _ = writeln!(out, "service {} {{", s.name());
    for m in &s.method {
        let _ = writeln!(
            out,
            "  rpc {} ({}{}) returns ({}{});",
            m.name(),
            if m.client_streaming() { "stream " } else { "" },
            m.input_type(),
            if m.server_streaming() { "stream " } else { "" },
            m.output_type(),
        );
    }
    out.push_str("}\n");
}

/// Built-in copies of the commonly used well-known types, for servers that don't serve them.
///
/// Only the fields matter for decoding, so these are minimal versions of the files that ship with
/// protoc.
fn well_known_type(name: &str) -> Option<&'static str>
{
    let body = match name {
        "google/protobuf/any.proto" => "message Any { string type_url = 1; bytes value = 2; }",
        "google/protobuf/duration.proto" => {
            "message Duration { int64 seconds = 1; int32 nanos = 2; }"
        }
        "google/protobuf/empty.proto" => "message Empty {}",
        "google/protobuf/field_mask.proto" => "message FieldMask { repeated string paths = 1; }",
        "google/protobuf/timestamp.proto" => {
            "message Timestamp { int64 seconds = 1; int32 nanos = 2; }"
        }
        "google/protobuf/wrappers.proto" => {
            "message DoubleValue { double value = 1; }
            message FloatValue { float value = 1; }
            message Int64Value { int64 value = 1; }
            message UInt64Value { uint64 value = 1; }
            message Int32Value { int32 value = 1; }
            message UInt32Value { uint32 value = 1; }
            message BoolValue { bool value = 1; }
            message StringValue { string value = 1; }
            message BytesValue { bytes value = 1; }"
        }
        "google/protobuf/struct.proto" => {
            "message Struct {
              message FieldsEntry { string key = 1; Value value = 2; }
              repeated FieldsEntry fields = 1;
            }
            message Value {
              oneof kind {
                NullValue null_value = 1;
                double number_value = 2;
                string string_value = 3;
                bool bool_value = 4;
                Struct struct_value = 5;
                ListValue list_value = 6;
              }
            }
            enum NullValue { NULL_VALUE = 0; }
            message ListValue { repeated Value values = 1; }"
        }
        _ => return None,
    };
    Some(body)
}

fn builtin_file(name: &str, body: &str) -> ReflectedFile
{
    // Collect the declared types. The built-in sources only use `message`, `enum` and `oneof`
    // blocks, so tracking braces is enough.
    let mut types = vec![];
    let mut scopes: Vec<Option<String>> = vec![];
    let mut declared: Option<String> = None;
    let mut tokens = body
        .split(|c: char| c.is_whitespace() || c == ';')
        .flat_map(|t| {
            // Split `Name{` and `}` off identifiers.
            let mut parts = vec![];
            let mut rest = t;
            while let Some(idx) = rest.find(['{', '}']) {
                parts.push(&rest[..idx]);
                parts.push(&rest[idx..idx + 1]);
                rest = &rest[idx + 1..];
            }
            parts.push(rest);
            parts
        })
        .filter(|t| !t.is_empty());
    while let Some(token) = tokens.next() {
        match token {
            "message" | "enum" => {
                let name = tokens.next().expect("Declaration has a name");
                let mut full_name = "google.protobuf".to_string();
                for scope in scopes.iter().flatten() {
                    full_name.push('.');
                    full_name.push_str(scope);
                }
                types.push(format!("{}.{}", full_name, name));
                declared = Some(name.to_string());
            }
            "{" => scopes.push(declared.take()),
            "}" => {
                scopes.pop();
            }
            _ => {}
        }
    }

    ReflectedFile {
        name: name.to_string(),
        source: well_known_source(name, body),
        types,
    }
}

fn well_known_source(name: &str, body: &str) -> String
{
    format!(
        "// {} (built into proxide)\nsyntax = \"proto3\";\npackage google.protobuf;\n{}\n",
        name, body
    )
}

#[cfg(test)]
mod test
{
    use super::super::test::{factory_from_files, grpc_frame, message, request, text_to_string};
    use super::*;
    use crate::decoders::DecoderFactory;
    use crate::session::RequestPart;
    use prost_types::FileDescriptorSet;

    const REFLECTION_TEST_PROTO: &str =
        include_str!("../../../test/rust_grpc/proto/reflection_test.proto");

    #[test]
    fn target_parse()
    {
        let ok = |value: &str, authority: &str, host: &str, tls: Option<bool>| {
            assert_eq!(
                Target::parse(value).unwrap(),
                Target {
                    authority: authority.to_string(),
                    host: host.to_string(),
                    tls
                },
                "{}",
                value
            );
        };
        ok("localhost:50051", "localhost:50051", "localhost", None);
        ok(
            "http://localhost:50051",
            "localhost:50051",
            "localhost",
            Some(false),
        );
        ok(
            "https://example.com",
            "example.com:443",
            "example.com",
            Some(true),
        );
        ok(
            "http://example.com/",
            "example.com:80",
            "example.com",
            Some(false),
        );
        ok("[::1]:50051", "[::1]:50051", "::1", None);
        ok("https://[::1]", "[::1]:443", "::1", Some(true));

        for bad in [
            "localhost",
            "ftp://localhost:1",
            "localhost:port",
            "localhost:70000",
            ":50051",
            "http://localhost:1/path",
            "",
        ] {
            assert!(Target::parse(bad).is_err(), "{:?} should be rejected", bad);
        }
    }

    fn descriptor_set() -> BTreeMap<String, FileDescriptorProto>
    {
        FileDescriptorSet::decode(grpc_tester::FILE_DESCRIPTOR_SET)
            .unwrap()
            .file
            .into_iter()
            .map(|f| (f.name().to_string(), f))
            .collect()
    }

    #[test]
    fn rendered_descriptors_keep_the_definitions()
    {
        let files = descriptor_set();
        let rendered = render_file(&files["reflection_test.proto"]);

        for expected in [
            "package reflection_test;",
            "message Item {",
            "  enum Kind {",
            "  message Tag {",
            "  optional string nickname = 2;",
            "  optional int32 count = 3;",
            "  repeated .reflection_test.Item.Tag tags = 5;",
            "  message AttributesEntry {",
            "    int64 value = 2;",
            "  repeated .reflection_test.Item.AttributesEntry attributes = 6;",
            "  repeated .reflection_test.Item.TagsByIdEntry tags_by_id = 7;",
            "  oneof source {",
            "    string url = 8;",
            "    bytes blob = 9;",
            "  .google.protobuf.Timestamp created = 11;",
            "  rpc WatchItems (stream .reflection_test.ItemRequest) returns (stream .reflection_test.Item);",
        ] {
            assert!(
                rendered.lines().any(|l| l == expected),
                "{:?} not in:\n{}",
                expected,
                rendered
            );
        }

        // protofish doesn't support map syntax, and optional fields are not real oneofs.
        assert!(!rendered.contains("map<"), "{}", rendered);
        assert!(!rendered.contains("oneof _"), "{}", rendered);
    }

    #[test]
    fn rendered_descriptors_parse()
    {
        let sources: Vec<String> = descriptor_set().values().map(render_file).collect();
        let sources: Vec<&str> = sources.iter().map(String::as_str).collect();
        protofish::context::Context::parse(sources).unwrap();
    }

    #[test]
    fn proto2_descriptors_render_as_proto3()
    {
        use prost_types::{field_descriptor_proto::Label, DescriptorProto};

        let field = |name: &str, number: i32, label: Label, ty: Type| FieldDescriptorProto {
            name: Some(name.to_string()),
            number: Some(number),
            label: Some(label as i32),
            r#type: Some(ty as i32),
            ..Default::default()
        };
        let fd = FileDescriptorProto {
            name: Some("legacy.proto".to_string()),
            package: Some("legacy".to_string()),
            syntax: Some("proto2".to_string()),
            message_type: vec![DescriptorProto {
                name: Some("Old".to_string()),
                field: vec![
                    field("id", 1, Label::Required, Type::Int32),
                    field("label", 2, Label::Optional, Type::String),
                    field("values", 3, Label::Repeated, Type::Sint64),
                    field("grp", 4, Label::Optional, Type::Group),
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        let rendered = render_file(&fd);
        for expected in [
            "syntax = \"proto3\";",
            "  int32 id = 1;",
            "  optional string label = 2;",
            "  repeated sint64 values = 3;",
            "  // group grp = 4 is not supported",
        ] {
            assert!(
                rendered.lines().any(|l| l == expected),
                "{:?} in:\n{}",
                expected,
                rendered
            );
        }
        protofish::context::Context::parse(&[rendered]).unwrap();
    }

    #[test]
    fn builtin_well_known_types_match_protoc()
    {
        let files = descriptor_set();
        for name in [
            "google/protobuf/any.proto",
            "google/protobuf/duration.proto",
            "google/protobuf/empty.proto",
            "google/protobuf/field_mask.proto",
            "google/protobuf/struct.proto",
            "google/protobuf/timestamp.proto",
            "google/protobuf/wrappers.proto",
        ] {
            let builtin = well_known_source(name, well_known_type(name).unwrap());
            let builtin = protofish::context::Context::parse([&builtin]).unwrap();

            // Compare against the protoc copies where the test protos import them.
            let fd = match files.get(name) {
                Some(fd) => fd,
                None => continue,
            };
            let real = protofish::context::Context::parse(&[render_file(fd)]).unwrap();
            for m in &fd.message_type {
                let full_name = format!("google.protobuf.{}", m.name());
                let fields = |ctx: &protofish::context::Context| {
                    let msg = ctx.get_message(&full_name).expect(&full_name);
                    msg.iter_fields()
                        .map(|f| (f.number, f.name.clone(), format!("{:?}", f.multiplicity)))
                        .collect::<Vec<_>>()
                };
                assert_eq!(fields(&builtin), fields(&real), "{}", full_name);
            }
        }
    }

    #[test]
    fn reflected_file_lists_nested_types()
    {
        let file = reflected_file(&descriptor_set()["reflection_test.proto"]);
        assert_eq!(file.name, "reflection_test.proto");
        let mut types = file.types;
        types.sort();
        assert_eq!(
            types,
            [
                "reflection_test.Color",
                "reflection_test.Item",
                "reflection_test.Item.AttributesEntry",
                "reflection_test.Item.Kind",
                "reflection_test.Item.Tag",
                "reflection_test.Item.TagsByIdEntry",
                "reflection_test.ItemRequest",
            ]
        );
    }

    #[test]
    fn builtin_file_lists_its_types()
    {
        let file = builtin_file(
            "google/protobuf/struct.proto",
            well_known_type("google/protobuf/struct.proto").unwrap(),
        );
        let mut types = file.types;
        types.sort();
        assert_eq!(
            types,
            [
                "google.protobuf.ListValue",
                "google.protobuf.NullValue",
                "google.protobuf.Struct",
                "google.protobuf.Struct.FieldsEntry",
                "google.protobuf.Value",
            ]
        );

        // The listed types are exactly the ones protofish finds.
        let ctx = protofish::context::Context::parse([&file.source]).unwrap();
        for t in &types {
            assert!(ctx.get_type(t).is_some(), "{}", t);
        }
    }

    #[test]
    fn unknown_well_known_type_is_not_built_in()
    {
        assert!(well_known_type("google/protobuf/descriptor.proto").is_none());
        assert!(well_known_type("other.proto").is_none());
    }

    fn varint(mut v: u64) -> Vec<u8>
    {
        let mut out = vec![];
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    fn len_field(number: u64, payload: &[u8]) -> Vec<u8>
    {
        let mut out = varint(number << 3 | 2);
        out.extend(varint(payload.len() as u64));
        out.extend_from_slice(payload);
        out
    }

    fn varint_field(number: u64, value: u64) -> Vec<u8>
    {
        let mut out = varint(number << 3);
        out.extend(varint(value));
        out
    }

    /// An `Item` that uses every field of reflection_test.proto.
    fn item_payload() -> Vec<u8>
    {
        let tag = [len_field(1, b"k"), varint_field(2, 3 /* sint32 -2 */)].concat();
        let mut checksum = varint(14 << 3 | 1);
        checksum.extend_from_slice(&9u64.to_le_bytes());

        [
            len_field(1, b"widget"),
            len_field(2, b""),  // explicitly empty optional
            varint_field(3, 0), // explicitly zero optional
            varint_field(4, 1), // KIND_TOOL
            len_field(5, &tag),
            len_field(6, &[len_field(1, b"size"), varint_field(2, 7)].concat()),
            len_field(7, &[varint_field(1, 5), len_field(2, &tag)].concat()),
            len_field(9, &[1, 2]), // oneof blob
            varint_field(10, 2),   // COLOR_BLUE
            len_field(11, &varint_field(1, 5)),
            len_field(12, &len_field(1, b"note")),
            len_field(13, &1.5f64.to_le_bytes()),
            checksum,
        ]
        .concat()
    }

    fn decode_item(factory: &dyn DecoderFactory) -> String
    {
        let msg = message(RequestPart::Response, &grpc_frame(&item_payload()));
        let decoder = factory
            .try_create(&request("/reflection_test.ItemService/GetItem"), &msg)
            .expect("ItemService/GetItem should be known");
        text_to_string(&decoder.decode(&msg))
    }

    fn assert_decodes_item(text: &str)
    {
        for expected in [
            "Item {",
            "name: \"widget\"",
            "nickname: \"\"",
            "count: 0",
            "kind: KIND_TOOL",
            "weight: -2",
            "attributes: AttributesEntry {",
            "tags_by_id: TagsByIdEntry {",
            "blob: ",
            "color: COLOR_BLUE",
            "created: Timestamp {",
            "note: StringValue {",
            "scores: [ 1.5 ]",
            "checksum: 9",
        ] {
            assert!(text.contains(expected), "{:?} not in:\n{}", expected, text);
        }
        assert!(!text.contains("[#"), "unknown fields in:\n{}", text);
        assert!(!text.contains("Incomplete"), "{}", text);
    }

    /// The same messages must decode identically from reflection and from the .proto files.
    fn assert_same_as_proto_files(reflected_sources: &[String])
    {
        let reflected: Vec<&str> = reflected_sources.iter().map(String::as_str).collect();
        let reflected = factory_from_files(&reflected).unwrap().unwrap();

        let timestamp = well_known_source(
            "timestamp",
            well_known_type("google/protobuf/timestamp.proto").unwrap(),
        );
        let wrappers = well_known_source(
            "wrappers",
            well_known_type("google/protobuf/wrappers.proto").unwrap(),
        );
        // protofish doesn't support map syntax, so write the maps as the entry messages they are
        // encoded as.
        let proto = REFLECTION_TEST_PROTO
            .replace(
                "map<string, int64> attributes = 6;",
                "message AttributesEntry { string key = 1; int64 value = 2; }
                repeated AttributesEntry attributes = 6;",
            )
            .replace(
                "map<int32, Tag> tags_by_id = 7;",
                "message TagsByIdEntry { int32 key = 1; Tag value = 2; }
                repeated TagsByIdEntry tags_by_id = 7;",
            );
        assert!(!proto.contains("map<"));
        let from_files = factory_from_files(&[&proto, &timestamp, &wrappers])
            .unwrap()
            .unwrap();

        let text = decode_item(reflected.as_ref());
        assert_decodes_item(&text);
        assert_eq!(text, decode_item(from_files.as_ref()));
    }

    #[test]
    fn rendered_descriptors_decode_like_proto_files()
    {
        let sources: Vec<String> = descriptor_set().values().map(render_file).collect();
        assert_same_as_proto_files(&sources);
    }

    #[tokio::test]
    async fn fetch_from_reflection_server()
    {
        let mut server = grpc_tester::server::GrpcServer::start().await.unwrap();
        let address = server.http();

        let sources = tokio::task::spawn_blocking(move || {
            fetch_proto_sources(&Target::parse(&address).unwrap())
        })
        .await
        .unwrap()
        .unwrap();
        server.stop().unwrap();

        let names: Vec<&str> = sources.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "google/protobuf/duration.proto",
                "google/protobuf/timestamp.proto",
                "google/protobuf/wrappers.proto",
                "reflection_test.proto",
                "rust_grpc.proto",
            ]
        );
        let sources: Vec<String> = sources.into_iter().map(|f| f.source).collect();

        // Both services and the imported well-known types were fetched.
        let all = sources.join("\n");
        for expected in [
            "service TestService {",
            "service ItemService {",
            "package google.protobuf;",
            "message Timestamp {",
            "message StringValue {",
            "message Duration {",
        ] {
            assert!(all.contains(expected), "{:?} not in:\n{}", expected, all);
        }
        assert!(!all.contains("ServerReflection"), "{}", all);
        assert_same_as_proto_files(&sources);
    }

    #[test]
    fn unreachable_server_fails_without_tls_retry()
    {
        let port = portpicker::pick_unused_port().unwrap();
        let target = Target::parse(&format!("127.0.0.1:{}", port)).unwrap();
        let e = fetch_proto_sources(&target).unwrap_err().to_string();
        assert!(e.starts_with("Failed to connect to 127.0.0.1:"), "{}", e);
        assert!(!e.contains("TLS"), "{}", e);
    }
}
