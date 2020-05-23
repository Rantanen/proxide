use super::prelude::*;
use crossterm::event::KeyCode;
use uuid::Uuid;

use crate::session::RequestPart;

pub struct MessageView
{
    pub request: Uuid,
    pub part: RequestPart,
    pub offset: u16,
}

impl<B: Backend> View<B> for MessageView
{
    fn draw(&mut self, ctx: &UiContext, f: &mut Frame<B>, chunk: Rect)
    {
        let request = match ctx.data.requests.get_by_uuid(self.request) {
            Some(r) => r,
            None => return,
        };

        let (title, data) = match self.part {
            RequestPart::Request => ("Request Data", &request.request_msg),
            RequestPart::Response => ("Response Data", &request.response_msg),
        };
        let title = format!("{} (offset {})", title, self.offset);
        data.draw(
            &ctx.runtime.decoder_factories,
            &request.request_data,
            &title,
            f,
            chunk,
            true,
            self.offset,
        );
    }

    fn on_input(&mut self, _session: &UiContext, e: CTEvent, size: Rect) -> HandleResult<B>
    {
        match e {
            CTEvent::Key(key) => match key.code {
                KeyCode::Char('k') | KeyCode::Up => self.offset = self.offset.saturating_sub(1),
                KeyCode::Char('j') | KeyCode::Down => self.offset = self.offset.saturating_add(1),
                KeyCode::PageDown => self.offset = self.offset.saturating_add(size.height - 5),
                KeyCode::PageUp => self.offset = self.offset.saturating_sub(size.height - 5),
                KeyCode::Tab => {
                    self.part = match self.part {
                        RequestPart::Request => RequestPart::Response,
                        RequestPart::Response => RequestPart::Request,
                    }
                }
                _ => return HandleResult::Ignore,
            },
            _ => return HandleResult::Ignore,
        };
        HandleResult::Update
    }

    fn on_change(&mut self, _ctx: &UiContext, change: &SessionChange) -> bool
    {
        match change {
            SessionChange::NewConnection { .. } => false,
            SessionChange::NewRequest { .. } => false,
            SessionChange::Request { .. } => false,
            SessionChange::NewMessage { request, part }
            | SessionChange::Message { request, part } => {
                *part == self.part && *request == self.request
            }
        }
    }

    fn help_text(&self, _session: &UiContext, _size: Rect) -> String
    {
        "Up/Down, j/k, PgUp/PgDn: Scroll; Tab: Switch Request/Response".to_string()
    }
}
