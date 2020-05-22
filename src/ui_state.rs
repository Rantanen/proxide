use bytes::BytesMut;
use chrono::{prelude::*, Duration};
use crossterm::event::{Event as CTEvent, KeyCode};
use http::{HeaderMap, Method, Uri};
use std::collections::HashMap;
use std::net::SocketAddr;
use tui::backend::Backend;
use tui::buffer::Buffer;
use tui::layout::{Constraint, Direction, Layout, Rect};
use tui::style::{Modifier, Style};
use tui::terminal::Frame;
use tui::widgets::{Block, Borders, Paragraph, Row, Table, TableState, Text, Widget};
use uuid::Uuid;

use crate::decoders::{Decoder, DecoderFactory};

#[derive(Debug)]
pub enum UiEvent
{
    Crossterm(crossterm::event::Event),
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

pub struct ProxideUi<B>
{
    pub state: State,
    pub ui_stack: Vec<Box<dyn View<B>>>,
    pub size: Rect,
}

pub struct State
{
    pub connections: ProxideTable<ConnectionData>,
    pub requests: ProxideTable<EncodedRequest>,
    pub active_window: Window,
    pub decoder_factories: Vec<Box<dyn DecoderFactory>>,
}

#[derive(PartialEq)]
pub enum Window
{
    _Connections,
    Requests,
    Details,
}

pub enum HandleResult<B: Backend>
{
    Ignore,
    Update,
    Quit,
    PushView(Box<dyn View<B>>),
}

pub trait View<B: Backend>
{
    fn draw(&mut self, state: &mut State, f: &mut Frame<B>, chunk: Rect);
    fn on_input(&mut self, state: &mut State, e: CTEvent, size: Rect) -> HandleResult<B>;
    fn help_text(&self, state: &State, size: Rect) -> String;
}

#[derive(Default)]
struct MainView
{
    details_view: DetailsView,
}

impl<B: Backend> View<B> for MainView
{
    fn draw(&mut self, state: &mut State, f: &mut Frame<B>, chunk: Rect)
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
            .split(chunk);

        // state.connections.draw(&mut f, chunks[0]);
        state
            .requests
            .draw(f, chunks[0], state.active_window == Window::Requests);

        self.details_view.draw(state, f, chunks[1]);
    }

    fn on_input(&mut self, state: &mut State, e: CTEvent, size: Rect) -> HandleResult<B>
    {
        // Handle active window input first.
        let handled = match state.active_window {
            Window::_Connections => state.connections.on_input::<B>(e, size),
            Window::Requests => state.requests.on_input(e, size),
            Window::Details => HandleResult::Ignore,
        };

        if let HandleResult::Ignore = handled {
            match e {
                CTEvent::Key(key) => match key.code {
                    KeyCode::Char('r') => state.active_window = Window::Requests,
                    KeyCode::Char('d') => state.active_window = Window::Details,
                    KeyCode::Char('q') => {
                        return HandleResult::PushView(Box::new(MessageView(true, 0)))
                    }
                    KeyCode::Char('s') => {
                        return HandleResult::PushView(Box::new(MessageView(false, 0)))
                    }
                    _ => return HandleResult::Ignore,
                },
                _ => return HandleResult::Ignore,
            };
        }

        HandleResult::Update
    }

    fn help_text(&self, _state: &State, _size: Rect) -> String
    {
        "Up/Down, j/k: Move up/down".to_string()
    }
}

