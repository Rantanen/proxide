use chrono::Duration;
use tui::widgets::{Block, Borders};

pub fn create_block(title: &str, active: bool) -> Block
{
    let mut block = Block::default().title(title).borders(Borders::ALL);
    if active {
        block = block.border_type(tui::widgets::BorderType::Thick);
    }
    block
}

pub fn format_duration(d: Duration) -> String
{
    match d {
        t if t > Duration::seconds(10) => format!("{} s", t.num_seconds()),
        t => format!("{} ms", t.num_milliseconds()),
    }
}
