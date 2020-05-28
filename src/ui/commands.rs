use chrono::prelude::*;
use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyModifiers};
use tui::backend::Backend;

use crate::session;
use crate::ui::state::HandleResult;
use crate::ui::state::UiContext;
use crate::ui::toast;

pub struct CommandState
{
    pub help: String,
    pub prompt: String,
    pub input: String,
    pub text_cursor: usize,
    pub display_cursor: u16,
    pub executable: Box<dyn Executable>,
}

impl CommandState
{
    pub fn cursor(&self) -> u16
    {
        self.display_cursor
    }

    pub fn on_input<B: Backend>(
        &mut self,
        ctx: &mut UiContext,
        e: CrosstermEvent,
    ) -> HandleResult<B>
    {
        match e {
            CrosstermEvent::Key(key) => match key.code {
                KeyCode::Esc => return HandleResult::ExitCommand,
                KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                    return HandleResult::ExitCommand
                }
                _ if key.modifiers == KeyModifiers::CONTROL => return HandleResult::Ignore,
                KeyCode::Char(c) => self.insert(c),
                KeyCode::Enter => {
                    self.executable.execute(&self.input, ctx);
                    return HandleResult::ExitCommand;
                }
                KeyCode::Left => self.move_cursor(-1),
                KeyCode::Right => self.move_cursor(1),
                KeyCode::Backspace => self.remove_cursor(-1),
                KeyCode::Delete => self.remove_cursor(0),
                _ => return HandleResult::Ignore,
            },
            _ => return HandleResult::Ignore,
        }
        HandleResult::Update
    }

    fn insert(&mut self, c: char)
    {
        self.input.insert(self.text_cursor, c);
        self.text_cursor += c.len_utf8();
        self.display_cursor += 1;
    }

    fn move_cursor(&mut self, mut step: i16)
    {
        let signum = step.signum();
        while step != 0 {
            if signum < 0 && self.text_cursor == 0
                || signum > 0 && self.text_cursor == self.input.len()
            {
                return;
            }

            if signum < 0 {
                self.text_cursor -= 1;
                while !self.input.is_char_boundary(self.text_cursor) {
                    self.text_cursor -= 1;
                }
                self.display_cursor -= 1;
            } else {
                self.text_cursor += 1;
                while !self.input.is_char_boundary(self.text_cursor) {
                    self.text_cursor += 1;
                }
                self.display_cursor += 1;
            }
            step -= signum;
        }
    }

    fn remove_cursor(&mut self, offset: i16)
    {
        self.move_cursor(offset);
        self.input.remove(self.text_cursor);
    }
}

pub trait Executable
{
    fn execute(&self, cmd: &str, ctx: &mut UiContext);
}

pub struct SearchCommand;
impl Executable for SearchCommand
{
    fn execute(&self, cmd: &str, ctx: &mut UiContext)
    {
        ctx.data
            .requests
            .add_filter(Box::new(session::filters::SearchFilter {
                pattern: cmd.to_string(),
                index: ctx.runtime.search_index.clone(),
            }))
    }
}

pub struct ColonCommand;
impl Executable for ColonCommand
{
    fn execute(&self, cmd: &str, ctx: &mut UiContext)
    {
        match cmd {
            "export" => export_session(ctx),
            "clear" => clear_session(ctx),
            _ => toast::show_error(format!("Unknown command: {}", cmd)),
        }
    }
}

pub fn clear_session(ctx: &mut UiContext)
{
    ctx.data.requests = Default::default();
    ctx.data.connections = Default::default();
}

pub fn export_session(ctx: &UiContext)
{
    let filename = format!("session-{}.txt", Local::now().format("%H_%M_%S"));
    match ctx.data.write_to_file(&filename) {
        Ok(_) => toast::show_message(format!("Exported session to '{}'", filename)),
        Err(e) => toast::show_error(e.to_string()),
    }
}