#[derive(Clone, Default)]
struct DetailsView;
impl<B: Backend> View<B> for DetailsView
{
    fn draw(&mut self, state: &mut State, f: &mut Frame<B>, chunk: Rect)
    {
        let request = match state.requests.selected_mut() {
            Some(r) => r,
            None => return,
        };

        let block = create_block("[D]etails", state.active_window == Window::Details);
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

        let duration = match request.request_data.end_timestamp {
            None => "(Pending)".to_string(),
            Some(end) => format_duration(end - request.request_data.start_timestamp),
        };

        let text = vec![
            Text::raw("\n"),
            Text::raw(format!(" Method:     {}\n", request.request_data.method)),
            Text::raw(format!(" URI:        {}\n", request.request_data.uri)),
            Text::raw(format!(
                " Timestamp:  {}\n",
                request.request_data.start_timestamp.to_string()
            )),
            Text::raw(format!(
                " Status:     {}\n",
                request.request_data.status.to_string()
            )),
            Text::raw(format!(" Duration:   {}\n", duration)),
        ];
        let details = Paragraph::new(text.iter());
        f.render_widget(details, details_chunks[0]);

        request.request_msg.draw(
            &state.decoder_factories,
            &request.request_data,
            "Re[q]uest Data",
            f,
            req_resp_chunks[0],
            false,
            0,
        );
        request.response_msg.draw(
            &state.decoder_factories,
            &request.request_data,
            "Re[s]ponse Data",
            f,
            req_resp_chunks[1],
            false,
            0,
        );
    }

    fn on_input(&mut self, _state: &mut State, _e: CTEvent, _size: Rect) -> HandleResult<B>
    {
        HandleResult::Ignore
    }

    fn help_text(&self, _state: &State, _size: Rect) -> String
    {
        String::new()
    }
}

struct MessageView(bool, u16);
impl<B: Backend> View<B> for MessageView
{
    fn draw(&mut self, state: &mut State, f: &mut Frame<B>, chunk: Rect)
    {
        let request = match state.requests.selected_mut() {
            Some(r) => r,
            None => return,
        };

        let (title, data) = match self.0 {
            true => ("Request Data", &mut request.request_msg),
            false => ("Response Data", &mut request.response_msg),
        };
        let title = format!("{} (offset {})", title, self.1);
        data.draw(
            &state.decoder_factories,
            &request.request_data,
            &title,
            f,
            chunk,
            true,
            self.1,
        );
    }

    fn on_input(&mut self, _state: &mut State, e: CTEvent, size: Rect) -> HandleResult<B>
    {
        match e {
            CTEvent::Key(key) => match key.code {
                KeyCode::Char('k') | KeyCode::Up => self.1 = self.1.saturating_sub(1),
                KeyCode::Char('j') | KeyCode::Down => self.1 = self.1.saturating_add(1),
                KeyCode::PageDown => self.1 = self.1.saturating_add(size.height - 5),
                KeyCode::PageUp => self.1 = self.1.saturating_sub(size.height - 5),
                KeyCode::Tab => self.0 = !self.0,
                _ => return HandleResult::Ignore,
            },
            _ => return HandleResult::Ignore,
        };
        HandleResult::Update
    }

    fn help_text(&self, _state: &State, _size: Rect) -> String
    {
        "Up/Down, j/k, PgUp/PgDn: Scroll; Tab: Switch Request/Response".to_string()
    }
}

impl<B: Backend> ProxideUi<B>
{
    pub fn new(decoders: Vec<Box<dyn DecoderFactory>>, size: Rect) -> Self
    {
        Self {
            state: State {
                connections: ProxideTable::new(),
                requests: ProxideTable::new(),
                active_window: Window::Requests,
                decoder_factories: decoders,
            },
            ui_stack: vec![Box::new(MainView::default())],
            size,
        }
    }

    pub fn handle(&mut self, e: UiEvent) -> HandleResult<B>
    {
        match e {
            UiEvent::NewConnection(e) => self.state.on_new_connection(e),
            UiEvent::NewRequest(e) => self.state.on_new_request(e),
            UiEvent::NewResponse(e) => self.state.on_new_response(e),
            // UiEvent::RequestStatus(e) => self.state.on_request_status(e),
            UiEvent::MessageData(e) => self.state.on_message_data(e),
            UiEvent::MessageDone(e) => self.state.on_message_done(e),
            UiEvent::RequestDone(e) => self.state.on_request_done(e),
            UiEvent::ConnectionClosed { .. } => {}
            UiEvent::Crossterm(e) => return self.on_input(e, self.size),
            // UiEvent::LogMessage(m) => self.debug.msgs.push_back(m),
        }

        return HandleResult::Update;
    }

