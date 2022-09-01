use crossterm::event::{Event as CTEvent, KeyCode, KeyModifiers};
use tui::backend::Backend;

use crate::ui::state::HandleResult;
use crate::ui::state::UiContext;

mod colon_command;
pub use colon_command::ColonCommand;

pub struct CommandState<B: Backend>
{
    pub help: String,
    pub prompt: String,
    pub input: String,
    pub text_cursor: usize,
    pub display_cursor: u16,
    pub executable: Box<dyn Executable<B>>,
}

impl<B: Backend> CommandState<B>
{
    pub fn cursor(&self) -> u16
    {
        self.display_cursor
    }

    pub fn on_input(&mut self, ctx: &mut UiContext, e: &CTEvent) -> Option<HandleResult<B>>
    {
        match e {
            CTEvent::Key(key) => match key.code {
                KeyCode::Esc => return Some(HandleResult::ExitCommand(None)),
                KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                    return Some(HandleResult::ExitCommand(None))
                }
                _ if key.modifiers == KeyModifiers::CONTROL => return None,
                KeyCode::Char(c) => self.insert(c),
                KeyCode::Enter => {
                    let result = self.executable.execute(&self.input, ctx);
                    return Some(HandleResult::ExitCommand(result.map(Box::new)));
                }
                KeyCode::Left => self.move_cursor(-1),
                KeyCode::Right => self.move_cursor(1),
                KeyCode::Backspace => self.remove_cursor(-1),
                KeyCode::Delete => self.remove_cursor(0),
                _ => return None,
            },
            _ => return None,
        }
        Some(HandleResult::Update)
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

pub trait Executable<B: Backend>
{
    fn execute(&self, cmd: &str, ctx: &mut UiContext) -> Option<HandleResult<B>>;
}

/*
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
*/

pub fn export_session<B: Backend>(ctx: &UiContext) -> Option<HandleResult<B>>
{
    colon_command::export_session(ctx, &Default::default())
}
