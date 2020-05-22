use chrono::prelude::*;
use http::{HeaderMap, Method, Uri};
use std::net::SocketAddr;

use super::*;

#[derive(Debug)]
pub enum SessionEvent
{
    NewConnection(NewConnectionEvent),
    NewRequest(NewRequestEvent),
    NewResponse(NewResponseEvent),
    ConnectionClosed
    {
        uuid: Uuid,
        status: Status,
    },
    MessageData(MessageDataEvent),
    MessageDone(MessageDoneEvent),
    RequestDone(RequestDoneEvent),
}

#[derive(Debug)]
pub struct NewConnectionEvent
{
    pub uuid: Uuid,
    pub client_addr: SocketAddr,
    pub timestamp: DateTime<Local>,
}

#[derive(Debug)]
pub struct NewRequestEvent
{
    pub connection_uuid: Uuid,
    pub uuid: Uuid,
    pub uri: Uri,
    pub method: Method,
    pub headers: HeaderMap,
    pub timestamp: DateTime<Local>,
}

#[derive(Debug)]
pub struct NewResponseEvent
{
    pub connection_uuid: Uuid,
    pub uuid: Uuid,
    pub headers: HeaderMap,
    pub timestamp: DateTime<Local>,
}

#[derive(Debug)]
pub struct MessageDataEvent
{
    pub uuid: Uuid,
    pub data: bytes::Bytes,
    pub part: RequestPart,
}

#[derive(Debug)]
pub struct MessageDoneEvent
{
    pub uuid: Uuid,
    pub part: RequestPart,
    pub status: Status,
    pub timestamp: DateTime<Local>,
    pub trailers: Option<HeaderMap>,
}

#[derive(Debug)]
pub struct RequestDoneEvent
{
    pub uuid: Uuid,
    pub status: Status,
    pub timestamp: DateTime<Local>,
}

impl Session
{
    pub fn handle(&mut self, e: SessionEvent)
    {
        match e {
            SessionEvent::NewConnection(e) => self.on_new_connection(e),
            SessionEvent::NewRequest(e) => self.on_new_request(e),
            SessionEvent::NewResponse(e) => self.on_new_response(e),
            SessionEvent::MessageData(e) => self.on_message_data(e),
            SessionEvent::MessageDone(e) => self.on_message_done(e),
            SessionEvent::RequestDone(e) => self.on_request_done(e),
            SessionEvent::ConnectionClosed { .. } => {}
        }
    }

    fn on_new_connection(&mut self, e: NewConnectionEvent)
    {
        let data = ConnectionData {
            uuid: e.uuid,
            client_addr: e.client_addr,
            start_timestamp: e.timestamp,
            end_timestamp: None,
            status: Status::InProgress,
        };
        self.connections.push(e.uuid, data);
    }

    fn on_new_request(&mut self, e: NewRequestEvent)
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
                    start_timestamp: e.timestamp,
                    end_timestamp: None,
                },
                request_msg: EncodedMessage::new(RequestPart::Request)
                    .with_headers(e.headers)
                    .with_start_timestamp(e.timestamp),
                response_msg: EncodedMessage::new(RequestPart::Response),
            },
        );
    }

    fn on_new_response(&mut self, e: NewResponseEvent)
    {
        let request = self.requests.get_mut_by_uuid(e.uuid);
        if let Some(request) = request {
            request.response_msg.data.headers = e.headers;
            request.response_msg.data.start_timestamp = Some(e.timestamp);
        }
    }

    fn on_message_data(&mut self, e: MessageDataEvent)
    {
        let request = self.requests.get_mut_by_uuid(e.uuid);
        if let Some(request) = request {
            let part_msg = match e.part {
                RequestPart::Request => &mut request.request_msg,
                RequestPart::Response => &mut request.response_msg,
            };
            part_msg.data.content.extend(e.data);
        }
    }

    fn on_message_done(&mut self, e: MessageDoneEvent)
    {
        let request = self.requests.get_mut_by_uuid(e.uuid);
        if let Some(request) = request {
            let part_msg = match e.part {
                RequestPart::Request => &mut request.request_msg,
                RequestPart::Response => &mut request.response_msg,
            };
            part_msg.data.end_timestamp = Some(e.timestamp);
        }
    }

    fn on_request_done(&mut self, e: RequestDoneEvent)
    {
        let request = self.requests.get_mut_by_uuid(e.uuid);
        if let Some(request) = request {
            request.request_data.end_timestamp = Some(e.timestamp);
            request.request_data.status = e.status;
        }
    }
}