    fn on_input(&mut self, e: CTEvent, size: Rect) -> HandleResult<B>
    {
        match self
            .ui_stack
            .last_mut()
            .unwrap()
            .on_input(&mut self.state, e, size)
        {
            r @ HandleResult::Update | r @ HandleResult::Quit => return r,
            HandleResult::Ignore => {}
            HandleResult::PushView(v) => {
                self.ui_stack.push(v);
                return HandleResult::Update;
            }
        }

        match e {
            CTEvent::Resize(width, height) => {
                self.size = Rect {
                    x: 0,
                    y: 0,
                    width,
                    height,
                };
                HandleResult::Update
            }
            CTEvent::Key(key) => match key.code {
                KeyCode::Char('Q') => return HandleResult::Quit,
                KeyCode::Esc => {
                    if self.ui_stack.len() > 1 {
                        self.ui_stack.pop();
                    }
                    HandleResult::Update
                }
                _ => HandleResult::Ignore,
            },
            _ => HandleResult::Ignore,
        }
    }

    pub fn draw(&mut self, mut f: Frame<B>)
    {
        let chunk = f.size();

        let view_chunk = Rect {
            x: 0,
            y: 0,
            width: chunk.width,
            height: chunk.height - 2,
        };
        let view = self.ui_stack.last_mut().unwrap();
        view.draw(&mut self.state, &mut f, view_chunk);

        let help_chunk = Rect {
            x: 1,
            y: chunk.height - 2,
            width: chunk.width - 2,
            height: 1,
        };
        let help_text = view.help_text(&self.state, self.size);
        let help_line = TextLine(&help_text);
        f.render_widget(help_line, help_chunk);
    }
}

impl State
{
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
            request.response_msg.ui_state = None;
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
            part_msg.ui_state = None;
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
            part_msg.ui_state = None;
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

