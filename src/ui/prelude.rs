pub use crossterm::event::{Event as CTEvent, KeyCode};
pub use tui::backend::Backend;
pub use tui::layout::{Constraint, Direction, Layout, Rect};
pub use tui::terminal::Frame;
pub use tui::widgets::{Block, BorderType, Borders, Text};

pub use crate::session::events::SessionChange;
pub use crate::ui::state::{HandleResult, UiContext};

use chrono::Duration;
use tui::style::{Modifier, Style};

pub fn create_block(title: &str) -> Block
{
    Block::default().title(title).borders(Borders::ALL)
}

pub fn create_control_block(title: &str, is_active: bool) -> Block
{
    let b = Block::default().title(title).borders(Borders::ALL);
    match is_active {
        true => b
            .border_type(BorderType::Double)
            .border_style(Style::default().modifier(Modifier::BOLD)),
        false => b,
    }
}

pub fn format_duration(d: Duration) -> String
{
    match d {
        t if t > Duration::seconds(10) => format!("{} s", t.num_seconds()),
        t => format!("{} ms", t.num_milliseconds()),
    }
}
