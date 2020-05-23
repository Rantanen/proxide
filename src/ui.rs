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

use crate::decoders::DecoderFactory;
use crate::session::events::SessionEvent;

mod controls;
pub mod prelude;
mod state;
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
    decoders: Vec<Box<dyn DecoderFactory>>,
    session_rx: mpsc::Receiver<SessionEvent>,
) -> Result<()>
{
    enable_raw_mode().context(TermError {})?;
    execute!(stdout(), EnterAlternateScreen).context(TermError {})?;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend).context(IoError {})?;
    terminal.hide_cursor().context(IoError {})?;

    let mut state = ProxideUi::new(session, decoders, terminal.size().unwrap());

    terminal.draw(|f| state.draw(f)).unwrap();

    let (ui_tx, ui_rx) = std::sync::mpsc::channel();

    let crossterm_tx = ui_tx.clone();
    thread::spawn(move || loop {
        let e = event::read().unwrap();
        crossterm_tx.send(UiEvent::Crossterm(e)).unwrap();
    });

    thread::spawn(move || {
        while let Ok(e) = session_rx.recv() {
            ui_tx.send(UiEvent::SessionEvent(e)).unwrap();
        }
    });

    loop {
        let e = ui_rx.recv().unwrap();
        match state.handle(e) {
            HandleResult::PushView(..) => unreachable!("PushView is handled by the state"),
            HandleResult::Ignore => {}
            HandleResult::Update => terminal.draw(|f| state.draw(f)).unwrap(),
            HandleResult::Quit => break,
        }
    }

    disable_raw_mode().context(TermError {})?;
    execute!(stdout(), LeaveAlternateScreen).context(TermError {})?;

    Ok(())
}