    fn on_input<B: Backend>(&mut self, e: CTEvent, _size: Rect) -> HandleResult<B>
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
                _ => return HandleResult::Ignore,
            },
            _ => return HandleResult::Ignore,
        };
        HandleResult::Update
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
    pub fn _draw<B: Backend>(&mut self, f: &mut Frame<B>, chunk: Rect, is_active: bool)
    {
        let block = create_block("[C]onnections", is_active);
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

impl ProxideTable<EncodedRequest>
{
    pub fn draw<B: Backend>(&mut self, f: &mut Frame<B>, chunk: Rect, is_active: bool)
    {
        let block = create_block("[R]equests", is_active);
        let table = Table::new(
            ["Request", "Timestamp", "St."].iter(),
            self.items.iter().map(|item| {
                Row::Data(
                    vec![
                        format!(
                            "{} {}",
                            item.request_data.method,
                            match item.request_data.uri.path_and_query() {
                                Some(p) => p.to_string(),
                                None => "/".to_string(),
                            }
                        ),
                        item.request_data
                            .start_timestamp
                            .format("%H:%M:%S")
                            .to_string(),
                        item.request_data.status.to_string(),
                    ]
                    .into_iter(),
                )
            }),
        )
        .block(block)
        .widths(&[
            Constraint::Percentage(100),
            Constraint::Length(10),
            Constraint::Length(5),
        ])
        .highlight_symbol("> ")
        .highlight_style(Style::default().modifier(Modifier::BOLD));

        f.render_stateful_widget(table, chunk, &mut self.state)
    }
}

pub struct ConnectionData
{
    pub uuid: Uuid,
    pub client_addr: SocketAddr,
    pub start_timestamp: DateTime<Local>,
    pub end_timestamp: Option<DateTime<Local>>,
    pub status: Status,
}

pub struct RequestData
{
    pub uuid: Uuid,
    pub connection_uuid: Uuid,
    pub method: Method,
    pub uri: Uri,
    pub start_timestamp: DateTime<Local>,
    pub end_timestamp: Option<DateTime<Local>>,
    pub status: Status,
}

pub struct EncodedRequest
{
    request_data: RequestData,
    request_msg: EncodedMessage,
    response_msg: EncodedMessage,
}

pub struct EncodedMessage
{
    pub data: MessageData,
    ui_state: Option<MessageDataUiState>,
}

impl EncodedMessage
{
    fn new(part: RequestPart) -> Self
    {
        Self {
            ui_state: Default::default(),
            data: MessageData::new(part),
        }
    }

    fn with_headers(mut self, h: HeaderMap) -> Self
    {
        self.data.headers = h;
        self
    }

    fn with_start_timestamp(mut self, ts: DateTime<Local>) -> Self
    {
        self.data.start_timestamp = Some(ts);
        self
    }
}

pub struct MessageData
{
    pub headers: HeaderMap,
    pub trailers: HeaderMap,
    pub content: BytesMut,
    pub start_timestamp: Option<DateTime<Local>>,
    pub end_timestamp: Option<DateTime<Local>>,
    pub part: RequestPart,
}

impl MessageData
{
    fn new(part: RequestPart) -> Self
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
}

struct MessageDataUiState
{
    decoders: Vec<Box<dyn Decoder>>,
    active_decoder: usize,
}

impl EncodedMessage
{
    fn draw<B: Backend>(
        &mut self,
        decoders: &[Box<dyn DecoderFactory>],
        request: &RequestData,
        title: &str,
        f: &mut Frame<B>,
        chunk: Rect,
        is_active: bool,
        offset: u16,
    )
    {
        let duration = match (self.data.start_timestamp, self.data.end_timestamp) {
            (Some(start), Some(end)) => format!(", {}", format_duration(end - start)),
            _ => String::new(),
        };

        let request_title = format!("{} ({} bytes{})", title, self.data.content.len(), duration);
        let block = create_block(&request_title, is_active);

        if self.ui_state.is_none() {
            let decoders: Vec<Box<dyn Decoder>> = decoders
                .iter()
                .map(|d| d.try_create(request, &self.data))
                .filter_map(|o| o)
                .collect();

            self.ui_state = Some(MessageDataUiState {
                active_decoder: decoders.len() - 1,
                decoders,
            });
        }

        let ui_state = self.ui_state.as_ref().unwrap();
        let text = ui_state.decoders[ui_state.active_decoder].decode(&self.data);
        let request_data = Paragraph::new(text.iter())
            .block(block)
            .wrap(false)
            .scroll(offset);
        f.render_widget(request_data, chunk);
    }
}

#[derive(Debug, PartialEq)]
pub enum Status
{
    InProgress,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy)]
pub enum RequestPart
{
    Request,
    Response,
}

impl std::fmt::Display for Status
{
    fn fmt(&self, w: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error>
    {
        match self {
            Status::InProgress => write!(w, ".."),
            Status::Succeeded => write!(w, "OK"),
            Status::Failed => write!(w, "Fail"),
        }
    }
}

fn create_block(title: &str, active: bool) -> Block
{
    let mut block = Block::default().title(title).borders(Borders::ALL);
    if active {
        block = block.border_type(tui::widgets::BorderType::Thick);
    }
    block
}

struct TextLine<'a>(&'a str);
impl<'a> Widget for TextLine<'a>
{
    fn render(self, area: Rect, buf: &mut Buffer)
    {
        buf.set_stringn(
            area.x,
            area.y,
            self.0,
            area.width as usize,
            Style::default(),
        );
    }
}

fn format_duration(d: Duration) -> String
{
    match d {
        t if t > Duration::seconds(10) => format!("{} s", t.num_seconds()),
        t => format!("{} ms", t.num_milliseconds()),
    }
}
