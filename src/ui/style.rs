use tui::style::{Color, Style};

pub fn request_row_style(
    control_active: bool,
    filtered: bool,
    current_group: bool,
    current_filter: bool,
) -> Style
{
    Style::default().fg(
        match (control_active, current_group, filtered, current_filter) {
            (false, _, false, true) => Color::Rgb(0x44, 0x88, 0x44),
            (false, _, true, true) => Color::Rgb(0x77, 0xee, 0x77),
            (true, true, false, _) => Color::Yellow,
            (true, true, true, _) => Color::LightYellow,
            (_, _, false, _) => Color::DarkGray,
            (_, _, true, _) => Color::Gray,
        },
    )
}

pub fn filter_row_style(control_active: bool, enabled: bool, matches_request: bool) -> Style
{
    Style::default().fg(match (control_active, enabled, matches_request) {
        (false, true, true) => Color::Rgb(0x77, 0xee, 0x77),
        (false, false, true) => Color::Rgb(0x44, 0x88, 0x44),
        (_, true, _) => Color::Gray,
        (_, false, _) => Color::DarkGray,
    })
}
