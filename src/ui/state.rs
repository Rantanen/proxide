use crossterm::event::{Event as CTEvent, KeyCode};
use tui::backend::Backend;
use tui::buffer::Buffer;
use tui::layout::{Constraint, Rect};
use tui::style::{Modifier, Style};
use tui::terminal::Frame;
use tui::widgets::{Paragraph, Row, Table, TableState, Widget};

use crate::decoders::{Decoder, DecoderFactory};
use crate::session::events::SessionEvent;
use crate::session::*;
use crate::ui::prelude::*;
use crate::ui::views::{self, View};

#[derive(Debug)]
pub enum UiEvent
{
    Crossterm(crossterm::event::Event),
    SessionEvent(SessionEvent),
}

pub struct ProxideUi<B>
{
    pub context: UiContext,
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
    pub size: Rect,
}

pub enum HandleResult<B: Backend>
{
    Ignore,
    Update,
    Quit,
    PushView(Box<dyn View<B>>),
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
                state: State {
                    size,
                    ..State::default()
                },
                runtime: Runtime {
                    decoder_factories: decoders,
                },
            },
            ui_stack: vec![Box::new(views::MainView::default())],
        }
    }

    pub fn handle(&mut self, e: UiEvent) -> HandleResult<B>
    {
        match e {
            UiEvent::SessionEvent(e) => self
                .context
                .data
                .handle(e)
                .map(|change| {
                    match self
                        .ui_stack
                        .last_mut()
                        .unwrap()
                        .on_change(&mut self.context, &change)
                    {
                        true => HandleResult::Update,
                        false => HandleResult::Ignore,
                    }
                })
                .unwrap_or(HandleResult::Ignore),
            UiEvent::Crossterm(e) => return self.on_input(e, self.context.state.size),
        }
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
                self.context.state.size = Rect {
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
        let help_text = view.help_text(&self.context, self.context.state.size);
        let help_line = TextLine(&help_text);
        f.render_widget(help_line, help_chunk);
    }
}

pub struct ProxideTable<T>
{
    phantom: std::marker::PhantomData<T>,
}

impl<T> Default for ProxideTable<T>
{
    fn default() -> Self
    {
        Self {
            phantom: std::marker::PhantomData::<T>,
        }
    }
}

impl ProxideTable<EncodedRequest>
{
    pub fn draw<B: Backend>(
        &self,
        content: &IndexedVec<EncodedRequest>,
        state: &mut TableState,
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

        f.render_stateful_widget(table, chunk, state)
    }
}

impl EncodedMessage
{
    pub fn draw<B: Backend>(
        &self,
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
