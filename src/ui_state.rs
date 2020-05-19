use chrono::prelude::*;
use crossterm::event::{Event as CTEvent, KeyCode};
use http::{HeaderMap, Method, Uri};
use std::collections::{HashMap, LinkedList};
use std::net::SocketAddr;
use std::rc::Rc;
use tui::backend::Backend;
use tui::layout::{Constraint, Direction, Layout, Rect};
use tui::style::{Color, Modifier, Style};
use tui::terminal::Frame;
use tui::widgets::{Block, Borders, Paragraph, Row, Table, TableState, Text};
use uuid::Uuid;

use crate::decoders::{Decoder, GrpcDecoder, RawDecoder};
use crate::proto::Protobuf;

#[derive(Debug)]
pub enum UiEvent
{
    Crossterm(crossterm::event::Event),
    NewConnection(NewConnectionEvent),
    NewRequest(NewRequestEvent),
    ConnectionClosed
    {
        uuid: Uuid,
        status: Status,
    },
    RequestStatus(RequestStatusEvent),
    RequestData(RequestDataEvent),
    ResponseData(ResponseDataEvent),
    LogMessage(String),
}

#[derive(Debug)]
pub struct NewConnectionEvent
{
    pub uuid: Uuid,
    pub client_addr: SocketAddr,
}

#[derive(Debug)]
pub struct NewRequestEvent
{
    pub connection_uuid: Uuid,
    pub uuid: Uuid,
    pub uri: Uri,
    pub method: Method,
    pub headers: HeaderMap,
}

#[derive(Debug)]
pub struct RequestStatusEvent
{
    pub uuid: Uuid,
    pub status: Status,
}

#[derive(Debug)]
pub struct RequestDataEvent
{
    pub uuid: Uuid,
    pub data: bytes::Bytes,
}

#[derive(Debug)]
pub struct ResponseDataEvent
{
    pub uuid: Uuid,
    pub data: bytes::Bytes,
}

pub struct State
{
    pub connections: ProxideTable<ConnectionData>,
    pub requests: ProxideTable<RequestData>,
    pub active_window: Window,
    pub protobuf: Rc<Protobuf>,
    pub debug: DebugLog,
}

pub enum Window
{
    Connections,
    Requests,
}

pub enum HandleResult
{
    Ignore,
    Update,
    Quit,
}

impl State
{
    pub fn new(pb: Protobuf) -> State
    {
        State {
            connections: ProxideTable::new(),
            requests: ProxideTable::new(),
            active_window: Window::Requests,
            protobuf: Rc::new(pb),
            debug: DebugLog {
                msgs: LinkedList::new(),
            },
        }
    }

    pub fn handle(&mut self, e: UiEvent) -> HandleResult
    {
        match e {
            UiEvent::NewConnection(e) => self.on_new_connection(e),
            UiEvent::NewRequest(e) => self.on_new_request(e),
            UiEvent::RequestStatus(e) => self.on_request_status(e),
            UiEvent::RequestData(e) => {
                self.on_request_data(e);
                return HandleResult::Ignore;
            }
            UiEvent::ResponseData(e) => {
                self.on_response_data(e);
                return HandleResult::Ignore;
            }
            UiEvent::Crossterm(e) => return self.on_input(e),
            // UiEvent::LogMessage(m) => self.debug.msgs.push_back(m),
            _ => {}
        }

        return HandleResult::Update;
    }

    fn on_new_connection(&mut self, e: NewConnectionEvent)
    {
        let data = ConnectionData {
            uuid: e.uuid,
            client_addr: e.client_addr,
            start_timestamp: Local::now(),
            end_timestamp: None,
            status: Status::InProgress,
        };
        self.connections.push(e.uuid, data);
    }

    fn on_new_request(&mut self, e: NewRequestEvent)
    {
        let (req, resp) = match e.headers["Content-Type"].to_str().unwrap() {
            "application/grpc" => GrpcDecoder::decoders(&e.uri, self.protobuf.clone()),
            _ => RawDecoder::decoders(),
        };

        self.requests.push(
            e.uuid,
            RequestData {
                uuid: e.uuid,
                connection_uuid: e.connection_uuid,
                uri: e.uri,
                method: e.method,
                headers: e.headers,
                status: Status::Pending,
                start_timestamp: Local::now(),
                end_timestamp: None,
                request_size: 0,
                request_data: req,
                response_size: 0,
                response_data: resp,
            },
        );
    }

    fn on_request_status(&mut self, e: RequestStatusEvent)
    {
        let request = self.requests.get_mut_by_uuid(e.uuid);
        if let Some(request) = request {
            if e.status == Status::Failed || e.status == Status::Succeeded {
                request.end_timestamp = Some(Local::now());
            }
            request.status = e.status;
        }
    }

