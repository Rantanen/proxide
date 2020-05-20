use bytes::BytesMut;
use http::HeaderMap;
use snafu::Snafu;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, oneshot};

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum ContextError
{
    #[snafu(display("Invalid operation: {}", description))]
    InvalidOperation
    {
        description: &'static str
    },
}

pub type Result<S, E = ContextError> = std::result::Result<S, E>;

pub struct Context
{
    sender: broadcast::Sender<Event>,
    abort_tx: Mutex<Option<oneshot::Sender<()>>>,
    connections: Mutex<Vec<Arc<ConnectionDetails>>>,
}

impl Context
{
    pub fn new(abort_tx: oneshot::Sender<()>) -> Context
    {
        let (tx, mut rx) = broadcast::channel(16);

        tokio::spawn(async move {
            loop {
                let event = rx.recv().await.unwrap();
                println!("{:#?}", event);
            }
        });

        Self {
            sender: tx,
            abort_tx: Mutex::new(Some(abort_tx)),
            connections: Mutex::new(vec![]),
        }
    }

    pub fn emit(&self, event: Event)
    {
        // We don't care for whether anyone listens to our updates. We're here
        // just to broadcast them for anyone who does care.
        let _ = self.sender.send(event);
    }

    pub fn new_connection(&self, connection: Arc<ConnectionDetails>)
    {
        let mut guard = self.connections.lock().expect("Mutex poioned");
        guard.push(connection.clone());
        self.emit(Event::NewConnection(connection.clone()));
    }

    pub fn stop(&self) -> Result<()>
    {
        let mut guard = self.abort_tx.lock().expect("Mutex poisoned");
        let tx = guard.take();
        match tx {
            Some(tx) => {
                if tx.send(()).is_err() {
                    Err(ContextError::InvalidOperation {
                        description: "No listener for stop",
                    })
                } else {
                    Ok(())
                }
            }
            None => Err(ContextError::InvalidOperation {
                description: "Context already stopped",
            }),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event
{
    NewConnection(Arc<ConnectionDetails>),
    NewRequest(Arc<RequestDetails>),
    ConnectionUpdated(Arc<ConnectionDetails>),
    RequestUpdated(Arc<RequestDetails>),
    MessageDetailsUpdated(Arc<Mutex<MessageDetails>>),
}

#[derive(Debug)]
pub struct ConnectionDetails
{
    pub addr: SocketAddr,
    pub status: Mutex<Status>,
    pub requests: Mutex<Vec<Arc<RequestDetails>>>,
}

#[derive(Debug)]
pub struct RequestDetails
{
    pub status: Mutex<Status>,
    pub request: Arc<Mutex<MessageDetails>>,
    pub response: Arc<Mutex<MessageDetails>>,
}

#[derive(Debug)]
pub struct MessageDetails
{
    pub status: Status,
    pub headers: MessageHeaders,
    pub content: BytesMut,
    pub trailers: MessageHeaders,
}

impl RequestDetails
{
    pub fn new(headers: &HeaderMap) -> RequestDetails
    {
        RequestDetails {
            status: Mutex::new(Status::Pending),
            request: Arc::new(Mutex::new(MessageDetails::from_headers(headers))),
            response: Arc::new(Mutex::new(MessageDetails::new())),
        }
    }
}
impl HasStatus for ConnectionDetails
{
    fn get_status(&self) -> &Mutex<Status>
    {
        &self.status
    }
}
impl HasStatus for RequestDetails
{
    fn get_status(&self) -> &Mutex<Status>
    {
        &self.status
    }
}

impl MessageDetails
{
    pub fn new() -> MessageDetails
    {
        MessageDetails {
            status: Status::Pending,
            headers: Default::default(),
            content: Default::default(),
            trailers: Default::default(),
        }
    }

    fn from_headers(headers: &HeaderMap) -> MessageDetails
    {
        MessageDetails {
            status: Status::Pending,
            headers: headers.into(),
            content: Default::default(),
            trailers: Default::default(),
        }
    }
}

impl Default for Status
{
    fn default() -> Status
    {
        Status::Pending
    }
}

#[derive(Debug, Default)]
pub struct MessageHeaders
{
    pub map: HeaderMap,
}

impl From<&HeaderMap> for MessageHeaders
{
    fn from(headers: &HeaderMap) -> MessageHeaders
    {
        MessageHeaders {
            map: headers.clone(),
        }
    }
}

impl MessageHeaders
{
    pub fn append(&mut self, headers: &http::header::HeaderMap)
    {
        self.map.extend(headers.clone());
    }
}

pub trait HasStatus
{
    fn get_status(&self) -> &Mutex<Status>;
}

pub trait UpdateStatus: HasStatus
{
    fn set_status<S, E>(&self, r: &Result<S, E>)
    {
        let mut guard = self.get_status().lock().expect("Mutex poisoned");
        match r {
            Ok(_) => *guard = Status::Succeeded,
            Err(_) => *guard = Status::Failed,
        }
    }
}
impl<T: HasStatus> UpdateStatus for T {}
