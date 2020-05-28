use std::collections::HashMap;
use uuid::Uuid;

use crate::decoders::Decoders;
use crate::session::*;

pub struct SearchIndex
{
    requests: HashMap<Uuid, IndexedRequest>,
}

pub enum IndexRequest
{
    Message
    {
        request: Uuid, part: RequestPart
    },
}

struct IndexedRequest
{
    // request: Uuid,
    request_msg: IndexedMessage,
    response_msg: IndexedMessage,
}

struct IndexedMessage
{
    // part: RequestPart,
    data: Vec<String>,
}

impl SearchIndex
{
    pub fn new(session: &Session, decoders: &Decoders) -> Self
    {
        let mut idx = SearchIndex {
            requests: Default::default(),
        };

        for r in session.requests.iter() {
            idx.index_message(session, decoders, r.request_data.uuid, RequestPart::Request);
            idx.index_message(
                session,
                decoders,
                r.request_data.uuid,
                RequestPart::Response,
            );
        }

        idx
    }

    pub fn is_match(&self, request: Uuid, pattern: &str) -> bool
    {
        self.requests
            .get(&request)
            .map(|r| {
                r.request_msg.data.iter().any(|text| text.contains(pattern))
                    || r.response_msg
                        .data
                        .iter()
                        .any(|text| text.contains(pattern))
            })
            .unwrap_or(false)
    }

    pub fn index(&mut self, session: &Session, decoders: &Decoders, request: IndexRequest)
    {
        match request {
            IndexRequest::Message {
                request: uuid,
                part,
            } => self.index_message(session, decoders, uuid, part),
        }
    }

    fn index_message(
        &mut self,
        session: &Session,
        decoders: &Decoders,
        request: Uuid,
        part: RequestPart,
    )
    {
        let session_request = match session.requests.get_by_uuid(request) {
            Some(r) => r,
            None => return,
        };

        let req = self
            .requests
            .entry(request)
            .or_insert_with(|| IndexedRequest::new(request));

        let (msg, data) = match part {
            RequestPart::Request => (&mut req.request_msg, &session_request.request_msg),
            RequestPart::Response => (&mut req.response_msg, &session_request.response_msg),
        };

        msg.data = decoders.index(&session_request.request_data, data);
    }
}

impl IndexedRequest
{
    fn new(_uuid: Uuid) -> Self
    {
        Self {
            // request: uuid,
            request_msg: IndexedMessage::new(RequestPart::Request),
            response_msg: IndexedMessage::new(RequestPart::Response),
        }
    }
}

impl IndexedMessage
{
    fn new(_part: RequestPart) -> Self
    {
        Self {
            // part,
            data: Default::default(),
        }
    }
}
