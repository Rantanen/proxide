use uuid::Uuid;

use super::*;

pub trait ItemFilter<T>
{
    fn key(&self) -> FilterType;
    fn filter(&self, item: &T) -> bool;
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub enum FilterType
{
    Connection,
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
