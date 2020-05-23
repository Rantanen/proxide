use tui::layout::{Constraint, Direction, Layout, Rect};
use tui::style::{Modifier, Style};
use tui::widgets::{Row, Table, TableState};

use super::prelude::*;
use super::{DetailsView, MessageView};
use crate::session::{EncodedRequest, IndexedVec, RequestPart};

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
    requests_state: ProxideTable,
    active_window: Window,
}

#[derive(Default)]
struct ProxideTable
{
    tui_state: TableState,
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
        self.requests_state.draw_requests(
            &ctx.data.requests,
            f,
            chunks[0],
            self.active_window == Window::Requests,
        );

        if let Some(request) = self.requests_state.selected(&ctx.data.requests) {
            self.details_view.draw_control(
                request.request_data.uuid,
                ctx,
                f,
                chunks[1],
                self.active_window == Window::Details,
            );
        }
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

    fn on_change(&mut self, ctx: &UiContext, change: &SessionChange) -> bool
    {
        match change {
            SessionChange::NewConnection { .. } => false,
            SessionChange::NewRequest { request, .. } => {
                if self.requests_state.user_selected.is_none() {
                    self.requests_state
                        .tui_state
                        .select(Some(ctx.data.requests.items.len() - 1));
                }
                true
            }
            SessionChange::Request { .. } => true,
            msg @ SessionChange::NewMessage { .. } | msg @ SessionChange::Message { .. } => {
                <dyn View<B>>::on_change(&mut self.details_view, ctx, msg)
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

impl ProxideTable
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

    pub fn draw_requests<B: Backend>(
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

        f.render_stateful_widget(table, chunk, &mut self.tui_state)
    }
}
