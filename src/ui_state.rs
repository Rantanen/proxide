use chrono::Duration;
use crossterm::event::{Event as CTEvent, KeyCode};
use std::collections::HashMap;
use tui::backend::Backend;
use tui::buffer::Buffer;
use tui::layout::{Constraint, Direction, Layout, Rect};
use tui::style::{Modifier, Style};
use tui::terminal::Frame;
use tui::widgets::{Block, Borders, Paragraph, Row, Table, TableState, Text, Widget};
use uuid::Uuid;

use crate::decoders::{Decoder, DecoderFactory};
use crate::session::events::SessionEvent;
use crate::session::*;

#[derive(Debug)]
pub enum UiEvent
{
    Crossterm(crossterm::event::Event),
    SessionEvent(SessionEvent),
}

pub struct ProxideUi<B>
{
    pub context: UiContext,
    pub size: Rect,
    pub ui_stack: Vec<Box<dyn View<B>>>,
}

pub struct Runtime
{
    pub decoder_factories: Vec<Box<dyn DecoderFactory>>,
}

pub struct UiContext
{
    pub data: Session,
    pub state: State,
    pub runtime: Runtime,
}

#[derive(Default)]
pub struct State
{
    pub connections: ProxideTable<ConnectionData>,
    pub requests: ProxideTable<EncodedRequest>,
    pub active_window: Window,
}

#[derive(PartialEq)]
pub enum Window
{
    _Connections,
    Requests,
    Details,
}

impl Default for Window
{
    fn default() -> Self
    {
        Self::Requests
    }
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
    fn draw(&mut self, context: &mut UiContext, f: &mut Frame<B>, chunk: Rect);
    fn on_input(&mut self, context: &mut UiContext, e: CTEvent, size: Rect) -> HandleResult<B>;
    fn help_text(&self, state: &UiContext, size: Rect) -> String;
}

#[derive(Default)]
struct MainView
{
    details_view: DetailsView,
}

impl<B: Backend> View<B> for MainView
{
    fn draw(&mut self, context: &mut UiContext, f: &mut Frame<B>, chunk: Rect)
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
        context.state.requests.draw(
            &context.data.requests,
            f,
            chunks[0],
            context.state.active_window == Window::Requests,
        );

