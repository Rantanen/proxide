use bytes::BytesMut;
use chrono::prelude::*;
use http::{HeaderMap, Method, Uri};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use uuid::Uuid;

pub mod events;
pub mod serialization;

#[derive(Serialize, Deserialize, Default)]
pub struct Session
{
    pub connections: IndexedVec<ConnectionData>,
    pub requests: IndexedVec<EncodedRequest>,
}

#[derive(Serialize, Deserialize)]
pub struct IndexedVec<T>
{
    pub items: Vec<T>,
    pub items_by_uuid: HashMap<Uuid, usize>,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Protocol
{
    Connect,
    Tls,
    Http2,
}

#[derive(Serialize, Deserialize)]
pub struct ConnectionData
{
    pub uuid: Uuid,
    pub client_addr: SocketAddr,
    pub protocol_stack: Vec<Protocol>,
    pub start_timestamp: DateTime<Local>,
    pub end_timestamp: Option<DateTime<Local>>,
    pub status: Status,
}

#[derive(Serialize, Deserialize)]
pub struct RequestData
{
    pub uuid: Uuid,
    pub connection_uuid: Uuid,

    #[serde(with = "http_serde::method")]
    pub method: Method,

    #[serde(with = "http_serde::uri")]
    pub uri: Uri,

    pub start_timestamp: DateTime<Local>,
    pub end_timestamp: Option<DateTime<Local>>,
    pub status: Status,
}

#[derive(Serialize, Deserialize)]
pub struct EncodedRequest
{
    pub request_data: RequestData,
    pub request_msg: MessageData,
    pub response_msg: MessageData,
}

#[derive(Serialize, Deserialize)]
pub struct MessageData
{
    #[serde(with = "http_serde::header_map")]
    pub headers: HeaderMap,

    #[serde(with = "http_serde::header_map")]
    pub trailers: HeaderMap,

    #[serde(with = "serde_base64")]
    pub content: BytesMut,

    pub start_timestamp: Option<DateTime<Local>>,
    pub end_timestamp: Option<DateTime<Local>>,
    pub part: RequestPart,
}

#[derive(Debug, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub enum Status
{
    InProgress,
    Succeeded,
    Failed,
}

#[derive(Debug, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub enum RequestPart
{
    Request,
    Response,
}

impl MessageData
{
    pub fn new(part: RequestPart) -> Self
    {
        Self {
            headers: Default::default(),
            trailers: Default::default(),
            content: Default::default(),
            start_timestamp: None,
            end_timestamp: None,
            part,
        }
    }

    pub fn with_headers(mut self, h: HeaderMap) -> Self
    {
        self.headers = h;
        self
    }

    pub fn with_start_timestamp(mut self, ts: DateTime<Local>) -> Self
    {
        self.start_timestamp = Some(ts);
        self
    }
}

impl<T> IndexedVec<T>
{
    pub fn push(&mut self, uuid: Uuid, item: T)
    {
        self.items_by_uuid.insert(uuid, self.items.len());
        self.items.push(item);
    }

    pub fn get_index_by_uuid(&self, uuid: Uuid) -> Option<usize>
    {
        self.items_by_uuid.get(&uuid).copied()
    }

    pub fn get_by_uuid(&self, uuid: Uuid) -> Option<&T>
    {
        let idx = self.items_by_uuid.get(&uuid)?;
        self.items.get(*idx)
    }

    pub fn get_mut_by_uuid(&mut self, uuid: Uuid) -> Option<&mut T>
    {
        let idx = self.items_by_uuid.get(&uuid)?;
        self.items.get_mut(*idx)
    }
}

impl<T> std::ops::Deref for IndexedVec<T>
{
    type Target = [T];
    fn deref(&self) -> &Self::Target
    {
        &self.items
    }
}

impl<T> Default for IndexedVec<T>
{
    fn default() -> Self
    {
        Self {
            items: Default::default(),
            items_by_uuid: Default::default(),
        }
    }
}

pub trait HasKey
{
    fn key(&self) -> Uuid;
}

impl HasKey for EncodedRequest
{
    fn key(&self) -> Uuid
    {
        self.request_data.uuid
    }
}

impl std::fmt::Display for Protocol
{
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result
    {
        write!(
            f,
            "{}",
            match self {
                Protocol::Connect => "CONNECT",
                Protocol::Tls => "TLS",
                Protocol::Http2 => "HTTP/2",
            },
        )
    }
}

mod serde_base64
{
    use bytes::BytesMut;
    pub fn serialize<S: serde::Serializer>(data: &BytesMut, s: S) -> Result<S::Ok, S::Error>
    {
        use serde::Serialize;

        if s.is_human_readable() {
            s.serialize_str(&base64::encode(&data))
        } else {
            data.serialize(s)
        }
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BytesMut, D::Error>
    {
        use serde::Deserialize;

        if d.is_human_readable() {
            String::deserialize(d)
                .and_then(|s| {
                    base64::decode(&s).map_err(|err| serde::de::Error::custom(err.to_string()))
                })
                .map(|b| BytesMut::from(b.as_slice()))
        } else {
            BytesMut::deserialize(d)
        }
    }
}
