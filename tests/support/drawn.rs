//! Driving a Client's Application from its keys and reading what it draws, for
//! a suite that follows something through to the screen.

use crossterm::event::{Event as InputEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Frame, Terminal, backend::TestBackend};
use suru::tui::{Application, ApplicationTransition};

pub fn rendered_rows_at(
    width: u16,
    height: u16,
    render: impl FnOnce(&mut Frame<'_>),
) -> Vec<String> {
    let mut terminal =
        Terminal::new(TestBackend::new(width, height)).expect("create test terminal");
    terminal
        .draw(render)
        .expect("render headless TUI application");
    let buffer = terminal.backend().buffer();
    buffer
        .content()
        .chunks(buffer.area.width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect()
}

pub fn rendered_application_rows_at(
    application: &Application,
    width: u16,
    height: u16,
) -> Vec<String> {
    rendered_rows_at(width, height, |frame| application.render(frame))
}

pub fn type_terminal_text(application: &mut Application, text: &str) {
    for character in text.chars() {
        assert_eq!(
            application
                .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                    KeyCode::Char(character),
                    KeyModifiers::NONE,
                )))
                .expect("type terminal text"),
            ApplicationTransition::Continue
        );
    }
}
