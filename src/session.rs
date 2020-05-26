use bytes::BytesMut;
use chrono::prelude::*;
use http::{HeaderMap, Method, Uri};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use uuid::Uuid;

pub mod events;
pub mod filters;
pub mod serialization;

use filters::{FilterType, ItemFilter};

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

    #[serde(skip)]
    pub filtered_items: Vec<usize>,

    #[serde(skip, default = "HashMap::new")]
    pub filters: HashMap<FilterType, Box<dyn ItemFilter<T>>>,
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

impl Session
{
    fn post_deserialize(&mut self)
    {
        self.requests.refilter();
    }
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
        if self.filters.iter().all(|(_, f)| f.filter(&item)) {
            self.filtered_items.push(self.items.len())
        }
        self.items.push(item);
    }

    pub fn get_index_by_uuid(&self, uuid: Uuid) -> Option<usize>
    {
        let idx = self.items_by_uuid.get(&uuid)?;
        Some(
            self.filtered_items
                .binary_search(&idx)
                .unwrap_or_else(|e| e),
        )
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

    pub fn len_filtered(&self) -> usize
    {
        self.filtered_items.len()
    }

    pub fn is_empty_filtered(&self) -> bool
    {
        self.filtered_items.is_empty()
    }

    pub fn get(&self, idx: usize) -> Option<&T>
    {
        self.filtered_items
            .get(idx)
            .and_then(|idx| self.items.get(*idx))
    }

    pub fn iter(&self) -> impl Iterator<Item = &T>
    {
        self.filtered_items.iter().map(move |idx| &self.items[*idx])
    }

    pub fn add_filter(&mut self, filter: Box<dyn ItemFilter<T>>)
    {
        self.filters.insert(filter.key(), filter);
        self.refilter();
    }

    pub fn remove_filter(&mut self, filter_type: FilterType)
    {
        self.filters.remove(&filter_type);
        self.refilter();
    }

    fn refilter(&mut self)
    {
        self.filtered_items = self
            .items
            .iter()
            .enumerate()
            .filter_map(
                |(idx, item)| match self.filters.iter().all(|(_, f)| f.filter(item)) {
                    true => Some(idx),
                    false => None,
                },
            )
            .collect();
    }

    fn post_deserialize(&mut self)
    {
        self.refilter()
    }
}

impl<T> std::ops::Index<usize> for IndexedVec<T>
{
    type Output = T;
    fn index(&self, index: usize) -> &Self::Output
    {
        &self.items[self.filtered_items[index]]
    }
}

impl<T> Default for IndexedVec<T>
{
    fn default() -> Self
    {
        Self {
            items: Default::default(),
            items_by_uuid: Default::default(),

            filtered_items: Default::default(),
            filters: Default::default(),
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