    fn on_request_data(&mut self, e: RequestDataEvent)
    {
        let request = self.requests.get_mut_by_uuid(e.uuid);
        if let Some(request) = request {
            request.request_size += e.data.len();
            request.request_data.extend(e.data);
        }
    }

    fn on_response_data(&mut self, e: ResponseDataEvent)
    {
        let request = self.requests.get_mut_by_uuid(e.uuid);
        if let Some(request) = request {
            request.response_size += e.data.len();
            request.response_data.extend(e.data);
        }
    }

    fn on_input(&mut self, e: CTEvent) -> HandleResult
    {
        // Handle active window input first.
        let handled = match self.active_window {
            Window::Connections => self.connections.on_input(e),
            Window::Requests => self.requests.on_input(e),
        };

        if !handled {
            match e {
                CTEvent::Key(key) => match key.code {
                    KeyCode::Char('q') => return HandleResult::Quit,
                    _ => {}
                },
                _ => {}
            };
        }

        HandleResult::Update
    }

    pub fn draw<B: Backend>(&mut self, mut f: Frame<B>)
    {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .margin(0)
            .constraints(
                [
                    // Constraint::Length(55),
                    Constraint::Length(70),
                    Constraint::Percentage(100),
                ]
                .as_ref(),
            )
            .split(f.size());

        // self.connections.draw(&mut f, chunks[0]);
        self.requests.draw(&mut f, chunks[0]);

        if let Some(request) = self.requests.selected_mut() {
            request.draw(&mut f, chunks[1]);
        }
    }
}

pub struct ProxideTable<T>
{
    items: Vec<T>,
    items_by_uuid: HashMap<Uuid, usize>,
    state: TableState,
    user_selected: Option<usize>,
}

impl<T> ProxideTable<T>
{
    fn new() -> Self
    {
        Self {
            items: vec![],
            items_by_uuid: HashMap::new(),
            state: TableState::default(),
            user_selected: None,
        }
    }

    fn push(&mut self, uuid: Uuid, item: T)
    {
        self.items_by_uuid.insert(uuid, self.items.len());
        self.items.push(item);

        if self.user_selected.is_none() {
            self.state.select(Some(self.items.len() - 1))
        }
    }

    fn get_mut_by_uuid(&mut self, uuid: Uuid) -> Option<&mut T>
    {
        let idx = self.items_by_uuid.get(&uuid)?;
        self.items.get_mut(*idx)
    }

    fn on_input(&mut self, e: CTEvent) -> bool
    {
        match e {
            CTEvent::Key(key) => match key.code {
                KeyCode::Char('k') | KeyCode::Up => self.user_select(
                    self.user_selected
                        .or_else(|| self.state.selected())
                        .map(|i| i.saturating_sub(1)),
                ),
                KeyCode::Char('j') | KeyCode::Down => self.user_select(
                    self.user_selected
                        .or_else(|| self.state.selected())
                        .map(|i| i + 1),
                ),
                KeyCode::Esc => self.user_select(None),
                _ => return false,
            },
            _ => return false,
        };
        true
    }

    fn user_select(&mut self, idx: Option<usize>)
    {
        match idx {
            None => {
                self.user_selected = None;
                if self.items.is_empty() {
                    self.state.select(None);
                } else {
                    self.state.select(Some(self.items.len() - 1));
                }
            }
            Some(mut idx) => {
                if idx >= self.items.len() {
                    idx = self.items.len() - 1;
                }
                self.user_selected = Some(idx);
                self.state.select(self.user_selected);
            }
        }
    }

    fn selected_mut(&mut self) -> Option<&mut T>
    {
        if let Some(idx) = self.state.selected() {
            Some(&mut self.items[idx])
        } else {
            None
        }
    }
}

impl ProxideTable<ConnectionData>
{
    pub fn draw<B: Backend>(&mut self, f: &mut Frame<B>, chunk: Rect)
    {
        let block = Block::default()
            .title("[C]onnections")
            .borders(Borders::ALL);
        let table = Table::new(
            ["Source", "Timestamp", "Status"].iter(),
            self.items.iter().map(|item| {
                Row::Data(
                    vec![
                        item.client_addr.to_string(),
                        item.start_timestamp.format("%H:%M:%S").to_string(),
                        item.status.to_string(),
                    ]
                    .into_iter(),
                )
            }),
        )
        .block(block)
        .widths(&[
            Constraint::Length(25),
            Constraint::Percentage(50),
            Constraint::Length(15),
        ])
        .highlight_symbol("> ")
        .highlight_style(Style::default().modifier(Modifier::BOLD));

        f.render_stateful_widget(table, chunk, &mut self.state)
    }
}

