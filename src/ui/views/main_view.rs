use tui::layout::{Constraint, Direction, Layout, Rect};

use super::prelude::*;
use crate::session::EncodedRequest;

use crate::ui::commands;
use crate::ui::sub_views::{DetailsPane, FilterPane, TableView};

pub struct MainView
{
    details_view: DetailsPane,
    requests_state: TableView<EncodedRequest>,
    filter_pane: Option<FilterPane>,
    filter_pane_active: bool,
}

impl Default for MainView
{
    fn default() -> Self
    {
        Self {
            details_view: DetailsPane::default(),
            filter_pane: None,
            filter_pane_active: false,
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

        let requests_state = &self.requests_state;
        let selected_filter = match self.filter_pane_active {
            true => self
                .filter_pane
                .as_mut()
                .and_then(|fm| fm.selected_filter(requests_state.get_filter())),
            false => None,
        };

        self.requests_state.draw_requests(
            &ctx.data.requests,
            selected_filter,
            !self.filter_pane_active,
            f,
            chunks[0],
        );

        let request = self.requests_state.selected(&ctx.data.requests);
        if let Some(filter_menu) = &mut self.filter_pane {
            filter_menu.draw(
                self.requests_state.get_filter(),
                ctx,
                request,
                self.filter_pane_active,
                f,
                chunks[1],
            );
        } else if let Some(request) = request {
            self.details_view
                .draw_control(request.request_data.uuid, ctx, f, chunks[1]);
        }
    }

    fn on_input(&mut self, ctx: &UiContext, e: CTEvent, size: Rect) -> Option<HandleResult<B>>
    {
        if self.filter_pane.is_some() && self.filter_pane_active {
            let filter = &mut self.requests_state.get_filter_mut(&ctx.data.requests);
            self.filter_pane
                .as_mut()
                .unwrap()
                .on_active_input(filter, e)
        } else {
            self.requests_state
                .on_active_input(&ctx.data.requests, e, size)
        }
        .or_else(|| self.do_filter_input(ctx, e))
        .or_else(|| self.do_details_input(ctx, e))
        .or_else(|| self.do_self_input(ctx, e))
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
            SessionChange::NewMessage { request: req, .. }
            | SessionChange::Message { request: req, .. } => self
                .requests_state
                .selected(&ctx.data.requests)
                .map(|r| r.request_data.uuid == *req)
                .unwrap_or(false),
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
    fn do_filter_input<B: Backend>(
        &mut self,
        ctx: &UiContext,
        e: CTEvent,
    ) -> Option<HandleResult<B>>
    {
        // Handle whatever is on the right side first.
        if let Some(filter_menu) = &mut self.filter_pane {
            // Filter menu.
            let request = self.requests_state.selected(&ctx.data.requests);
            match filter_menu.on_global_input(
                self.requests_state.get_filter_mut(&ctx.data.requests),
                ctx,
                request,
                e,
            )? {
                HandleResult::ExitView => {
                    self.filter_pane = None;
                    Some(HandleResult::Update)
                }
                other => Some(other),
            }
        } else {
            None
        }
    }

    fn do_details_input<B: Backend>(
        &mut self,
        ctx: &UiContext,
        e: CTEvent,
    ) -> Option<HandleResult<B>>
    {
        self.requests_state
            .selected(&ctx.data.requests)
            .and_then(|req| self.details_view.on_input(req, e))
    }

    fn do_self_input<B: Backend>(&mut self, ctx: &UiContext, e: CTEvent)
        -> Option<HandleResult<B>>
    {
        match e {
            CTEvent::Key(key) => match key.code {
                KeyCode::Char('F') => {
                    self.requests_state
                        .get_filter_mut(&ctx.data.requests)
                        .toggle();
                    Some(HandleResult::Update)
                }
                KeyCode::Char('f') => {
                    // Toggle the filter menu.
                    match self.filter_pane {
                        Some(_) => {
                            self.filter_pane = None;
                            self.filter_pane_active = false;
                        }
                        None => {
                            // We are intentionally not making the menu active immediately.
                            //
                            // This allows the user to continue navigating through the items
                            // to find the correct ones to filter by.
                            self.filter_pane = Some(FilterPane::new())
                        }
                    }
                    Some(HandleResult::Update)
                }
                KeyCode::F(12) => {
                    commands::export_session(ctx);
                    None
                }
                KeyCode::Tab => {
                    match self.filter_pane {
                        Some(_) => self.filter_pane_active = !self.filter_pane_active,
                        None => self.filter_pane_active = false,
                    };
                    Some(HandleResult::Update)
                }
                _ => None,
            },
            _ => None,
        }
    }
}
