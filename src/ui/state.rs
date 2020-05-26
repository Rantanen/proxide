use crossterm::event::{Event as CrosstermEvent, KeyCode};
use std::sync::mpsc::Sender;
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
use crate::ui::menus::Menu;
use crate::ui::views::{self, View};

pub enum UiEvent
{
    Crossterm(CrosstermEvent),
    Toast(ToastEvent),
    SessionEvent(Box<SessionEvent>),
}

pub struct ProxideUi<B>
{
    pub context: UiContext,
    pub ui_stack: Vec<Box<dyn View<B>>>,
    pub menu_stack: Vec<Box<dyn Menu<B>>>,
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
    pub tx: Sender<UiEvent>,
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
    OpenMenu(Box<dyn Menu<B>>),
    PushView(Box<dyn View<B>>),
    ExitView,
    ExitMenu,
}

impl<B: Backend> ProxideUi<B>
{
    pub fn new(
        session: Session,
        tx: Sender<UiEvent>,
        decoders: Vec<Box<dyn DecoderFactory>>,
        size: Rect,
    ) -> Self
    {
        Self {
            context: UiContext {
                data: session,
                runtime: Runtime {
                    decoder_factories: decoders,
                    tx,
                },
                size,
            },
            ui_stack: vec![Box::new(views::MainView::default())],
            menu_stack: vec![],
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
                    .handle(*e)
                    .into_iter()
                    .map(|change| {
                        self.ui_stack
                            .last_mut()
                            .unwrap()
                            .on_change(&self.context, &change)
                    })
                    .collect();
                match results.into_iter().any(|b| b) {
                    true => HandleResult::Update,
                    false => HandleResult::Ignore,
                }
            }
            UiEvent::Crossterm(e) => self.on_input(e, self.context.size),
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
        let result = if let Some(menu) = self.menu_stack.last_mut() {
            menu.on_input(&mut self.context, e)
        } else {
            self.ui_stack
                .last_mut()
                .unwrap()
                .on_input(&self.context, e, size)
        };

        match result {
            r @ HandleResult::Update | r @ HandleResult::Quit => return r,
            HandleResult::Ignore => {}
            HandleResult::PushView(v) => {
                self.ui_stack.push(v);
                return HandleResult::Update;
            }
            HandleResult::OpenMenu(m) => {
                self.menu_stack.push(m);
                return HandleResult::Update;
            }
            HandleResult::ExitView => {
                self.menu_stack = vec![];
                self.ui_stack.pop();
                return HandleResult::Update;
            }
            HandleResult::ExitMenu => {
                self.menu_stack.pop();
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
                KeyCode::Char('Q') => HandleResult::Quit,
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

        let mut idx = self.ui_stack.len() - 1;
        while self.ui_stack[idx].transparent() {
            idx -= 1;
        }

        for i in idx..self.ui_stack.len() {
            &mut self.ui_stack[i].draw(&self.context, &mut f, view_chunk);
        }

        // The bottom area is reserved for menus or help text depending on
        // whether a menu is up.
        let help_text = if let Some(menu) = self.menu_stack.last() {
            menu.help_text(&self.context)
        } else {
            let view = self.ui_stack.last_mut().unwrap();
            view.help_text(&self.context, self.context.size)
        };

        let text_chunk = Rect {
            x: 1,
            y: chunk.height - 2,
            width: chunk.width - 2,
            height: 2,
        };

        f.render_widget(
            Paragraph::new([Text::raw(&help_text)].iter()).wrap(false),
            text_chunk,
        );

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

pub struct TextLine<'a>(pub &'a str);
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
