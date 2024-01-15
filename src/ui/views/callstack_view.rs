use super::prelude::*;
use crate::session::ClientCallstack;
use crossterm::event::KeyCode;
use std::convert::TryFrom;
use tui::widgets::{Paragraph, Wrap};
use uuid::Uuid;

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

        let client_thread = match crate::connection::ClientThreadId::try_from(&request.request_msg)
        {
            Ok(thread_id) => thread_id,
            Err(_) => return,
        };

        let title = format!(
            "Client call[s]tack, Process: {}, Thread: {}",
            client_thread.process_id(),
            client_thread.thread_id()
        );
        let message: String = match &request.request_data.client_callstack {
            Some(ClientCallstack::Unsupported) => {
                "Callstack unavailable:\n* Unsupported operating system.".to_string()
            },
            Some(ClientCallstack::Throttled) => {
                "Callstack unavailable:\n* The maximum number of parallel callstack capture operations was reached.".to_string()
            },
            Some(ClientCallstack::Callstack( thread)) => message_from_thread( thread ),
            Some(ClientCallstack::Error(error)) => {
                format!("{:?}", error)
            },
            None => ".. (Pending)".to_string(),
        };
        let block = create_block(&title);
        let request_data = Paragraph::new(message)
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
            SessionChange::Callstack { request } => *request == self.request,
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

fn message_from_thread(thread: &crate::session::callstack::Thread) -> String
{
    let title = format!("{} ({})", thread.name(), thread.id());
    let callstack = thread
        .frames()
        .iter()
        .flat_map(|f| f.symbols())
        .map(|s| s.name())
        .fold(String::default(), |mut acc, name| {
            acc.push_str(name);
            acc.push('\n');
            acc
        });
    format!("{}\n\n{}", title, callstack)
}
