//! Reading keys yourself instead of letting the terminal echo them.
//! Run with: cargo run --example raw_input    (Esc to quit)

use std::io::{Write, stdout};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{self, Clear, ClearType};
use crossterm::{cursor, execute};

fn main() -> std::io::Result<()> {
    // Raw mode: no echo, no waiting for Enter, Ctrl+C isn't special anymore.
    terminal::enable_raw_mode()?;

    let mut typed = String::new();
    loop {
        // Redraw the "input box": go to column 0, wipe the line, print our text.
        execute!(stdout(), cursor::MoveToColumn(0), Clear(ClearType::CurrentLine))?;
        print!("> {typed}");
        stdout().flush()?;

        // Blocks until something happens (key, resize, mouse...).
        if let Event::Key(key) = event::read()? {
            // Windows sends both a Press and a Release event for every key.
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Esc => break,
                KeyCode::Backspace => {
                    typed.pop();
                }
                KeyCode::Char(c) => typed.push(c),
                _ => {}
            }
        }
    }

    // Always undo raw mode, or the terminal stays broken after you exit.
    terminal::disable_raw_mode()?;
    // In raw mode "\n" only moves down, not back to column 0, so use "\r\n" there.
    println!("\nyou typed: {typed}");
    Ok(())
}
