use http::{HeaderMap, Method, Uri};
use std::net::SocketAddr;
use std::time::SystemTime;

use super::*;

#[derive(Serialize, Deserialize, Debug)]
pub enum SessionEvent
{
    NewConnection(NewConnectionEvent),
    NewRequest(NewRequestEvent),
    NewResponse(NewResponseEvent),
    MessageData(MessageDataEvent),
    MessageDone(MessageDoneEvent),
    RequestDone(RequestDoneEvent),
    ConnectionDone(ConnectionDoneEvent),
    ClientCallstackProcessed(ClientCallstackProcessedEvent),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct NewConnectionEvent
{
    pub uuid: Uuid,
    pub protocol_stack: Vec<Protocol>,
    pub client_addr: SocketAddr,
    pub timestamp: SystemTime,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct NewRequestEvent
{
    pub connection_uuid: Uuid,
    pub uuid: Uuid,
    #[serde(with = "http_serde::uri")]
    pub uri: Uri,
    #[serde(with = "http_serde::method")]
    pub method: Method,
    #[serde(with = "http_serde::header_map")]
    pub headers: HeaderMap,
    pub timestamp: SystemTime,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct NewResponseEvent
{
    pub connection_uuid: Uuid,
    pub uuid: Uuid,
    #[serde(with = "http_serde::header_map")]
    pub headers: HeaderMap,
    pub timestamp: SystemTime,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MessageDataEvent
{
    pub uuid: Uuid,
    pub data: bytes::Bytes,
    pub part: RequestPart,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MessageDoneEvent
{
    pub uuid: Uuid,
    pub part: RequestPart,
    pub status: Status,
    pub timestamp: SystemTime,
    #[serde(with = "super::serialization::opt_header_map")]
    pub trailers: Option<HeaderMap>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RequestDoneEvent
{
    pub uuid: Uuid,
    pub status: Status,
    pub timestamp: SystemTime,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ConnectionDoneEvent
{
    pub uuid: Uuid,
    pub status: Status,
    pub timestamp: SystemTime,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ClientCallstackProcessedEvent
{
    pub uuid: Uuid,
    pub callstack: ClientCallstack,
}

pub enum SessionChange
{
    NewConnection
    {
        connection: Uuid
    },
    NewRequest
    {
        connection: Uuid, request: Uuid
    },
    Request
    {
        request: Uuid
    },
    NewMessage
    {
        request: Uuid, part: RequestPart
    },
    Message
    {
        request: Uuid, part: RequestPart
    },
    Connection
    {
        connection: Uuid
    },
    Callstack
    {
        request: Uuid
    },
}

impl Session
{
    pub fn handle(&mut self, e: SessionEvent) -> Vec<SessionChange>
    {
        match e {
            SessionEvent::NewConnection(e) => self.on_new_connection(e),
            SessionEvent::NewRequest(e) => self.on_new_request(e),
            SessionEvent::NewResponse(e) => self.on_new_response(e),
            SessionEvent::MessageData(e) => self.on_message_data(e),
            SessionEvent::MessageDone(e) => self.on_message_done(e),
            SessionEvent::RequestDone(e) => self.on_request_done(e),
            SessionEvent::ConnectionDone(e) => self.on_connection_done(e),
            SessionEvent::ClientCallstackProcessed(e) => self.on_client_callstack_processed(e),
        }
    }

    fn on_new_connection(&mut self, e: NewConnectionEvent) -> Vec<SessionChange>
    {
        let data = ConnectionData {
            uuid: e.uuid,
            protocol_stack: e.protocol_stack,
            client_addr: e.client_addr,
            start_timestamp: e.timestamp.into(),
            end_timestamp: None,
            status: Status::InProgress,
        };
        self.connections.push(e.uuid, data);
        vec![SessionChange::NewConnection { connection: e.uuid }]
    }

    fn on_new_request(&mut self, e: NewRequestEvent) -> Vec<SessionChange>
    {
        self.requests.push(
            e.uuid,
            EncodedRequest {
                request_data: RequestData {
                    uuid: e.uuid,
                    connection_uuid: e.connection_uuid,
                    uri: e.uri,
                    method: e.method,
                    status: Status::InProgress,
                    start_timestamp: e.timestamp.into(),
                    end_timestamp: None,
                    client_callstack: None,
                },
                request_msg: MessageData::new(RequestPart::Request)
                    .with_headers(e.headers)
                    .with_start_timestamp(e.timestamp.into()),
                response_msg: MessageData::new(RequestPart::Response),
            },
        );
        vec![
            SessionChange::NewRequest {
                connection: e.connection_uuid,
                request: e.uuid,
            },
            SessionChange::NewMessage {
                request: e.uuid,
                part: RequestPart::Request,
            },
        ]
    }

    fn on_new_response(&mut self, e: NewResponseEvent) -> Vec<SessionChange>
    {
        let request = self.requests.get_mut_by_uuid(e.uuid);
        if let Some(request) = request {
            request.response_msg.headers = e.headers;
            request.response_msg.start_timestamp = Some(e.timestamp.into());
            vec![SessionChange::NewMessage {
                request: e.uuid,
                part: RequestPart::Response,
            }]
        } else {
            vec![]
        }
    }

    fn on_message_data(&mut self, e: MessageDataEvent) -> Vec<SessionChange>
    {
        let request = self.requests.get_mut_by_uuid(e.uuid);
        if let Some(request) = request {
            let part_msg = match e.part {
                RequestPart::Request => &mut request.request_msg,
                RequestPart::Response => &mut request.response_msg,
            };
            part_msg.content.extend(e.data);
            vec![SessionChange::Message {
                request: e.uuid,
                part: e.part,
            }]
        } else {
            vec![]
        }
    }

    fn on_message_done(&mut self, e: MessageDoneEvent) -> Vec<SessionChange>
    {
        let request = self.requests.get_mut_by_uuid(e.uuid);
        if let Some(request) = request {
            let part_msg = match e.part {
                RequestPart::Request => &mut request.request_msg,
                RequestPart::Response => &mut request.response_msg,
            };
            part_msg.end_timestamp = Some(e.timestamp.into());
            vec![SessionChange::Message {
                request: e.uuid,
                part: e.part,
            }]
        } else {
            vec![]
        }
    }

    fn on_request_done(&mut self, e: RequestDoneEvent) -> Vec<SessionChange>
    {
        let request = self.requests.get_mut_by_uuid(e.uuid);
        if let Some(request) = request {
            request.request_data.end_timestamp = Some(e.timestamp.into());
            request.request_data.status = e.status;
            vec![SessionChange::Request { request: e.uuid }]
        } else {
            vec![]
        }
    }

    fn on_connection_done(&mut self, e: ConnectionDoneEvent) -> Vec<SessionChange>
    {
        let conn = self.connections.get_mut_by_uuid(e.uuid);
        if let Some(conn) = conn {
            conn.end_timestamp = Some(e.timestamp.into());
            conn.status = e.status;
            vec![SessionChange::Connection { connection: e.uuid }]
        } else {
            vec![]
        }
    }

    fn on_client_callstack_processed(
        &mut self,
        e: ClientCallstackProcessedEvent,
    ) -> Vec<SessionChange>
    {
        let request = self.requests.get_mut_by_uuid(e.uuid);
        if let Some(request) = request {
            request.request_data.client_callstack = Some(e.callstack);
            vec![SessionChange::Callstack { request: e.uuid }]
        } else {
            vec![]
        }
    }
}
