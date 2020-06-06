use crossterm::event::KeyCode;

use super::views::prelude::*;

pub struct ChordState
{
    state: String,
}

pub enum ChordResult<'a>
{
    State(&'a str),
    Cancel,
    Ignore,
}

impl ChordState
{
    pub fn new(c: char) -> Self
    {
        Self {
            state: c.to_string(),
        }
    }

    pub fn handle(&mut self, e: CTEvent) -> ChordResult
    {
        if let CTEvent::Key(key) = e {
            match key.code {
                KeyCode::Char(c) => {
                    self.state.push(c);
                    ChordResult::State(&self.state)
                }
                KeyCode::Esc => ChordResult::Cancel,
                _ => ChordResult::Ignore,
            }
        } else {
            ChordResult::Ignore
        }
    }
}
