use crossterm::event::KeyCode;
use uuid::Uuid;

use super::views::prelude::*;
use crate::session::filters::FilterType;

pub trait Menu<B: Backend>
{
    fn help_text(&self, ctx: &UiContext) -> String;
    fn on_input(&mut self, ctx: &mut UiContext, e: CTEvent) -> HandleResult<B>;
}

pub struct FilterMenu
{
    pub request: Option<Uuid>,
}
impl<B: Backend> Menu<B> for FilterMenu
{
    fn help_text(&self, ctx: &UiContext) -> String
    {
        let options = vec![format!(
            "[c]: {} by current connection",
            get_enable_disable(&ctx, FilterType::Connection)
        )];
        format!("{}\n[x]: Clear all filters", options.join(", "))
    }

    fn on_input(&mut self, ctx: &mut UiContext, e: CTEvent) -> HandleResult<B>
    {
        if let CTEvent::Key(key) = e {
            match key.code {
                KeyCode::Char('x') => {
                    ctx.data.requests.clear_filters();
                    return HandleResult::ExitMenu;
                }
                KeyCode::Char('c') => return self.on_connection_filter(ctx),
                _ => {}
            }
        }

        HandleResult::Ignore
    }
}

impl FilterMenu
{
    fn on_connection_filter<B: Backend>(&self, ctx: &mut UiContext) -> HandleResult<B>
    {
        match is_enabled(ctx, FilterType::Connection) {
            true => ctx.data.requests.remove_filter(FilterType::Connection),
            false => {
                if let Some(req) = self
                    .request
                    .and_then(|req| ctx.data.requests.get_by_uuid(req))
                {
                    let connection = req.request_data.connection_uuid;
                    ctx.data.requests.add_filter(Box::new(
                        crate::session::filters::ConnectionFilter { connection },
                    ));
                }
            }
        }

        HandleResult::ExitMenu
    }
}

fn get_enable_disable(ctx: &UiContext, filter_type: FilterType) -> &str
{
    match is_enabled(ctx, filter_type) {
        true => "Disable",
        false => "Enable",
    }
}

fn is_enabled(ctx: &UiContext, filter_type: FilterType) -> bool
{
    ctx.data.requests.filters.contains_key(&filter_type)
}