use crossterm::{
    event, execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use snafu::{ResultExt, Snafu};
use std::thread;
use std::{
    io::{stdout, Write},
    sync::mpsc,
};
use tui::{backend::CrosstermBackend, Terminal};

use crate::decoders::Decoders;
use crate::session::events::SessionEvent;

mod commands;
mod controls;
mod menus;
mod prelude;
mod state;
mod toast;
mod views;

use state::{HandleResult, ProxideUi, UiEvent};

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error
{
    #[snafu(display("IO error: {}", source))]
    IoError
    {
        source: std::io::Error
    },

    #[snafu(display("Terminal error: {}", source))]
    TermError
    {
        source: crossterm::ErrorKind
    },
}

pub type Result<S, E = Error> = std::result::Result<S, E>;

pub fn main(
    session: crate::session::Session,
    decoders: Decoders,
    session_rx: mpsc::Receiver<SessionEvent>,
) -> Result<()>
{
    enable_raw_mode().context(TermError {})?;
    execute!(stdout(), EnterAlternateScreen).context(TermError {})?;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend).context(IoError {})?;
    terminal.hide_cursor().context(IoError {})?;

    let (ui_tx, ui_rx) = std::sync::mpsc::channel();

    let mut state = ProxideUi::new(session, ui_tx.clone(), decoders, terminal.size().unwrap());

    let toast_tx = ui_tx.clone();
    thread::spawn(move || {
        loop {
            // If the send fails, the UI has stopped so we can exit the thread.
            if toast_tx.send(UiEvent::Toast(toast::recv())).is_err() {
                break;
            }
        }
    });

    let crossterm_tx = ui_tx.clone();
    thread::spawn(move || {
        while let Ok(e) = event::read() {
            // If the send fails, the UI has stopped so we can exit the thread.
            if crossterm_tx.send(UiEvent::Crossterm(e)).is_err() {
                break;
            }
        }
    });

    thread::spawn(move || {
        while let Ok(e) = session_rx.recv() {
            // If the send fails, the UI has stopped so we can exit the thread.
            if ui_tx.send(UiEvent::SessionEvent(Box::new(e))).is_err() {
                break;
            }
        }
    });

    // Ensure the UI is drawn at least once even if no events come in.
    state.draw(&mut terminal).context(IoError {})?;
    loop {
        let e = ui_rx.recv().unwrap();
        match state.handle(e) {
            HandleResult::PushView(..) => unreachable!("PushView is handled by the state"),
            HandleResult::ExitView => unreachable!("ExitView is handled by the state"),
            HandleResult::OpenMenu(..) => unreachable!("PushMenu is handled by the state"),
            HandleResult::ExitMenu => unreachable!("ExitMenu is handled by the state"),
            HandleResult::ExitCommand => unreachable!("ExitCommand is handled by the state"),
            HandleResult::Ignore => {}
            HandleResult::Update => state.draw(&mut terminal).context(IoError {})?,
            HandleResult::Quit => break,
        }
    }

    disable_raw_mode().context(TermError {})?;
    execute!(stdout(), LeaveAlternateScreen).context(TermError {})?;

    Ok(())
}