impl ProxideTable<RequestData>
{
    pub fn draw<B: Backend>(&mut self, f: &mut Frame<B>, chunk: Rect)
    {
        let block = Block::default().title("[R]equests").borders(Borders::ALL);
        let table = Table::new(
            ["Method", "Uri", "Timestamp", "Status"].iter(),
            self.items.iter().map(|item| {
                Row::Data(
                    vec![
                        item.method.to_string(),
                        match item.uri.path_and_query() {
                            Some(p) => p.to_string(),
                            None => "/".to_string(),
                        },
                        item.start_timestamp.format("%H:%M:%S").to_string(),
                        item.status.to_string(),
                    ]
                    .into_iter(),
                )
            }),
        )
        .block(block)
        .widths(&[
            Constraint::Length(10),
            Constraint::Percentage(100),
            Constraint::Length(10),
            Constraint::Length(15),
        ])
        .highlight_symbol("> ")
        .highlight_style(Style::default().modifier(Modifier::BOLD));

        f.render_stateful_widget(table, chunk, &mut self.state)
    }
}

impl RequestData
{
    pub fn draw<B: Backend>(&mut self, f: &mut Frame<B>, chunk: Rect)
    {
        let block = Block::default()
            .title("Request [D]etails")
            .borders(Borders::ALL);
        f.render_widget(block, chunk);

        let details_chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(0)
            .constraints([Constraint::Length(6), Constraint::Percentage(50)].as_ref())
            .split(block.inner(chunk));
        let mut c = details_chunks[1];
        c.x -= 1;
        c.width += 2;
        c.height += 1;
        let req_resp_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .margin(0)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .split(block.inner(c));

        let duration = match self.end_timestamp {
            None => "(Pending)".to_string(),
            Some(s) => (s - self.start_timestamp).to_string(),
        };

        let text = vec![
            Text::raw("\n"),
            Text::raw(format!(" Method:     {}\n", self.method)),
            Text::raw(format!(" URI:        {}\n", self.uri)),
            Text::raw(format!(
                " Timestamp:  {}\n",
                self.start_timestamp.to_string()
            )),
            Text::raw(format!(" Status:     {}\n", self.status.to_string())),
            Text::raw(format!(" Duration:   {}\n", duration)),
        ];
        let details = Paragraph::new(text.iter());
        f.render_widget(details, details_chunks[0]);

        let text = self.request_data.as_text();
        let request_title = format!("Request Data ({} bytes)", self.request_size);
        let request_data = Paragraph::new(text.iter())
            .block(Block::default().title(&request_title).borders(Borders::ALL))
            .wrap(false);
        f.render_widget(request_data, req_resp_chunks[0]);

        let text = self.response_data.as_text();
        let response_title = format!("Response Data ({} bytes)", self.response_size);
        let response_data = Paragraph::new(text.iter())
            .block(
                Block::default()
                    .title(&response_title)
                    .borders(Borders::ALL),
            )
            .wrap(false);
        f.render_widget(response_data, req_resp_chunks[1]);
    }
}

pub struct ConnectionData
{
    uuid: Uuid,
    client_addr: SocketAddr,
    start_timestamp: DateTime<Local>,
    end_timestamp: Option<DateTime<Local>>,
    status: Status,
}

pub struct RequestData
{
    uuid: Uuid,
    connection_uuid: Uuid,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    start_timestamp: DateTime<Local>,
    end_timestamp: Option<DateTime<Local>>,
    status: Status,
    request_size: usize,
    request_data: Box<dyn Decoder>,
    response_size: usize,
    response_data: Box<dyn Decoder>,
}

pub struct DebugLog
{
    msgs: LinkedList<String>,
}

impl DebugLog
{
    pub fn draw<B: Backend>(&mut self, f: &mut Frame<B>, chunk: Rect)
    {
        let size = chunk.height - 2;
        while self.msgs.len() > size as usize {
            self.msgs.pop_front();
        }

        let text: Vec<_> = self.msgs.iter().map(|msg| Text::raw(msg)).collect();
        let messages = Paragraph::new(text.iter())
            .block(Block::default().title("Debug Log").borders(Borders::ALL));
        f.render_widget(messages, chunk);
    }
}

pub enum Protocol
{
    Unknown,
    Grpc(String, String),
}

#[derive(Debug, PartialEq)]
pub enum Status
{
    Pending,
    InProgress,
    Succeeded,
    Failed,
}

impl std::fmt::Display for Status
{
    fn fmt(&self, w: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error>
    {
        match self {
            Status::Pending => write!(w, "Pending"),
            Status::InProgress => write!(w, "In progress"),
            Status::Succeeded => write!(w, "Succeeded"),
            Status::Failed => write!(w, "Failed"),
        }
    }
}
