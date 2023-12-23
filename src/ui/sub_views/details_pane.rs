use std::convert::TryFrom;
use tui::layout::{Constraint, Direction, Layout};
use tui::text::{Span, Spans, Text};
use tui::widgets::Paragraph;
use uuid::Uuid;

use crate::ui::prelude::*;

use crate::session::{EncodedRequest, RequestPart};
use crate::ui::views::{CallstackView, ClientThreadId, MessageView};

#[derive(Clone, Default)]
pub struct DetailsPane;
impl DetailsPane
{
    pub fn on_input<B: Backend>(
        &mut self,
        req: &EncodedRequest,
        e: &CTEvent,
    ) -> Option<HandleResult<B>>
    {
        if let CTEvent::Key(key) = e {
            match key.code {
                KeyCode::Char('q') => self.create_message_view(req, RequestPart::Request),
                KeyCode::Char('e') => self.create_message_view(req, RequestPart::Response),
                KeyCode::Char('s') => self.create_callstack_view(req),
                _ => None,
            }
        } else {
            None
        }
    }

    pub fn draw_control<B: Backend>(
        &mut self,
        request: Uuid,
        ctx: &UiContext,
        f: &mut Frame<B>,
        chunk: Rect,
    )
    {
        let request = match ctx.data.requests.get_by_uuid(request) {
            Some(r) => r,
            None => return,
        };
        let conn = match ctx
            .data
            .connections
            .get_by_uuid(request.request_data.connection_uuid)
        {
            Some(r) => r,
            None => return,
        };

        let block = create_block("Details");

        let details_chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(0)
            .constraints([Constraint::Length(6), Constraint::Percentage(50)].as_ref())
            .split(block.inner(chunk));
        let mut c = details_chunks[1];
        c.x -= 1;
        c.width += 2;
        c.height += 1;
        let vertical_chunks: Vec<Rect> = if ClientThreadId::try_from(&request.request_msg).is_ok() {
            Layout::default()
                .direction(Direction::Vertical)
                .margin(0)
                .constraints([Constraint::Percentage(80), Constraint::Percentage(20)].as_ref())
                .split(block.inner(c))
        } else {
            Vec::from([block.inner(c)])
        };
        let req_resp_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .margin(0)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .split(vertical_chunks[0]);

        f.render_widget(block, chunk);

        let duration = match request.request_data.end_timestamp {
            None => "(Pending)".to_string(),
            Some(end) => format_duration(end - request.request_data.start_timestamp),
        };

        let spans = vec![
            Span::raw("\n"),
            Span::raw(format!(
                " Request:    {} {}\n",
                request.request_data.method, request.request_data.uri
            )),
            Span::raw(format!(
                " Protocol:   {}\n",
                conn.protocol_stack
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" -> ")
            )),
            Span::raw(format!(
                " Timestamp:  {}\n",
                request.request_data.start_timestamp
            )),
            Span::raw(format!(
                " Status:     {} (in {})\n",
                request.request_data.status, duration
            )),
        ];
        let details = Paragraph::new(Text::from(
            spans.into_iter().map(Spans::from).collect::<Vec<_>>(),
        ));
        f.render_widget(details, details_chunks[0]);

        MessageView {
            request: request.request_data.uuid,
            part: RequestPart::Request,
            offset: 0,
        }
        .draw(ctx, f, req_resp_chunks[0]);
        MessageView {
            request: request.request_data.uuid,
            part: RequestPart::Response,
            offset: 0,
        }
        .draw(ctx, f, req_resp_chunks[1]);

        // The right side view is split vertically only if the client included its process id and thread id in the request
        // enabling the callstack capture.
        if vertical_chunks.len() > 1 {
            CallstackView {
                request: request.request_data.uuid,
                offset: 0,
            }
            .draw(ctx, f, vertical_chunks[1]);
        }
    }

    fn create_message_view<B: Backend>(
        &mut self,
        req: &EncodedRequest,
        part: RequestPart,
    ) -> Option<HandleResult<B>>
    {
        Some(HandleResult::PushView(Box::new(MessageView {
            request: req.request_data.uuid,
            part,
            offset: 0,
        })))
    }

    fn create_callstack_view<B: Backend>(&mut self, req: &EncodedRequest)
        -> Option<HandleResult<B>>
    {
        if ClientThreadId::try_from(&req.request_msg).is_ok() {
            Some(HandleResult::PushView(Box::new(CallstackView {
                request: req.request_data.uuid,
                offset: 0,
            })))
        } else {
            None
        }
    }
}