        self.details_view.draw(context, f, chunks[1]);
    }

    fn on_input(&mut self, context: &mut UiContext, e: CTEvent, size: Rect) -> HandleResult<B>
    {
        // Handle active window input first.
        let handled = match context.state.active_window {
            Window::_Connections => {
                context
                    .state
                    .connections
                    .on_input::<B>(&context.data.connections, e, size)
            }
            Window::Requests => context
                .state
                .requests
                .on_input(&context.data.requests, e, size),
            Window::Details => HandleResult::Ignore,
        };

        if let HandleResult::Ignore = handled {
            match e {
                CTEvent::Key(key) => match key.code {
                    KeyCode::Char('r') => context.state.active_window = Window::Requests,
                    KeyCode::Char('d') => context.state.active_window = Window::Details,
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

    fn help_text(&self, _state: &UiContext, _size: Rect) -> String
    {
        "Up/Down, j/k: Move up/down".to_string()
    }
}

#[derive(Clone, Default)]
struct DetailsView;
impl<B: Backend> View<B> for DetailsView
{
    fn draw(&mut self, context: &mut UiContext, f: &mut Frame<B>, chunk: Rect)
    {
        let request = match context
            .state
            .requests
            .selected_mut(&mut context.data.requests)
        {
            Some(r) => r,
            None => return,
        };

        let block = create_block("[D]etails", context.state.active_window == Window::Details);
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
            &context.runtime.decoder_factories,
            &request.request_data,
            "Re[q]uest Data",
            f,
            req_resp_chunks[0],
            false,
            0,
        );
        request.response_msg.draw(
            &context.runtime.decoder_factories,
            &request.request_data,
            "Re[s]ponse Data",
            f,
            req_resp_chunks[1],
            false,
            0,
        );
    }

    fn on_input(&mut self, _session: &mut UiContext, _e: CTEvent, _size: Rect) -> HandleResult<B>
    {
        HandleResult::Ignore
    }

    fn help_text(&self, _session: &UiContext, _size: Rect) -> String
    {
        String::new()
    }
}

struct MessageView(bool, u16);
impl<B: Backend> View<B> for MessageView
{
    fn draw(&mut self, context: &mut UiContext, f: &mut Frame<B>, chunk: Rect)
    {
        let request = match context
            .state
            .requests
            .selected_mut(&mut context.data.requests)
        {
            Some(r) => r,
            None => return,
        };

        let (title, data) = match self.0 {
            true => ("Request Data", &mut request.request_msg),
            false => ("Response Data", &mut request.response_msg),
        };
        let title = format!("{} (offset {})", title, self.1);
        data.draw(
            &context.runtime.decoder_factories,
            &request.request_data,
            &title,
            f,
            chunk,
            true,
            self.1,
        );
    }

    fn on_input(&mut self, _session: &mut UiContext, e: CTEvent, size: Rect) -> HandleResult<B>
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

    fn help_text(&self, _session: &UiContext, _size: Rect) -> String
    {
        "Up/Down, j/k, PgUp/PgDn: Scroll; Tab: Switch Request/Response".to_string()
    }
}

impl<B: Backend> ProxideUi<B>
{
    pub fn new(decoders: Vec<Box<dyn DecoderFactory>>, size: Rect) -> Self
    {
        Self {
            context: UiContext {
                data: Session {
                    connections: IndexedVec::new(),
                    requests: IndexedVec::new(),
                },
                state: State::default(),
                runtime: Runtime {
                    decoder_factories: decoders,
                },
            },
            ui_stack: vec![Box::new(MainView::default())],
            size,
        }
    }

    pub fn handle(&mut self, e: UiEvent) -> HandleResult<B>
    {
        match e {
            UiEvent::SessionEvent(e) => self.context.data.handle(e),
            UiEvent::Crossterm(e) => return self.on_input(e, self.size),
        }

        return HandleResult::Update;
    }

    fn on_input(&mut self, e: CTEvent, size: Rect) -> HandleResult<B>
    {
        match self
            .ui_stack
            .last_mut()
            .unwrap()
            .on_input(&mut self.context, e, size)
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
        view.draw(&mut self.context, &mut f, view_chunk);

        let help_chunk = Rect {
            x: 1,
            y: chunk.height - 2,
            width: chunk.width - 2,
            height: 1,
        };
        let help_text = view.help_text(&self.context, self.size);
        let help_line = TextLine(&help_text);
        f.render_widget(help_line, help_chunk);
    }
}

pub struct ProxideTable<T>
{
    state: TableState,
    user_selected: Option<usize>,
    phantom: std::marker::PhantomData<T>,
}

impl<T> Default for ProxideTable<T>
{
    fn default() -> Self
    {
        Self {
            state: Default::default(),
            user_selected: None,
            phantom: std::marker::PhantomData::<T>,
        }
    }
}

impl<T> ProxideTable<T>
{
    fn on_input<B: Backend>(
        &mut self,
        content: &IndexedVec<T>,
        e: CTEvent,
        _size: Rect,
    ) -> HandleResult<B>
    {
        match e {
            CTEvent::Key(key) => match key.code {
                KeyCode::Char('k') | KeyCode::Up => self.user_select(
                    content,
                    self.user_selected
                        .or_else(|| self.state.selected())
                        .map(|i| i.saturating_sub(1)),
                ),
                KeyCode::Char('j') | KeyCode::Down => self.user_select(
                    content,
                    self.user_selected
                        .or_else(|| self.state.selected())
                        .map(|i| i + 1),
                ),
                KeyCode::Esc => self.user_select(content, None),
                _ => return HandleResult::Ignore,
            },
            _ => return HandleResult::Ignore,
        };
        HandleResult::Update
    }

    fn user_select(&mut self, content: &IndexedVec<T>, idx: Option<usize>)
    {
        match idx {
            None => {
                self.user_selected = None;
                if content.items.is_empty() {
                    self.state.select(None);
                } else {
                    self.state.select(Some(content.items.len() - 1));
                }
            }
            Some(mut idx) => {
                if idx >= content.items.len() {
                    idx = content.items.len() - 1;
                }
                self.user_selected = Some(idx);
                self.state.select(self.user_selected);
            }
        }
    }

    fn selected_mut<'a>(&self, content: &'a mut IndexedVec<T>) -> Option<&'a mut T>
    {
        if let Some(idx) = self.state.selected() {
            Some(&mut content.items[idx])
        } else {
            None
        }
    }
}

impl ProxideTable<ConnectionData>
{
    pub fn _draw<B: Backend>(
        &mut self,
        content: &IndexedVec<ConnectionData>,
        f: &mut Frame<B>,
        chunk: Rect,
        is_active: bool,
    )
    {
        let block = create_block("[C]onnections", is_active);
        let table = Table::new(
            ["Source", "Timestamp", "Status"].iter(),
            content.items.iter().map(|item| {
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
    pub fn draw<B: Backend>(
        &mut self,
        content: &IndexedVec<EncodedRequest>,
        f: &mut Frame<B>,
        chunk: Rect,
        is_active: bool,
    )
    {
        let block = create_block("[R]equests", is_active);
        let table = Table::new(
            ["Request", "Timestamp", "St."].iter(),
            content.items.iter().map(|item| {
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

        let decoders: Vec<Box<dyn Decoder>> = decoders
            .iter()
            .map(|d| d.try_create(request, &self.data))
            .filter_map(|o| o)
            .collect();

        let ui_state = EncodedMessageUiState {
            active_decoder: decoders.len() - 1,
            decoders,
        };

        let text = ui_state.decoders[ui_state.active_decoder].decode(&self.data);
        let request_data = Paragraph::new(text.iter())
            .block(block)
            .wrap(false)
            .scroll(offset);
        f.render_widget(request_data, chunk);
    }
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
