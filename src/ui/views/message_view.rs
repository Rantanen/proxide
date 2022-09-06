use super::prelude::*;
use crate::decoders::Decoder;
use crossterm::event::KeyCode;
use tui::widgets::{Paragraph, Wrap};
use uuid::Uuid;

use crate::session::{MessageData, RequestData, RequestPart};
use crate::ui::toast;

pub struct MessageView
{
    pub request: Uuid,
    pub part: RequestPart,
    pub offset: u16,
}

impl MessageView
{
    fn export(&self, ctx: &UiContext)
    {
        let (request, message) = match self.get_message(ctx) {
            Some(t) => t,
            None => return toast::show_error("No active message!"),
        };

        let text: String = self
            .get_decoder(ctx, request, message)
            .decode(message)
            .into_iter()
            .map(|spans| {
                spans
                    .0
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect();

        let filename = format!("export-{}.txt", request.start_timestamp.format("%H_%M_%S"));

        match std::fs::write(&filename, text) {
            Ok(_) => toast::show_message(format!("Message exported to '{}'", filename)),
            Err(e) => toast::show_error(format!("Could not write file '{}'\n{}", filename, e)),
        }
    }

    fn get_message<'a>(&self, ctx: &'a UiContext) -> Option<(&'a RequestData, &'a MessageData)>
    {
        let request = match ctx.data.requests.get_by_uuid(self.request) {
            Some(r) => r,
            None => return None,
        };

        let data = match self.part {
            RequestPart::Request => &request.request_msg,
            RequestPart::Response => &request.response_msg,
        };

        Some((&request.request_data, data))
    }

    fn get_decoder(
        &self,
        ctx: &UiContext,
        request: &RequestData,
        message: &MessageData,
    ) -> Box<dyn Decoder>
    {
        ctx.runtime
            .decoders
            .get_decoders(request, message)
            .last()
            .expect("Raw decoder should always be present")
    }
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
            RequestPart::Request => ("Re[q]uest Data", &request.request_msg),
            RequestPart::Response => ("R[e]sponse Data", &request.response_msg),
        };
        let title = format!("{} (offset {})", title, self.offset);

        let duration = match (data.start_timestamp, data.end_timestamp) {
            (Some(start), Some(end)) => format!(", {}", format_duration(end - start)),
            _ => String::new(),
        };

        let request_title = format!("{} ({} bytes{})", title, data.content.len(), duration);
        let block = create_block(&request_title);

        let (request, message) = match self.get_message(ctx) {
            Some(t) => t,
            None => return,
        };
        let decoder = self.get_decoder(ctx, request, message);
        let text = decoder.decode(message);

        let request_data = Paragraph::new(text)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((self.offset, 0));
        f.render_widget(request_data, chunk);
    }

    fn on_input(&mut self, ctx: &UiContext, e: &CTEvent, size: Rect) -> Option<HandleResult<B>>
    {
        match e {
            CTEvent::Key(key) => match key.code {
                KeyCode::Char('k') | KeyCode::Up => self.offset = self.offset.saturating_sub(1),
                KeyCode::Char('j') | KeyCode::Down => self.offset = self.offset.saturating_add(1),
                KeyCode::PageDown => self.offset = self.offset.saturating_add(size.height - 5),
                KeyCode::PageUp => self.offset = self.offset.saturating_sub(size.height - 5),
                KeyCode::F(12) => {
                    self.export(ctx);
                    return None;
                }
                KeyCode::Char('q') => match self.part {
                    RequestPart::Request => return Some(HandleResult::ExitView),
                    RequestPart::Response => self.part = RequestPart::Request,
                },
                KeyCode::Char('e') => match self.part {
                    RequestPart::Request => self.part = RequestPart::Response,
                    RequestPart::Response => return Some(HandleResult::ExitView),
                },
                KeyCode::Tab => {
                    self.part = match self.part {
                        RequestPart::Request => RequestPart::Response,
                        RequestPart::Response => RequestPart::Request,
                    }
                }
                _ => return None,
            },
            _ => return None,
        };
        Some(HandleResult::Update)
    }

    fn on_change(&mut self, _ctx: &UiContext, change: &SessionChange) -> bool
    {
        match change {
            SessionChange::NewConnection { .. } => false,
            SessionChange::Connection { .. } => false,
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
        format!(
            "{}\n{}",
            "[Up/Down, j/k, PgUp/PgDn]: Scroll; [Tab]: Switch Request/Response; [F12]: Export to file",
            "[q/e]: Toggle request/response, [Esc]: Back to main view"
        )
    }
}
