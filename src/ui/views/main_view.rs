use tui::layout::{Constraint, Direction, Layout, Rect};

use super::prelude::*;
use super::{DetailsView, MessageView};
use crate::session::{IndexedVec, RequestPart};

#[derive(PartialEq)]
pub enum Window
{
    Requests,
    Details,
}

#[derive(Default)]
pub struct MainView
{
    details_view: DetailsView,
    requests_state: ProxideTableState,
    active_window: Window,
}

#[derive(Default)]
struct ProxideTableState
{
    tui_state: tui::widgets::TableState,
    user_selected: Option<usize>,
}

impl Default for Window
{
    fn default() -> Self
    {
        Self::Requests
    }
}

impl<B: Backend> View<B> for MainView
{
    fn draw(&mut self, ctx: &UiContext, f: &mut Frame<B>, chunk: Rect)
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
        ctx.state.requests.draw(
            &ctx.data.requests,
            &mut self.requests_state.tui_state,
            f,
            chunks[0],
            self.active_window == Window::Requests,
        );

        self.details_view.draw(ctx, f, chunks[1]);
    }

    fn on_input(&mut self, ctx: &UiContext, e: CTEvent, size: Rect) -> HandleResult<B>
    {
        // Handle active window input first.
        let handled = match self.active_window {
            Window::Requests => self
                .requests_state
                .on_input::<B, _>(&ctx.data.requests, e, size),
            Window::Details => HandleResult::Ignore,
        };

        if let HandleResult::Ignore = handled {
            match e {
                CTEvent::Key(key) => match key.code {
                    KeyCode::Char('r') => self.active_window = Window::Requests,
                    KeyCode::Char('d') => self.active_window = Window::Details,
                    KeyCode::Char('q') => {
                        return self.create_message_view(ctx, RequestPart::Request)
                    }
                    KeyCode::Char('s') => {
                        return self.create_message_view(ctx, RequestPart::Request)
                    }
                    _ => return HandleResult::Ignore,
                },
                _ => return HandleResult::Ignore,
            };
        }

        HandleResult::Update
    }

    fn on_change(&self, ctx: &UiContext, change: &SessionChange) -> bool
    {
        match change {
            SessionChange::Connections => false,
            SessionChange::Connection { .. } => true,
            SessionChange::Request { .. } => true,
            msg @ SessionChange::Message { .. } => {
                <dyn View<B>>::on_change(&self.details_view, ctx, msg)
            }
        }
    }

    fn help_text(&self, _state: &UiContext, _size: Rect) -> String
    {
        "Up/Down, j/k: Move up/down".to_string()
    }
}

impl MainView
{
    fn create_message_view<B: Backend>(&self, ctx: &UiContext, part: RequestPart)
        -> HandleResult<B>
    {
        self.requests_state
            .selected(&ctx.data.requests)
            .map(|req| {
                HandleResult::PushView(Box::new(MessageView {
                    request: req.request_data.uuid,
                    part,
                    offset: 0,
                }))
            })
            .unwrap_or(HandleResult::Ignore)
    }
}

impl ProxideTableState
{
    pub fn on_input<B: Backend, T>(
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
                        .or_else(|| self.tui_state.selected())
                        .map(|i| i.saturating_sub(1)),
                ),
                KeyCode::Char('j') | KeyCode::Down => self.user_select(
                    content,
                    self.user_selected
                        .or_else(|| self.tui_state.selected())
                        .map(|i| i + 1),
                ),
                KeyCode::Esc => self.user_select(content, None),
                _ => return HandleResult::Ignore,
            },
            _ => return HandleResult::Ignore,
        };
        HandleResult::Update
    }

    pub fn user_select<T>(&mut self, content: &IndexedVec<T>, idx: Option<usize>)
    {
        match idx {
            None => {
                self.user_selected = None;
                if content.items.is_empty() {
                    self.tui_state.select(None);
                } else {
                    self.tui_state.select(Some(content.items.len() - 1));
                }
            }
            Some(mut idx) => {
                if idx >= content.items.len() {
                    idx = content.items.len() - 1;
                }
                self.user_selected = Some(idx);
                self.tui_state.select(self.user_selected);
            }
        }
    }

    pub fn selected<'a, T>(&self, content: &'a IndexedVec<T>) -> Option<&'a T>
    {
        self.tui_state
            .selected()
            .and_then(|idx| content.items.get(idx))
    }
}
