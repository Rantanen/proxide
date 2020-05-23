use crossterm::event::{Event as CrosstermEvent, KeyCode};
use tui::backend::Backend;
use tui::buffer::Buffer;
use tui::layout::Rect;
use tui::style::{Color, Style};
use tui::terminal::Frame;
use tui::widgets::{Block, Borders, Clear, Paragraph, Text, Widget};
use uuid::Uuid;

use super::toast::ToastEvent;
use crate::decoders::DecoderFactory;
use crate::session::events::SessionEvent;
use crate::session::*;
use crate::ui::views::{self, View};

#[derive(Debug)]
pub enum UiEvent
{
    Crossterm(CrosstermEvent),
    Toast(ToastEvent),
    SessionEvent(SessionEvent),
}

pub struct ProxideUi<B>
{
    pub context: UiContext,
    pub ui_stack: Vec<Box<dyn View<B>>>,
    pub toasts: Vec<Toast>,
}

pub struct Toast
{
    uuid: Uuid,
    text: String,
    error: bool,
}

pub struct Runtime
{
    pub decoder_factories: Vec<Box<dyn DecoderFactory>>,
}

pub struct UiContext
{
    pub data: Session,
    pub runtime: Runtime,
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
    pub fn new(session: Session, decoders: Vec<Box<dyn DecoderFactory>>, size: Rect) -> Self
    {
        Self {
            context: UiContext {
                data: session,
                runtime: Runtime {
                    decoder_factories: decoders,
                },
                size,
            },
            ui_stack: vec![Box::new(views::MainView::default())],
            toasts: vec![],
        }
    }

    pub fn handle(&mut self, e: UiEvent) -> HandleResult<B>
    {
        match e {
            UiEvent::SessionEvent(e) => {
                let results: Vec<_> = self
                    .context
                    .data
                    .handle(e)
                    .into_iter()
                    .map(|change| {
                        self.ui_stack
                            .last_mut()
                            .unwrap()
                            .on_change(&mut self.context, &change)
                    })
                    .collect();
                match results.into_iter().any(|b| b) {
                    true => HandleResult::Update,
                    false => HandleResult::Ignore,
                }
            }
            UiEvent::Crossterm(e) => return self.on_input(e, self.context.size),
            UiEvent::Toast(e) => {
                match e {
                    ToastEvent::Show { uuid, text, error } => {
                        self.toasts.push(Toast { uuid, text, error })
                    }
                    ToastEvent::Close { uuid } => {
                        self.toasts.retain(|t| t.uuid != uuid);
                    }
                }
                HandleResult::Update
            }
        }
    }

    fn on_input(&mut self, e: CrosstermEvent, size: Rect) -> HandleResult<B>
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
            CrosstermEvent::Resize(width, height) => {
                self.context.size = Rect {
                    x: 0,
                    y: 0,
                    width,
                    height,
                };
                HandleResult::Update
            }
            CrosstermEvent::Key(key) => match key.code {
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
        let help_text = view.help_text(&self.context, self.context.size);
        let help_line = TextLine(&help_text);
        f.render_widget(help_line, help_chunk);

        // Draw toasts on top of everything.
        let mut offset = 1;
        for t in &self.toasts {
            offset = t.draw(offset, &mut f);
        }
    }
}

impl Toast
{
    fn draw<B: Backend>(&self, offset: u16, f: &mut Frame<B>) -> u16
    {
        let screen = f.size();
        let lines: Vec<_> = self.text.split('\n').collect();
        let max_line = lines.iter().fold(0, |max, line| max.max(line.len()));

        let mut rect = Rect {
            x: 0,
            y: offset,
            width: (max_line + 2) as u16,
            height: (lines.len() + 2) as u16,
        };
        rect.x = screen.width - rect.width - 2;

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(match self.error {
                true => Color::LightRed,
                false => Color::Reset,
            }));
        f.render_widget(Clear, rect);
        f.render_widget(
            Paragraph::new([Text::raw(&self.text)].iter())
                .block(block)
                .wrap(false),
            rect,
        );

        offset + rect.height
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
