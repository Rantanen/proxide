use chrono::prelude::*;
use serde::Serialize;
use tui::layout::{Constraint, Direction, Layout, Rect};

use super::prelude::*;
use super::{DetailsView, MessageView};
use crate::session::{EncodedRequest, RequestPart};

use crate::ui::controls::TableView;
use crate::ui::toast;

#[derive(PartialEq)]
pub enum Window
{
    Requests,
    Details,
}

pub struct MainView
{
    details_view: DetailsView,
    requests_state: TableView<EncodedRequest>,
    active_window: Window,
}

impl Default for Window
{
    fn default() -> Self
    {
        Self::Requests
    }
}

impl Default for MainView
{
    fn default() -> Self
    {
        Self {
            details_view: DetailsView::default(),
            requests_state: TableView::<EncodedRequest>::new("[R]equests")
                .with_column("Requests", Constraint::Percentage(100), |item| {
                    format!(
                        "{} {}",
                        item.request_data.method,
                        item.request_data
                            .uri
                            .path_and_query()
                            .map(ToString::to_string)
                            .unwrap_or_else(|| "/".to_string())
                    )
                })
                .with_column("Timestamp", Constraint::Length(10), |item| {
                    item.request_data
                        .start_timestamp
                        .format("%H:%M:%S")
                        .to_string()
                })
                .with_column("St.", Constraint::Length(5), |item| {
                    item.request_data.status.to_string()
                }),
            active_window: Window::Requests,
        }
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
                .on_input::<B>(&ctx.data.requests, e, size),
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
                    KeyCode::F(12) => {
                        self.export(ctx);
                        return HandleResult::Ignore;
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
            SessionChange::NewRequest { .. } => {
                self.requests_state
                    .auto_select(&ctx.data.requests, Some(usize::MAX));
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
        "Up/Down, j/k: Move up/down; F12: Export session to file".to_string()
    }
}

impl MainView
{
    fn export(&self, ctx: &UiContext)
    {
        let filename = format!("session-{}.txt", Local::now().format("%H_%M_%S"));
        let mut file = match std::fs::File::create(&filename) {
            Ok(f) => f,
            Err(e) => return toast::show_error(format!("Error opening file.\n{}", e)),
        };

        match ctx
            .data
            .serialize(&mut rmp_serde::Serializer::new(&mut file))
        {
            Ok(_) => toast::show_message(format!("Exported session to '{}'", filename)),
            Err(e) => toast::show_error(format!("Failed to serialize the session:\n{}", e)),
        }
    }

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
