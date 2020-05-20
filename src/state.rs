
#[derive(Debug)]
pub enum UiEvent
{
    NewConnection
    {
        uuid: Uuid, client_addr: SocketAddr
    },
    NewRequest
    {
        uuid: Uuid, connection_uuid: Uuid
    },
    ConnectionClosed
    {
        uuid: Uuid, status: context::Status
    },
    RequestStatus
    {
        uuid: Uuid, status: context::Status
    },
    RequestData
    {
        uuid: Uuid, data: bytes::Bytes
    },
    ResponseData
    {
        uuid: Uuid, data: bytes::Bytes
    },
}

