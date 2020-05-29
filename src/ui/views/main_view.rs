use tui::layout::{Constraint, Direction, Layout, Rect};

use super::prelude::*;
use super::{DetailsView, MessageView};
use crate::session::{EncodedRequest, RequestPart};

use crate::ui::commands;
use crate::ui::controls::TableView;
use crate::ui::menus::FilterMenu;

pub struct MainView
{
    details_view: DetailsView,
    requests_state: TableView<EncodedRequest>,
}

impl Default for MainView
{
    fn default() -> Self
    {
        Self {
            details_view: DetailsView::default(),
            requests_state: TableView::<EncodedRequest>::new("Requests")
                .with_group_filter(|current, maybe| {
                    current.request_data.connection_uuid == maybe.request_data.connection_uuid
                })
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
        self.requests_state
            .draw_requests(&ctx.data.requests, f, chunks[0]);

        if let Some(request) = self.requests_state.selected(&ctx.data.requests) {
            self.details_view
                .draw_control(request.request_data.uuid, ctx, f, chunks[1]);
        }
    }

    fn on_input(&mut self, ctx: &UiContext, e: CTEvent, size: Rect) -> HandleResult<B>
    {
        // Handle the request control first.
        let handled = self
            .requests_state
            .on_input::<B>(&ctx.data.requests, e, size);

        if let HandleResult::Ignore = handled {
            match e {
                CTEvent::Key(key) => match key.code {
                    KeyCode::Char('q') => {
                        return self.create_message_view(ctx, RequestPart::Request)
                    }
                    KeyCode::Char('e') => {
                        return self.create_message_view(ctx, RequestPart::Response)
                    }
                    KeyCode::Char('f') => {
                        self.requests_state.lock(&ctx.data.requests);
                        return HandleResult::OpenMenu(Box::new(FilterMenu {
                            request: self
                                .requests_state
                                .selected(&ctx.data.requests)
                                .map(|request| request.request_data.uuid),
                        }));
                    }
                    KeyCode::F(12) => {
                        commands::export_session(ctx);
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
        format!("{}\n{}",
            "[Up/Down,j/k]: Previous/Next request ([Shift]: Follow connection); [F12]: Export session to file; [Shift-Q]: Quit",
            "[F]: Manage filters")
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
