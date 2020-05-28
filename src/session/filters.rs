use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

use super::*;
use crate::search::SearchIndex;

pub trait ItemFilter<T>
{
    fn key(&self) -> FilterType;
    fn filter(&self, item: &T) -> bool;
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub enum FilterType
{
    Connection,
    Search,
}

pub struct SearchFilter
{
    pub pattern: String,
    pub index: Rc<RefCell<SearchIndex>>,
}

impl ItemFilter<EncodedRequest> for SearchFilter
{
    fn key(&self) -> FilterType
    {
        FilterType::Search
    }

    fn filter(&self, item: &EncodedRequest) -> bool
    {
        self.index
            .borrow()
            .is_match(item.request_data.uuid, &self.pattern)
    }
}

pub struct ConnectionFilter
{
    pub connection: Uuid,
}

impl ItemFilter<EncodedRequest> for ConnectionFilter
{
    fn key(&self) -> FilterType
    {
        FilterType::Connection
    }

    fn filter(&self, item: &EncodedRequest) -> bool
    {
        item.request_data.connection_uuid == self.connection
    }
}
