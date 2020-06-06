use tui::style::{Color, Style};

pub fn row_style(
    control_active: bool,
    filtered: bool,
    current_group: bool,
    current_filter: bool,
) -> Style
{
    Style::default().fg(
        match (control_active, current_group, filtered, current_filter) {
            (_, _, false, true) => Color::Rgb(0x44, 0x88, 0x44),
            (_, _, true, true) => Color::LightGreen,
            (false, _, false, _) => Color::DarkGray,
            (false, _, true, _) => Color::Gray,
            (true, true, false, _) => Color::Yellow,
            (true, true, true, _) => Color::LightYellow,
            (true, false, false, _) => Color::DarkGray,
            (true, false, true, _) => Color::Gray,
        },
    )
}
