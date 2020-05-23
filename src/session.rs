use crate::decoders::Decoder;
use bytes::BytesMut;
use chrono::prelude::*;
use http::{HeaderMap, Method, Uri};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use uuid::Uuid;

pub mod events;

#[derive(Serialize, Deserialize)]
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

#[derive(Serialize, Deserialize)]
pub struct ConnectionData
{
    pub uuid: Uuid,
    pub client_addr: SocketAddr,
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
    pub fn new() -> Self
    {
        Self {
            items: vec![],
            items_by_uuid: HashMap::new(),
        }
    }

    pub fn push(&mut self, uuid: Uuid, item: T)
    {
        self.items_by_uuid.insert(uuid, self.items.len());
        self.items.push(item);
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
