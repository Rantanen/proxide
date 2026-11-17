use super::prelude::*;
use crossterm::event::KeyCode;
use http::HeaderValue;
use std::convert::TryFrom;
use tui::widgets::{Paragraph, Wrap};
use uuid::Uuid;

use crate::session::MessageData;

/// When available, identifies the thread in the calling or client process.
/// The client should reports its process id with the proxide-client-process-id" header and
/// the thread id with the "proxide-client-thread-id" header.
/// This enables the proxide proxy to capture client's callstack when it is making the call if the proxide
/// and the client are running on the same host.
pub struct ClientThreadId
{
    process_id: u32,
    thread_id: i64,
}

pub struct CallstackView
{
    pub request: Uuid,
    pub offset: u16,
}

impl CallstackView {}

impl<B: Backend> View<B> for CallstackView
{
    fn draw(&mut self, ctx: &UiContext, f: &mut Frame<B>, chunk: Rect)
    {
        let request = match ctx.data.requests.get_by_uuid(self.request) {
            Some(r) => r,
            None => return,
        };

        let client_thread = match ClientThreadId::try_from(&request.request_msg) {
            Ok(thread_id) => thread_id,
            Err(_) => return,
        };

        let title = format!(
            "Client call[s]tack, Process: {}, Thread: {}",
            client_thread.process_id, client_thread.thread_id
        );
        let block = create_block(&title);
        let request_data = Paragraph::new("Unimplemented.")
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((self.offset, 0));
        f.render_widget(request_data, chunk);
    }

    fn on_input(&mut self, _ctx: &UiContext, e: &CTEvent, size: Rect) -> Option<HandleResult<B>>
    {
        match e {
            CTEvent::Key(key) => match key.code {
                KeyCode::Char('k') | KeyCode::Up => self.offset = self.offset.saturating_sub(1),
                KeyCode::Char('j') | KeyCode::Down => self.offset = self.offset.saturating_add(1),
                KeyCode::PageDown => self.offset = self.offset.saturating_add(size.height - 5),
                KeyCode::PageUp => self.offset = self.offset.saturating_sub(size.height - 5),
                KeyCode::F(12) => {
                    return None;
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
            SessionChange::NewMessage { .. } => false,
            SessionChange::Message { .. } => false,
        }
    }

    fn help_text(&self, _state: &UiContext, _size: Rect) -> String
    {
        format!(
            "{}\n{}",
            "[Up/Down, j/k, PgUp/PgDn]: Scroll; [F12]: Export to file", "[Esc]: Back to main view"
        )
    }
}

impl TryFrom<&MessageData> for ClientThreadId
{
    type Error = ();

    fn try_from(value: &MessageData) -> Result<Self, Self::Error>
    {
        let process_id: Option<u32> =
            number_or_none(&value.headers.get("proxide-client-process-id"));
        let thread_id: Option<i64> = number_or_none(&value.headers.get("proxide-client-thread-id"));
        match (process_id, thread_id) {
            (Some(process_id), Some(thread_id)) => Ok(ClientThreadId {
                process_id,
                thread_id,
            }),
            _ => Err(()),
        }
    }
}

fn number_or_none<N>(header: &Option<&HeaderValue>) -> Option<N>
where
    N: std::str::FromStr,
{
    if let Some(value) = header {
        value
            .to_str()
            .map(|s| N::from_str(s).map(|n| Some(n)).unwrap_or(None))
            .unwrap_or(None)
    } else {
        None
    }
}
