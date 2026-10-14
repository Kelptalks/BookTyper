// Release builds are a plain Windows app with no console window behind them.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod epub;
mod library;
mod marble;
mod progress;
mod theme;
mod typing;

fn main() -> eframe::Result {
    // Open a book straight from the command line (cargo run -- book.epub),
    // otherwise start on the library screen.
    let path = std::env::args().nth(1).map(std::path::PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("BookTyper")
            .with_inner_size([1100.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native("BookTyper", options, Box::new(|cc| Ok(Box::new(app::App::new(cc, path)))))
}
