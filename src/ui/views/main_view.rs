use tui::layout::{Constraint, Direction, Layout, Rect};

use super::prelude::*;
use super::{DetailsView, MessageView};
use crate::session::{EncodedRequest, RequestPart};

use crate::ui::commands;
use crate::ui::controls::TableView;
use crate::ui::menus::RequestFilterMenu;

pub struct MainView
{
    details_view: DetailsView,
    requests_state: TableView<EncodedRequest>,
    filter_menu: Option<RequestFilterMenu>,
    menu_active: bool,
}

impl Default for MainView
{
    fn default() -> Self
    {
        Self {
            details_view: DetailsView::default(),
            filter_menu: None,
            menu_active: false,
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
            .draw_requests(&ctx.data.requests, f, chunks[0], !self.menu_active);

        let request = self.requests_state.selected(&ctx.data.requests);
        if let Some(filter_menu) = &mut self.filter_menu {
            filter_menu.draw(
                self.requests_state.get_filter(),
                ctx,
                request,
                f,
                chunks[1],
                self.menu_active,
            );
        } else if let Some(request) = request {
            self.details_view
                .draw_control(request.request_data.uuid, ctx, f, chunks[1]);
        }
    }

    fn on_input(&mut self, ctx: &UiContext, e: CTEvent, size: Rect) -> Option<HandleResult<B>>
    {
        if self.filter_menu.is_some() && self.menu_active {
            self.do_filter_input(ctx, e, size)
        } else {
            self.do_request_input(ctx, e, size)
        }
        .or_else(|| self.do_details_input(ctx, e, size))
        .or_else(|| self.do_self_input(ctx, e, size))
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
        size: Rect,
    ) -> Option<HandleResult<B>>
    {
        // Handle whatever is on the right side first.
        if let Some(filter_menu) = &mut self.filter_menu {
            // Filter menu.
            let request = self.requests_state.selected(&ctx.data.requests);
            match filter_menu.on_input(
                self.requests_state.get_filter_mut(&ctx.data.requests),
                ctx,
                request,
                e,
            )? {
                HandleResult::ExitView => {
                    self.filter_menu = None;
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
        size: Rect,
    ) -> Option<HandleResult<B>>
    {
        self.requests_state
            .selected(&ctx.data.requests)
            .and_then(|req| self.details_view.on_input(req, ctx, e, size))
    }

    fn do_request_input<B: Backend>(
        &mut self,
        ctx: &UiContext,
        e: CTEvent,
        size: Rect,
    ) -> Option<HandleResult<B>>
    {
        // Request list.
        self.requests_state
            .on_input::<B>(&ctx.data.requests, e, size)
    }

    fn do_self_input<B: Backend>(
        &mut self,
        ctx: &UiContext,
        e: CTEvent,
        size: Rect,
    ) -> Option<HandleResult<B>>
    {
        match e {
            CTEvent::Key(key) => match key.code {
                KeyCode::Char('F') => {
                    self.requests_state
                        .get_filter_mut(&ctx.data.requests)
                        .toggle_filter();
                    Some(HandleResult::Update)
                }
                KeyCode::Char('f') => {
                    self.filter_menu = Some(RequestFilterMenu::new());
                    self.menu_active = true;
                    Some(HandleResult::Update)
                }
                KeyCode::F(12) => {
                    commands::export_session(ctx);
                    None
                }
                KeyCode::Tab => {
                    match self.filter_menu {
                        Some(_) => self.menu_active = !self.menu_active,
                        None => self.menu_active = false,
                    };
                    Some(HandleResult::Update)
                }
                _ => None,
            },
            _ => None,
        }
    }
}
