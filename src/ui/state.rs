use crossterm::event::{Event as CrosstermEvent, KeyCode};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use tui::backend::Backend;
use tui::buffer::Buffer;
use tui::layout::Rect;
use tui::style::{Color, Style};
use tui::terminal::Frame;
use tui::widgets::{Block, Borders, Clear, Paragraph, Text, Widget};
use uuid::Uuid;

use super::toast::ToastEvent;
use crate::decoders::Decoders;
use crate::search;
use crate::session::events::SessionEvent;
use crate::session::*;
use crate::ui::commands;
use crate::ui::views::{self, View};

pub enum UiEvent
{
    Redraw,
    Crossterm(CrosstermEvent),
    Toast(ToastEvent),
    SessionEvent(Box<SessionEvent>),
}

pub struct ProxideUi<B>
{
    pub context: UiContext,
    pub ui_stack: Vec<Box<dyn View<B>>>,
    pub toasts: Vec<Toast>,
    pub input_command: Option<commands::CommandState>,
}

pub struct Toast
{
    uuid: Uuid,
    text: String,
    error: bool,
}

pub struct Runtime
{
    pub decoders: Decoders,
    pub search_index: Rc<RefCell<search::SearchIndex>>,
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
    Update,
    Quit,
    PushView(Box<dyn View<B>>),
    ExitView,
    ExitCommand,
}

impl<B: Backend> ProxideUi<B>
{
    pub fn new(session: Session, tx: Sender<UiEvent>, decoders: Decoders, size: Rect) -> Self
    {
        Self {
            context: UiContext {
                runtime: Runtime {
                    search_index: Rc::new(RefCell::new(search::SearchIndex::new(
                        &session, &decoders,
                    ))),
                    decoders,
                    tx,
                },
                data: session,
                size,
            },
            ui_stack: vec![Box::new(views::MainView::default())],
            toasts: vec![],
            input_command: None,
        }
    }

    pub fn handle(&mut self, e: UiEvent) -> Option<HandleResult<B>>
    {
        match e {
            UiEvent::Redraw => unreachable!("This is handled by the parent loop"),
            UiEvent::SessionEvent(e) => {
                // Capture the index request so we know to do indexing after the session has been
                // updated.
                let index_request = match &*e {
                    SessionEvent::MessageDone(msg) => Some(search::IndexRequest::Message {
                        request: msg.uuid,
                        part: msg.part,
                    }),
                    _ => None,
                };

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

                if let Some(ixreq) = index_request {
                    self.context.runtime.search_index.borrow_mut().index(
                        &self.context.data,
                        &self.context.runtime.decoders,
                        ixreq,
                    );
                }

                match results.into_iter().any(|b| b) {
                    true => Some(HandleResult::Update),
                    false => None,
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
                Some(HandleResult::Update)
            }
        }
    }

    fn on_input(&mut self, e: CrosstermEvent, size: Rect) -> Option<HandleResult<B>>
    {
        let result = if let Some(cmd) = &mut self.input_command {
            cmd.on_input(&mut self.context, e)
        } else {
            self.ui_stack
                .last_mut()
                .unwrap()
                .on_input(&self.context, e, size)
        };

        if let Some(result) = result {
            match result {
                r @ HandleResult::Update | r @ HandleResult::Quit => return Some(r),
                HandleResult::PushView(v) => {
                    self.ui_stack.push(v);
                    return Some(HandleResult::Update);
                }
                HandleResult::ExitView => {
                    self.ui_stack.pop();
                    return Some(HandleResult::Update);
                }
                HandleResult::ExitCommand => {
                    self.input_command = None;
                    return Some(HandleResult::Update);
                }
            }
        }

        Some(match e {
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
                KeyCode::Char(':') => {
                    self.input_command = Some(commands::CommandState {
                        help: "Enter command".to_string(),
                        prompt: ":".to_string(),
                        input: Default::default(),
                        text_cursor: 0,
                        display_cursor: 0,
                        executable: Box::new(commands::ColonCommand),
                    });
                    HandleResult::Update
                }
                /*
                KeyCode::Char('/') => {
                    self.input_command = Some(commands::CommandState {
                        help: "Search".to_string(),
                        prompt: "/".to_string(),
                        input: Default::default(),
                        text_cursor: 0,
                        display_cursor: 0,
                        executable: Box::new(commands::SearchCommand),
                    });
                    HandleResult::Update
                }
                */
                KeyCode::Char('Q') => HandleResult::Quit,
                KeyCode::Esc => {
                    if self.ui_stack.len() > 1 {
                        self.ui_stack.pop();
                    }
                    HandleResult::Update
                }
                _ => return None,
            },
            _ => return None,
        })
    }

    pub fn draw(&mut self, terminal: &mut tui::Terminal<B>) -> std::io::Result<()>
    {
        terminal.hide_cursor()?;
        terminal.draw(|f| self.draw_views(f))?;

        if let Some(cmd) = &self.input_command {
            let x = cmd.cursor();
            terminal.set_cursor(x + 2, terminal.size()?.height - 1)?;
            terminal.show_cursor()?;
        }
        Ok(())
    }

    pub fn draw_views(&mut self, mut f: Frame<B>)
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
            self.ui_stack[i].draw(&self.context, &mut f, view_chunk);
        }

        // The bottom area is reserved for menus or help text depending on
        // whether a menu is up.
        let help_text = if let Some(cmd) = &self.input_command {
            format!("{}\n{}{}", cmd.help, cmd.prompt, cmd.input)
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
